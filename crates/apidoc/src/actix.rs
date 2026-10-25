//! 薄 actix-web 适配器（feature "actix"）：只做路由注册 + 状态注入（web::Data<Arc<Core>>），
//! 守卫/分支/响应渲染全部在 crate::route_core（与 axum 同一份实现，行为 1:1
//! 由结构性共享保证：404/400 语义、Content-Type、mock 匹配逻辑）。
//! 服务端调试路由默认关闭；开启需 DebugConfig + 全局鉴权 + feature `debug-http`。

use actix_cors::Cors;
use actix_web::{web, HttpResponse, Scope};
use crate::route_core::{self, Core, R};
use crate::ApidocConfig;
use std::collections::{BTreeMap, HashMap};
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
    // `"*"` 与空列表同样走通配：actix-cors 的白名单遇到 `*` 只在 wrap 期报
    // 一个没有上下文的 Transform error（见 route_core::CorsConfig 注释）。
    if config.allow_origins.is_empty() || config.allow_origins.iter().any(|o| o == "*") {
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
    let scope = web::scope("/apidoc")
        .app_data(core)
        // UI 页与宠物图标不设守卫（见 route_core::UI/PET）。
        .route("", web::get().to(|| async { response(route_core::UI) }))
        .route("/pet.svg", web::get().to(|| async { response(route_core::PET) }))
        // 登录用 POST：密码走 body，不进 URL/日志（GET 的 ?password= 已废弃）
        .route("/auth", web::post().to(auth))
        // 以下数据路由均经 route_core 守卫，失败 401 DENIED_BODY
        .route("/api.json", web::get().to(api_json))
        .route("/export", web::get().to(export))
        .route("/mock", web::get().to(mock))
        .route("/share", web::get().to(share))
        .route("/generate", web::get().to(generate))
        // P0 服务端调试：POST body 为 DebugRequest；未启用时 route_core 返回 403
        .route("/debug", web::post().to(debug))
        // P6 webhook 入站捕获：POST 公开（HMAC 可选），GET 走守卫。
        .route("/webhook", web::get().to(webhook_list))
        .service(
            web::resource("/webhook/{id}")
                // 未认证写入口：缓冲上限是绝对硬顶（默认 actix 256 KiB 会低于配置上限）。
                .app_data(web::PayloadConfig::new(crate::webhook::MAX_BODY))
                .route(web::get().to(webhook_events))
                .route(web::post().to(webhook_capture)),
        );
    #[cfg(feature = "debug-session")]
    let scope = scope.route("/debug/stream", web::get().to(debug_stream));
    scope
}

// 各 handler 一行转发 core：判定与渲染在 route_core（两框架共享同一实现）。
async fn auth(core: web::Data<Arc<Core>>, body: web::Bytes) -> HttpResponse {
    response(core.auth(&body))
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

async fn debug(
    core: web::Data<Arc<Core>>,
    q: web::Query<HashMap<String, String>>,
    body: web::Bytes,
) -> HttpResponse {
    response(core.debug(&q, &body).await)
}

async fn webhook_list(
    core: web::Data<Arc<Core>>,
    q: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    response(core.webhook_list(&q))
}

async fn webhook_events(
    core: web::Data<Arc<Core>>,
    path: web::Path<String>,
    q: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    response(core.webhook_events(&q, &path))
}

async fn webhook_capture(
    core: web::Data<Arc<Core>>,
    path: web::Path<String>,
    req: actix_web::HttpRequest,
    body: web::Bytes,
) -> HttpResponse {
    response(core.webhook_capture(&path, &header_map(&req), &body))
}

/// 捕获展示用头快照；非 UTF-8 值跳过，键统一小写（与 axum 侧一致）。
fn header_map(req: &actix_web::HttpRequest) -> BTreeMap<String, String> {
    req.headers()
        .iter()
        .filter_map(|(k, v)| {
            Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string()))
        })
        .collect()
}

#[cfg(feature = "debug-session")]
async fn debug_stream(
    req: actix_web::HttpRequest,
    body: web::Payload,
    core: web::Data<Arc<Core>>,
    q: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse, actix_web::Error> {
    let cfg = match core.stream_config(&q) {
        Ok(c) => c,
        Err(r) => return Ok(response(r)),
    };
    let (response, session, stream) = actix_ws::handle(&req, body)?;
    actix_web::rt::spawn(async move {
        crate::debug::session::run(&cfg, None, ActixSocket { tx: session, rx: stream }).await;
    });
    Ok(response)
}

#[cfg(feature = "debug-session")]
struct ActixSocket {
    tx: actix_ws::Session,
    rx: actix_ws::MessageStream,
}

#[cfg(feature = "debug-session")]
impl crate::debug::session::DebugSocket for ActixSocket {
    async fn send_text(&mut self, text: String) -> Result<(), String> {
        self.tx.text(text).await.map_err(|e| e.to_string())
    }

    async fn recv_text(&mut self) -> Option<Result<String, String>> {
        loop {
            match self.rx.recv().await {
                Some(Ok(actix_ws::Message::Text(t))) => return Some(Ok(t.to_string())),
                Some(Ok(actix_ws::Message::Binary(_))) => {
                    return Some(Err("binary control frame".into()))
                }
                Some(Ok(actix_ws::Message::Close(_))) | None => return None,
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Some(Err(e.to_string())),
            }
        }
    }

    async fn close(&mut self) {
        // actix_ws::Session::close 消费 self；Session 是 Clone，用副本关闭。
        let tx = self.tx.clone();
        let _ = tx.close(None).await;
    }
}

/// route_core::R → HttpResponse：Content-Type 为 None 时不设头（对齐现状空响应）。
fn response(r: R) -> HttpResponse {
    let mut builder =
        HttpResponse::build(actix_web::http::StatusCode::from_u16(r.status).expect("valid status"));
    if let Some(ct) = r.ct {
        builder.content_type(ct);
    }
    // 这些响应里带着 /apidoc/auth 的凭据与 token，任何一层缓存都不该留存
    builder.insert_header(("Cache-Control", "no-store"));
    builder.body(r.body)
}
