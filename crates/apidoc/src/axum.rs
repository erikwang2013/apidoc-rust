//! 薄 axum 适配器（feature "axum"）：只做路由注册 + 状态注入（State<Arc<Core>>），
//! 守卫/分支/响应渲染全部在 crate::route_core（与 actix 同一份实现，行为 1:1
//! 由结构性共享保证，不再靠两份复制维护）。
//! 不做 UI 内嵌三方文档工具、不做服务端代理（规划已定）。

use crate::route_core::{self, Core, R};
use crate::ApidocConfig;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

pub use crate::route_core::CorsConfig;

/// 生成 CORS 层。用法：`.merge(apidoc::axum::apidoc_routes(cfg)).layer(apidoc::axum::cors_layer(cfg))`。
/// 注意 layer 必须在 merge 之后调用（或对最终 router 调用），否则 CORS 只覆盖
/// /apidoc 两条路由；M4 要的是用户自己的业务路由也能被浏览器跨域直连。
/// 方法/头默认全放行（M4 要任意 method 与 Content-Type/Authorization 等头）。
pub fn cors_layer(config: CorsConfig) -> CorsLayer {
    let origin = if config.allow_origins.is_empty() {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(
            config
                .allow_origins
                .into_iter()
                .map(|o| HeaderValue::from_str(&o).expect("valid origin")),
        )
    };
    CorsLayer::new().allow_origin(origin).allow_methods(Any).allow_headers(Any)
}

/// 挂载 GET /apidoc（UI）与 GET /apidoc/api.json（数据），返回 Router<()>，
/// 用户用 `Router::new()....merge(apidoc_routes(cfg))` 接入。
/// api.json 内容 = DocRegistry::collect() 原样输出（核心数据模型零改动）；
/// 分组是纯 UI 侧启发式（见 ui.html），M3 的 group 注解上线后 UI 优先用注解。
pub fn apidoc_routes(config: ApidocConfig) -> Router {
    // 构建期一次性物化全部文档产物（api.json/export/mock），请求期零重算。
    let core = Arc::new(Core::build(config));
    Router::new()
        // UI 页与宠物图标不设守卫（见 route_core::UI/PET）。
        .route("/apidoc", get(|| async { response(route_core::UI) }))
        .route("/apidoc/pet.svg", get(|| async { response(route_core::PET) }))
        .route("/apidoc/auth", get(auth))
        // 以下数据路由均经 route_core 守卫，失败 401 DENIED_BODY
        .route("/apidoc/api.json", get(api_json))
        .route("/apidoc/export", get(export))
        .route("/apidoc/mock", get(mock))
        .route("/apidoc/share", get(share))
        .route("/apidoc/generate", get(generate))
        .with_state(core)
}

// 各 handler 一行转发 core：判定与渲染在 route_core（两框架共享同一实现）。
async fn auth(State(core): State<Arc<Core>>, Query(q): Query<HashMap<String, String>>) -> Response {
    response(core.auth(&q))
}

async fn api_json(State(core): State<Arc<Core>>, Query(q): Query<HashMap<String, String>>) -> Response {
    response(core.api_json(&q))
}

async fn export(State(core): State<Arc<Core>>, Query(q): Query<HashMap<String, String>>) -> Response {
    response(core.export(&q))
}

async fn mock(State(core): State<Arc<Core>>, Query(q): Query<HashMap<String, String>>) -> Response {
    response(core.mock(&q))
}

async fn share(State(core): State<Arc<Core>>, Query(q): Query<HashMap<String, String>>) -> Response {
    response(core.share(&q))
}

async fn generate(State(core): State<Arc<Core>>, Query(q): Query<HashMap<String, String>>) -> Response {
    response(core.generate(&q))
}

/// route_core::R → axum Response：Content-Type 为 None 时不设头（对齐现状空响应）。
fn response(r: R) -> Response {
    let mut res = Response::new(Body::from(r.body));
    *res.status_mut() = StatusCode::from_u16(r.status).expect("valid status");
    if let Some(ct) = r.ct {
        res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    }
    res
}
