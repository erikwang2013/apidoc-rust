#![cfg(feature = "debug-coap")]
//! P4 CoAP：本地 UDP 回环，请求编码、响应解码、Observe 通知窗口、超时与守卫。

use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::DebugConfig;
use coap_lite::{CoapOption, MessageClass, MessageType, Packet, RequestType, ResponseType};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::net::UdpSocket;

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        timeout_ms: 2_000,
        ..Default::default()
    }
}

fn req(
    kind: &str,
    target: String,
    opts: &[(&str, &str)],
    body: Option<Payload>,
) -> DebugRequest {
    DebugRequest {
        kind: kind.into(),
        target,
        method: None,
        headers: BTreeMap::new(),
        body,
        opts: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        endpoint: None,
        timeout_ms: Some(2_000),
    }
}

fn response_packet(request: &Packet, code: ResponseType) -> Packet {
    let mut response = Packet::new();
    response.header.set_version(1);
    response.header.set_type(MessageType::Acknowledgement);
    response.header.message_id = request.header.message_id;
    response.header.code = MessageClass::Response(code);
    response.set_token(request.get_token().to_vec());
    response
}

#[tokio::test]
async fn get_roundtrip_returns_code_payload_and_options() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        let packet = Packet::from_bytes(&buf[..n]).unwrap();
        assert_eq!(packet.header.get_type(), MessageType::Confirmable);
        assert_eq!(packet.header.code, MessageClass::Request(RequestType::Get));
        let request = coap_lite::CoapRequest::<()>::from_packet(packet, ());
        assert_eq!(request.get_path(), "sensors/temp");
        assert!(request.message.get_option(CoapOption::ContentFormat).is_some());

        let mut response = response_packet(&request.message, ResponseType::Content);
        response.add_option(CoapOption::ContentFormat, vec![0]);
        response.payload = b"22.5".to_vec();
        server
            .send_to(&response.to_bytes().unwrap(), peer)
            .await
            .unwrap();
    });

    let result = execute(
        &cfg(),
        &req(
            "coap",
            format!("coap://{addr}"),
            &[("path", "/sensors/temp")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["code"], "2.05");
    assert_eq!(value["payload"]["encoding"], "utf8");
    assert_eq!(value["payload"]["data"], "22.5");
    assert_eq!(value["options"]["content-format"], "0");
    assert_eq!(result.headers.get("x-peer").map(String::as_str), Some(&*addr.to_string()));
    handle.await.unwrap();
}

#[tokio::test]
async fn observe_collects_notifications_until_messages_limit() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        let request = Packet::from_bytes(&buf[..n]).unwrap();
        assert_eq!(
            request.get_observe_value().unwrap().unwrap(),
            0,
            "observe registration"
        );
        let token = request.get_token().to_vec();

        let mut first = response_packet(&request, ResponseType::Content);
        first.set_observe_value(1);
        first.add_option(CoapOption::ContentFormat, vec![0]);
        first.payload = b"one".to_vec();
        server.send_to(&first.to_bytes().unwrap(), peer).await.unwrap();

        let mut second = Packet::new();
        second.header.set_version(1);
        second.header.set_type(MessageType::NonConfirmable);
        second.header.message_id = request.header.message_id.wrapping_add(1);
        second.header.code = MessageClass::Response(ResponseType::Content);
        second.set_token(token);
        second.set_observe_value(2);
        second.payload = b"two".to_vec();
        server.send_to(&second.to_bytes().unwrap(), peer).await.unwrap();
    });

    let result = execute(
        &cfg(),
        &req(
            "coap",
            format!("coap://{addr}"),
            &[("path", "/obs"), ("observe", "true"), ("messages", "2")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    let notifications = value["notifications"].as_array().unwrap();
    assert_eq!(notifications.len(), 2);
    assert_eq!(notifications[0]["payload"]["data"], "one");
    assert_eq!(notifications[1]["payload"]["data"], "two");
    assert_eq!(notifications[1]["options"]["observe"], "2");
    handle.await.unwrap();
}

#[tokio::test]
async fn request_timeout_is_bounded() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let started = std::time::Instant::now();
    let err = execute(
        &cfg(),
        &req(
            "coap",
            format!("coap://{addr}"),
            &[("path", "/"), ("duration_ms", "50")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    // 50ms 的 duration_ms；放宽到 5s 只用于容忍高负载 CI/并行构建的调度延迟。
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn coaps_and_unknown_methods_are_rejected() {
    let err = execute(
        &cfg(),
        &req("coap", "coaps://127.0.0.1:5683".into(), &[], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("coaps"), "{err}");

    let err = execute(
        &cfg(),
        &req(
            "coap",
            "coap://127.0.0.1:5683".into(),
            &[("method", "PATCH")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("unsupported coap method"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("coap", "coap://127.0.0.1:5683".into(), &[], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
