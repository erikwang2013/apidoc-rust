#![cfg(feature = "debug-dubbo")]
//! P6 Dubbo Nacos 注册中心：本地 HTTP mock 返回实例列表，本地 TCP mock 充当 provider。

use apidoc::debug::hessian;
use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::DebugConfig;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

fn opts(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut map: BTreeMap<String, String> = [
        ("service", "com.example.DemoService"),
        ("method", "sayHello"),
        ("param_types", "Ljava/lang/String;"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    for (k, v) in extra {
        map.insert(k.to_string(), v.to_string());
    }
    map
}

fn req(target: &str, opts: BTreeMap<String, String>) -> DebugRequest {
    DebugRequest {
        kind: "dubbo".into(),
        target: target.into(),
        method: None,
        headers: BTreeMap::new(),
        body: Some(Payload {
            encoding: "utf8".into(),
            data: r#"["world"]"#.into(),
        }),
        opts,
        endpoint: None,
        timeout_ms: Some(3000),
    }
}

/// 最小 Dubbo provider：读一帧请求，回 hessian2 字符串 "hi"。
async fn dubbo_provider() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = [0u8; 16];
        stream.read_exact(&mut head).await.unwrap();
        assert_eq!(&head[0..2], &[0xda, 0xbb], "dubbo magic");
        let len = u32::from_be_bytes([head[12], head[13], head[14], head[15]]) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.unwrap();
        let mut input = &body[..];
        for _ in 0..5 {
            // dubbo version / service / version / method / param_types
            hessian::decode(&mut input).unwrap();
        }
        hessian::decode(&mut input).unwrap(); // arg
        hessian::decode(&mut input).unwrap(); // empty attachments
        let mut resp = vec![0x01]; // RESPONSE_VALUE
        resp.extend_from_slice(&hessian::encode(&json!("hi")).unwrap());
        let mut out = vec![0xda, 0xbb, 0x02, 20];
        out.extend_from_slice(&head[4..12]); // same request id
        out.extend_from_slice(&(resp.len() as u32).to_be_bytes());
        out.extend_from_slice(&resp);
        stream.write_all(&out).await.unwrap();
    });
    (addr, task)
}

/// 单请求 HTTP mock：返回固定 JSON，交回请求行供断言。
async fn nacos_mock(body: &str) -> (SocketAddr, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&request).into_owned()
    });
    (addr, task)
}

#[tokio::test]
async fn registry_resolves_first_healthy_provider() {
    let (provider, provider_task) = dubbo_provider().await;
    let hosts = json!({"hosts": [
        {"ip": "127.0.0.1", "port": 1, "healthy": false},
        {"ip": "127.0.0.1", "port": provider.port(), "healthy": true},
    ]})
    .to_string();
    let (nacos, nacos_task) = nacos_mock(&hosts).await;
    // 小写 %3a：mock 断言出站查询被解码后重新编码为大写 %3A。
    let registry = format!("nacos://{nacos}?service=com.example%3aDemo&group=DEFAULT_GROUP");
    let res = execute(
        &cfg(),
        &req("", opts(&[("registry", &registry)])),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&res.body.data).unwrap(),
        json!("hi")
    );
    let expected = provider.to_string();
    assert_eq!(
        res.headers.get("x-dubbo-provider").map(String::as_str),
        Some(expected.as_str())
    );

    let request_line = nacos_task.await.unwrap();
    assert!(
        request_line.starts_with(
            "GET /nacos/v1/ns/instance/list?serviceName=com.example%3ADemo&groupName=DEFAULT_GROUP HTTP/1.1\r\n"
        ),
        "{request_line}"
    );
    assert!(
        !request_line.contains("namespaceId"),
        "empty namespace must be omitted: {request_line}"
    );
    provider_task.await.unwrap();
}

#[tokio::test]
async fn empty_hosts_is_an_error() {
    let (nacos, _nacos_task) = nacos_mock(r#"{"hosts":[]}"#).await;
    let registry = format!("nacos://{nacos}?service=com.example.DemoService");
    let err = execute(&cfg(), &req("", opts(&[("registry", &registry)])), None)
        .await
        .unwrap_err();
    assert!(err.contains("no healthy"), "{err}");
}

#[tokio::test]
async fn zookeeper_registry_is_rejected() {
    let err = execute(
        &cfg(),
        &req(
            "",
            opts(&[("registry", "zookeeper://127.0.0.1:2181")]),
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("only nacos://") && err.contains("zookeeper"), "{err}");
}

#[tokio::test]
async fn opts_target_wins_over_registry() {
    let (provider, provider_task) = dubbo_provider().await;
    // registry 一旦被访问就会连接失败（端口 1）；成功即证明直连生效。
    let registry = "nacos://127.0.0.1:1?service=unused";
    let res = execute(
        &cfg(),
        &req(
            "",
            opts(&[
                ("target", &provider.to_string()),
                ("registry", registry),
            ]),
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&res.body.data).unwrap(),
        json!("hi")
    );
    assert!(
        res.headers.get("x-dubbo-provider").is_none(),
        "direct target is not a registry selection"
    );
    provider_task.await.unwrap();
}

#[tokio::test]
async fn request_target_wins_over_registry() {
    let (provider, provider_task) = dubbo_provider().await;
    let res = execute(
        &cfg(),
        &req(
            &provider.to_string(),
            opts(&[("registry", "nacos://127.0.0.1:1?service=unused")]),
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&res.body.data).unwrap(),
        json!("hi")
    );
    provider_task.await.unwrap();
}

#[tokio::test]
async fn registry_host_is_guarded_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("", opts(&[("registry", "nacos://127.0.0.1:1?service=s")])),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
