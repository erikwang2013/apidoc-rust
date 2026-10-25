#![cfg(feature = "debug-media")]
//! P5 RTSP：本地回环 OPTIONS/DESCRIBE/SETUP/PLAY、CSeq 校验、401 明确报错与读超时。

use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::DebugConfig;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn req(target: String, opts: &[(&str, &str)], body: Option<&str>) -> DebugRequest {
    DebugRequest {
        kind: "rtsp".into(),
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
        timeout_ms: Some(2000),
    }
}

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0, "client closed before headers");
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return String::from_utf8_lossy(&buf).to_string();
        }
    }
}

fn cseq_of(request: &str) -> String {
    request
        .lines()
        .find_map(|line| line.strip_prefix("CSeq: "))
        .unwrap_or("1")
        .trim()
        .to_string()
}

/// 一次性 RTSP 服务器：回一个固定响应后关闭。
async fn serve_once(
    response: impl FnOnce(&str) -> String + Send + 'static,
) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        stream
            .write_all(response(&request).as_bytes())
            .await
            .unwrap();
    });
    addr
}

#[tokio::test]
async fn options_roundtrip_and_cseq_echo() {
    let addr = serve_once(|request| {
        assert!(request.starts_with("OPTIONS rtsp://"), "{request}");
        assert!(request.contains("User-Agent: apidoc-rust"));
        format!(
            "RTSP/1.0 200 OK\r\nCSeq: {}\r\nPublic: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN\r\n\r\n",
            cseq_of(request)
        )
    })
    .await;

    let res = execute(
        &cfg(),
        &req(
            format!("rtsp://{addr}/stream"),
            &[("method", "OPTIONS")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.status, 200);
    assert!(
        res.headers["Public"].contains("DESCRIBE"),
        "{:?}",
        res.headers
    );
    assert!(res.body.data.is_empty());
}

#[tokio::test]
async fn describe_returns_sdp_body() {
    let sdp = "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=test\r\nm=video 0 RTP/AVP 96\r\n";
    let sdp_for_server = sdp.to_string();
    let addr = serve_once(move |request| {
        assert!(request.starts_with("DESCRIBE rtsp://"), "{request}");
        format!(
            "RTSP/1.0 200 OK\r\nCSeq: {}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
            cseq_of(request),
            sdp_for_server.len(),
            sdp_for_server
        )
    })
    .await;

    let res = execute(
        &cfg(),
        &req(
            format!("rtsp://{addr}/stream"),
            &[("method", "DESCRIBE"), ("cseq", "7")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.status, 200);
    assert_eq!(res.body.data, sdp);
    assert_eq!(res.headers["Content-Type"], "application/sdp");
}

#[tokio::test]
async fn setup_sends_transport_and_play_sends_session() {
    let addr = serve_once(|request| {
        assert!(request.contains("Session: 12345678"), "{request}");
        format!(
            "RTSP/1.0 200 OK\r\nCSeq: {}\r\nSession: 12345678\r\n\r\n",
            cseq_of(request)
        )
    })
    .await;
    let res = execute(
        &cfg(),
        &req(
            format!("rtsp://{addr}/stream"),
            &[("method", "PLAY"), ("session", "12345678")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.status, 200);
    assert_eq!(res.headers["Session"], "12345678");

    let addr = serve_once(|request| {
        assert!(
            request.contains("Transport: RTP/AVP/TCP;interleaved=0-1"),
            "{request}"
        );
        assert!(request.starts_with("SETUP rtsp://"), "{request}");
        format!("RTSP/1.0 200 OK\r\nCSeq: {}\r\n\r\n", cseq_of(request))
    })
    .await;
    let res = execute(
        &cfg(),
        &req(
            format!("rtsp://{addr}/stream"),
            &[("method", "SETUP")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.status, 200);
}

#[tokio::test]
async fn unauthorized_reports_authentication_unsupported() {
    let addr = serve_once(|request| {
        format!(
            "RTSP/1.0 401 Unauthorized\r\nCSeq: {}\r\nWWW-Authenticate: Digest realm=\"x\", nonce=\"y\"\r\nContent-Length: 0\r\n\r\n",
            cseq_of(request)
        )
    })
    .await;
    let err = execute(
        &cfg(),
        &req(
            format!("rtsp://{addr}/stream"),
            &[("method", "DESCRIBE")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("401"), "{err}");
    assert!(err.contains("authentication"), "{err}");
}

#[tokio::test]
async fn silent_server_times_out_without_hanging() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
    });
    let started = Instant::now();
    let err = execute(
        &cfg(),
        &req(
            format!("rtsp://{addr}/stream"),
            &[("method", "OPTIONS")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn mismatched_cseq_is_rejected() {
    let addr = serve_once(|_| "RTSP/1.0 200 OK\r\nCSeq: 99\r\n\r\n".to_string()).await;
    let err = execute(
        &cfg(),
        &req(
            format!("rtsp://{addr}/stream"),
            &[("method", "OPTIONS")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("CSeq"), "{err}");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("rtsp://127.0.0.1:554/stream".into(), &[], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
