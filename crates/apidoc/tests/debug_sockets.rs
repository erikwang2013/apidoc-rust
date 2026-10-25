#![cfg(feature = "debug-sockets")]
//! P2 原始 socket：TCP/UDP/Raw/Unix 回环、SSRF 默认拒绝与读超时。

use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::DebugConfig;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

fn req(kind: &str, target: String, body: Option<&str>, opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: kind.into(),
        target,
        method: None,
        headers: Default::default(),
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

async fn tcp_echo() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await.unwrap();
        stream.write_all(&buf).await.unwrap();
    });
    addr
}

fn allow_private() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn tcp_echo_roundtrip() {
    let addr = tcp_echo().await;
    let res = execute(
        &allow_private(),
        &req(
            "tcp",
            format!("tcp://{addr}"),
            Some("ping"),
            &[("expect_bytes", "4")],
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.status, 0);
    assert_eq!(res.body.data, "ping");
}

#[tokio::test]
async fn raw_tcp_uses_same_path() {
    let addr = tcp_echo().await;
    let res = execute(
        &allow_private(),
        &req(
            "raw",
            format!("tcp://{addr}"),
            Some("ping"),
            &[("transport", "tcp"), ("expect_bytes", "4")],
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.body.data, "ping");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("tcp", "tcp://127.0.0.1:9".into(), None, &[]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}

#[tokio::test]
async fn udp_echo_roundtrip() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 16];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        server.send_to(&buf[..n], peer).await.unwrap();
    });
    let res = execute(
        &allow_private(),
        &req(
            "udp",
            format!("udp://{addr}"),
            Some("pong"),
            &[("read_ms", "500")],
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.body.data, "pong");
}

#[tokio::test]
async fn tcp_read_timeout_returns_empty_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let start = Instant::now();
    let res = execute(
        &allow_private(),
        &req(
            "tcp",
            format!("tcp://{addr}"),
            Some("x"),
            &[("read_ms", "100")],
        ),
        None,
    )
    .await
    .unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(res.body.data.is_empty());
}

/// UDP 与 TCP 共用 check_host：默认拒绝回环，显式允许后同一目标可通。
#[tokio::test]
async fn udp_loopback_denied_by_default_then_allowed_when_private() {
    let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 16];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        server.send_to(&buf[..n], peer).await.unwrap();
    });

    let err = execute(
        &DebugConfig::default(),
        &req(
            "udp",
            format!("udp://{addr}"),
            Some("pong"),
            &[("read_ms", "100")],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");

    let res = execute(
        &allow_private(),
        &req(
            "udp",
            format!("udp://{addr}"),
            Some("pong"),
            &[("read_ms", "500")],
        ),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.body.data, "pong");
}

#[cfg(unix)]
#[tokio::test]
async fn unix_stream_echo_and_default_deny() {
    use tokio::net::UnixListener;

    let path = std::env::temp_dir().join(format!("apidoc-debug-test-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await.unwrap();
        stream.write_all(&buf).await.unwrap();
    });
    let target = format!("unix://{}", path.display());

    let err = execute(
        &allow_private(),
        &req("unix", target.clone(), Some("ping"), &[("expect_bytes", "4")]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("unix"), "{err}");

    let cfg = DebugConfig {
        allow_private_networks: true,
        allow_unix_sockets: true,
        ..Default::default()
    };
    let res = execute(
        &cfg,
        &req("unix", target, Some("ping"), &[("expect_bytes", "4")]),
        None,
    )
    .await
    .unwrap();
    assert_eq!(res.body.data, "ping");

    let _ = std::fs::remove_file(&path);
}
