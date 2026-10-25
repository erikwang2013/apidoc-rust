#![cfg(feature = "debug-http3")]
//! P5 HTTP/3：本地 quinn+h3 回环（测试 CA 注入）、https-only 校验与默认 SSRF 拒绝。

use apidoc::debug::http3;
use apidoc::debug::{execute, DebugRequest};
use apidoc::DebugConfig;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

fn req(target: String, method: &str, body: Option<&str>) -> DebugRequest {
    DebugRequest {
        kind: "http3".into(),
        target,
        method: Some(method.into()),
        headers: BTreeMap::new(),
        body: body.map(|s| apidoc::debug::Payload {
            encoding: "utf8".into(),
            data: s.into(),
        }),
        opts: BTreeMap::new(),
        endpoint: None,
        timeout_ms: Some(5000),
    }
}

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

/// 启动一个 h3 服务端，返回 (UDP 端口, 测试 CA 的 RootCertStore)。
async fn start_h3_server() -> (u16, rustls::RootCertStore) {
    let key = rcgen::KeyPair::generate().unwrap();
    let params = rcgen::CertificateParams::new(vec!["127.0.0.1".to_string()]).unwrap();
    let cert = params.self_signed(&key).unwrap();
    let cert_der = cert.der().clone();
    let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into();

    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert_der.clone()], key_der)
    .unwrap();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap();
    let server_cfg = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    let endpoint = quinn::Endpoint::server(server_cfg, "127.0.0.1:0".parse().unwrap()).unwrap();
    let port = endpoint.local_addr().unwrap().port();

    tokio::spawn(async move {
        // 自签名拒绝测试中客户端会在 TLS 阶段断开，这里安静退出而不是 panic。
        let Some(incoming) = endpoint.accept().await else {
            return;
        };
        let Ok(connection) = incoming.await else {
            return;
        };
        let Ok(mut h3_conn) =
            h3::server::Connection::new(h3_quinn::Connection::new(connection)).await
        else {
            return;
        };
        if let Ok(Some(resolver)) = h3_conn.accept().await {
            let (request, mut stream) = resolver.resolve_request().await.unwrap();
            assert_eq!(request.method(), http::Method::GET);
            let response = http::Response::builder()
                .status(200)
                .header("x-test", "h3")
                .body(())
                .unwrap();
            stream.send_response(response).await.unwrap();
            stream
                .send_data(Bytes::from_static(b"h3-pong"))
                .await
                .unwrap();
            stream.finish().await.unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der).unwrap();
    (port, roots)
}

#[tokio::test]
async fn h3_get_roundtrip_with_test_ca() {
    let (port, roots) = start_h3_server().await;
    let res = http3::execute_with_roots(
        &cfg(),
        &req(format!("https://127.0.0.1:{port}/hello"), "GET", None),
        Some(&roots),
    )
    .await
    .unwrap();
    assert_eq!(res.status, 200);
    assert_eq!(res.headers["x-test"], "h3");
    assert_eq!(res.body.data, "h3-pong");
}

#[tokio::test]
async fn self_signed_server_is_rejected_with_system_roots() {
    let (port, _roots) = start_h3_server().await;
    let err = http3::execute(
        &cfg(),
        &req(format!("https://127.0.0.1:{port}/hello"), "GET", None),
    )
    .await
    .unwrap_err();
    assert!(err.contains("http3 connect failed"), "{err}");
}

#[tokio::test]
async fn non_https_target_is_rejected() {
    let err = execute(
        &cfg(),
        &req("http://127.0.0.1:443/x".into(), "GET", None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("https://"), "{err}");
    let err = execute(
        &cfg(),
        &req("h3://127.0.0.1:443/x".into(), "GET", None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("https://") || err.contains("scheme"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("https://127.0.0.1:443/x".into(), "GET", None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
