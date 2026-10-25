//! 服务端 WebSocket 客户端执行器：连接目标 WS，把目标帧转成会话桥的 frame 消息。
//! P2 只支持 ws/wss；自定义握手 header 暂不支持（P6 再做自定义 client request）。

use super::guard;
use super::session::StreamSession;
use super::{DebugRequest, Payload};
use crate::DebugConfig;
use async_trait::async_trait;
use base64::Engine;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct WsSession {
    sink: SplitSink<WsStream, Message>,
}

#[async_trait]
impl StreamSession for WsSession {
    async fn send(&mut self, payload: Payload) -> Result<(), String> {
        let msg = if payload.encoding == "utf8" {
            Message::Text(payload.data)
        } else {
            Message::Binary(payload.decode()?)
        };
        self.sink.send(msg).await.map_err(|e| e.to_string())
    }

    async fn close(&mut self) {
        let _ = self.sink.close().await;
    }
}

pub async fn connect_stream(
    cfg: &DebugConfig,
    req: &DebugRequest,
    frames: mpsc::Sender<String>,
) -> Result<Box<dyn StreamSession>, String> {
    let checked = guard::check(cfg, &req.target).await?;
    if req.target.starts_with("wss://") {
        return Err("wss requires pinned TLS; not supported in this build (use ws://)".into());
    }
    // 手工连到已校验 IP，再在原始 URL 的 Host 语义上完成 WS 握手（防 DNS rebinding）。
    let mut request = req
        .target
        .as_str()
        .into_client_request()
        .map_err(|e| format!("invalid ws target: {e}"))?;
    for (key, value) in &req.headers {
        let name = tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(key.as_bytes())
            .map_err(|e| format!("invalid ws header name `{key}`: {e}"))?;
        let value = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(value)
            .map_err(|e| format!("invalid ws header value for `{key}`: {e}"))?;
        request.headers_mut().insert(name, value);
    }
    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let tcp = tokio::time::timeout(connect_timeout, tokio::net::TcpStream::connect(checked.addrs[0]))
        .await
        .map_err(|_| "ws connect timed out".to_string())?
        .map_err(|e| format!("ws connect failed: {e}"))?;
    let (ws, _) = tokio_tungstenite::client_async(request, MaybeTlsStream::Plain(tcp))
        .await
        .map_err(|e| format!("ws handshake failed: {e}"))?;
    let (sink, mut stream) = ws.split();
    let tx = frames.clone();
    tokio::spawn(async move {
        while let Some(item) = stream.next().await {
            let value = match item {
                Ok(Message::Text(text)) => serde_json::json!({
                    "type": "frame",
                    "dir": "in",
                    "encoding": "utf8",
                    "data": text.to_string(),
                    "ts": now_ms(),
                }),
                Ok(Message::Binary(bytes)) => serde_json::json!({
                    "type": "frame",
                    "dir": "in",
                    "encoding": "base64",
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                    "ts": now_ms(),
                }),
                Ok(Message::Close(_)) => {
                    let _ = tx
                        .send(
                            serde_json::json!({
                                "type": "close", "code": 1000, "reason": "target closed"
                            })
                            .to_string(),
                        )
                        .await;
                    break;
                }
                Ok(_) => continue,
                Err(e) => {
                    let _ = tx
                        .send(serde_json::json!({ "type": "error", "error": e.to_string() }).to_string())
                        .await;
                    break;
                }
            };
            if tx.send(value.to_string()).await.is_err() {
                break;
            }
        }
    });
    Ok(Box::new(WsSession { sink }))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wss_fails_closed_without_pinned_tls() {
        let req = DebugRequest {
            kind: "websocket".into(),
            target: "wss://127.0.0.1:1/channel".into(),
            method: None,
            headers: Default::default(),
            body: None,
            opts: Default::default(),
            endpoint: None,
            timeout_ms: None,
        };
        let (frames, _rx) = mpsc::channel(1);
        let cfg = DebugConfig {
            allow_private_networks: true,
            ..Default::default()
        };
        let err = connect_stream(&cfg, &req, frames)
            .await
            .err()
            .expect("wss must fail closed");
        assert!(err.contains("wss") && err.contains("TLS"), "{err}");
    }
}
