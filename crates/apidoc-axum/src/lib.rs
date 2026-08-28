//! 薄 axum 适配器：挂载 /apidoc（UI 页）与 /apidoc/api.json（数据）。
//! 不做 UI 内嵌三方文档工具、不做服务端代理（规划已定）。

use apidoc::{ApiDoc, ApidocConfig, DocRegistry};
use apidoc_mock::{generate_mock, mock_specs, MockEndpointSpec};
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
/// 两种模式都不开 allow_credentials —— 反射任意 Origin + 凭据才是会被安全
/// 审查打回的组合，M2 直接不提供该开关。
/// ponytail: allow_credentials 字段暂不做，等真有"白名单 + 凭据"需求再加。
pub struct CorsConfig {
    pub allow_origins: Vec<String>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        CorsConfig { allow_origins: Vec::new() }
    }
}

/// 生成 CORS 层。用法：`.merge(apidoc_axum::apidoc_routes(cfg)).layer(apidoc_axum::cors_layer(cfg))`。
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
    let doc = ApiDoc { config, endpoints: DocRegistry::collect() };
    // 构建期序列化一次：ApiDoc 无 Clone（核心约束），axum handler 要求 Clone，
    // 且 async 不能返回对自身捕获的借用 —— 预序列化 String 是唯一干净解。
    let api_json = serde_json::to_string(&doc).expect("ApiDoc must serialize");
    // M4 mock：只需可 Clone 的子集，handler 捕获 Arc<Vec<MockEndpointSpec>>，
    // 不碰 ApiDoc/DocEndpoint，api.json 输出零变化。
    let mocks: Arc<Vec<MockEndpointSpec>> = Arc::new(mock_specs(&doc.endpoints));
    Router::new()
        .route("/apidoc", get(|| async { Html(include_str!("ui.html")) }))
        .route("/apidoc/api.json", get(move || {
            let body = api_json.clone();
            async move {
                let mut headers = HeaderMap::new();
                headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
                (headers, body).into_response()
            }
        }))
        .route("/apidoc/mock", get(move |Query(q): Query<HashMap<String, String>>| {
            let mocks = mocks.clone();
            async move {
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
        }))
}
