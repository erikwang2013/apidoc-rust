//! 薄 axum 适配器（feature "axum"）：挂载 /apidoc（UI 页）与 /apidoc/api.json（数据）。
//! 不做 UI 内嵌三方文档工具、不做服务端代理（规划已定）。

use crate::auth::{self, AuthConfig};
use crate::export;
use crate::mock::{generate_mock, mock_specs, MockEndpointSpec};
use crate::{AppConfig, ApidocConfig, DocRegistry};
use axum::extract::Query;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

/// CORS 策略（为 M4 在线调试跨域直连目标接口做准备）。
/// - `allow_origins` 为空（默认）：`Access-Control-Allow-Origin: *`，
///   不携带凭据。宽松但安全：`*` + 无凭据只允许跨域读响应，这是调试工具的意图。
/// - `allow_origins` 非空：精确匹配白名单（安全收紧模式，如 ["http://localhost:3000"]）。
///   两种模式都不开 allow_credentials —— 反射任意 Origin + 凭据才是会被安全
///   审查打回的组合，M2 直接不提供该开关。
///   ponytail: allow_credentials 字段暂不做，等真有"白名单 + 凭据"需求再加。
#[derive(Default)]
pub struct CorsConfig {
    pub allow_origins: Vec<String>,
}

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
    CorsLayer::new()
        .allow_origin(origin)
        .allow_methods(Any)
        .allow_headers(Any)
}

/// 挂载 GET /apidoc（UI）与 GET /apidoc/api.json（数据），返回 Router<()>，
/// 用户用 `Router::new()....merge(apidoc_routes(cfg))` 接入。
/// api.json 内容 = DocRegistry::collect() 原样输出（核心数据模型零改动）；
/// 分组是纯 UI 侧启发式（见 ui.html），M3 的 group 注解上线后 UI 优先用注解。
pub fn apidoc_routes(config: ApidocConfig) -> Router {
    let doc = DocRegistry::collect_doc(config);
    // 构建期序列化一次：ApiDoc 无 Clone（核心约束），axum handler 要求 Clone，
    // 且 async 不能返回对自身捕获的借用 —— 预序列化 String 是唯一干净解。
    let api_json = serde_json::to_string(&doc).expect("ApiDoc must serialize");
    // M4 mock：只需可 Clone 的子集，handler 捕获 Arc<Vec<MockEndpointSpec>>，
    // 不碰 ApiDoc/DocEndpoint，api.json 输出零变化。
    let mocks: Arc<Vec<MockEndpointSpec>> = Arc::new(mock_specs(&doc.endpoints));
    // M5 export：与 api.json 同模式，构建期预序列化三份 String。
    let md = export::markdown::render(&doc);
    let ts = export::typescript::render(&doc);
    // VERSION 为包内文件（crates/apidoc/VERSION），发布打包安全；发版时与根目录 VERSION 同步
    let sw = serde_json::to_string(&export::swagger::render(&doc, include_str!("../VERSION").trim()))
        .expect("swagger must serialize");
    // 鉴权配置与应用树按需捕获（password/secret_key 只在构建期内存中）
    let auth_cfg: Option<Arc<AuthConfig>> = doc.config.auth.clone().map(Arc::new);
    let app_cfgs: Arc<Vec<AppConfig>> = Arc::new(doc.config.apps.clone());
    // 数据路由守卫，失败 401；auth 未启用恒放行。无捕获闭包为 Copy，
    // 可被三个 handler 各自 move 一份
    let denied = || (StatusCode::UNAUTHORIZED, auth::DENIED_BODY).into_response();
    let guard = |q: &HashMap<String, String>, auth_cfg: Option<&AuthConfig>, app_cfgs: &[AppConfig]| {
        auth::auth_guard_ok(
            q.get("token").map(String::as_str).unwrap_or(""),
            q.get("appKey").map(String::as_str),
            auth_cfg,
            app_cfgs,
        )
    };
    // /apidoc/auth 响应映射与 actix 共用（auth_result_response）
    let auth_resp = |r: auth::AuthResult| {
        let (status, body) = auth::auth_result_response(r);
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        (StatusCode::from_u16(status).unwrap(), headers, body).into_response()
    };
    // auth_cfg / app_cfgs 被多个路由共享：外层块 clone 一份给当前路由的 move
    // 闭包（避免 E0382）；闭包体内再 clone 成局部变量，async move 只捕获局部
    // 变量（避免 FnOnce）。api_json/mocks/md/ts/sw 单路由独占，一层 clone 即可。
    Router::new()
        .route("/apidoc", get(|| async { Html(crate::UI_HTML) }))
        // M6a：GET /apidoc/auth?password=<md5>&appKey=...（appKey 应用密码优先）
        .route("/apidoc/auth", get({
            let auth_cfg = auth_cfg.clone();
            let app_cfgs = app_cfgs.clone();
            move |Query(q): Query<HashMap<String, String>>| {
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                async move {
                    auth_resp(auth::auth_issue(
                        q.get("password").map(String::as_str).unwrap_or(""),
                        q.get("appKey").map(String::as_str),
                        auth_cfg.as_deref(),
                        &app_cfgs,
                    ))
                }
            }
        }))
        .route("/apidoc/api.json", get({
            let auth_cfg = auth_cfg.clone();
            let app_cfgs = app_cfgs.clone();
            let body = api_json.clone();
            move |Query(q): Query<HashMap<String, String>>| {
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                async move {
                    if !guard(&q, auth_cfg.as_deref(), &app_cfgs) {
                        return denied();
                    }
                    let mut headers = HeaderMap::new();
                    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
                    (headers, body).into_response()
                }
            }
        }))
        .route("/apidoc/export", get({
            let auth_cfg = auth_cfg.clone();
            let app_cfgs = app_cfgs.clone();
            let (md, ts, sw) = (md.clone(), ts.clone(), sw.clone());
            move |Query(q): Query<HashMap<String, String>>| {
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                async move {
                    if !guard(&q, auth_cfg.as_deref(), &app_cfgs) {
                        return denied();
                    }
                    let (ct, body) = match q.get("format").map(String::as_str) {
                        Some("md") => ("text/markdown", md),
                        Some("ts") => ("application/typescript", ts),
                        Some("swagger") => ("application/json", sw),
                        _ => return StatusCode::BAD_REQUEST.into_response(),
                    };
                    let mut headers = HeaderMap::new();
                    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
                    (headers, body).into_response()
                }
            }
        }))
        .route("/apidoc/mock", get({
            let auth_cfg = auth_cfg.clone();
            let app_cfgs = app_cfgs.clone();
            let mocks = mocks.clone();
            move |Query(q): Query<HashMap<String, String>>| {
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                async move {
                    if !guard(&q, auth_cfg.as_deref(), &app_cfgs) {
                        return denied();
                    }
                    let url = q.get("url").map(String::as_str).unwrap_or("");
                    let method = q.get("method").map(String::as_str).unwrap_or("");
                    let (status, body) = match mocks.iter().find(|s| s.url == url && s.method == method) {
                        Some(spec) => (
                            StatusCode::OK,
                            serde_json::to_string(&generate_mock(spec)).expect("mock must serialize"),
                        ),
                        None => (StatusCode::NOT_FOUND, r#"{"error":"endpoint not found"}"#.to_string()),
                    };
                    let mut headers = HeaderMap::new();
                    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
                    (status, headers, body).into_response()
                }
            }
        }))
}
