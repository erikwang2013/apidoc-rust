//! I1: UIC-1 debug must be scoped to the endpoint's own app, not the client appKey.
#![cfg(feature = "axum")]

use apidoc::axum::apidoc_routes;
use apidoc::{ApidocConfig, AppConfig};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

#[allow(dead_code)]
#[apidoc::app("A")]
#[apidoc::title("A endpoint")]
#[apidoc::url("/a")]
#[apidoc::method("GET")]
fn a_ep() {}

#[allow(dead_code)]
#[apidoc::app("B")]
#[apidoc::title("B endpoint")]
#[apidoc::url("/b")]
#[apidoc::method("GET")]
fn b_ep() {}

fn app() -> axum::Router {
    apidoc_routes(ApidocConfig {
        title: "Scoped".into(),
        apps: vec![
            AppConfig {
                key: "A".into(),
                title: "A".into(),
                items: Vec::new(),
                password: Some("pa".into()),
            },
            AppConfig {
                key: "B".into(),
                title: "B".into(),
                items: Vec::new(),
                password: Some("pb".into()),
            },
        ],
        ..Default::default()
    })
}

async fn send(
    app: &axum::Router,
    path: &str,
    body: Value,
    token: &str,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if !token.is_empty() {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn login(app: &axum::Router, app_key: &str, password: &str) -> String {
    let (status, body) = send(
        app,
        "/apidoc/ui/v1/verifyAuth",
        json!({"appKey": app_key, "password": password}),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"]["token"].as_str().unwrap().to_string()
}

async fn first_menu_key(app: &axum::Router, app_key: &str, token: &str) -> String {
    let (status, menus) = send(
        app,
        "/apidoc/ui/v1/menus",
        json!({"appKey": app_key}),
        token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{menus}");
    menus["data"]["data"][0]["children"][0]["menuKey"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn debug_rejects_cross_app_and_mismatched_app_key() {
    let app = app();
    let token_a = login(&app, "A", "pa").await;
    let token_b = login(&app, "B", "pb").await;
    let a_id = first_menu_key(&app, "A", &token_a).await;
    let b_id = first_menu_key(&app, "B", &token_b).await;

    for endpoint in ["debugSend", "debugStart"] {
        let path = format!("/apidoc/ui/v1/{endpoint}");

        // 令牌 A + 端点 B（客户端如实报 appKey=B）→ 401：守卫绑定端点自己的应用。
        let (status, resp) = send(
            &app,
            &path,
            json!({"appKey": "B", "path": b_id, "input": {}}),
            &token_a,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{endpoint}: cross-app token accepted: {resp}");

        // 令牌 A + 端点 B 但客户端 appKey 谎报 A → 404：不一致即拒绝。
        let (status, resp) = send(
            &app,
            &path,
            json!({"appKey": "A", "path": b_id, "input": {}}),
            &token_a,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{endpoint}: mismatched appKey accepted: {resp}");

        // 令牌 A + 端点 A → 守卫通过；调试未启用/未编译时仍是 ok:false 的 200。
        let (status, resp) = send(
            &app,
            &path,
            json!({"appKey": "A", "path": a_id, "input": {}}),
            &token_a,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{endpoint}: same-app token rejected: {resp}");
        assert_eq!(resp["data"]["ok"], false);
    }
}
