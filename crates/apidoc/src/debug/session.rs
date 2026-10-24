//! P2 长连接会话桥：客户端（文档 UI）↔ 服务端协议执行器。
//!
//! 控制协议见总纲 §2.2：connect / send / close → open / frame / close / error。
//! 适配器只负责把框架的 WebSocket 包装成 `DebugSocket`；协议执行器实现
//! `StreamSession`，把目标协议的帧通过 mpsc 回传给会话循环。

use crate::debug::Payload;
use async_trait::async_trait;
use std::future::Future;
use tokio::sync::mpsc;

/// 每个连接最多转发的帧数（与 UI 的 MAX_FRAMES 对齐）。
pub const MAX_FRAMES: usize = 1000;

/// 适配器侧 socket。故意不约束 Send：actix-ws 的 MessageStream 是 !Send，
/// 而 axum 的 WebSocket 是 Send——Send 由具体适配器的 async fn 自动体现。
pub(crate) trait DebugSocket {
    async fn send_text(&mut self, text: String) -> Result<(), String>;
    async fn recv_text(&mut self) -> Option<Result<String, String>>;
    async fn close(&mut self);
}

#[async_trait]
pub trait StreamSession: Send {
    async fn send(&mut self, payload: Payload) -> Result<(), String>;
    async fn close(&mut self);
}

/// 生产入口：从 connect 帧解析请求，交给协议执行器。
pub(crate) async fn run(
    cfg: &crate::DebugConfig,
    ep: Option<&crate::DocEndpoint>,
    socket: impl DebugSocket,
) {
    let cfg = cfg.clone();
    let ep = ep.cloned();
    run_with(socket, move |req, frames| async move {
        super::stream_dispatch(&cfg, &req, ep.as_ref(), frames).await
    })
    .await;
}

/// 可注入的连接函数，供单测使用；生产路径由 `debug::stream_dispatch` 提供。
pub(crate) async fn run_with<S, F, Fut>(mut socket: S, connect: F)
where
    S: DebugSocket,
    F: FnOnce(super::DebugRequest, mpsc::Sender<String>) -> Fut,
    Fut: Future<Output = Result<Box<dyn StreamSession>, String>>,
{
    let Some(Ok(first)) = socket.recv_text().await else {
        return;
    };
    let first: serde_json::Value = match serde_json::from_str(&first) {
        Ok(v) => v,
        Err(e) => {
            send_error(&mut socket, &format!("invalid control frame: {e}")).await;
            return;
        }
    };
    if first.get("op").and_then(|v| v.as_str()) != Some("connect") {
        send_error(&mut socket, "first control frame must be {\"op\":\"connect\"}").await;
        return;
    }
    let req: super::DebugRequest = match serde_json::from_value(first) {
        Ok(r) => r,
        Err(e) => {
            send_error(&mut socket, &format!("invalid connect frame: {e}")).await;
            return;
        }
    };

    let (tx, mut rx) = mpsc::channel::<String>(32);
    let mut session = match connect(req, tx).await {
        Ok(s) => s,
        Err(e) => {
            send_error(&mut socket, &e).await;
            socket.close().await;
            return;
        }
    };
    if socket.send_text(r#"{"type":"open"}"#.to_string()).await.is_err() {
        session.close().await;
        socket.close().await;
        return;
    }

    let mut frames = 0usize;
    loop {
        tokio::select! {
            biased;
            out = rx.recv() => {
                match out {
                    Some(text) => {
                        if socket.send_text(text).await.is_err() {
                            break;
                        }
                        frames += 1;
                        if frames >= MAX_FRAMES {
                            send_error(&mut socket, "max_frames reached").await;
                            break;
                        }
                    }
                    None => break,
                }
            }
            incoming = socket.recv_text() => {
                match incoming {
                    Some(Ok(text)) => {
                        let value: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(e) => {
                                send_error(&mut socket, &format!("invalid control frame: {e}")).await;
                                continue;
                            }
                        };
                        match value.get("op").and_then(|v| v.as_str()) {
                            Some("send") => {
                                let payload: Payload = match serde_json::from_value(value) {
                                    Ok(p) => p,
                                    Err(e) => {
                                        send_error(&mut socket, &format!("invalid send payload: {e}")).await;
                                        continue;
                                    }
                                };
                                if let Err(e) = session.send(payload).await {
                                    send_error(&mut socket, &e).await;
                                }
                            }
                            Some("close") => break,
                            _ => send_error(&mut socket, "unknown op; expected send or close").await,
                        }
                    }
                    Some(Err(e)) => {
                        send_error(&mut socket, &e).await;
                        break;
                    }
                    None => break,
                }
            }
        }
    }
    // select 竞态兜底：关闭前把已就绪的出站帧发完。
    while let Ok(text) = rx.try_recv() {
        if socket.send_text(text).await.is_err() {
            break;
        }
    }
    session.close().await;
    let _ = socket
        .send_text(r#"{"type":"close","code":1000,"reason":"client close"}"#.to_string())
        .await;
    socket.close().await;
}

async fn send_error<S: DebugSocket>(socket: &mut S, msg: &str) {
    let text = serde_json::json!({ "type": "error", "error": msg }).to_string();
    let _ = socket.send_text(text).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::Payload;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    struct MockSocket {
        input: VecDeque<String>,
        output: Arc<Mutex<Vec<String>>>,
    }

    impl DebugSocket for MockSocket {
        async fn send_text(&mut self, text: String) -> Result<(), String> {
            self.output.lock().unwrap().push(text);
            Ok(())
        }
        async fn recv_text(&mut self) -> Option<Result<String, String>> {
            self.input.pop_front().map(Ok)
        }
        async fn close(&mut self) {}
    }

    struct EchoSession {
        frames: mpsc::Sender<String>,
    }

    #[async_trait]
    impl StreamSession for EchoSession {
        async fn send(&mut self, payload: Payload) -> Result<(), String> {
            let msg = serde_json::json!({
                "type": "frame",
                "dir": "in",
                "encoding": payload.encoding,
                "data": payload.data,
                "ts": 0
            })
            .to_string();
            self.frames.send(msg).await.map_err(|e| e.to_string())
        }
        async fn close(&mut self) {}
    }

    #[tokio::test]
    async fn session_connect_send_frame_close() {
        let out = Arc::new(Mutex::new(Vec::new()));
        let socket = MockSocket {
            input: VecDeque::from(vec![
                r#"{"op":"connect","kind":"websocket","target":"ws://127.0.0.1:1/x"}"#.to_string(),
                r#"{"op":"send","encoding":"utf8","data":"hi"}"#.to_string(),
                r#"{"op":"close"}"#.to_string(),
            ]),
            output: out.clone(),
        };
        run_with(socket, |_req, frames| async move {
            Ok(Box::new(EchoSession { frames }) as Box<dyn StreamSession>)
        })
        .await;
        let lines = out.lock().unwrap();
        assert_eq!(lines.first().map(String::as_str), Some(r#"{"type":"open"}"#));
        assert!(
            lines.iter().any(|l| l.contains(r#""type":"frame""#) && l.contains(r#""data":"hi""#)),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains(r#""type":"close""#)), "{lines:?}");
    }
}
