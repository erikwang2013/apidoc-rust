#![cfg(feature = "debug-industrial")]
//! P4 OPC UA Binary：SecurityPolicy None 的 HEL/OPN/Session 握手 + Browse/Read/Write。
//! 本地 TCP 回环 mock；多分块与认证失败必须显式报错，密码不进错误信息。

use apidoc::debug::{execute, DebugRequest};
use apidoc::DebugConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const NONE_POLICY: &str = "http://opcfoundation.org/UA/SecurityPolicy#None";

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        timeout_ms: 2_000,
        ..Default::default()
    }
}

fn req(target: String, opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: "opcua".into(),
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

fn nid(id: u32) -> Vec<u8> {
    if id <= 0xFF {
        vec![0x00, id as u8]
    } else {
        let mut out = vec![0x01, 0x00];
        out.extend_from_slice(&(id as u16).to_le_bytes());
        out
    }
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as i32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn put_null_bytes(out: &mut Vec<u8>) {
    out.extend_from_slice(&(-1i32).to_le_bytes());
}

fn response_header(status: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0i64.to_le_bytes()); // timestamp
    out.extend_from_slice(&1u32.to_le_bytes()); // request handle
    out.extend_from_slice(&status.to_le_bytes());
    out.push(0); // diagnostic info
    out.extend_from_slice(&(-1i32).to_le_bytes()); // string table
    out.extend_from_slice(&[0x00, 0x00, 0x00]); // additional header null
    out
}

fn frame(kind: &[u8; 3], channel: u32, token: u32, body: &[u8]) -> Vec<u8> {
    let mut out = vec![kind[0], kind[1], kind[2], b'F'];
    out.extend_from_slice(&((24 + body.len()) as u32).to_le_bytes());
    out.extend_from_slice(&channel.to_le_bytes());
    out.extend_from_slice(&token.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // sequence
    out.extend_from_slice(&1u32.to_le_bytes()); // request id
    out.extend_from_slice(body);
    out
}

fn opn_frame(body: &[u8]) -> Vec<u8> {
    let mut out = vec![b'O', b'P', b'N', b'F'];
    out.extend_from_slice(&((32 + NONE_POLICY.len() + body.len()) as u32).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    put_str(&mut out, NONE_POLICY);
    put_null_bytes(&mut out); // sender certificate
    put_null_bytes(&mut out); // receiver thumbprint
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(body);
    out
}

fn ack_frame() -> Vec<u8> {
    let mut out = vec![b'A', b'C', b'K', b'F'];
    out.extend_from_slice(&24u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out
}

fn open_channel_body() -> Vec<u8> {
    let mut body = nid(446);
    body.extend(response_header(0));
    body.extend_from_slice(&0u32.to_le_bytes()); // server protocol version
    body.extend_from_slice(&1u32.to_le_bytes()); // channel id
    body.extend_from_slice(&1u32.to_le_bytes()); // token id
    body.extend_from_slice(&0i64.to_le_bytes()); // created at
    body.extend_from_slice(&3_600_000u32.to_le_bytes()); // revised lifetime
    body
}

fn create_session_body() -> Vec<u8> {
    let mut body = nid(461);
    body.extend(response_header(0));
    body.extend(nid(100)); // session id
    body.extend(nid(200)); // authentication token
    body
}

async fn read_client_message(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).await.unwrap();
    assert_eq!(header[3], b'F', "single-chunk request");
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let mut rest = vec![0u8; size - 8];
    stream.read_exact(&mut rest).await.unwrap();
    let kind = String::from_utf8(header[..3].to_vec()).unwrap();
    let body = match kind.as_str() {
        "HEL" => rest,
        "OPN" => {
            let mut pos = 4; // channel id
            let policy_len = i32::from_le_bytes(rest[pos..pos + 4].try_into().unwrap());
            pos += 4 + policy_len.max(0) as usize;
            for _ in 0..2 {
                let len = i32::from_le_bytes(rest[pos..pos + 4].try_into().unwrap());
                pos += 4 + len.max(0) as usize;
            }
            pos += 8; // sequence + request id
            rest[pos..].to_vec()
        }
        "MSG" => rest[16..].to_vec(), // channel + token + sequence + request, then body
        _ => rest,
    };
    (kind, body)
}

/// HEL/ACK + OPN + CreateSession + ActivateSession；返回 ActivateSession 请求体。
async fn open_session(stream: &mut TcpStream, activate_status: u32) -> Vec<u8> {
    let (kind, _) = read_client_message(stream).await;
    assert_eq!(kind, "HEL");
    stream.write_all(&ack_frame()).await.unwrap();

    let (kind, _) = read_client_message(stream).await;
    assert_eq!(kind, "OPN");
    stream.write_all(&opn_frame(&open_channel_body())).await.unwrap();

    let (kind, body) = read_client_message(stream).await;
    assert_eq!(kind, "MSG");
    assert_eq!(&body[..4], &nid(461)[..], "CreateSession request");
    stream
        .write_all(&frame(b"MSG", 1, 1, &create_session_body()))
        .await
        .unwrap();

    let (kind, body) = read_client_message(stream).await;
    assert_eq!(kind, "MSG");
    assert_eq!(&body[..4], &nid(467)[..], "ActivateSession request");
    let mut response = nid(467);
    response.extend(response_header(activate_status));
    stream.write_all(&frame(b"MSG", 1, 1, &response)).await.unwrap();
    body
}

#[tokio::test]
async fn read_returns_data_value() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        open_session(&mut stream, 0).await;
        let (kind, body) = read_client_message(&mut stream).await;
        assert_eq!(kind, "MSG");
        assert_eq!(&body[..4], &nid(631)[..], "ReadRequest");

        let mut response = nid(631);
        response.extend(response_header(0));
        response.extend_from_slice(&1i32.to_le_bytes()); // results
        response.push(0x01); // DataValue: value only
        response.push(11); // Variant: Double
        response.extend_from_slice(&22.5f64.to_le_bytes());
        stream
            .write_all(&frame(b"MSG", 1, 1, &response))
            .await
            .unwrap();
    });

    let result = execute(
        &cfg(),
        &req(
            format!("opc.tcp://{addr}"),
            &[("operation", "read"), ("node_id", "ns=2;i=5")],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["value"], 22.5);
    assert_eq!(value["status"], "Good");
    handle.await.unwrap();
}

#[tokio::test]
async fn browse_returns_references() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        open_session(&mut stream, 0).await;
        let (kind, body) = read_client_message(&mut stream).await;
        assert_eq!(kind, "MSG");
        assert_eq!(&body[..4], &nid(527)[..], "BrowseRequest");

        let mut response = nid(527);
        response.extend(response_header(0));
        response.extend_from_slice(&1i32.to_le_bytes()); // results
        response.extend_from_slice(&0u32.to_le_bytes()); // status
        put_null_bytes(&mut response); // continuation point
        response.extend_from_slice(&1i32.to_le_bytes()); // references
        response.extend(nid(31)); // reference type
        response.push(1); // is forward
        response.extend_from_slice(&[0x01, 0x02, 0x05, 0x00]); // ns=2;i=5
        response.extend_from_slice(&2u16.to_le_bytes()); // browse name namespace
        put_str(&mut response, "Demo");
        response.push(0x02); // localized text: text only
        put_str(&mut response, "Demo Value");
        response.extend_from_slice(&2u32.to_le_bytes()); // node class = Variable
        response.extend(nid(0)); // type definition null
        stream
            .write_all(&frame(b"MSG", 1, 1, &response))
            .await
            .unwrap();
    });

    let result = execute(
        &cfg(),
        &req(
            format!("opc.tcp://{addr}"),
            &[("operation", "browse"), ("node_id", "i=85")],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    let references = value["references"].as_array().unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0]["node_id"], "ns=2;i=5");
    assert_eq!(references[0]["browse_name"], "ns=2;Demo");
    assert_eq!(references[0]["display_name"], "Demo Value");
    assert_eq!(references[0]["node_class"], 2);
    handle.await.unwrap();
}

#[tokio::test]
async fn write_sends_value_and_returns_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        open_session(&mut stream, 0).await;
        let (kind, body) = read_client_message(&mut stream).await;
        assert_eq!(kind, "MSG");
        assert_eq!(&body[..4], &nid(673)[..], "WriteRequest");
        assert!(
            body.windows(4).any(|w| w == [0x01, 0x02, 0x05, 0x00]),
            "write targets ns=2;i=5"
        );
        let mut variant = vec![0x01, 11];
        variant.extend_from_slice(&42.5f64.to_le_bytes());
        assert!(body.ends_with(&variant), "write value is Double 42.5");

        let mut response = nid(673);
        response.extend(response_header(0));
        response.extend_from_slice(&1i32.to_le_bytes()); // results
        response.extend_from_slice(&0u32.to_le_bytes()); // status
        stream
            .write_all(&frame(b"MSG", 1, 1, &response))
            .await
            .unwrap();
    });

    let result = execute(
        &cfg(),
        &req(
            format!("opc.tcp://{addr}"),
            &[
                ("operation", "write"),
                ("node_id", "ns=2;i=5"),
                ("value", "42.5"),
            ],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&result.body.data).unwrap();
    assert_eq!(value["status"], "Good");
    handle.await.unwrap();
}

#[tokio::test]
async fn authentication_failure_does_not_leak_password() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let body = open_session(&mut stream, 0x8021_0000).await; // BadIdentityTokenRejected
        assert!(body.windows(7).any(|w| w == b"hunter2"), "password was sent");
    });

    let err = execute(
        &cfg(),
        &req(
            format!("opc.tcp://{addr}"),
            &[
                ("operation", "read"),
                ("username", "alice"),
                ("password", "hunter2"),
            ],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("BadIdentityTokenRejected"), "{err}");
    assert!(!err.contains("hunter2"), "password leaked: {err}");
    handle.await.unwrap();
}

#[tokio::test]
async fn multi_chunk_responses_are_explicitly_unsupported() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        open_session(&mut stream, 0).await;
        let _ = read_client_message(&mut stream).await;
        let mut chunk = vec![b'M', b'S', b'G', b'C'];
        chunk.extend_from_slice(&24u32.to_le_bytes());
        chunk.extend_from_slice(&[0u8; 16]);
        stream.write_all(&chunk).await.unwrap();
    });

    let err = execute(
        &cfg(),
        &req(format!("opc.tcp://{addr}"), &[("operation", "read")]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("multi-chunk"), "{err}");
    handle.await.unwrap();
}

#[tokio::test]
async fn unsupported_operation_and_default_guard_are_rejected() {
    let err = execute(
        &cfg(),
        &req(
            "opc.tcp://127.0.0.1:4840".into(),
            &[("operation", "subscribe")],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("unsupported opcua operation"), "{err}");

    let err = execute(
        &DebugConfig::default(),
        &req("opc.tcp://127.0.0.1:4840".into(), &[]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
