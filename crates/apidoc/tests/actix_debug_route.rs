#![cfg(all(feature = "actix", feature = "debug-http"))]
//! P0 服务端调试路由（actix 侧）：只验证路由/extractor 接线与关闭时的 403。
//! 转发与 SSRF 逻辑由同一 `Core::debug` 承担，已在 axum_debug_route.rs 覆盖。

use actix_web::{test, App};
use apidoc::ApidocConfig;

#[actix_web::test]
async fn disabled_route_returns_403() {
    let app = test::init_service(
        App::new().service(apidoc::actix::apidoc_routes(ApidocConfig::default())),
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/apidoc/debug")
        .set_payload("{}")
        .to_request();
    let res = test::call_service(&app, req).await;
    assert_eq!(res.status().as_u16(), 403);
}
