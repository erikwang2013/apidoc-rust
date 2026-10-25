#![cfg(feature = "debug-sockets")]
//! P3 STOMP over TCP：本地 mock server 的 UTF-8/二进制 MESSAGE、头转义、
//! 收集窗口与 SSRF 默认拒绝。

use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::DebugConfig;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn req(target: String, opts: &[(&str, &str)], body: Option<&str>) -> DebugRequest {
    DebugRequest {
        kind: "stomp".into(),
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

/// 独立于被测编码器的 mock server 读取器：故意不反转义，便于断言线上字节。
struct ServerReader {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl ServerReader {
    fn new(stream: TcpStream) -> Self {
        ServerReader {
            stream,
            buf: Vec::new(),
        }
    }

    async fn fill(&mut self) -> bool {
        let mut chunk = [0u8; 4096];
        let n = self.stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            return false;
        }
        self.buf.extend_from_slice(&chunk[..n]);
        true
    }

    async fn read_frame(&mut self) -> (String, BTreeMap<String, String>, Vec<u8>) {
        let head_end = loop {
            if let Some(pos) = self.buf.windows(2).position(|w| w == b"\n\n") {
                break pos;
            }
            assert!(self.fill().await, "client closed before frame headers");
        };
        let head: Vec<u8> = self.buf.drain(..head_end + 2).collect();
        let text = String::from_utf8(head[..head_end].to_vec()).unwrap();
        let mut lines = text.split('\n');
        let command = lines.next().unwrap().to_string();
        let mut headers = BTreeMap::new();
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.to_string(), value.to_string());
            }
        }
        let body = match headers
            .get("content-length")
            .and_then(|v| v.parse::<usize>().ok())
        {
            Some(len) => {
                while self.buf.len() < len + 1 {
                    assert!(self.fill().await, "client closed before body");
                }
                let body = self.buf.drain(..len).collect();
                assert_eq!(self.buf.drain(..1).collect::<Vec<_>>(), vec![0]);
                body
            }
            None => {
                let pos = loop {
                    if let Some(pos) = self.buf.iter().position(|b| *b == 0) {
                        break pos;
                    }
                    assert!(self.fill().await, "client closed before NUL");
                };
                let body = self.buf.drain(..pos).collect();
                self.buf.drain(..1);
                body
            }
        };
        (command, headers, body)
    }

    async fn send(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).await.unwrap();
    }
}

#[tokio::test]
async fn stomp_messages_utf8_binary_and_header_escaping() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut r = ServerReader::new(stream);

        let (command, headers, body) = r.read_frame().await;
        assert_eq!(command, "CONNECT");
        assert!(body.is_empty());
        assert_eq!(
            headers.get("accept-version").map(String::as_str),
            Some("1.2")
        );
        assert_eq!(headers.get("login").map(String::as_str), Some("demo"));
        // passcode 里的 ':' 必须转义成 `\c`。
        assert_eq!(
            headers.get("passcode").map(String::as_str),
            Some(r"se\ccret")
        );
        r.send(b"CONNECTED\nversion:1.2\n\n\0").await;

        let (command, headers, body) = r.read_frame().await;
        assert_eq!(command, "SUBSCRIBE");
        assert_eq!(
            headers.get("destination").map(String::as_str),
            Some("/queue/a")
        );
        assert!(body.is_empty());

        let (command, headers, body) = r.read_frame().await;
        assert_eq!(command, "SEND");
        assert_eq!(body, b"ping");
        // 请求 header 里的 ':' / 换行必须转义。
        assert_eq!(headers.get("x-note").map(String::as_str), Some(r"a\cb\nc"));

        let mut message = Vec::new();
        message.extend_from_slice(
            b"MESSAGE\ndestination:/queue/a\nx-note:line1\\nline2\\ccolon\ncontent-length:5\n\n",
        );
        message.extend_from_slice(b"hello\0");
        r.send(&message).await;

        let mut binary = Vec::new();
        binary.extend_from_slice(b"MESSAGE\ndestination:/queue/a\ncontent-length:3\n\n");
        binary.extend_from_slice(&[0xff, 0x00, 0x80, 0]);
        r.send(&binary).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    });

    let mut request = req(
        format!("tcp://{addr}"),
        &[
            ("destination", "/queue/a"),
            ("login", "demo"),
            ("passcode", "se:cret"),
            ("messages", "2"),
            ("duration_ms", "1000"),
        ],
        Some("ping"),
    );
    request.headers.insert("x-note".into(), "a:b\nc".into());

    let res = execute(&allow_private(), &request, None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&res.body.data).unwrap();
    let messages = v["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["destination"], "/queue/a");
    assert_eq!(messages[0]["payload"]["encoding"], "utf8");
    assert_eq!(messages[0]["payload"]["data"], "hello");
    assert_eq!(messages[0]["headers"]["x-note"], "line1\nline2:colon");
    assert_eq!(messages[1]["payload"]["encoding"], "base64");
    let decoded = base64_decode(messages[1]["payload"]["data"].as_str().unwrap());
    assert_eq!(decoded, vec![0xff, 0x00, 0x80]);
    server.await.unwrap();
}

#[tokio::test]
async fn stomp_collection_window_returns_empty_without_messages() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut r = ServerReader::new(stream);
        let (command, _, _) = r.read_frame().await;
        assert_eq!(command, "CONNECT");
        r.send(b"CONNECTED\nversion:1.2\n\n\0").await;
        let (command, _, _) = r.read_frame().await;
        assert_eq!(command, "SUBSCRIBE");
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let start = Instant::now();
    let res = execute(
        &allow_private(),
        &req(
            format!("tcp://{addr}"),
            &[("destination", "/queue/a"), ("duration_ms", "100")],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    let v: serde_json::Value = serde_json::from_str(&res.body.data).unwrap();
    assert!(v["messages"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn stomp_missing_destination_is_reported() {
    let err = execute(
        &allow_private(),
        &req("tcp://127.0.0.1:1".into(), &[], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("destination"), "{err}");
}

#[tokio::test]
async fn stomp_loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req(
            "tcp://127.0.0.1:61613".into(),
            &[("destination", "/queue/a")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}

#[tokio::test]
async fn stomp_ws_transport_is_rejected_by_server_executor() {
    let err = execute(
        &allow_private(),
        &req(
            "tcp://127.0.0.1:61613".into(),
            &[("destination", "/queue/a"), ("transport", "ws")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("use tcp"), "{err}");
}

#[tokio::test]
async fn stomp_target_credentials_are_rejected() {
    let err = execute(
        &allow_private(),
        &req(
            "tcp://user:topsecret@127.0.0.1:61613".into(),
            &[("destination", "/queue/a")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("via opts"), "{err}");
    assert!(!err.contains("topsecret"), "{err}");
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}
