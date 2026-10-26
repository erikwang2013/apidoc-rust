#![cfg(feature = "axum")]

use apidoc::auth::AuthConfig;
use apidoc::axum::apidoc_routes;
use apidoc::{ApidocConfig, AppConfig};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

#[allow(dead_code)]
#[apidoc::title("Items")]
#[apidoc::url("/items")]
#[apidoc::method("GET")]
#[apidoc::param(name = "page", ty = "int", desc = "page", default = "1")]
#[apidoc::returned(name = "id", ty = "int", required, desc = "id")]
#[apidoc::md("# Items")]
fn items() {}

#[allow(dead_code)]
#[apidoc::title("Chat")]
#[apidoc::protocol("websocket", target = "wss://example.com/chat", room = "lobby")]
#[apidoc::event(direction = "out", name = "join", ty = "json", desc = "加入房间")]
fn chat() {}

fn app() -> axum::Router {
    apidoc_routes(ApidocConfig {
        title: "Demo".into(),
        auth: Some(AuthConfig {
            enable: true,
            password: "pw".into(),
            secret_key: "s".into(),
            expire: 3600,
        }),
        ..Default::default()
    })
}

async fn send(app: &axum::Router, method: &str, path: &str, body: &str, token: &str) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
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

async fn login(app: &axum::Router) -> String {
    let (status, body) = send(app, "POST", "/apidoc/ui/v1/verifyAuth", r#"{"password":"pw"}"#, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"]["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn capabilities_are_public() {
    let (status, body) = send(&app(), "GET", "/apidoc/ui/v1/capabilities", "", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["contract"], "1.0");
    assert_eq!(body["data"]["backend"], "rust");
}

#[tokio::test]
async fn auth_menus_and_detail() {
    let app = app();
    let token = login(&app).await;

    let (status, menus) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/menus",
        r#"{"appKey":"default"}"#,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{menus}");
    let children = menus["data"]["data"][0]["children"].as_array().expect("children");
    let http_id = children
        .iter()
        .find(|item| item["url"] == "/items")
        .and_then(|item| item["menuKey"].as_str())
        .expect("http menuKey")
        .to_string();

    let (status, detail) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/detail",
        &json!({"appKey":"default","path":http_id}).to_string(),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["data"]["protocol"]["name"], "http");
    assert_eq!(detail["data"]["debug"]["mode"], "browser-http");

    let ws_id = children
        .iter()
        .find(|item| item["url"] == "wss://example.com/chat")
        .and_then(|item| item["menuKey"].as_str())
        .expect("ws menuKey")
        .to_string();
    let (status, ws) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/detail",
        &json!({"appKey":"default","path":ws_id}).to_string(),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ws}");
    assert_eq!(ws["data"]["debug"]["mode"], "browser-websocket");
    assert!(ws["data"]["definitions"].as_array().is_some_and(|d| !d.is_empty()));
}

#[tokio::test]
async fn docs_and_auth_failures() {
    let app = app();
    let token = login(&app).await;
    let (status, menus) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/docMenus",
        r#"{"appKey":"default"}"#,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{menus}");
    let id = menus["data"][0]["children"][0]["path"].as_str().unwrap().to_string();
    let (status, doc) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/docDetail",
        &json!({"appKey":"default","path":id}).to_string(),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert!(doc["data"]["content"].as_str().unwrap().contains("# Items"));

    let (status, _) = send(&app, "GET", "/apidoc/ui/v1/config", "", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/verifyAuth",
        r#"{"password":"bad"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// C2: 原生 /apidoc/debug 必须保持全局密码独占——应用密码令牌不得调用。
#[tokio::test]
async fn native_debug_requires_global_token() {
    let app = apidoc_routes(ApidocConfig {
        title: "Demo".into(),
        auth: Some(AuthConfig {
            enable: true,
            password: "pw".into(),
            secret_key: "s".into(),
            expire: 3600,
        }),
        apps: vec![AppConfig {
            key: "A".into(),
            title: "A".into(),
            items: Vec::new(),
            password: Some("app-pw".into()),
        }],
        ..Default::default()
    });

    let (status, body) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/verifyAuth",
        r#"{"password":"app-pw","appKey":"A"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let app_token = body["data"]["token"].as_str().unwrap();
    let global_token = login(&app).await;

    let (status, _) = send(
        &app,
        "POST",
        &format!("/apidoc/debug?appKey=A&token={app_token}"),
        "{}",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "app token must not reach native debug");

    // 全局令牌通过守卫后落到"服务端调试未启用"（403），证明守卫本身放行。
    let (status, _) = send(
        &app,
        "POST",
        &format!("/apidoc/debug?appKey=A&token={global_token}"),
        "{}",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// 未知分享必须 404，不能回落到全局密码悄悄签发普通令牌。
#[tokio::test]
async fn verify_auth_rejects_unknown_share_key() {
    let (status, body) = send(
        &app(),
        "POST",
        "/apidoc/ui/v1/verifyAuth",
        r#"{"password":"pw","shareKey":"no-such-share"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], 404);
}
