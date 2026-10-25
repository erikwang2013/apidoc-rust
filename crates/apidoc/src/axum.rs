//! 薄 axum 适配器（feature "axum"）：只做路由注册 + 状态注入（State<Arc<Core>>），
//! 守卫/分支/响应渲染全部在 crate::route_core（与 actix 同一份实现，行为 1:1
//! 由结构性共享保证，不再靠两份复制维护）。
//! 服务端调试路由默认关闭；开启需 DebugConfig + 全局鉴权 + feature `debug-http`。

use crate::route_core::{self, Core, R};
use crate::ApidocConfig;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

pub use crate::route_core::CorsConfig;

/// 生成 CORS 层。用法：
/// `.merge(apidoc::axum::apidoc_routes(cfg.clone())).layer(apidoc::axum::cors_layer(cors_cfg))`
/// —— 两个参数是两份不同配置（`ApidocConfig` 与 `CorsConfig`），不是同一个值。
/// 注意 layer 必须在 merge 之后调用（或对最终 router 调用），否则 CORS 只覆盖
/// /apidoc 两条路由；M4 要的是用户自己的业务路由也能被浏览器跨域直连。
/// 方法/头默认全放行（M4 要任意 method 与 Content-Type/Authorization 等头）。
pub fn cors_layer(config: CorsConfig) -> CorsLayer {
    let wildcard = config.allow_origins.iter().any(|o| o == "*");
    let origin = if config.allow_origins.is_empty() || wildcard {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(config.allow_origins.into_iter().map(|o| {
            HeaderValue::from_str(&o)
                .unwrap_or_else(|_| panic!("apidoc: invalid origin `{o}` in CorsConfig.allow_origins"))
        }))
    };
    // expose_headers(Any) 与 actix 侧对齐：actix-cors 默认把所有响应头放进
    // Access-Control-Expose-Headers，tower-http 默认一个都不放——同一份配置
    // 在两个适配器上必须能读到同样的头。
    CorsLayer::new()
        .allow_origin(origin)
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any)
}

/// 挂载 GET /apidoc（UI）与 GET /apidoc/api.json（数据），返回 Router<()>，
/// 用户用 `Router::new()....merge(apidoc_routes(cfg))` 接入。
/// api.json 内容 = DocRegistry::collect() 原样输出（核心数据模型零改动）；
/// 分组是纯 UI 侧启发式（见 ui.html），M3 的 group 注解上线后 UI 优先用注解。
pub fn apidoc_routes(config: ApidocConfig) -> Router {
    // 构建期一次性物化全部文档产物（api.json/export/mock），请求期零重算。
    let core = Arc::new(Core::build(config));
    let debug_limit = core.debug_body_limit();
    let router = Router::new()
        // UI 页与宠物图标不设守卫（见 route_core::UI/PET）。
        .route("/apidoc", get(|| async { response(route_core::UI) }))
        .route("/apidoc/pet.svg", get(|| async { response(route_core::PET) }))
        // 登录用 POST：密码走 body，不进 URL/日志（GET 的 ?password= 已废弃）
        .route("/apidoc/auth", post(auth))
        // 以下数据路由均经 route_core 守卫，失败 401 DENIED_BODY
        .route("/apidoc/api.json", get(api_json))
        .route("/apidoc/export", get(export))
        .route("/apidoc/mock", get(mock))
        .route("/apidoc/share", get(share))
        .route("/apidoc/generate", get(generate))
        // P0 服务端调试：POST body 为 DebugRequest；未启用时 route_core 返回 403
        .route(
            "/apidoc/debug",
            post(debug).layer(DefaultBodyLimit::max(debug_limit)),
        )
        // P6 webhook 入站捕获：POST 公开（HMAC 可选），GET 走守卫。
        .route("/apidoc/webhook", get(webhook_list))
        .route(
            "/apidoc/webhook/{id}",
            get(webhook_events)
                .post(webhook_capture)
                // 未认证写入口：缓冲上限是 DebugConfig 之外的绝对硬顶。
                .layer(DefaultBodyLimit::max(crate::webhook::MAX_BODY)),
        );
    // P2 长连接会话桥：仅在 feature 开启时注册。
    #[cfg(feature = "debug-session")]
    let router = router.route("/apidoc/debug/stream", get(debug_stream));
    router.with_state(core)
}

// 各 handler 一行转发 core：判定与渲染在 route_core（两框架共享同一实现）。
async fn auth(State(core): State<Arc<Core>>, body: Bytes) -> Response {
    response(core.auth(&body))
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

async fn debug(
    State(core): State<Arc<Core>>,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    response(core.debug(&q, &body).await)
}

async fn webhook_list(
    State(core): State<Arc<Core>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    response(core.webhook_list(&q))
}

async fn webhook_events(
    State(core): State<Arc<Core>>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    response(core.webhook_events(&q, &id))
}

async fn webhook_capture(
    State(core): State<Arc<Core>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    response(core.webhook_capture(&id, &header_map(&headers), &body))
}

/// 捕获展示用头快照；非 UTF-8 值跳过，键统一小写（与 actix 侧一致）。
fn header_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(k, v)| {
            Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string()))
        })
        .collect()
}

#[cfg(feature = "debug-session")]
async fn debug_stream(
    State(core): State<Arc<Core>>,
    Query(q): Query<HashMap<String, String>>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    match core.stream_config(&q) {
        Ok(cfg) => ws.on_upgrade(move |socket| async move {
            crate::debug::session::run(&cfg, None, AxumSocket { ws: socket }).await;
        }),
        Err(r) => response(r),
    }
}

#[cfg(feature = "debug-session")]
struct AxumSocket {
    ws: axum::extract::ws::WebSocket,
}

#[cfg(feature = "debug-session")]
impl crate::debug::session::DebugSocket for AxumSocket {
    async fn send_text(&mut self, text: String) -> Result<(), String> {
        use axum::extract::ws::Message;
        self.ws
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| e.to_string())
    }

    async fn recv_text(&mut self) -> Option<Result<String, String>> {
        use axum::extract::ws::Message;
        loop {
            match self.ws.recv().await {
                Some(Ok(Message::Text(t))) => return Some(Ok(t.to_string())),
                Some(Ok(Message::Binary(_))) => return Some(Err("binary control frame".into())),
                Some(Ok(Message::Close(_))) | None => return None,
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Some(Err(e.to_string())),
            }
        }
    }

    async fn close(&mut self) {
        use axum::extract::ws::Message;
        let _ = self.ws.send(Message::Close(None)).await;
    }
}

/// route_core::R → axum Response：Content-Type 为 None 时不设头（对齐现状空响应）。
fn response(r: R) -> Response {
    let mut res = Response::new(Body::from(r.body));
    *res.status_mut() = StatusCode::from_u16(r.status).expect("valid status");
    if let Some(ct) = r.ct {
        res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    }
    // 这些响应里带着 /apidoc/auth 的凭据与 token，任何一层缓存都不该留存
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}
