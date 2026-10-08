#![cfg(feature = "axum")]
//! M2 适配器集成测试（axum 侧）：driver 只建 Router + 取响应，
//! 断言体与 actix 侧共用 tests/common/m2.rs（逐条 1:1）。
//! 覆盖：路由可达性、api.json 结构、CORS 头、UI 数据流一致性。

mod common;
#[path = "common/m2.rs"]
mod m2;

use apidoc::axum::{apidoc_routes, cors_layer, CorsConfig};
use apidoc::ApidocConfig;
use axum::body::{to_bytes, Body};
use axum::http::{header, Request};
use axum::Router;
use tower::ServiceExt;

/// driver：建 Router、发 GET（可带 Origin）、把响应归一化成 common::Resp。
async fn get(cfg: ApidocConfig, uri: String, origin: Option<String>) -> common::Resp {
    let mut req = Request::builder().uri(uri.as_str());
    if let Some(o) = origin {
        req = req.header(header::ORIGIN, o);
    }
    let res = Router::new()
        .merge(apidoc_routes(cfg))
        .layer(cors_layer(CorsConfig::default()))
        .oneshot(req.body(Body::empty()).unwrap())
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
async fn apidoc_route_returns_html_page() {
    m2::assert_apidoc_route_returns_html_page("axum", get).await;
}

#[tokio::test]
async fn pet_route_serves_svg_and_ui_references_it() {
    m2::assert_pet_route_serves_svg_and_ui_references_it("axum", get).await;
}

#[tokio::test]
async fn api_json_route_returns_valid_doc() {
    m2::assert_api_json_route_returns_valid_doc("axum", get).await;
}

#[tokio::test]
async fn export_route_dispatches_by_format_and_rejects_unknown() {
    m2::assert_export_route_dispatches_by_format_and_rejects_unknown("axum", get).await;
}

#[tokio::test]
async fn cors_header_present_on_origin_request() {
    m2::assert_cors_header_present_on_origin_request("axum", get).await;
}

#[test]
fn ui_html_has_grouping_markers() {
    m2::assert_ui_html_has_grouping_markers("axum");
}

#[test]
fn ui_html_fields_all_present_in_api_json() {
    m2::assert_ui_html_fields_all_present_in_api_json("axum");
}
