#![cfg(feature = "debug-iso8583")]
//! P2 ISO 8583：黄金向量、位图（含次位图）、二进制域、ep.params 字段来源与 SSRF 拒绝。

use apidoc::debug::{execute, iso8583, DebugRequest, Payload};
use apidoc::{DebugConfig, DocEndpoint, DocParam};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const FIELDS: &str = r#"[
    {"field":"2","type":"llvar","len":19,"kind":"n"},
    {"field":"3","type":"fixed","len":6,"kind":"n"},
    {"field":"4","type":"fixed","len":12,"kind":"n"},
    {"field":"39","type":"fixed","len":2,"kind":"n"}
]"#;

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

fn req(target: String, opts: &[(&str, &str)], body: &str) -> DebugRequest {
    DebugRequest {
        kind: "iso8583".into(),
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

fn golden_request() -> Vec<u8> {
    let body = json!({
        "2": "4111111111111111",
        "3": "000000",
        "4": "000000000100",
    });
    let mut expected = Vec::new();
    expected.extend_from_slice(b"0200");
    expected.extend_from_slice(&[0x70, 0, 0, 0, 0, 0, 0, 0]); // fields 2,3,4
    expected.extend_from_slice(b"164111111111111111"); // LLVAR 16 + PAN
    expected.extend_from_slice(b"000000");
    expected.extend_from_slice(b"000000000100");
    let encoded = iso8583::encode_message("0200", FIELDS, &body).unwrap();
    assert_eq!(encoded, expected, "golden 0200 request bytes");
    encoded
}

fn golden_response() -> Vec<u8> {
    let mut msg = Vec::new();
    msg.extend_from_slice(b"0210");
    msg.extend_from_slice(&[0x40, 0, 0, 0, 0x02, 0, 0, 0]); // fields 2 and 39
    msg.extend_from_slice(b"164111111111111111");
    msg.extend_from_slice(b"00");
    let parsed = iso8583::decode_message(&msg, FIELDS).unwrap();
    assert_eq!(
        parsed,
        json!({"mti":"0210","fields":{"2":"4111111111111111","39":"00"}})
    );
    msg
}

#[test]
fn golden_vectors() {
    golden_request();
    golden_response();
}

#[test]
fn secondary_bitmap_roundtrip() {
    let fields = r#"[{"field":"70","type":"fixed","len":3,"kind":"n"}]"#;
    let msg = iso8583::encode_message("0800", fields, &json!({"70":"301"})).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(b"0800");
    expected.extend_from_slice(&[0x80, 0, 0, 0, 0, 0, 0, 0]); // secondary present
    expected.extend_from_slice(&[0x04, 0, 0, 0, 0, 0, 0, 0]); // field 70
    expected.extend_from_slice(b"301");
    assert_eq!(msg, expected);
    assert_eq!(
        iso8583::decode_message(&msg, fields).unwrap(),
        json!({"mti":"0800","fields":{"70":"301"}})
    );
}

#[test]
fn binary_field_packs_bcd_and_length_is_digits() {
    let fields = r#"[{"field":"55","type":"lllvar","len":999,"kind":"b"}]"#;
    let msg = iso8583::encode_message("0200", fields, &json!({"55":"9f2608aabbccddeeff"})).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(b"0200");
    expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0x02, 0]); // field 55
    expected.extend_from_slice(b"018"); // 18 hex digits
    expected.extend_from_slice(&[0x9f, 0x26, 0x08, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
    assert_eq!(msg, expected);
    assert_eq!(
        iso8583::decode_message(&msg, fields).unwrap(),
        json!({"mti":"0200","fields":{"55":"9F2608AABBCCDDEEFF"}})
    );
}

#[test]
fn unknown_field_is_reported() {
    let err = iso8583::encode_message("0200", FIELDS, &json!({"5":"1"})).unwrap_err();
    assert!(err.contains("not configured"), "{err}");
}

fn ep_param(name: &'static str, ty: &'static str, field: &'static str) -> DocParam {
    DocParam {
        name,
        ty,
        required: false,
        default: None,
        desc: None,
        mock: None,
        field: Some(field),
        children: &[],
    }
}

#[tokio::test]
async fn tcp_roundtrip_uses_endpoint_params_as_field_schema() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let expected = golden_request();
    let response = golden_response();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut len = [0u8; 2];
        stream.read_exact(&mut len).await.unwrap();
        let mut body = vec![0u8; u16::from_be_bytes(len) as usize];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(body, expected, "framed request body");
        // 回包不带长度前缀的解析也要先验证，这里仍按默认 2 字节 framing。
        stream.write_all(&(response.len() as u16).to_be_bytes()).await.unwrap();
        stream.write_all(&response).await.unwrap();
    });

    let ep = DocEndpoint {
        protocol: "iso8583".into(),
        params: vec![
            ep_param("pan", "llvar:n19", "2"),
            ep_param("processing_code", "n6", "3"),
            ep_param("amount", "n12", "4"),
            ep_param("response_code", "n2", "39"),
        ],
        ..Default::default()
    };
    let res = execute(
        &cfg(),
        &req(
            format!("tcp://{addr}"),
            &[("mti", "0200")],
            r#"{"2":"4111111111111111","3":"000000","4":"000000000100"}"#,
        ),
        Some(&ep),
    )
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v["mti"], "0210");
    assert_eq!(v["fields"]["2"], "4111111111111111");
    assert_eq!(v["fields"]["39"], "00");
    assert_eq!(res.headers.get("x-mti").map(String::as_str), Some("0210"));
    server.await.unwrap();
}

#[tokio::test]
async fn framing_none_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let expected = golden_request();
    let response = golden_response();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut body = vec![0u8; expected.len()];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(body, expected);
        stream.write_all(&response).await.unwrap();
    });
    let res = execute(
        &cfg(),
        &req(
            format!("tcp://{addr}"),
            &[("mti", "0200"), ("fields", FIELDS), ("framing", "none")],
            r#"{"2":"4111111111111111","3":"000000","4":"000000000100"}"#,
        ),
        None,
    )
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v["mti"], "0210");
    server.await.unwrap();
}

#[tokio::test]
async fn ebcdic_is_rejected() {
    let err = execute(
        &cfg(),
        &req(
            "tcp://127.0.0.1:9".into(),
            &[("mti", "0200"), ("fields", FIELDS), ("encoding", "ebcdic")],
            "{}",
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("ebcdic"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req(
            "tcp://127.0.0.1:9".into(),
            &[("mti", "0200"), ("fields", FIELDS)],
            r#"{"2":"4111111111111111","3":"000000","4":"000000000100"}"#,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
