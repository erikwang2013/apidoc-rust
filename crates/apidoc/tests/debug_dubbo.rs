#![cfg(feature = "debug-dubbo")]
//! P2 Hessian2 黄金向量与 Dubbo 帧：本地 TCP 回环、参数编码、SSRF 默认拒绝。

use apidoc::debug::hessian;
use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::{DebugConfig, DocEndpoint, DocParam};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

fn param(name: &'static str, ty: &'static str) -> DocParam {
    DocParam {
        name,
        ty,
        required: true,
        default: None,
        desc: None,
        mock: None,
        field: None,
        children: &[],
    }
}

fn req(kind: &str, target: String, opts: &[(&str, &str)], body: &str) -> DebugRequest {
    DebugRequest {
        kind: kind.into(),
        target,
        method: None,
        headers: BTreeMap::new(),
        body: Some(Payload {
            encoding: "utf8".into(),
            data: body.into(),
        }),
        opts: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        endpoint: None,
        timeout_ms: Some(3000),
    }
}

fn opts() -> Vec<(&'static str, &'static str)> {
    vec![
        ("service", "com.example.DemoService"),
        ("method", "sayHello"),
        ("version", "1.0.0"),
        ("param_types", "Ljava/lang/String;"),
    ]
}

/// 读 Dubbo 请求帧并逐字段断言，然后回一个 hessian2 成功响应。
async fn dubbo_server(expected_args: Vec<Value>) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = [0u8; 16];
        stream.read_exact(&mut head).await.unwrap();
        assert_eq!(&head[0..2], &[0xda, 0xbb], "dubbo magic");
        assert_eq!(head[2], 0xc2, "request | two-way | hessian2 serialization");
        assert_eq!(head[3], 0, "request status");
        let len = u32::from_be_bytes([head[12], head[13], head[14], head[15]]) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.unwrap();

        let mut input = &body[..];
        assert_eq!(hessian::decode(&mut input).unwrap(), json!("2.0.2"));
        assert_eq!(
            hessian::decode(&mut input).unwrap(),
            json!("com.example.DemoService")
        );
        assert_eq!(hessian::decode(&mut input).unwrap(), json!("1.0.0"));
        assert_eq!(hessian::decode(&mut input).unwrap(), json!("sayHello"));
        assert_eq!(
            hessian::decode(&mut input).unwrap(),
            json!("Ljava/lang/String;"),
            "parameter types desc is one joined string"
        );
        for arg in expected_args {
            assert_eq!(hessian::decode(&mut input).unwrap(), arg);
        }
        assert_eq!(
            hessian::decode(&mut input).unwrap(),
            json!({}),
            "empty attachments map"
        );
        assert!(input.is_empty(), "unexpected trailing request bytes");

        let mut resp = vec![0x01]; // RESPONSE_VALUE
        resp.extend_from_slice(&hessian::encode(&json!("hello world")).unwrap());
        let mut out = vec![0xda, 0xbb, 0x02, 20];
        out.extend_from_slice(&head[4..12]); // same request id
        out.extend_from_slice(&(resp.len() as u32).to_be_bytes());
        out.extend_from_slice(&resp);
        stream.write_all(&out).await.unwrap();
    });
    (addr, handle)
}

#[test]
fn hessian_golden_vectors() {
    assert_eq!(hessian::encode(&Value::Null).unwrap(), vec![b'N']);
    assert_eq!(hessian::encode(&json!(true)).unwrap(), vec![b'T']);
    assert_eq!(hessian::encode(&json!(false)).unwrap(), vec![b'F']);
    assert_eq!(hessian::encode(&json!(1)).unwrap(), vec![b'I', 0, 0, 0, 1]);
    assert_eq!(
        hessian::encode(&json!(1i64 << 40)).unwrap(),
        vec![b'L', 0, 0, 1, 0, 0, 0, 0, 0]
    );
    let mut double = vec![b'D'];
    double.extend_from_slice(&1.5f64.to_bits().to_be_bytes());
    assert_eq!(hessian::encode(&json!(1.5)).unwrap(), double);
    assert_eq!(
        hessian::encode(&json!("hello")).unwrap(),
        vec![b'S', 0, 5, b'h', b'e', b'l', b'l', b'o']
    );
    assert_eq!(
        hessian::encode(&json!([1, "x"])).unwrap(),
        vec![0x57, b'I', 0, 0, 0, 1, b'S', 0, 1, b'x', 0x5a]
    );
    assert_eq!(
        hessian::encode(&json!({"a": 1})).unwrap(),
        vec![b'H', b'S', 0, 1, b'a', b'I', 0, 0, 0, 1, b'Z']
    );
}

#[test]
fn hessian_roundtrip_and_compact_decode() {
    let value = json!({
        "s": "hi",
        "i": 42,
        "l": 5_000_000_000i64,
        "f": 1.5,
        "b": true,
        "n": null,
        "list": [1, "two", false],
        "map": {"k": "v"},
    });
    let encoded = hessian::encode(&value).unwrap();
    let mut input = &encoded[..];
    assert_eq!(hessian::decode(&mut input).unwrap(), value);
    assert!(input.is_empty());

    // 紧凑形式：Java 端常发，解码必须支持。
    assert_eq!(hessian::decode(&mut &[0x90u8][..]).unwrap(), json!(0));
    assert_eq!(hessian::decode(&mut &[0x80u8][..]).unwrap(), json!(-16));
    assert_eq!(hessian::decode(&mut &[0xbfu8][..]).unwrap(), json!(47));
    assert_eq!(hessian::decode(&mut &[0xc0u8, 0x00][..]).unwrap(), json!(-2048));
    assert_eq!(hessian::decode(&mut &[0xe0u8][..]).unwrap(), json!(0));
    assert_eq!(hessian::decode(&mut &[0xd8u8][..]).unwrap(), json!(-8));
    assert_eq!(
        hessian::decode(&mut &[0x05u8, b'h', b'e', b'l', b'l', b'o'][..]).unwrap(),
        json!("hello")
    );
    assert_eq!(
        hessian::decode(&mut &[0x7au8, 0x91, 0x92][..]).unwrap(),
        json!([1, 2])
    );

    // 紧凑 double：0x5b/0x5c 是 0.0/1.0，0x5d/0x5e/0x5f 分别带 1/2/4 字节负载。
    assert_eq!(hessian::decode(&mut &[0x5bu8][..]).unwrap(), json!(0.0));
    assert_eq!(hessian::decode(&mut &[0x5cu8][..]).unwrap(), json!(1.0));
    assert_eq!(
        hessian::decode(&mut &[0x5du8, 0xfe][..]).unwrap(),
        json!(-2.0)
    );
    assert_eq!(
        hessian::decode(&mut &[0x5eu8, 0xff, 0xfe][..]).unwrap(),
        json!(-2.0)
    );
    let mut float = vec![0x5fu8];
    float.extend_from_slice(&1.5f32.to_bits().to_be_bytes());
    assert_eq!(hessian::decode(&mut &float[..]).unwrap(), json!(1.5));
}

#[test]
fn hessian_class_definitions_are_rejected() {
    let err = hessian::decode(&mut &[b'C', 0x00][..]).unwrap_err();
    assert!(err.contains("class"), "{err}");
}

#[tokio::test]
async fn dubbo_object_body_roundtrip() {
    let (addr, server) = dubbo_server(vec![json!("world")]).await;
    let ep = DocEndpoint {
        protocol: "dubbo".into(),
        params: vec![param("name", "string")],
        ..Default::default()
    };
    let res = execute(
        &cfg(),
        &req(
            "dubbo",
            format!("tcp://{addr}"),
            &opts(),
            r#"{"name":"world"}"#,
        ),
        Some(&ep),
    )
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v, json!("hello world"));
    assert_eq!(res.headers.get("x-dubbo-status").map(String::as_str), Some("20"));
    server.await.unwrap();
}

#[tokio::test]
async fn hessian_kind_array_body_roundtrip() {
    let (addr, server) = dubbo_server(vec![json!("world")]).await;
    let res = execute(
        &cfg(),
        &req("hessian", format!("tcp://{addr}"), &opts(), r#"["world"]"#),
        None,
    )
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v, json!("hello world"));
    server.await.unwrap();
}

#[tokio::test]
async fn missing_param_types_is_reported() {
    let err = execute(
        &cfg(),
        &req(
            "dubbo",
            "tcp://127.0.0.1:9".into(),
            &[("service", "s"), ("method", "m")],
            r#"["x"]"#,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("param_types"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("dubbo", "tcp://127.0.0.1:9".into(), &opts(), r#"["x"]"#),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
