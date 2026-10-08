#![cfg(feature = "actix")]
//! M6a 适配器集成测试（actix 侧）：driver 只建 App + 取响应，
//! 断言体与 axum 侧共用 tests/common/m6.rs（逐条 1:1）。
//! 覆盖：/apidoc/auth 签发、数据路由 401 守卫（UI 不守卫）、应用密码优先于全局密码。

mod common;
#[path = "common/m6.rs"]
mod m6;

// 别名导入：actix_web::test 同时是模块和属性宏，直接 `use actix_web::test` 会遮蔽内置 #[test]
use actix_web::test as actix_test;
use actix_web::App;
use apidoc::actix::{apidoc_routes, cors_layer, CorsConfig};
use apidoc::ApidocConfig;

/// driver：建 App、发 GET、把响应归一化成 common::Resp（断言体共享）。
async fn get(cfg: ApidocConfig, uri: String) -> common::Resp {
    let svc = actix_test::init_service(
        App::new()
            .service(apidoc_routes(cfg))
            .wrap(cors_layer(CorsConfig::default())),
    )
    .await;
    let res = actix_test::call_service(&svc, actix_test::TestRequest::get().uri(&uri).to_request()).await;
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
async fn auth_disabled_route_404_and_data_open() {
    m6::assert_auth_disabled_route_404_and_data_open("actix", get).await;
}

#[actix_web::test]
async fn auth_enabled_guards_data_routes_and_keeps_ui_open() {
    m6::assert_auth_enabled_guards_data_routes_and_keeps_ui_open("actix", get).await;
}

#[actix_web::test]
async fn auth_issue_and_token_flow() {
    m6::assert_auth_issue_and_token_flow("actix", get).await;
}

#[actix_web::test]
async fn app_password_takes_priority_over_global() {
    m6::assert_app_password_takes_priority_over_global("actix", get).await;
}
