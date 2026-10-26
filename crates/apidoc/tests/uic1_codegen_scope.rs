//! I3: `renderCodeTemplate` must scope `selected` and an empty selection to the
//! guarded `appKey`, never to other apps' endpoints.
#![cfg(feature = "axum")]

use apidoc::axum::apidoc_routes;
use apidoc::{ApidocConfig, AppConfig, CodegenTemplate};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

#[allow(dead_code)]
#[apidoc::app("A")]
#[apidoc::title("Secret A")]
#[apidoc::url("/a/secret")]
#[apidoc::method("GET")]
fn a_ep() {}

#[allow(dead_code)]
#[apidoc::app("B")]
#[apidoc::title("Public B")]
#[apidoc::url("/b/public")]
#[apidoc::method("GET")]
fn b_ep() {}

fn config() -> ApidocConfig {
    ApidocConfig {
        title: "Codegen scope".into(),
        apps: vec![
            AppConfig {
                key: "A".into(),
                title: "App A".into(),
                items: Vec::new(),
                password: Some("a-pw".into()),
            },
            AppConfig {
                key: "B".into(),
                title: "App B".into(),
                items: Vec::new(),
                password: None,
            },
        ],
        codegen: vec![CodegenTemplate {
            name: "scope.ts".into(),
            template: "{{#each endpoints}}{{title}}|{{method}} {{url}}\n{{/each}}".into(),
        }],
        ..Default::default()
    }
}

async fn send(app: &axum::Router, path: &str, body: Value, token: &str) -> (StatusCode, Value) {
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

async fn first_menu_key(app: &axum::Router, app_key: &str, token: &str) -> String {
    let (status, menus) = send(app, "/apidoc/ui/v1/menus", json!({"appKey": app_key}), token).await;
    assert_eq!(status, StatusCode::OK, "{menus}");
    menus["data"]["data"][0]["children"][0]["menuKey"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn render(app: &axum::Router, app_key: &str, selected: Value, token: &str) -> (StatusCode, String) {
    let (status, body) = send(
        app,
        "/apidoc/ui/v1/renderCodeTemplate",
        json!({"appKey": app_key, "template": "scope.ts", "selected": selected}),
        token,
    )
    .await;
    (status, body["data"]["code"].as_str().unwrap_or("").to_string())
}

#[tokio::test]
async fn render_template_never_reaches_other_apps_endpoints() {
    let app = apidoc_routes(config());
    let (status, login) = send(
        &app,
        "/apidoc/ui/v1/verifyAuth",
        json!({"appKey": "A", "password": "a-pw"}),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let token_a = login["data"]["token"].as_str().unwrap();

    let a_id = first_menu_key(&app, "A", token_a).await;
    let b_id = first_menu_key(&app, "B", "").await;

    // Cross-app selection: app A token + appKey A + B's endpoint id → B is ignored.
    let (status, code) = render(&app, "A", json!([b_id]), token_a).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!code.contains("Public B"), "cross-app endpoint leaked: {code}");

    // Empty selection = all endpoints of the guarded app, not the whole document.
    let (status, code) = render(&app, "A", json!([]), token_a).await;
    assert_eq!(status, StatusCode::OK);
    assert!(code.contains("Secret A"), "{code}");
    assert!(!code.contains("Public B"), "empty selection leaked app B: {code}");

    // Mixed selection keeps only the in-app endpoint.
    let (status, code) = render(&app, "A", json!([a_id, b_id]), token_a).await;
    assert_eq!(status, StatusCode::OK, "{code}");
    assert!(code.contains("Secret A"), "{code}");
    assert!(!code.contains("Public B"), "mixed selection leaked app B: {code}");
}
