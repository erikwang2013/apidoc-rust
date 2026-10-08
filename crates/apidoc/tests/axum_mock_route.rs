#![cfg(feature = "axum")]
//! M4 适配器集成测试（axum 侧）：driver 只建 Router + 取响应，
//! 断言体与 actix 侧共用 tests/common/mock.rs（逐条 1:1）。
//! 覆盖：/apidoc/mock 命中/未命中、not_debug 端点不过滤、api.json 回归哨兵。

mod common;
#[path = "common/mock.rs"]
mod mock;

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
async fn mock_route_hit_returns_three_keys_and_mock_values() {
    mock::assert_mock_route_hit_returns_three_keys_and_mock_values("axum", get).await;
}

#[tokio::test]
async fn mock_route_miss_returns_404() {
    mock::assert_mock_route_miss_returns_404("axum", get).await;
}

#[tokio::test]
async fn mock_route_requires_exact_method_match() {
    mock::assert_mock_route_requires_exact_method_match("axum", get).await;
}

#[tokio::test]
async fn mock_route_without_params_returns_404() {
    mock::assert_mock_route_without_params_returns_404("axum", get).await;
}

#[tokio::test]
async fn mock_route_serves_not_debug_endpoints() {
    mock::assert_mock_route_serves_not_debug_endpoints("axum", get).await;
}

#[tokio::test]
async fn api_json_regression_sentinel() {
    mock::assert_api_json_regression_sentinel("axum", get).await;
}
