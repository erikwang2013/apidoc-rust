#![cfg(feature = "debug-media")]
//! P5 RTP/RTCP：黄金向量、CSRC/扩展/padding 偏移、UDP 回环与 SSRF 默认拒绝。

use apidoc::debug::{execute, rtcp, rtp, DebugRequest, Payload};
use apidoc::DebugConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::net::UdpSocket;

fn req(kind: &str, target: String, body: Option<&str>, opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: kind.into(),
        target,
        method: None,
        headers: BTreeMap::new(),
        body: body.map(|s| Payload {
            encoding: "utf8".into(),
            data: s.into(),
        }),
        opts: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        endpoint: None,
        timeout_ms: Some(3000),
    }
}

fn allow_private() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

#[test]
fn rtp_header_roundtrip() {
    let bytes = rtp::encode_packet(96, 7, 12345, 0xdead_beef, b"hi");
    assert_eq!(
        &bytes[..12],
        &[0x80, 96, 0x00, 0x07, 0x00, 0x00, 0x30, 0x39, 0xde, 0xad, 0xbe, 0xef]
    );
    let packet = rtp::decode_packet(&bytes).unwrap();
    assert_eq!(packet.version, 2);
    assert_eq!(packet.payload_type, 96);
    assert_eq!(packet.sequence, 7);
    assert_eq!(packet.timestamp, 12345);
    assert_eq!(packet.ssrc, 0xdead_beef);
    assert_eq!(packet.payload, b"hi");
    assert!(!packet.marker && !packet.padding && !packet.extension);
}

#[test]
fn rtp_csrc_extension_and_padding_offsets() {
    // CC=2，同步源之后是两个 CSRC 再跟载荷。
    let mut bytes = rtp::encode_packet(96, 1, 0, 1, b"hi");
    bytes[0] |= 0x02;
    bytes.splice(12..12, [0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x0b]);
    let packet = rtp::decode_packet(&bytes).unwrap();
    assert_eq!(packet.csrc, vec![10, 11]);
    assert_eq!(packet.payload, b"hi");

    // X=1：4 字节扩展头 + 1 个 32bit 字的扩展数据。
    let mut bytes = rtp::encode_packet(96, 1, 0, 1, b"hi");
    bytes[0] |= 0x10;
    bytes.splice(12..12, [0xab, 0xac, 0x00, 0x01, 0xde, 0xad, 0xbe, 0xef]);
    let packet = rtp::decode_packet(&bytes).unwrap();
    assert!(packet.extension);
    assert_eq!(packet.payload, b"hi");

    // P=1：末尾 4 字节是 padding（计数含自身）。
    let mut bytes = rtp::encode_packet(96, 1, 0, 1, b"hi");
    bytes[0] |= 0x20;
    bytes.extend_from_slice(&[0, 0, 0, 4]);
    let packet = rtp::decode_packet(&bytes).unwrap();
    assert!(packet.padding);
    assert_eq!(packet.payload, b"hi");
}

#[test]
fn rtp_rejects_truncated_headers_and_bad_padding() {
    assert!(rtp::decode_packet(&[0x80, 96, 0, 1]).is_err());
    let mut bytes = rtp::encode_packet(96, 1, 0, 1, b"hi");
    bytes[0] |= 0x0f; // CC=15 但只有 2 字节载荷
    assert!(rtp::decode_packet(&bytes).is_err());
    let mut bytes = rtp::encode_packet(96, 1, 0, 1, b"hi");
    bytes[0] |= 0x20;
    bytes.push(9); // padding 计数大于载荷
    assert!(rtp::decode_packet(&bytes).is_err());
}

#[test]
fn rtcp_sr_rr_sdes_bye_vectors() {
    let rr = rtcp::encode_packet(201, 0x0102_0304, &[]);
    assert_eq!(rr, vec![0x80, 201, 0x00, 0x01, 1, 2, 3, 4]);
    let packet = rtcp::decode_packet(&rr).unwrap();
    assert_eq!(packet.version, 2);
    assert_eq!(packet.packet_type, 201);
    assert_eq!(packet.length, 8);
    assert_eq!(packet.ssrc, Some(0x0102_0304));

    let sr = rtcp::encode_packet(200, 7, &[]);
    assert_eq!(sr.len(), 8); // 头部 + SSRC（sender info 由调用方 body 提供）
    assert_eq!(rtcp::decode_packet(&sr).unwrap().packet_type, 200);
    // SR 的不透明载荷（sender info/report blocks）不会被 8 bit 文本长度截断。
    let big_sr = rtcp::encode_packet(200, 7, &[0x5a; 300]);
    assert_eq!(big_sr.len(), 308);
    assert_eq!(rtcp::decode_packet(&big_sr).unwrap().length, 308);

    let sdes = rtcp::encode_packet(202, 7, b"user@host");
    let packet = rtcp::decode_packet(&sdes).unwrap();
    assert_eq!(packet.count, 1);
    assert_eq!(packet.text.as_deref(), Some("user@host"));

    let bye = rtcp::encode_packet(203, 7, b"done");
    let packet = rtcp::decode_packet(&bye).unwrap();
    assert_eq!(packet.count, 1);
    assert_eq!(packet.text.as_deref(), Some("done"));
    assert!(rtcp::decode_packet(&[0x80, 201]).is_err());
}

#[tokio::test]
async fn rtp_udp_loopback_send_and_receive() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        let incoming = rtp::decode_packet(&buf[..n]).unwrap();
        assert_eq!(incoming.payload, b"hello");
        assert_eq!(incoming.sequence, 10);
        let reply = rtp::encode_packet(97, 99, 4242, 7, b"pong");
        server.send_to(&reply, peer).await.unwrap();
    });

    let res = execute(
        &allow_private(),
        &req(
            "rtp",
            format!("rtp://{addr}"),
            Some("hello"),
            &[
                ("payload_type", "96"),
                ("sequence", "10"),
                ("ssrc", "42"),
                ("recv", "true"),
                ("duration_ms", "1000"),
            ],
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.headers["x-sent"], "1");
    assert_eq!(res.headers["x-received"], "1");
    let value: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(value["packets"][0]["payload"]["data"], "pong");
    assert_eq!(value["packets"][0]["payload_type"], 97);
}

#[tokio::test]
async fn rtcp_udp_loopback_receives_bye() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(rtcp::decode_packet(&buf[..n]).unwrap().packet_type, 201);
        server
            .send_to(&rtcp::encode_packet(203, 42, b"bye"), peer)
            .await
            .unwrap();
    });

    let res = execute(
        &allow_private(),
        &req(
            "rtcp",
            format!("rtcp://{addr}"),
            None,
            &[
                ("packet_type", "rr"),
                ("ssrc", "42"),
                ("recv", "true"),
                ("duration_ms", "1000"),
            ],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(value["packets"][0]["packet_type"], 203);
    assert_eq!(value["packets"][0]["text"], "bye");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("rtp", "rtp://127.0.0.1:5004".into(), None, &[]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
    let err = execute(
        &DebugConfig::default(),
        &req("rtcp", "rtcp://127.0.0.1:5005".into(), None, &[]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
