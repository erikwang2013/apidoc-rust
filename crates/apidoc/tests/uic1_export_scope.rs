//! I2: `exportAllData` must not bypass app/share passwords when global auth is off.
#![cfg(feature = "axum")]

use apidoc::axum::apidoc_routes;
use apidoc::{ApidocConfig, AppConfig};
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

/// Mixed deployment: global auth off, app A protected, app B public.
fn config() -> ApidocConfig {
    ApidocConfig {
        title: "Export scope".into(),
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

fn app_keys(exported: &Value) -> Vec<String> {
    exported["data"]["apiData"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["appKey"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn empty_body_export_only_lists_unprotected_apps() {
    let app = apidoc_routes(config());

    // No credentials: only app B (no password) may be exported.
    let (status, exported) = send(&app, "/apidoc/ui/v1/exportAllData", json!({}), "").await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(app_keys(&exported), ["B"], "{exported}");

    // A named protected app needs that app's token.
    let (status, _) = send(&app, "/apidoc/ui/v1/exportAllData", json!({"appKey": "A"}), "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, login) = send(
        &app,
        "/apidoc/ui/v1/verifyAuth",
        json!({"appKey": "A", "password": "a-pw"}),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let app_token = login["data"]["token"].as_str().unwrap();
    let (status, exported) = send(
        &app,
        "/apidoc/ui/v1/exportAllData",
        json!({"appKey": "A"}),
        app_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(app_keys(&exported), ["A"]);

    // An app token on the unnamed path still does not unlock other protected apps.
    let (status, exported) = send(&app, "/apidoc/ui/v1/exportAllData", json!({}), app_token).await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(app_keys(&exported), ["B"], "{exported}");
}

#[tokio::test]
async fn password_share_key_export_requires_share_token() {
    let dir = std::env::temp_dir().join(format!(
        "apidoc-uic1-export-scope-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("APIDOC_SHARE_DIR", &dir);

    let app = apidoc_routes(config());
    // Global auth is off, so owner CRUD is open (same model as Go/PHP).
    for body in [
        json!({"key":"locked","name":"Locked","type":"all","password":"share-pw"}),
        json!({"key":"open","name":"Open","type":"all"}),
    ] {
        let (status, added) = send(&app, "/apidoc/ui/v1/addApiShare", body, "").await;
        assert_eq!(status, StatusCode::OK, "{added}");
        assert_eq!(added["data"], true);
    }

    let (status, _) = send(&app, "/apidoc/ui/v1/exportAllData", json!({"key": "locked"}), "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "share password must gate key export");

    let (status, login) = send(
        &app,
        "/apidoc/ui/v1/verifyAuth",
        json!({"password": "share-pw", "shareKey": "locked"}),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let share_token = login["data"]["token"].as_str().unwrap();
    let (status, exported) = send(
        &app,
        "/apidoc/ui/v1/exportAllData",
        json!({"key": "locked"}),
        share_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(app_keys(&exported), ["A", "B"], "{exported}");

    // Public record: the key itself is the capability when no global credential exists.
    let (status, exported) = send(&app, "/apidoc/ui/v1/exportAllData", json!({"key": "open"}), "").await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(app_keys(&exported), ["A", "B"], "{exported}");

    let _ = std::fs::remove_dir_all(&dir);
}
