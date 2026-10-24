#![cfg(all(feature = "axum", feature = "debug-http"))]
//! P0 服务端调试路由：鉴权、转发、SSRF 红线。
//! actix 侧路由是同一 `Core::debug` 的一行转发，行为由共享实现保证。

use apidoc::auth::AuthConfig;
use apidoc::{ApidocConfig, DebugConfig};
use axum::body::{to_bytes, Body};
use axum::http::Request;
use axum::routing::post;
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn echo(body: axum::body::Bytes) -> axum::response::Response {
    axum::response::Response::builder()
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}

async fn start_echo() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/echo", post(echo))).await.unwrap();
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

fn debug_app() -> Router {
    apidoc::axum::apidoc_routes(ApidocConfig {
        title: "t".into(),
        auth: Some(AuthConfig {
            enable: true,
            password: "pw".into(),
            secret_key: "debug-test-key".into(),
            expire: 0,
        }),
        debug: Some(DebugConfig {
            enable_server_side: true,
            allow_private_networks: true,
            ..Default::default()
        }),
        ..Default::default()
    })
}

async fn post_debug(app: &Router, token: &str, payload: Value) -> (u16, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/apidoc/debug?token={token}"))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn disabled_route_returns_403() {
    let app = apidoc::axum::apidoc_routes(ApidocConfig::default());
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/apidoc/debug")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}

#[tokio::test]
async fn forwards_http_request_and_returns_normalized_response() {
    let addr = start_echo().await;
    let app = debug_app();
    let token = login(&app).await;
    let (status, v) = post_debug(
        &app,
        &token,
        json!({
            "kind": "http",
            "target": format!("http://{addr}/echo"),
            "method": "POST",
            "headers": { "content-type": "application/json" },
            "body": { "encoding": "utf8", "data": "{\"hello\":\"world\"}" }
        }),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["status"], 200);
    assert_eq!(v["body"]["encoding"], "utf8");
    assert_eq!(v["body"]["data"], "{\"hello\":\"world\"}");
}

#[tokio::test]
async fn metadata_address_stays_blocked_even_when_private_networks_allowed() {
    let app = debug_app();
    let token = login(&app).await;
    let (status, v) = post_debug(
        &app,
        &token,
        json!({
            "kind": "http",
            "target": "http://169.254.169.254/latest/meta-data",
            "method": "GET"
        }),
    )
    .await;
    assert_eq!(status, 502, "{v}");
    assert!(v["error"].as_str().unwrap().contains("blocked"), "{v}");
}

#[tokio::test]
async fn debug_route_requires_token() {
    let addr = start_echo().await;
    let app = debug_app();
    let (status, _) = post_debug(
        &app,
        "not-a-token",
        json!({ "kind": "http", "target": format!("http://{addr}/echo"), "method": "GET" }),
    )
    .await;
    assert_eq!(status, 401);
}
