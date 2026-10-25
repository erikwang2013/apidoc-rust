#![cfg(feature = "debug-media")]
//! P5 SIP：rsip 黄金文本解析、UDP/TCP 单事务匹配、401 明确报错与默认 SSRF 拒绝。

use apidoc::debug::{execute, sip, DebugRequest};
use apidoc::DebugConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

const CALL_ID: &str = "apidoc-test-call@example.com";
const BRANCH: &str = "z9hG4bKapidoc";

fn req(target: String, opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: "sip".into(),
        target,
        method: None,
        headers: BTreeMap::new(),
        body: None,
        opts: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        endpoint: None,
        timeout_ms: Some(2000),
    }
}

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

fn base_opts(
    method: &'static str,
    transport: &'static str,
    cseq: &'static str,
) -> Vec<(&'static str, &'static str)> {
    vec![
        ("method", method),
        ("transport", transport),
        ("branch", BRANCH),
        ("call_id", CALL_ID),
        ("cseq", cseq),
    ]
}

#[test]
fn invite_golden_text_parses_with_rsip() {
    let invite = "INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
        Via: SIP/2.0/UDP client.atlanta.example.com:5060;branch=z9hG4bK74bf9\r\n\
        Max-Forwards: 70\r\n\
        From: Alice <sip:alice@atlanta.example.com>;tag=9fxced76sl\r\n\
        To: Bob <sip:bob@biloxi.example.com>\r\n\
        Call-ID: 3848276298220188511@atlanta.example.com\r\n\
        CSeq: 1 INVITE\r\n\
        Contact: <sip:alice@client.atlanta.example.com;transport=udp>\r\n\
        Content-Length: 0\r\n\r\n";
    let (method, uri) = sip::parse_request(invite).unwrap();
    assert_eq!(method, "INVITE");
    assert_eq!(uri, "sip:bob@biloxi.example.com");
    assert!(sip::parse_request("not a sip message").is_err());
}

fn header_line(request: &str, name: &str) -> String {
    request
        .lines()
        .find(|line| {
            line.to_ascii_lowercase()
                .starts_with(&format!("{}:", name.to_ascii_lowercase()))
        })
        .unwrap_or_default()
        .to_string()
}

fn reply(request: &str, status_line: &str, extra: &str) -> String {
    format!(
        "{status_line}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{extra}Content-Length: 0\r\n\r\n",
        header_line(request, "Via"),
        header_line(request, "From"),
        header_line(request, "To"),
        header_line(request, "Call-ID"),
        header_line(request, "CSeq"),
    )
}

#[tokio::test]
async fn udp_options_transaction_matches_call_id_and_cseq() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        assert!(request.starts_with("OPTIONS sip:"), "{request}");
        assert!(request.contains(&format!("Call-ID: {CALL_ID}")));
        assert!(request.contains("CSeq: 5 OPTIONS"));
        assert!(request.contains(&format!("branch={BRANCH}")));
        let response = reply(&request, "SIP/2.0 200 OK", "");
        server.send_to(response.as_bytes(), peer).await.unwrap();
    });

    let res = execute(
        &cfg(),
        &req(format!("sip://{addr}"), &base_opts("OPTIONS", "udp", "5")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.status, 200);
    let value: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(value["status"], 200);
    assert_eq!(value["reason"], "OK");
    assert_eq!(value["headers"]["Call-ID"], CALL_ID);
}

#[tokio::test]
async fn provisional_response_is_skipped_until_final() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let trying = reply(&request, "SIP/2.0 100 Trying", "");
        server.send_to(trying.as_bytes(), peer).await.unwrap();
        let ok = reply(&request, "SIP/2.0 200 OK", "");
        server.send_to(ok.as_bytes(), peer).await.unwrap();
    });

    let res = execute(
        &cfg(),
        &req(format!("sip://{addr}"), &base_opts("INVITE", "udp", "1")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.status, 200);
    let value: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(value["reason"], "OK");
}

#[tokio::test]
async fn unauthorized_reports_authentication_unsupported() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let response = reply(
            &request,
            "SIP/2.0 401 Unauthorized",
            "WWW-Authenticate: Digest realm=\"x\", nonce=\"y\"\r\n",
        );
        server.send_to(response.as_bytes(), peer).await.unwrap();
    });

    let err = execute(
        &cfg(),
        &req(format!("sip://{addr}"), &base_opts("REGISTER", "udp", "2")),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("401"), "{err}");
    assert!(err.contains("authentication"), "{err}");
}

#[tokio::test]
async fn tcp_transaction_reads_framed_response() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&buf).to_string();
        assert!(request.contains("Via: SIP/2.0/TCP"), "{request}");
        let response = reply(&request, "SIP/2.0 200 OK", "");
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let res = execute(
        &cfg(),
        &req(format!("sip://{addr}"), &base_opts("OPTIONS", "tcp", "3")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.status, 200);
}

#[tokio::test]
async fn mismatched_call_id_does_not_complete_transaction() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let mut response = reply(&request, "SIP/2.0 200 OK", "");
        response = response.replace(CALL_ID, "someone-else@example.com");
        server.send_to(response.as_bytes(), peer).await.unwrap();
    });

    let err = execute(
        &cfg(),
        &req(format!("sip://{addr}"), &base_opts("OPTIONS", "udp", "4")),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req(
            "sip://127.0.0.1:5060".into(),
            &base_opts("OPTIONS", "udp", "1"),
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
