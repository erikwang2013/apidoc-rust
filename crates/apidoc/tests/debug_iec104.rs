#![cfg(feature = "debug-industrial")]
//! P4 IEC 60870-5-104：STARTDT 握手、总召唤/单点读、ASDU 解码，本地 TCP 回环。

use apidoc::debug::{execute, DebugRequest};
use apidoc::DebugConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        timeout_ms: 2_000,
        ..Default::default()
    }
}

fn req(target: String, opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: "iec104".into(),
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

async fn read_apdu(stream: &mut TcpStream) -> Vec<u8> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x68, "iec104 start byte");
    let mut body = vec![0u8; header[1] as usize];
    stream.read_exact(&mut body).await.unwrap();
    body
}

fn i_frame(ns: u16, nr: u16, asdu: &[u8]) -> Vec<u8> {
    let mut out = vec![0x68, (4 + asdu.len()) as u8];
    out.extend_from_slice(&(ns << 1).to_le_bytes());
    out.extend_from_slice(&(nr << 1).to_le_bytes());
    out.extend_from_slice(asdu);
    out
}

async fn mock(expected_asdu: Vec<u8>, response_asdu: Vec<u8>) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert_eq!(read_apdu(&mut stream).await, vec![0x07, 0x00, 0x00, 0x00]);
        stream.write_all(&[0x68, 0x04, 0x0B, 0x00, 0x00, 0x00]).await.unwrap();

        let request = read_apdu(&mut stream).await;
        assert_eq!(&request[..4], &[0x00, 0x00, 0x00, 0x00], "I-frame SEQ=0");
        assert_eq!(&request[4..], expected_asdu.as_slice());
        stream.write_all(&i_frame(0, 1, &response_asdu)).await.unwrap();
    });
    (addr, handle)
}

#[tokio::test]
async fn general_interrogation_parses_monitoring_asdus() {
    let mut response_asdu = vec![13, 0x01, 0x14, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00];
    response_asdu.extend_from_slice(&10.0f32.to_le_bytes());
    response_asdu.push(0x00); // quality
    response_asdu.extend_from_slice(&[100, 0x01, 0x0A, 0x00, 0x01, 0x00, 0, 0, 0, 20]);

    let (addr, handle) = mock(
        vec![100, 0x01, 0x06, 0x00, 0x01, 0x00, 0, 0, 0, 20],
        response_asdu,
    )
    .await;
    let result = execute(
        &cfg(),
        &req(
            format!("iec104://{addr}"),
            &[("operation", "interrogation"), ("duration_ms", "500")],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    let rows = value["asdu"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["type_id"], 13);
    assert_eq!(rows[0]["cause"], 20);
    assert_eq!(rows[0]["ioa"], 1);
    assert_eq!(rows[0]["value"], 10.0);
    assert_eq!(rows[1]["type_id"], 100);
    assert_eq!(rows[1]["cause"], 10);
    handle.await.unwrap();
}

#[tokio::test]
async fn single_point_read_uses_ioa() {
    let (addr, handle) = mock(
        vec![102, 0x01, 0x05, 0x00, 0x01, 0x00, 0x05, 0x00, 0x00],
        vec![1, 0x01, 0x14, 0x00, 0x01, 0x00, 0x05, 0x00, 0x00, 0x01],
    )
    .await;
    let result = execute(
        &cfg(),
        &req(
            format!("iec104://{addr}"),
            &[
                ("operation", "read"),
                ("ioa", "5"),
                ("duration_ms", "500"),
            ],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    let rows = value["asdu"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["type_id"], 1);
    assert_eq!(rows[0]["ioa"], 5);
    assert_eq!(rows[0]["value"], true);
    handle.await.unwrap();
}

#[tokio::test]
async fn invalid_operations_are_rejected_before_connecting() {
    let err = execute(
        &cfg(),
        &req(
            "iec104://127.0.0.1:2404".into(),
            &[("operation", "control")],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("unsupported iec104 operation"), "{err}");

    let err = execute(
        &cfg(),
        &req(
            "iec104://127.0.0.1:2404".into(),
            &[("operation", "read"), ("ioa", "16777216")],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("24 bits"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("iec104://127.0.0.1:2404".into(), &[]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
