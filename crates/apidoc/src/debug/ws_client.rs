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
use tokio::sync::mpsc;
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
    guard::check(cfg, &req.target).await?;
    // shortcut: connect_async 会再次解析 DNS；guard 已校验解析结果，但连接时重解析
    // 存在窄 TOCTOU 窗口。P6 改成先连已校验 IP 再手写握手。
    let (ws, _) = tokio_tungstenite::connect_async(req.target.as_str())
        .await
        .map_err(|e| format!("ws connect failed: {e}"))?;
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
