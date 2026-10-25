#![cfg(feature = "debug-coap")]
//! P4 LwM2M：register/update/deregister + 单资源 TLV read/write，本地 UDP 回环。

use apidoc::debug::{execute, DebugRequest};
use apidoc::DebugConfig;
use coap_lite::{CoapOption, MessageClass, MessageType, Packet, RequestType, ResponseType};
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

fn req(target: String, opts: &[(&str, &str)], body: Option<&str>) -> DebugRequest {
    DebugRequest {
        kind: "lwm2m".into(),
        target,
        method: None,
        headers: BTreeMap::new(),
        body: body.map(|data| apidoc::debug::Payload {
            encoding: "utf8".into(),
            data: data.into(),
        }),
        opts: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        endpoint: None,
        timeout_ms: Some(2_000),
    }
}

struct Reply {
    code: ResponseType,
    payload: Vec<u8>,
    content_format: Option<u32>,
    location: Vec<&'static str>,
}

async fn mock(
    path: &'static str,
    method: RequestType,
    expected_body: Vec<u8>,
    reply: Reply,
) -> (SocketAddr, JoinHandle<()>) {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        let packet = Packet::from_bytes(&buf[..n]).unwrap();
        assert_eq!(packet.header.get_type(), MessageType::Confirmable);
        assert_eq!(packet.header.code, MessageClass::Request(method));
        let request = coap_lite::CoapRequest::<()>::from_packet(packet, ());
        assert_eq!(request.get_path(), path);
        assert_eq!(request.message.payload, expected_body);

        let mut response = Packet::new();
        response.header.set_version(1);
        response.header.set_type(MessageType::Acknowledgement);
        response.header.message_id = request.message.header.message_id;
        response.header.code = MessageClass::Response(reply.code);
        response.set_token(request.message.get_token().to_vec());
        if let Some(format) = reply.content_format {
            response.add_option(CoapOption::ContentFormat, uint_option(format));
        }
        for segment in reply.location {
            response.add_option(CoapOption::LocationPath, segment.as_bytes().to_vec());
        }
        response.payload = reply.payload;
        server
            .send_to(&response.to_bytes().unwrap(), peer)
            .await
            .unwrap();
    });
    (addr, handle)
}

fn uint_option(value: u32) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    bytes
        .iter()
        .skip_while(|b| **b == 0)
        .copied()
        .collect::<Vec<u8>>()
}

#[tokio::test]
async fn register_returns_location() {
    let (addr, handle) = mock(
        "/rd",
        RequestType::Post,
        Vec::new(),
        Reply {
            code: ResponseType::Created,
            payload: Vec::new(),
            content_format: None,
            location: vec!["rd", "abc"],
        },
    )
    .await;
    let result = execute(
        &cfg(),
        &req(
            format!("coap://{addr}"),
            &[("operation", "register"), ("endpoint", "node-1")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["code"], "2.01");
    assert_eq!(value["location"], "/rd/abc");
    handle.await.unwrap();
}

#[tokio::test]
async fn update_and_deregister_use_location_path() {
    let (addr, handle) = mock(
        "/rd/abc",
        RequestType::Post,
        Vec::new(),
        Reply {
            code: ResponseType::Changed,
            payload: Vec::new(),
            content_format: None,
            location: Vec::new(),
        },
    )
    .await;
    let result = execute(
        &cfg(),
        &req(
            format!("coap://{addr}"),
            &[("operation", "update"), ("location", "/rd/abc")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["code"], "2.04");
    handle.await.unwrap();

    let (addr, handle) = mock(
        "/rd/abc",
        RequestType::Delete,
        Vec::new(),
        Reply {
            code: ResponseType::Deleted,
            payload: Vec::new(),
            content_format: None,
            location: Vec::new(),
        },
    )
    .await;
    let result = execute(
        &cfg(),
        &req(
            format!("coap://{addr}"),
            &[("operation", "deregister"), ("location", "/rd/abc")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["code"], "2.02");
    handle.await.unwrap();
}

#[tokio::test]
async fn read_decodes_single_resource_tlv() {
    let (addr, handle) = mock(
        "/3/0/9",
        RequestType::Get,
        Vec::new(),
        Reply {
            code: ResponseType::Content,
            // resource 9, 2-byte integer 300
            payload: vec![0xC0, 0x09, 0x02, 0x01, 0x2C],
            content_format: Some(11_542),
            location: Vec::new(),
        },
    )
    .await;
    let result = execute(
        &cfg(),
        &req(
            format!("coap://{addr}"),
            &[
                ("operation", "read"),
                ("object_id", "3"),
                ("instance_id", "0"),
                ("resource_id", "9"),
            ],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["code"], "2.05");
    assert_eq!(value["value"], 300);
    handle.await.unwrap();
}

#[tokio::test]
async fn write_encodes_resource_tlv() {
    let (addr, handle) = mock(
        "/3/0/13",
        RequestType::Put,
        // resource 13, inline length 1, value 5
        vec![0xC1, 0x0D, 0x05],
        Reply {
            code: ResponseType::Changed,
            payload: Vec::new(),
            content_format: None,
            location: Vec::new(),
        },
    )
    .await;
    let result = execute(
        &cfg(),
        &req(
            format!("coap://{addr}"),
            &[
                ("operation", "write"),
                ("object_id", "3"),
                ("instance_id", "0"),
                ("resource_id", "13"),
                ("value", "5"),
            ],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["code"], "2.04");
    handle.await.unwrap();
}

#[tokio::test]
async fn unexpected_response_code_is_reported() {
    let (addr, handle) = mock(
        "/3/0/9",
        RequestType::Get,
        Vec::new(),
        Reply {
            code: ResponseType::BadRequest,
            payload: Vec::new(),
            content_format: None,
            location: Vec::new(),
        },
    )
    .await;
    let err = execute(
        &cfg(),
        &req(
            format!("coap://{addr}"),
            &[
                ("operation", "read"),
                ("object_id", "3"),
                ("instance_id", "0"),
                ("resource_id", "9"),
            ],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("expected 2.05, got 4.00"), "{err}");
    handle.await.unwrap();
}

#[tokio::test]
async fn validation_and_guard_happen_before_network() {
    let err = execute(
        &cfg(),
        &req(
            "coap://127.0.0.1:5683".into(),
            &[("operation", "register")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("endpoint is required"), "{err}");

    let err = execute(
        &cfg(),
        &req(
            "coap://127.0.0.1:5683".into(),
            &[("operation", "observe")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("unsupported lwm2m operation"), "{err}");

    let err = execute(
        &DebugConfig::default(),
        &req("coap://127.0.0.1:5683".into(), &[], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
