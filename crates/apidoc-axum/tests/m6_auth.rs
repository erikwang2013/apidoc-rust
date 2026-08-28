//! M6a apidoc-axum 集成测试：/apidoc/auth 签发、数据路由 401 守卫（UI 不守卫）、
//! 应用密码优先于全局密码。

use apidoc::auth::{md5_hex, AuthConfig};
use apidoc::{AppConfig, ApidocConfig};
use apidoc_axum::{apidoc_routes, cors_layer, CorsConfig};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

#[allow(dead_code)]
#[apidoc::title("受保护接口")]
#[apidoc::url("/api/protected")]
#[apidoc::method("GET")]
fn protected_ep() {}

#[allow(dead_code)]
#[apidoc::app("api")]
#[apidoc::title("应用接口")]
#[apidoc::url("/api/app")]
#[apidoc::method("GET")]
fn app_ep() {}

fn app(auth: Option<AuthConfig>, apps: Vec<AppConfig>) -> Router {
    Router::new()
        .merge(apidoc_routes(ApidocConfig { title: "t".into(), description: None, auth, apps }))
        .layer(cors_layer(CorsConfig::default()))
}

// base64 token 含 + / 字符，query 传值须百分号编码（ui.js 用 encodeURIComponent）
fn enc(s: &str) -> String {
    s.replace('+', "%2B").replace('/', "%2F")
}

async fn get(router: Router, uri: &str) -> (StatusCode, String) {
    let res = router.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn enabled() -> AuthConfig {
    AuthConfig { enable: true, password: "secret".into(), secret_key: "k".into(), expire: 0 }
}

#[tokio::test]
async fn auth_disabled_route_404_and_data_open() {
    let r = app(None, Vec::new());
    let (s, _) = get(r.clone(), "/apidoc/auth").await;
    assert_eq!(s, StatusCode::NOT_FOUND, "auth 未启用时 /apidoc/auth 应 404");
    for uri in [
        "/apidoc/api.json",
        "/apidoc/mock?url=/api/protected&method=GET",
        "/apidoc/export?format=md",
    ] {
        let (s, _) = get(r.clone(), uri).await;
        assert_eq!(s, StatusCode::OK, "{uri} 应直接可访问");
    }
}

#[tokio::test]
async fn auth_enabled_guards_data_routes_and_keeps_ui_open() {
    let r = app(Some(enabled()), Vec::new());
    for uri in [
        "/apidoc/api.json",
        "/apidoc/mock?url=/api/protected&method=GET",
        "/apidoc/export?format=md",
    ] {
        let (s, body) = get(r.clone(), uri).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{uri} 缺 token 应 401");
        assert!(body.contains("password required"), "{uri} 缺 token 响应体错误");
    }
    // UI 页不受守卫
    let (s, body) = get(r, "/apidoc").await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains("<title>API Documentation</title>"));
}

#[tokio::test]
async fn auth_issue_and_token_flow() {
    let r = app(Some(enabled()), Vec::new());
    // 错误密码 → 401 password error
    let (s, body) = get(r.clone(), "/apidoc/auth?password=deadbeef").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(body.contains("password error"));
    // 正确密码 → token，随后数据路由全部放行
    let (s, body) = get(r.clone(), &format!("/apidoc/auth?password={}", md5_hex("secret"))).await;
    assert_eq!(s, StatusCode::OK);
    let token: String = serde_json::from_str::<Value>(&body).unwrap()["token"].as_str().unwrap().to_string();
    let (s, body) = get(r.clone(), &format!("/apidoc/api.json?token={}", enc(&token))).await;
    assert_eq!(s, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["endpoints"].as_array().unwrap().len(), 2, "根 endpoints 恒为全集");
    let (s, _) = get(r.clone(), &format!("/apidoc/export?format=md&token={}", enc(&token))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = get(r.clone(), &format!("/apidoc/mock?url=/api/protected&method=GET&token={}", enc(&token))).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn app_password_takes_priority_over_global() {
    let apps = vec![AppConfig {
        key: "api".into(),
        title: "API".into(),
        items: Vec::new(),
        password: Some("app-pw".into()),
    }];
    let r = app(Some(enabled()), apps);
    // 全局已启用：缺 token 拒绝
    let (s, _) = get(r.clone(), "/apidoc/api.json?appKey=api").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "缺 token 应拒绝");
    // 应用密码签发（appKey 优先：全局密码错也能拿到应用 token）
    let (s, body) = get(r.clone(), &format!("/apidoc/auth?password={}&appKey=api", md5_hex("app-pw"))).await;
    assert_eq!(s, StatusCode::OK);
    let token: String = serde_json::from_str::<Value>(&body).unwrap()["token"].as_str().unwrap().to_string();
    let (s, body) = get(r.clone(), &format!("/apidoc/api.json?appKey=api&token={}", enc(&token))).await;
    assert_eq!(s, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["apps"][0]["key"], "api", "应用树应进入 api.json");
    // 全局密码 token 不能通过应用密码
    let (s, body) = get(r.clone(), &format!("/apidoc/auth?password={}", md5_hex("secret"))).await;
    assert_eq!(s, StatusCode::OK);
    let gtoken: String = serde_json::from_str::<Value>(&body).unwrap()["token"].as_str().unwrap().to_string();
    let (s, _) = get(r, &format!("/apidoc/api.json?appKey=api&token={}", enc(&gtoken))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "全局 token 不应通过应用密码");
}
