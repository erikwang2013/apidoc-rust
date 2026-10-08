//! M6a 适配器集成测试的共享断言体（axum 侧 / actix 侧逐条 1:1 共用）：
//! /apidoc/auth 签发、数据路由 401 守卫（UI 不守卫）、应用密码优先于全局密码。
//! driver（tests/axum_m6_auth.rs / tests/actix_m6_auth.rs）只提供取响应的 get。
#![allow(dead_code)]

use std::future::Future;

use apidoc::auth::{md5_hex, AuthConfig};
use apidoc::{AppConfig, ApidocConfig};
use serde_json::Value;

use crate::common::{enc, Resp};

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

/// 两适配器共用的基线配置。
fn cfg(auth: Option<AuthConfig>, apps: Vec<AppConfig>) -> ApidocConfig {
    ApidocConfig { title: "t".into(), description: None, auth, apps, ..Default::default() }
}

fn enabled() -> AuthConfig {
    AuthConfig { enable: true, password: "secret".into(), secret_key: "k".into(), expire: 0 }
}

pub async fn assert_auth_disabled_route_404_and_data_open<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(cfg(None, Vec::new()), "/apidoc/auth".to_string()).await;
    assert_eq!(
        r.status, 404,
        "[{adapter}] auth 未启用时 /apidoc/auth 应 404，实际 {}",
        r.status
    );
    for uri in [
        "/apidoc/api.json",
        "/apidoc/mock?url=/api/protected&method=GET",
        "/apidoc/export?format=md",
    ] {
        let r = get(cfg(None, Vec::new()), uri.to_string()).await;
        assert_eq!(r.status, 200, "[{adapter}] {uri} 应直接可访问，实际 {}", r.status);
    }
}

pub async fn assert_auth_enabled_guards_data_routes_and_keeps_ui_open<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    for uri in [
        "/apidoc/api.json",
        "/apidoc/mock?url=/api/protected&method=GET",
        "/apidoc/export?format=md",
    ] {
        let r = get(cfg(Some(enabled()), Vec::new()), uri.to_string()).await;
        assert_eq!(r.status, 401, "[{adapter}] {uri} 缺 token 应 401，实际 {}", r.status);
        assert!(r.body.contains("password required"), "[{adapter}] {uri} 缺 token 响应体错误");
        // 401 的 Content-Type 是两适配器收敛后的契约（axum 原为 text/plain），钉住防回退
        assert_eq!(
            r.header("content-type").unwrap(), "application/json",
            "[{adapter}] {uri} 401 Content-Type 应为 application/json"
        );
    }
    // UI 页不受守卫
    let r = get(cfg(Some(enabled()), Vec::new()), "/apidoc".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] UI 页不应被守卫，实际 {}", r.status);
    assert!(
        r.body.contains("<title>API Documentation</title>"),
        "[{adapter}] UI 页缺少标题"
    );
}

pub async fn assert_auth_issue_and_token_flow<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    // 错误密码 → 401 password error
    let r = get(cfg(Some(enabled()), Vec::new()), "/apidoc/auth?password=deadbeef".to_string()).await;
    assert_eq!(r.status, 401, "[{adapter}] 错误密码期望 401，实际 {}", r.status);
    assert!(r.body.contains("password error"), "[{adapter}] 错误密码缺 password error 响应体");
    // 正确密码 → token，随后数据路由全部放行
    let uri = format!("/apidoc/auth?password={}", md5_hex("secret"));
    let r = get(cfg(Some(enabled()), Vec::new()), uri).await;
    assert_eq!(r.status, 200, "[{adapter}] 正确密码期望 200，实际 {}", r.status);
    let token: String = serde_json::from_str::<Value>(&r.body).unwrap()["token"].as_str().unwrap().to_string();
    let r = get(
        cfg(Some(enabled()), Vec::new()),
        format!("/apidoc/api.json?token={}", enc(&token)),
    )
    .await;
    assert_eq!(r.status, 200, "[{adapter}] 带 token 的 api.json 期望 200，实际 {}", r.status);
    let v: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(
        v["endpoints"].as_array().unwrap().len(), 2,
        "[{adapter}] 根 endpoints 恒为全集，实际 {}",
        v["endpoints"]
    );
    let r = get(
        cfg(Some(enabled()), Vec::new()),
        format!("/apidoc/export?format=md&token={}", enc(&token)),
    )
    .await;
    assert_eq!(r.status, 200, "[{adapter}] 带 token 的 export 期望 200，实际 {}", r.status);
    let r = get(
        cfg(Some(enabled()), Vec::new()),
        format!("/apidoc/mock?url=/api/protected&method=GET&token={}", enc(&token)),
    )
    .await;
    assert_eq!(r.status, 200, "[{adapter}] 带 token 的 mock 期望 200，实际 {}", r.status);
}

pub async fn assert_app_password_takes_priority_over_global<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let apps = vec![AppConfig {
        key: "api".into(),
        title: "API".into(),
        items: Vec::new(),
        password: Some("app-pw".into()),
    }];
    // 全局已启用：缺 token 拒绝
    let r = get(cfg(Some(enabled()), apps.clone()), "/apidoc/api.json?appKey=api".to_string()).await;
    assert_eq!(r.status, 401, "[{adapter}] 缺 token 应拒绝，实际 {}", r.status);
    // 应用密码签发（appKey 优先：全局密码错也能拿到应用 token）
    let uri = format!("/apidoc/auth?password={}&appKey=api", md5_hex("app-pw"));
    let r = get(cfg(Some(enabled()), apps.clone()), uri).await;
    assert_eq!(r.status, 200, "[{adapter}] 应用密码签发期望 200，实际 {}", r.status);
    let token: String = serde_json::from_str::<Value>(&r.body).unwrap()["token"].as_str().unwrap().to_string();
    let r = get(
        cfg(Some(enabled()), apps.clone()),
        format!("/apidoc/api.json?appKey=api&token={}", enc(&token)),
    )
    .await;
    assert_eq!(r.status, 200, "[{adapter}] 带应用 token 的 api.json 期望 200，实际 {}", r.status);
    let v: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(v["apps"][0]["key"], "api", "[{adapter}] 应用树应进入 api.json，实际 {}", v["apps"]);
    // 全局密码 token 不能通过应用密码
    let uri = format!("/apidoc/auth?password={}", md5_hex("secret"));
    let r = get(cfg(Some(enabled()), apps.clone()), uri).await;
    assert_eq!(r.status, 200, "[{adapter}] 全局密码签发期望 200，实际 {}", r.status);
    let gtoken: String = serde_json::from_str::<Value>(&r.body).unwrap()["token"].as_str().unwrap().to_string();
    let r = get(
        cfg(Some(enabled()), apps),
        format!("/apidoc/api.json?appKey=api&token={}", enc(&gtoken)),
    )
    .await;
    assert_eq!(r.status, 401, "[{adapter}] 全局 token 不应通过应用密码，实际 {}", r.status);
}
