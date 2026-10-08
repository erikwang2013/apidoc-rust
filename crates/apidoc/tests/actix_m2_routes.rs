#![cfg(feature = "actix")]
//! M2 适配器集成测试（actix 侧）：driver 只建 App + 取响应，
//! 断言体与 axum 侧共用 tests/common/m2.rs（逐条 1:1）。
//! 覆盖：路由可达性、api.json 结构、CORS 头、UI 数据流一致性；
//! actix 特有：cors_whitelist_matches_exactly_and_never_credentials
//! （actix-cors 白名单模式的反射/未命中行为，axum 侧无对应用例，保留在 driver）。

mod common;
#[path = "common/m2.rs"]
mod m2;

use actix_web::http::header;
// 别名导入：actix_web::test 同时是模块和属性宏，直接 `use actix_web::test` 会遮蔽内置 #[test]
use actix_web::test as actix_test;
use actix_web::App;
use apidoc::actix::{apidoc_routes, cors_layer, CorsConfig};
use apidoc::ApidocConfig;

/// driver：建 App、发 GET（可带 Origin）、把响应归一化成 common::Resp。
async fn get(cfg: ApidocConfig, uri: String, origin: Option<String>) -> common::Resp {
    let app = actix_test::init_service(
        App::new()
            .service(apidoc_routes(cfg))
            .wrap(cors_layer(CorsConfig::default())),
    )
    .await;
    let mut req = actix_test::TestRequest::get().uri(&uri);
    if let Some(o) = origin {
        req = req.insert_header((header::ORIGIN, o));
    }
    let res = actix_test::call_service(&app, req.to_request()).await;
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
async fn apidoc_route_returns_html_page() {
    m2::assert_apidoc_route_returns_html_page("actix", get).await;
}

#[actix_web::test]
async fn pet_route_serves_svg_and_ui_references_it() {
    m2::assert_pet_route_serves_svg_and_ui_references_it("actix", get).await;
}

#[actix_web::test]
async fn api_json_route_returns_valid_doc() {
    m2::assert_api_json_route_returns_valid_doc("actix", get).await;
}

#[actix_web::test]
async fn export_route_dispatches_by_format_and_rejects_unknown() {
    m2::assert_export_route_dispatches_by_format_and_rejects_unknown("actix", get).await;
}

#[actix_web::test]
async fn cors_header_present_on_origin_request() {
    m2::assert_cors_header_present_on_origin_request("actix", get).await;
}

// actix-cors 白名单模式：命中 Origin 反射原值且无凭据，未命中不输出 CORS 头
#[actix_web::test]
async fn cors_whitelist_matches_exactly_and_never_credentials() {
    let app = actix_test::init_service(
        App::new()
            .service(apidoc_routes(ApidocConfig {
                title: "t".into(),
                description: None,
                auth: None,
                tables: Vec::new(),
                cache: None,
                codegen: Vec::new(),
                apps: Vec::new(),
            }))
            .wrap(cors_layer(CorsConfig {
                allow_origins: vec!["http://localhost:3000".into()],
            })),
    )
    .await;
    let res = actix_test::call_service(
        &app,
        actix_test::TestRequest::get()
            .uri("/apidoc/api.json")
            .insert_header((header::ORIGIN, "http://localhost:3000"))
            .to_request(),
    )
    .await;
    assert_eq!(
        res.headers().get("access-control-allow-origin").unwrap(),
        "http://localhost:3000",
        "白名单命中应反射 Origin"
    );
    assert!(
        res.headers().get("access-control-allow-credentials").is_none(),
        "白名单模式也不得携带凭据"
    );
    let res = actix_test::call_service(
        &app,
        actix_test::TestRequest::get()
            .uri("/apidoc/api.json")
            .insert_header((header::ORIGIN, "http://evil.com"))
            .to_request(),
    )
    .await;
    assert!(
        res.headers().get("access-control-allow-origin").is_none(),
        "白名单未命中不应输出 CORS 头"
    );
}

#[test]
fn ui_html_has_grouping_markers() {
    m2::assert_ui_html_has_grouping_markers("actix");
}

#[test]
fn ui_html_fields_all_present_in_api_json() {
    m2::assert_ui_html_fields_all_present_in_api_json("actix");
}
