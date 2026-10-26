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

#[allow(dead_code)]
#[apidoc::title("User hook")]
#[apidoc::url("/hook/user")]
#[apidoc::method("POST")]
#[apidoc::protocol("webhook")]
#[apidoc::definition(
    file = "tests/fixtures/protocol/json-schema/user.json",
    sampleFile = "tests/fixtures/protocol/json-schema/user-sample.json"
)]
fn user_hook() {}

#[allow(dead_code)]
#[apidoc::title("Broken hook")]
#[apidoc::url("/hook/broken")]
#[apidoc::method("POST")]
#[apidoc::protocol("webhook")]
#[apidoc::definition(file = "tests/fixtures/protocol/json-schema/invalid.json")]
#[apidoc::definition(file = "../apidoc-macros/Cargo.toml")]
#[apidoc::definition(
    file = "tests/fixtures/protocol/json-schema/user.json",
    format = "bogus"
)]
#[apidoc::definition(file = "tests/fixtures/protocol/json-schema/traversal.json")]
fn broken_hook() {}

#[allow(dead_code)]
#[apidoc::title("Auto hook")]
#[apidoc::url("/hook/auto")]
#[apidoc::method("POST")]
#[apidoc::protocol("webhook")]
#[apidoc::definition(
    file = "tests/fixtures/protocol/openapi/api.yaml",
    operation = "getUser",
    sample = r#"{"id":1,"name":"a"}"#
)]
fn auto_hook() {}

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

async fn detail_by_url(app: &axum::Router, token: &str, url: &str) -> Value {
    let (status, menus) = send(
        app,
        "POST",
        "/apidoc/ui/v1/menus",
        r#"{"appKey":"default"}"#,
        token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{menus}");
    let children = menus["data"]["data"][0]["children"].as_array().expect("children");
    let menu_key = children
        .iter()
        .find(|item| item["url"] == url)
        .and_then(|item| item["menuKey"].as_str())
        .unwrap_or_else(|| panic!("menuKey for {url}"));
    let (status, detail) = send(
        app,
        "POST",
        "/apidoc/ui/v1/detail",
        &json!({"appKey":"default","path":menu_key}).to_string(),
        token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    detail
}

#[tokio::test]
async fn capabilities_are_public() {
    let (status, body) = send(&app(), "GET", "/apidoc/ui/v1/capabilities", "", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["contract"], "1.0");
    assert_eq!(body["data"]["backend"], "rust");
    assert_eq!(body["data"]["features"]["protocolDefinitions"], true);
}

/// 默认构建（无任何 debug-* feature）不能宣传 relay 能力。
#[cfg(not(feature = "debug-http"))]
#[tokio::test]
async fn capabilities_default_build_advertises_only_browser_modes() {
    let (status, body) = send(&app(), "GET", "/apidoc/ui/v1/capabilities", "", "").await;
    assert_eq!(status, StatusCode::OK);
    let debug = &body["data"]["features"]["debug"];
    assert_eq!(debug["relay"], json!([]));
    assert_eq!(
        debug["modes"],
        json!(["browser-http", "browser-websocket", "browser-sse"])
    );
}

/// debug-all 的 relay/modes 必须与契约中的 rust-full 变体逐项一致。
#[cfg(feature = "debug-all")]
#[tokio::test]
async fn capabilities_match_rust_full_fixture() {
    let (status, body) = send(&app(), "GET", "/apidoc/ui/v1/capabilities", "", "").await;
    assert_eq!(status, StatusCode::OK);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contract/uic-1/fixtures/capabilities.json");
    let fixtures: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .unwrap();
    let full = fixtures
        .as_array()
        .expect("fixtures array")
        .iter()
        .find(|case| case["name"] == "rust-full")
        .expect("rust-full fixture");
    let got = &body["data"]["features"]["debug"];
    let want = &full["response"]["data"]["features"]["debug"];
    assert_eq!(got["relay"], want["relay"]);
    assert_eq!(got["modes"], want["modes"]);
    assert_eq!(got["relay"].as_array().map(Vec::len), Some(30));
    assert_eq!(got["modes"].as_array().map(Vec::len), Some(4));
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
async fn protocol_definitions_are_parsed_and_aggregated() {
    let app = app();
    let token = login(&app).await;

    let success = detail_by_url(&app, &token, "/hook/user").await;
    let definitions = success["data"]["definitions"]
        .as_array()
        .expect("definitions");
    assert_eq!(definitions.len(), 1, "{success}");
    let definition = &definitions[0];
    assert_eq!(definition["format"], "json-schema");
    assert_eq!(
        definition["source"],
        "tests/fixtures/protocol/json-schema/user.json"
    );
    assert_eq!(definition["operation"], "");
    assert_eq!(definition["meta"], json!({}));
    assert_eq!(definition["messages"][0]["name"], "User");
    assert_eq!(definition["messages"][0]["direction"], "request");
    assert_eq!(definition["messages"][0]["fields"][0]["name"], "id");
    assert_eq!(definition["messages"][0]["sampleValidation"]["ok"], true);
    assert_eq!(
        definition["validation"],
        json!({"ok": true, "errors": [], "warnings": []})
    );
    assert_eq!(
        success["data"]["validation"],
        json!({"ok": true, "errors": [], "warnings": []})
    );

    let auto = detail_by_url(&app, &token, "/hook/auto").await;
    let auto_definition = &auto["data"]["definitions"][0];
    assert_eq!(auto_definition["format"], "openapi", "{auto}");
    assert_eq!(auto_definition["operation"], "getUser");
    assert_eq!(auto["data"]["validation"]["ok"], true);

    let broken = detail_by_url(&app, &token, "/hook/broken").await;
    let definitions = broken["data"]["definitions"]
        .as_array()
        .expect("definitions");
    assert_eq!(definitions.len(), 4, "{broken}");
    let parse_error = &definitions[0]["validation"]["errors"][0];
    assert_eq!(
        parse_error,
        &json!({
            "path": "",
            "line": 0,
            "column": 0,
            "severity": "error",
            "message": "JSON parse error: Syntax error",
        })
    );
    assert_eq!(definitions[1]["format"], "json-schema");
    assert_eq!(definitions[1]["source"], "../apidoc-macros/Cargo.toml");
    assert_eq!(
        definitions[1]["validation"]["errors"][0]["message"],
        "definition file not found or outside allowed roots"
    );
    assert_eq!(definitions[2]["format"], "bogus");
    assert_eq!(
        definitions[2]["validation"]["errors"][0]["message"],
        "unsupported definition format: bogus"
    );
    assert!(
        definitions[3]["validation"]["errors"]
            .as_array()
            .expect("traversal errors")
            .iter()
            .any(|error| error["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("unable to resolve $ref:"))),
        "{}",
        definitions[3]
    );
    assert_eq!(broken["data"]["validation"]["ok"], false);
    assert_eq!(
        broken["data"]["validation"]["errors"]
            .as_array()
            .expect("aggregated errors")
            .len(),
        4
    );
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
