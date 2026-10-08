//! 薄 actix-web 适配器（feature "actix"）：只做路由注册 + 状态注入（web::Data<Arc<Core>>），
//! 守卫/分支/响应渲染全部在 crate::route_core（与 axum 同一份实现，行为 1:1
//! 由结构性共享保证：404/400 语义、Content-Type、mock 匹配逻辑）。
//! 不做 UI 内嵌三方文档工具、不做服务端代理（规划已定）。

use actix_cors::Cors;
use actix_web::{web, HttpResponse, Scope};
use crate::route_core::{self, Core, R};
use crate::ApidocConfig;
use std::collections::HashMap;
use std::sync::Arc;

pub use crate::route_core::CorsConfig;

/// 生成 CORS 中间件。用法：`.service(apidoc::actix::apidoc_routes(cfg)).wrap(apidoc::actix::cors_layer(cfg))`。
/// wrap 作用于整个 App，用户自己的业务路由也能被浏览器跨域直连。
/// 方法/头默认全放行（M4 要任意 method 与 Content-Type/Authorization 等头）。
pub fn cors_layer(config: CorsConfig) -> Cors {
    // 不用 Cors::permissive()：它自带 supports_credentials=true，违反"永不凭据"。
    // Cors::default() 凭据为 false，空配置补 send_wildcard 产出字面 `*`（与 axum 一致），
    // 白名单命中时反射请求 Origin（与 tower-http AllowOrigin::list 一致）。
    let cors = Cors::default().allow_any_method().allow_any_header();
    if config.allow_origins.is_empty() {
        cors.allow_any_origin().send_wildcard()
    } else {
        config.allow_origins.iter().fold(cors, |c, o| c.allowed_origin(o))
    }
}

/// 挂载 GET /apidoc（UI）与 GET /apidoc/api.json（数据），返回 Scope，
/// 用户用 `App::new().service(apidoc_routes(cfg))` 接入。
/// api.json 内容 = DocRegistry::collect() 原样输出（核心数据模型零改动）；
/// 分组是纯 UI 侧启发式（见 ui.html），M3 的 group 注解上线后 UI 优先用注解。
/// ui.html 共享自本 crate 根（crates/apidoc/src/ui.html），与 axum 适配器同一份。
pub fn apidoc_routes(config: ApidocConfig) -> Scope {
    // 构建期一次性物化全部文档产物（api.json/export/mock），请求期零重算。
    let core = web::Data::new(Arc::new(Core::build(config)));
    web::scope("/apidoc")
        .app_data(core)
        // UI 页与宠物图标不设守卫（见 route_core::UI/PET）。
        .route("", web::get().to(|| async { response(route_core::UI) }))
        .route("/pet.svg", web::get().to(|| async { response(route_core::PET) }))
        .route("/auth", web::get().to(auth))
        // 以下数据路由均经 route_core 守卫，失败 401 DENIED_BODY
        .route("/api.json", web::get().to(api_json))
        .route("/export", web::get().to(export))
        .route("/mock", web::get().to(mock))
        .route("/share", web::get().to(share))
        .route("/generate", web::get().to(generate))
}

// 各 handler 一行转发 core：判定与渲染在 route_core（两框架共享同一实现）。
async fn auth(core: web::Data<Arc<Core>>, q: web::Query<HashMap<String, String>>) -> HttpResponse {
    response(core.auth(&q))
}

async fn api_json(core: web::Data<Arc<Core>>, q: web::Query<HashMap<String, String>>) -> HttpResponse {
    response(core.api_json(&q))
}

async fn export(core: web::Data<Arc<Core>>, q: web::Query<HashMap<String, String>>) -> HttpResponse {
    response(core.export(&q))
}

async fn mock(core: web::Data<Arc<Core>>, q: web::Query<HashMap<String, String>>) -> HttpResponse {
    response(core.mock(&q))
}

async fn share(core: web::Data<Arc<Core>>, q: web::Query<HashMap<String, String>>) -> HttpResponse {
    response(core.share(&q))
}

async fn generate(core: web::Data<Arc<Core>>, q: web::Query<HashMap<String, String>>) -> HttpResponse {
    response(core.generate(&q))
}

/// route_core::R → HttpResponse：Content-Type 为 None 时不设头（对齐现状空响应）。
fn response(r: R) -> HttpResponse {
    let mut builder =
        HttpResponse::build(actix_web::http::StatusCode::from_u16(r.status).expect("valid status"));
    if let Some(ct) = r.ct {
        builder.content_type(ct);
    }
    builder.body(r.body)
}
