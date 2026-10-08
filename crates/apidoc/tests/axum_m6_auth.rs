#![cfg(feature = "axum")]
//! M6a 适配器集成测试（axum 侧）：driver 只建 Router + 取响应，
//! 断言体与 actix 侧共用 tests/common/m6.rs（逐条 1:1）。
//! 覆盖：/apidoc/auth 签发、数据路由 401 守卫（UI 不守卫）、应用密码优先于全局密码。

mod common;
#[path = "common/m6.rs"]
mod m6;

use apidoc::axum::{apidoc_routes, cors_layer, CorsConfig};
use apidoc::ApidocConfig;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use axum::Router;
use tower::ServiceExt;

/// driver：建 Router、发 GET、把响应归一化成 common::Resp（断言体共享）。
async fn get(cfg: ApidocConfig, uri: String) -> common::Resp {
    let res = Router::new()
        .merge(apidoc_routes(cfg))
        .layer(cors_layer(CorsConfig::default()))
        .oneshot(Request::builder().uri(uri.as_str()).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    common::Resp::new(
        status.as_u16(),
        headers
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())),
        String::from_utf8(body.to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn auth_disabled_route_404_and_data_open() {
    m6::assert_auth_disabled_route_404_and_data_open("axum", get).await;
}

#[tokio::test]
async fn auth_enabled_guards_data_routes_and_keeps_ui_open() {
    m6::assert_auth_enabled_guards_data_routes_and_keeps_ui_open("axum", get).await;
}

#[tokio::test]
async fn auth_issue_and_token_flow() {
    m6::assert_auth_issue_and_token_flow("axum", get).await;
}

#[tokio::test]
async fn app_password_takes_priority_over_global() {
    m6::assert_app_password_takes_priority_over_global("axum", get).await;
}
