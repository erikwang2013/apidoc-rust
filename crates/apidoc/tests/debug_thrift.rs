#![cfg(feature = "debug-thrift")]
//! P2 Thrift TBinaryProtocol：黄金向量、TCP/HTTP 传输、参数编码与缺 schema 报错。

use apidoc::debug::thrift;
use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::{DebugConfig, DocEndpoint, DocParam};
use base64::Engine;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

fn param(name: &'static str, ty: &'static str, field: &'static str, required: bool) -> DocParam {
    DocParam {
        name,
        ty,
        required,
        default: None,
        desc: None,
        mock: None,
        field: Some(field),
        children: &[],
    }
}

fn ep(method: &str, params: Vec<DocParam>, returned: Vec<DocParam>) -> DocEndpoint {
    let mut opts = BTreeMap::new();
    opts.insert("method".to_string(), method.to_string());
    DocEndpoint {
        protocol: "thrift".into(),
        protocol_opts: opts,
        params,
        returned,
        ..Default::default()
    }
}

fn req(target: String, opts: &[(&str, &str)], body: &str) -> DebugRequest {
    DebugRequest {
        kind: "thrift".into(),
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

fn reply(message: &str) -> Vec<u8> {
    let mut out = Vec::new();
    thrift::write_message_begin(&mut out, "ping", 2, 0);
    thrift::write_field(&mut out, 11, 1);
    out.extend_from_slice(&(message.len() as i32).to_be_bytes());
    out.extend_from_slice(message.as_bytes());
    out.push(0);
    out
}

#[test]
fn binary_protocol_message_begin_vector() {
    let mut out = Vec::new();
    thrift::write_message_begin(&mut out, "ping", 1, 0);
    assert_eq!(
        out,
        vec![
            0x80, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, b'p', b'i', b'n', b'g', 0x00, 0x00,
            0x00, 0x00
        ]
    );
}

#[tokio::test]
async fn tcp_roundtrip_and_param_encoding() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // message begin(ping) 16 + field header 3 + string len 4 + "hi" 2 + STOP 1 = 26
        let mut request = vec![0u8; 26];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[16..19], &[11, 0, 1], "field header type/id");
        assert_eq!(&request[19..23], &2i32.to_be_bytes(), "string length");
        assert_eq!(&request[23..25], b"hi");
        assert_eq!(request[25], 0, "struct STOP");
        stream.write_all(&reply("pong")).await.unwrap();
    });

    let ep = ep(
        "ping",
        vec![param("name", "string", "1", true)],
        vec![param("message", "string", "1", false)],
    );
    let res = execute(
        &cfg(),
        &req(format!("tcp://{addr}"), &[("transport", "tcp")], r#"{"name":"hi"}"#),
        Some(&ep),
    )
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v["message"], "pong");
}

#[tokio::test]
async fn http_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request(&mut stream).await;
        let frame = reply("http-pong");
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-thrift\r\nContent-Length: {}\r\n\r\n",
            frame.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(&frame).await.unwrap();
    });

    let ep = ep("ping", vec![], vec![param("message", "string", "1", false)]);
    let res = execute(
        &cfg(),
        &req(
            format!("http://{addr}/thrift"),
            &[("transport", "http")],
            "{}",
        ),
        Some(&ep),
    )
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v["message"], "http-pong");
}

#[tokio::test]
async fn missing_method_is_reported() {
    let ep = DocEndpoint {
        protocol: "thrift".into(),
        ..Default::default()
    };
    let err = execute(
        &cfg(),
        &req("tcp://127.0.0.1:1".into(), &[], "{}"),
        Some(&ep),
    )
    .await
    .unwrap_err();
    assert!(err.contains("method"), "{err}");
}

#[tokio::test]
async fn missing_endpoint_schema_is_reported() {
    let err = execute(
        &cfg(),
        &req("tcp://127.0.0.1:1".into(), &[("method", "ping")], "{}"),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("endpoint"), "{err}");
}

/// binary 走 T_STRING 线型，但非 UTF-8 字节必须以 base64 无损往返。
#[tokio::test]
async fn binary_params_roundtrip_base64_and_preserve_non_utf8() {
    let raw = [0x00u8, 0xff, 0x80];
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
    let expected = encoded.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // message begin 16 + field 3 + len 4 + payload 3 + STOP 1
        let mut request = vec![0u8; 27];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[16..19], &[11, 0, 1], "binary uses the T_STRING wire type");
        assert_eq!(&request[19..23], &3i32.to_be_bytes());
        assert_eq!(&request[23..26], &raw);
        assert_eq!(request[26], 0, "struct STOP");

        let mut out = Vec::new();
        thrift::write_message_begin(&mut out, "ping", 2, 0);
        thrift::write_field(&mut out, 11, 1);
        out.extend_from_slice(&3i32.to_be_bytes());
        out.extend_from_slice(&raw);
        out.push(0);
        stream.write_all(&out).await.unwrap();
    });

    let ep = ep(
        "ping",
        vec![param("data", "binary", "1", true)],
        vec![param("data", "binary", "1", false)],
    );
    let res = execute(
        &cfg(),
        &req(
            format!("tcp://{addr}"),
            &[("transport", "tcp")],
            &format!(r#"{{"data":"{expected}"}}"#),
        ),
        Some(&ep),
    )
    .await
    .unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v["data"], expected);
}

async fn read_http_request(stream: &mut TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0, "client closed before headers");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
            let len = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let body_start = pos + 4;
            while buf.len() < body_start + len {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0, "client closed before body");
                buf.extend_from_slice(&chunk[..n]);
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
            return;
        }
    }
}
