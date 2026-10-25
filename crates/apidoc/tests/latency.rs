#![cfg(feature = "axum")]
//! 本地响应延迟门槛：apidoc 自身路由 p99 < 10ms（不含服务端调试的目标网络耗时）。
//!
//! 用 in-process `ServiceExt::oneshot` 测量路由 + 序列化本地开销；预热后取 p99。
//! 外部依赖/调试目标的耗时不在本测试范围内。

use apidoc::auth::AuthConfig;
use apidoc::webhook::WebhookConfig;
use apidoc::ApidocConfig;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use axum::Router;
use serde_json::Value;
use std::hint::black_box;
use std::time::{Duration, Instant};
use tower::ServiceExt;

#[allow(dead_code)]
#[apidoc::title("List items")]
#[apidoc::url("/api/items")]
#[apidoc::method("GET")]
#[apidoc::param(name = "id", ty = "int", mock = "1")]
#[apidoc::returned(name = "name", ty = "string")]
fn list() {}

fn config() -> ApidocConfig {
    ApidocConfig {
        title: "perf".into(),
        auth: Some(AuthConfig {
            enable: true,
            password: "pw".into(),
            secret_key: "perf-test-key".into(),
            expire: 0,
        }),
        webhooks: vec![WebhookConfig {
            id: "hook".into(),
            title: "Hook".into(),
            secret: None,
            max_body: 0,
            capacity: 0,
        }],
        ..Default::default()
    }
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn p99(app: &Router, make: impl Fn() -> Request<Body>, iterations: usize) -> Duration {
    for _ in 0..10 {
        let _ = app.clone().oneshot(make()).await.unwrap();
    }
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let res = app.clone().oneshot(make()).await.unwrap();
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        black_box(&body);
        times.push(started.elapsed());
    }
    times.sort();
    times[(iterations - 1) * 99 / 100]
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
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "current_thread")]
async fn all_apidoc_routes_p99_under_10ms() {
    let app = apidoc::axum::apidoc_routes(config());
    let token = login(&app).await;
    let tok = format!("token={token}");

    let uris = [
        "/apidoc".to_string(),
        format!("/apidoc/api.json?{tok}"),
        format!("/apidoc/export?format=md&{tok}"),
        format!("/apidoc/export?format=ts&{tok}"),
        format!("/apidoc/export?format=swagger&{tok}"),
        format!("/apidoc/export?format=asyncapi&{tok}"),
        format!("/apidoc/mock?url=%2Fapi%2Fitems&method=GET&{tok}"),
        format!("/apidoc/generate?template=api.ts&{tok}"),
        format!("/apidoc/share?url=%2Fapi%2Fitems&method=GET&{tok}"),
        format!("/apidoc/webhook?{tok}"),
        format!("/apidoc/webhook/hook?{tok}"),
    ];
    for uri in uris {
        let p = p99(&app, || get(&uri), 100).await;
        assert!(
            p < Duration::from_millis(10),
            "{uri} p99 {p:?} exceeds 10ms"
        );
    }

    // 登录与关闭状态下的 debug 路由（本地 401/403，不含目标网络）。
    let auth_req = || {
        Request::builder()
            .method("POST")
            .uri("/apidoc/auth")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"password":"pw"}"#))
            .unwrap()
    };
    let p = p99(&app, auth_req, 50).await;
    assert!(p < Duration::from_millis(10), "auth p99 {p:?} exceeds 10ms");

    let debug_req = || {
        Request::builder()
            .method("POST")
            .uri(format!("/apidoc/debug?{tok}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"kind":"http","target":"http://127.0.0.1:1"}"#))
            .unwrap()
    };
    let p = p99(&app, debug_req, 50).await;
    assert!(p < Duration::from_millis(10), "debug 403 p99 {p:?} exceeds 10ms");
}
