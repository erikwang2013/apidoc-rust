#![cfg(feature = "debug-industrial")]
//! P4 Modbus TCP：本地回环从站，读四种寄存器区、单写、异常码与参数校验。

use apidoc::debug::{execute, DebugRequest};
use apidoc::DebugConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        timeout_ms: 2_000,
        ..Default::default()
    }
}

fn req(opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: "modbus".into(),
        target: "modbus://127.0.0.1:1502".into(),
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

async fn mock(expected_pdu: Vec<u8>, response_pdu: Vec<u8>) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = [0u8; 7];
        stream.read_exact(&mut header).await.unwrap();
        assert_eq!(&header[2..4], &[0, 0], "modbus protocol id");
        let length = u16::from_be_bytes([header[4], header[5]]) as usize;
        let mut pdu = vec![0u8; length - 1];
        stream.read_exact(&mut pdu).await.unwrap();
        assert_eq!(pdu, expected_pdu);

        let mut out = Vec::new();
        out.extend_from_slice(&header[0..2]); // same transaction id
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&((response_pdu.len() + 1) as u16).to_be_bytes());
        out.push(header[6]); // same unit id
        out.extend_from_slice(&response_pdu);
        stream.write_all(&out).await.unwrap();
    });
    (addr, handle)
}

async fn with_server(
    opts: &[(&str, &str)],
    expected_pdu: Vec<u8>,
    response_pdu: Vec<u8>,
) -> (Value, JoinHandle<()>) {
    let (addr, handle) = mock(expected_pdu, response_pdu).await;
    let mut request = req(opts);
    request.target = format!("modbus://{addr}");
    let result = execute(&cfg(), &request, None).await.unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    (value, handle)
}

#[tokio::test]
async fn reads_holding_registers() {
    let (value, handle) = with_server(
        &[("operation", "read_holding"), ("address", "0"), ("quantity", "2")],
        vec![0x03, 0x00, 0x00, 0x00, 0x02],
        vec![0x03, 0x04, 0x00, 0x64, 0x00, 0xC8],
    )
    .await;
    assert_eq!(value["values"], serde_json::json!([100, 200]));
    handle.await.unwrap();
}

#[tokio::test]
async fn reads_coils_as_bits() {
    let (value, handle) = with_server(
        &[("operation", "read_coils"), ("address", "0"), ("quantity", "2")],
        vec![0x01, 0x00, 0x00, 0x00, 0x02],
        vec![0x01, 0x01, 0x02],
    )
    .await;
    assert_eq!(value["values"], serde_json::json!([false, true]));
    handle.await.unwrap();
}

#[tokio::test]
async fn writes_single_coil_and_register() {
    let (_value, handle) = with_server(
        &[
            ("operation", "write_coil"),
            ("address", "5"),
            ("value", "true"),
        ],
        vec![0x05, 0x00, 0x05, 0xFF, 0x00],
        vec![0x05, 0x00, 0x05, 0xFF, 0x00],
    )
    .await;
    handle.await.unwrap();

    let (value, handle) = with_server(
        &[
            ("operation", "write_register"),
            ("address", "5"),
            ("value", "300"),
        ],
        vec![0x06, 0x00, 0x05, 0x01, 0x2C],
        vec![0x06, 0x00, 0x05, 0x01, 0x2C],
    )
    .await;
    assert_eq!(value["values"], serde_json::json!([]));
    handle.await.unwrap();
}

#[tokio::test]
async fn device_exception_is_reported() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = [0u8; 7];
        stream.read_exact(&mut header).await.unwrap();
        let length = u16::from_be_bytes([header[4], header[5]]) as usize;
        let mut pdu = vec![0u8; length - 1];
        stream.read_exact(&mut pdu).await.unwrap();

        let response_pdu = [0x83, 0x02]; // read holding | exception, illegal data address
        let mut out = Vec::new();
        out.extend_from_slice(&header[0..2]);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&((response_pdu.len() + 1) as u16).to_be_bytes());
        out.push(header[6]);
        out.extend_from_slice(&response_pdu);
        stream.write_all(&out).await.unwrap();
    });

    let mut request = req(&[("operation", "read_holding"), ("address", "0")]);
    request.target = format!("modbus://{addr}");
    let err = execute(&cfg(), &request, None).await.unwrap_err();
    assert!(err.contains("modbus exception"), "{err}");
    handle.await.unwrap();
}

#[tokio::test]
async fn invalid_quantity_is_rejected_before_connecting() {
    let err = execute(
        &cfg(),
        &req(&[
            ("operation", "read_holding"),
            ("address", "0"),
            ("quantity", "0"),
        ]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("quantity must be 1..=125"), "{err}");

    let err = execute(
        &cfg(),
        &req(&[
            ("operation", "read_coils"),
            ("address", "65535"),
            ("quantity", "2"),
        ]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("exceeds the 16-bit address space"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(&DebugConfig::default(), &req(&[("address", "0")]), None)
        .await
        .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
