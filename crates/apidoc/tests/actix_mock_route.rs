#![cfg(feature = "actix")]
//! M4 适配器集成测试（actix 侧）：driver 只建 App + 取响应，
//! 断言体与 axum 侧共用 tests/common/mock.rs（逐条 1:1）。
//! 覆盖：/apidoc/mock 命中/未命中、not_debug 端点不过滤、api.json 回归哨兵。

mod common;
#[path = "common/mock.rs"]
mod mock;

// 别名导入：actix_web::test 同时是模块和属性宏，直接 `use actix_web::test` 会遮蔽内置 #[test]
use actix_web::test as actix_test;
use actix_web::App;
use apidoc::actix::{apidoc_routes, cors_layer, CorsConfig};
use apidoc::ApidocConfig;

/// driver：建 App、发 GET、把响应归一化成 common::Resp（断言体共享）。
async fn get(cfg: ApidocConfig, uri: String) -> common::Resp {
    let app = actix_test::init_service(
        App::new()
            .service(apidoc_routes(cfg))
            .wrap(cors_layer(CorsConfig::default())),
    )
    .await;
    let res = actix_test::call_service(&app, actix_test::TestRequest::get().uri(&uri).to_request()).await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = actix_test::read_body(res).await;
    common::Resp::new(
        status.as_u16(),
        headers
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())),
        String::from_utf8(body.to_vec()).unwrap(),
    )
}

#[actix_web::test]
async fn mock_route_hit_returns_three_keys_and_mock_values() {
    mock::assert_mock_route_hit_returns_three_keys_and_mock_values("actix", get).await;
}

#[actix_web::test]
async fn mock_route_miss_returns_404() {
    mock::assert_mock_route_miss_returns_404("actix", get).await;
}

#[actix_web::test]
async fn mock_route_requires_exact_method_match() {
    mock::assert_mock_route_requires_exact_method_match("actix", get).await;
}

#[actix_web::test]
async fn mock_route_without_params_returns_404() {
    mock::assert_mock_route_without_params_returns_404("actix", get).await;
}

#[actix_web::test]
async fn mock_route_serves_not_debug_endpoints() {
    mock::assert_mock_route_serves_not_debug_endpoints("actix", get).await;
}

#[actix_web::test]
async fn api_json_regression_sentinel() {
    mock::assert_api_json_regression_sentinel("actix", get).await;
}
