#![cfg(feature = "debug-industrial")]
//! P4 DNP3：黄金请求/响应向量经由本地 TCP 回环，验证链路帧与对象解析。

use apidoc::debug::{execute, DebugRequest};
use apidoc::DebugConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        timeout_ms: 2_000,
        ..Default::default()
    }
}

fn req(target: String, opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: "dnp3".into(),
        target,
        method: None,
        headers: BTreeMap::new(),
        body: None,
        opts: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        endpoint: None,
        timeout_ms: Some(2_000),
    }
}

fn hex_decode(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[tokio::test]
async fn integrity_poll_decodes_golden_response() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 15];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(
            request.to_vec(),
            hex_decode("05640bc4010000004de9c0c0013c0106ff50"),
            "integrity poll link frame"
        );
        stream
            .write_all(&hex_decode(
                "0564174400000100ec4ec0c0811e01000001012a00000001f9ff4c3cffff3a99",
            ))
            .await
            .unwrap();
    });

    let result = execute(
        &cfg(),
        &req(
            format!("dnp3://{addr}"),
            &[("operation", "integrity"), ("duration_ms", "500")],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    let points = value["points"].as_array().unwrap();
    assert_eq!(points.len(), 2);
    assert_eq!(points[0]["group"], 30);
    assert_eq!(points[0]["index"], 0);
    assert_eq!(points[0]["value"], 42);
    assert_eq!(points[0]["flags"], 1);
    assert_eq!(points[1]["index"], 1);
    assert_eq!(points[1]["value"], -7);
    handle.await.unwrap();
}

#[tokio::test]
async fn source_and_link_address_must_differ() {
    let err = execute(
        &cfg(),
        &req(
            "dnp3://127.0.0.1:20000".into(),
            &[("link_address", "7"), ("source_address", "7")],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("must differ"), "{err}");
}

#[tokio::test]
async fn unsupported_operation_is_rejected_before_connecting() {
    let err = execute(
        &cfg(),
        &req(
            "dnp3://127.0.0.1:20000".into(),
            &[("operation", "operate")],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("unsupported dnp3 operation"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("dnp3://127.0.0.1:20000".into(), &[]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
