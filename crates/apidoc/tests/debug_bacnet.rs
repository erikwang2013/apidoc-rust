#![cfg(feature = "debug-industrial")]
//! P4 BACnet/IP：Who-Is/I-Am 与 ReadProperty 本地 UDP 回环，广播规则 fail-closed。

use apidoc::debug::{execute, DebugRequest};
use apidoc::DebugConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
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
        kind: "bacnet".into(),
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

fn bvlc(broadcast: bool, apdu: &[u8]) -> Vec<u8> {
    let mut out = vec![0x81, if broadcast { 0x0B } else { 0x0A }];
    out.extend_from_slice(&((4 + 2 + apdu.len()) as u16).to_be_bytes());
    out.extend_from_slice(&[0x01, 0x00]);
    out.extend_from_slice(apdu);
    out
}

async fn mock(expected: Vec<u8>, reply_apdu: Vec<u8>) -> (SocketAddr, JoinHandle<()>) {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], expected.as_slice());
        server
            .send_to(&bvlc(false, &reply_apdu), peer)
            .await
            .unwrap();
    });
    (addr, handle)
}

fn i_am(instance: u32) -> Vec<u8> {
    let object_id = (8 << 22) | instance;
    let mut apdu = vec![0x10, 0x00, 0xC4];
    apdu.extend_from_slice(&object_id.to_be_bytes());
    apdu.extend_from_slice(&[0x22, 0x01, 0x40, 0x91, 0x03, 0x21, 0x00]);
    apdu
}

fn read_property_ack(instance: u32, value: f32) -> Vec<u8> {
    let object_id = (8 << 22) | instance;
    let mut apdu = vec![0x30, 0x00, 0x0C, 0xC4];
    apdu.extend_from_slice(&object_id.to_be_bytes());
    apdu.extend_from_slice(&[0x19, 0x55, 0x3E, 0x44]);
    apdu.extend_from_slice(&value.to_be_bytes());
    apdu.push(0x3F);
    apdu
}

#[tokio::test]
async fn who_is_collects_i_am_devices() {
    let (addr, handle) = mock(
        vec![0x81, 0x0A, 0x00, 0x08, 0x01, 0x00, 0x10, 0x08],
        i_am(26_000),
    )
    .await;
    let result = execute(
        &cfg(),
        &req(
            format!("bacnet://{addr}"),
            &[("operation", "who_is"), ("duration_ms", "500")],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    let devices = value["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["instance"], 26_000);
    assert_eq!(devices[0]["address"], addr.to_string());
    handle.await.unwrap();
}

#[tokio::test]
async fn read_property_returns_present_value() {
    let object_id = (8 << 22) | 1u32;
    let mut expected = vec![0x81, 0x0A, 0x00, 0x11, 0x01, 0x00, 0x00, 0x00, 0x00, 0x0C, 0xC4];
    expected.extend_from_slice(&object_id.to_be_bytes());
    expected.extend_from_slice(&[0x19, 0x55]);
    let (addr, handle) = mock(expected, read_property_ack(1, 7.25)).await;
    let result = execute(
        &cfg(),
        &req(
            format!("bacnet://{addr}"),
            &[
                ("operation", "read_property"),
                ("object_instance", "1"),
                ("duration_ms", "500"),
            ],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["value"], 7.25);
    handle.await.unwrap();
}

#[tokio::test]
async fn broadcast_who_is_is_sent_with_bvlc_broadcast() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(buf[0], 0x81);
        assert_eq!(buf[1], 0x0B, "BVLC broadcast type");
        assert_eq!(&buf[4..n], &[0x01, 0x00, 0x10, 0x08]);
        server.send_to(&bvlc(false, &i_am(7)), peer).await.unwrap();
    });
    let result = execute(
        &cfg(),
        &req(
            format!("bacnet://{addr}"),
            &[("broadcast", "true"), ("duration_ms", "500")],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["devices"][0]["instance"], 7);
    handle.await.unwrap();
}

#[tokio::test]
async fn broadcast_and_special_targets_fail_closed() {
    let err = execute(
        &DebugConfig::default(),
        &req("bacnet://8.8.8.8:47808".into(), &[("broadcast", "true")]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("allow_private_networks"), "{err}");

    let err = execute(
        &cfg(),
        &req("bacnet://255.255.255.255:47808".into(), &[("broadcast", "true")]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("255.255.255.255"), "{err}");

    let err = execute(
        &cfg(),
        &req("bacnet://239.1.1.1:47808".into(), &[("broadcast", "true")]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("multicast"), "{err}");

    let err = execute(
        &cfg(),
        &req(
            "bacnet://127.0.0.1:47808".into(),
            &[("operation", "read_property"), ("object_instance", "1"), ("broadcast", "true")],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("only valid for who_is"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("bacnet://127.0.0.1:47808".into(), &[]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
