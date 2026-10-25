#![cfg(all(feature = "axum", feature = "debug-session"))]
//! P2 会话桥集成测试：apidoc WS 路由 ↔ 本地 WS echo 目标。
//! 控制协议：connect / send / close → open / frame / close。

use apidoc::auth::AuthConfig;
use apidoc::{ApidocConfig, DebugConfig};
use axum::body::{to_bytes, Body};
use axum::http::Request;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

async fn start_echo_ws() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(msg)) = ws.next().await {
            if ws.send(msg).await.is_err() {
                break;
            }
        }
    });
    addr
}

async fn login(app: &Router) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/apidoc/auth")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"pw"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

async fn next_json(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> Value {
    loop {
        let msg = ws.next().await.expect("ws closed").expect("ws error");
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).expect("control frame must be JSON");
        }
    }
}

async fn start_doc_server() -> (std::net::SocketAddr, String) {
    let app = apidoc::axum::apidoc_routes(ApidocConfig {
        title: "t".into(),
        auth: Some(AuthConfig {
            enable: true,
            password: "pw".into(),
            secret_key: "debug-session-test".into(),
            expire: 0,
        }),
        debug: Some(DebugConfig {
            enable_server_side: true,
            allow_private_networks: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    let token = login(&app).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, token)
}

#[tokio::test]
async fn session_bridge_forwards_ws_frames() {
    let echo_addr = start_echo_ws().await;
    let (addr, token) = start_doc_server().await;

    let url = format!("ws://{addr}/apidoc/debug/stream?token={token}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    ws.send(Message::Text(
        json!({
            "op": "connect",
            "kind": "websocket",
            "target": format!("ws://{echo_addr}/")
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let open = next_json(&mut ws).await;
    assert_eq!(open["type"], "open");

    ws.send(Message::Text(
        json!({"op":"send","encoding":"utf8","data":"hello"}).to_string(),
    ))
    .await
    .unwrap();
    let frame = next_json(&mut ws).await;
    assert_eq!(frame["type"], "frame");
    assert_eq!(frame["dir"], "in");
    assert_eq!(frame["data"], "hello");

    ws.send(Message::Text(json!({"op":"close"}).to_string()))
        .await
        .unwrap();
    let close = next_json(&mut ws).await;
    assert_eq!(close["type"], "close");
}

/// 目标持续推帧：到 `session::MAX_FRAMES` 必须回 error 并关闭目标连接。
#[tokio::test]
async fn session_caps_frames_and_closes_target() {
    use std::time::Duration;
    use tokio::sync::oneshot;

    const MAX_FRAMES: usize = 1000; // 与 crates/apidoc/src/debug/session.rs 对齐
    let flood = MAX_FRAMES + 100;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_addr = listener.local_addr().unwrap();
    let (closed_tx, closed_rx) = oneshot::channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        for i in 0..flood {
            if ws.send(Message::Text(format!("f{i}"))).await.is_err() {
                break;
            }
        }
        // 帧发完后等会话侧关闭；close 帧或 EOF 都算释放。
        while let Some(Ok(_)) = ws.next().await {}
        let _ = closed_tx.send(());
    });

    let (addr, token) = start_doc_server().await;
    let url = format!("ws://{addr}/apidoc/debug/stream?token={token}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(Message::Text(
        json!({
            "op": "connect",
            "kind": "websocket",
            "target": format!("ws://{target_addr}/")
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let mut frames = 0usize;
    let mut saw_cap = false;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let msg = ws.next().await.expect("ws closed").expect("ws error");
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).expect("control frame must be JSON");
            match v["type"].as_str() {
                Some("frame") => frames += 1,
                Some("error") => {
                    saw_cap = v["error"].as_str().is_some_and(|e| e.contains("max_frames"));
                }
                Some("close") => break,
                _ => {}
            }
        }
    })
    .await
    .expect("session did not stop after max_frames");

    assert!(saw_cap, "missing max_frames error");
    assert!(
        (MAX_FRAMES..flood).contains(&frames),
        "forwarded {frames} frames, expected cap between {MAX_FRAMES} and {flood}"
    );
    tokio::time::timeout(Duration::from_secs(5), closed_rx)
        .await
        .expect("target connection was not closed")
        .expect("flood task ended without signalling");
}
