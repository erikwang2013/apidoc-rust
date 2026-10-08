#![cfg(feature = "actix")]
//! v2 适配器路由级集成测试（actix 侧）：driver 只建 App + 取响应，
//! 断言体与 axum 侧共用 tests/common/v2.rs（逐条 1:1，同断言、同期望）。
//! 覆盖：/apidoc/generate 模板分发、/apidoc/share 深链构造、鉴权守卫一致性
//! （share 按被分享的应用做守卫、share 链接 token 反向解锁 api.json 的端到端
//! 闭环）、缓存接线不影响输出、v1 api.json 红线、UI 分享按钮与前后置脚本接线。
//! 本文件只测「适配器路由与配置接线」；纯逻辑单测在 codegen.rs / cache.rs /
//! share.rs / m3_table.rs / ui_events.rs，此处不重复。

mod common;
#[path = "common/v2.rs"]
mod v2;

// 别名导入：actix_web::test 同时是模块和属性宏，直接 `use actix_web::test` 会遮蔽内置 #[test]
use actix_web::test as actix_test;
use actix_web::App;
use apidoc::actix::apidoc_routes;
use apidoc::ApidocConfig;

/// driver：建 App、发 GET、把响应归一化成 common::Resp（断言体共享）。
async fn get(cfg: ApidocConfig, uri: String) -> common::Resp {
    let app = actix_test::init_service(App::new().service(apidoc_routes(cfg))).await;
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
async fn generate_route_serves_builtin_templates() {
    v2::assert_generate_route_serves_builtin_templates("actix", get).await;
}

#[actix_web::test]
async fn generate_route_rejects_unknown_and_missing_template() {
    v2::assert_generate_route_rejects_unknown_and_missing_template("actix", get).await;
}

#[actix_web::test]
async fn share_route_builds_deep_links() {
    v2::assert_share_route_builds_deep_links("actix", get).await;
}

#[actix_web::test]
async fn share_token_round_trips_through_api_json_guard() {
    v2::assert_share_token_round_trips_through_api_json_guard("actix", get).await;
}

#[actix_web::test]
async fn share_requires_app_token_when_app_password_configured() {
    v2::assert_share_requires_app_token_when_app_password_configured("actix", get).await;
}

#[actix_web::test]
async fn share_stays_open_without_any_credentials_configured() {
    v2::assert_share_stays_open_without_any_credentials_configured("actix", get).await;
}

#[actix_web::test]
async fn cache_wiring_keeps_mock_output_identical() {
    v2::assert_cache_wiring_keeps_mock_output_identical("actix", get).await;
}

#[actix_web::test]
async fn api_json_has_no_v2_keys_on_v1_config() {
    v2::assert_api_json_has_no_v2_keys_on_v1_config("actix", get).await;
}

#[actix_web::test]
async fn ui_page_wires_share_and_pre_post_scripts() {
    v2::assert_ui_page_wires_share_and_pre_post_scripts("actix", get).await;
}
