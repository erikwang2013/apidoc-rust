//! 薄 actix-web 适配器（feature "actix"）：挂载 /apidoc（UI 页）与 /apidoc/api.json（数据）。
//! 与 axum 适配器行为 1:1 对齐（404/400 语义、Content-Type、mock 匹配逻辑），
//! 不做 UI 内嵌三方文档工具、不做服务端代理（规划已定）。

use actix_cors::Cors;
use actix_web::{web, HttpResponse, Scope};
use crate::auth::{self, AuthConfig};
use crate::export;
use crate::mock::{generate_mock, mock_specs, MockEndpointSpec};
use crate::{cache, codegen, share};
use crate::{AppConfig, ApidocConfig, CacheConfig, CodegenTemplate, DocRegistry};
use std::collections::HashMap;
use std::sync::Arc;

/// CORS 策略（为 M4 在线调试跨域直连目标接口做准备）。
///
/// `allow_origins` 为空（默认）：`Access-Control-Allow-Origin: *`，不携带凭据。
/// 宽松但安全：`*` + 无凭据只允许跨域读响应，这是调试工具的意图。
///
/// `allow_origins` 非空：精确匹配白名单（安全收紧模式，如 ["http://localhost:3000"]）。
///
/// 两种模式都不开 allow_credentials —— 反射任意 Origin + 凭据才是会被安全
/// 审查打回的组合，M2 直接不提供该开关。
/// ponytail: allow_credentials 字段暂不做，等真有"白名单 + 凭据"需求再加。
#[derive(Default)]
pub struct CorsConfig {
    pub allow_origins: Vec<String>,
}

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
        config
            .allow_origins
            .iter()
            .fold(cors, |c, o| c.allowed_origin(o))
    }
}

/// 挂载 GET /apidoc（UI）与 GET /apidoc/api.json（数据），返回 Scope，
/// 用户用 `App::new().service(apidoc_routes(cfg))` 接入。
/// api.json 内容 = DocRegistry::collect() 原样输出（核心数据模型零改动）；
/// 分组是纯 UI 侧启发式（见 ui.html），M3 的 group 注解上线后 UI 优先用注解。
/// ui.html 共享自本 crate 根（crates/apidoc/src/ui.html），与 axum 适配器同一份。
pub fn apidoc_routes(config: ApidocConfig) -> Scope {
    let doc = DocRegistry::collect_doc(config);
    // 构建期序列化一次：ApiDoc 无 Clone（核心约束），预序列化 String 是唯一干净解。
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
    // v2：分享要按 url+method 找 endpoint、代码生成要整个文档模型 ——
    // ApiDoc 无 Clone（核心约束），用 Arc 捕获（构建期不可变，共享安全）。
    let doc = Arc::new(doc);
    let codegen_cfgs: Arc<Vec<CodegenTemplate>> = Arc::new(doc.config.codegen.clone());
    // v2 文档缓存：仅 mock 每请求有实际计算（api.json/export 构建期已物化一次）。
    let cache_cfg: Option<CacheConfig> = doc.config.cache.clone();
    // v2 分享 token 签发只看 auth + apps，构造轻量快照。
    let share_cfg = Arc::new(ApidocConfig {
        auth: doc.config.auth.clone(),
        apps: doc.config.apps.clone(),
        ..Default::default()
    });
    // auth_cfg / app_cfgs 被多个路由共享：外层块 clone 一份给当前路由的 move
    // 闭包（避免 E0382）；闭包体内再 clone 成局部变量，async move 只捕获局部
    // 变量（避免 FnOnce）。api_json/mocks/md/ts/sw 单路由独占，一层 clone 即可。
    web::scope("/apidoc")
        .route(
            "",
            web::get().to(|| async {
                HttpResponse::Ok()
                    .content_type("text/html; charset=utf-8")
                    .body(crate::UI_HTML)
            }),
        )
        // UI 宠物图标：favicon 与页头 logo（UI 页内容的一部分，与 UI 页同样不设守卫）
        .route(
            "/pet.svg",
            web::get().to(|| async {
                HttpResponse::Ok()
                    .content_type("image/svg+xml")
                    .body(crate::PET_SVG)
            }),
        )
        // GET /apidoc/auth?password=<md5>&appKey=...（appKey 应用密码优先）
        .route(
            "/auth",
            web::get().to({
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                move |q: web::Query<HashMap<String, String>>| {
                    let auth_cfg = auth_cfg.clone();
                    let app_cfgs = app_cfgs.clone();
                    async move {
                        let (status, body) = auth::auth_result_response(auth::auth_issue(
                            q.get("password").map(String::as_str).unwrap_or(""),
                            q.get("appKey").map(String::as_str),
                            auth_cfg.as_deref(),
                            &app_cfgs,
                        ));
                        HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap())
                            .content_type("application/json")
                            .body(body)
                    }
                }
            }),
        )
        .route(
            "/api.json",
            web::get().to({
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                let body = api_json.clone();
                move |q: web::Query<HashMap<String, String>>| {
                    let auth_cfg = auth_cfg.clone();
                    let app_cfgs = app_cfgs.clone();
                    let body = body.clone();
                    async move {
                        if !auth_guard_ok(&q, auth_cfg.as_deref(), &app_cfgs) {
                            return HttpResponse::Unauthorized()
                                .content_type("application/json")
                                .body(auth::DENIED_BODY);
                        }
                        HttpResponse::Ok().content_type("application/json").body(body)
                    }
                }
            }),
        )
        .route(
            "/export",
            web::get().to({
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                let (md, ts, sw) = (md.clone(), ts.clone(), sw.clone());
                move |q: web::Query<HashMap<String, String>>| {
                    let auth_cfg = auth_cfg.clone();
                    let app_cfgs = app_cfgs.clone();
                    let (md, ts, sw) = (md.clone(), ts.clone(), sw.clone());
                    async move {
                        if !auth_guard_ok(&q, auth_cfg.as_deref(), &app_cfgs) {
                            return HttpResponse::Unauthorized()
                                .content_type("application/json")
                                .body(auth::DENIED_BODY);
                        }
                        let (ct, body) = match q.get("format").map(String::as_str) {
                            Some("md") => ("text/markdown", md),
                            Some("ts") => ("application/typescript", ts),
                            Some("swagger") => ("application/json", sw),
                            _ => return HttpResponse::BadRequest().finish(),
                        };
                        HttpResponse::Ok().content_type(ct).body(body)
                    }
                }
            }),
        )
        .route(
            "/mock",
            web::get().to({
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                let mocks = mocks.clone();
                let cache_cfg = cache_cfg.clone();
                move |q: web::Query<HashMap<String, String>>| {
                    let auth_cfg = auth_cfg.clone();
                    let app_cfgs = app_cfgs.clone();
                    let mocks = mocks.clone();
                    let cache_cfg = cache_cfg.clone();
                    async move {
                        if !auth_guard_ok(&q, auth_cfg.as_deref(), &app_cfgs) {
                            return HttpResponse::Unauthorized()
                                .content_type("application/json")
                                .body(auth::DENIED_BODY);
                        }
                        let url = q.get("url").map(String::as_str).unwrap_or("");
                        let method = q.get("method").map(String::as_str).unwrap_or("");
                        match mocks.iter().find(|s| s.url == url && s.method == method) {
                            Some(spec) => {
                                let build = || {
                                    serde_json::to_string(&generate_mock(spec))
                                        .expect("mock must serialize")
                                };
                                // v2 文档缓存：开启时按 (url,method) 记忆化；未开启走原路径
                                let body = match &cache_cfg {
                                    Some(c) if c.enable => {
                                        cache::memo(&format!("mock:{url}:{method}"), c.ttl, build)
                                    }
                                    _ => build(),
                                };
                                HttpResponse::Ok().content_type("application/json").body(body)
                            }
                            None => HttpResponse::NotFound()
                                .content_type("application/json")
                                .body(r#"{"error":"endpoint not found"}"#),
                        }
                    }
                }
            }),
        )
        // v2：GET /apidoc/share?app=&url=&method=&base= → {"url":"..."}
        .route(
            "/share",
            web::get().to({
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                let doc = doc.clone();
                let share_cfg = share_cfg.clone();
                move |q: web::Query<HashMap<String, String>>| {
                    let auth_cfg = auth_cfg.clone();
                    let app_cfgs = app_cfgs.clone();
                    let doc = doc.clone();
                    let share_cfg = share_cfg.clone();
                    async move {
                        // 守卫必须校验"被分享的那个应用"：本路由用 `app` 指定应用（与 UI 深链
                        // 参数一致），若沿用其它路由的 `appKey` 守卫，守卫看不到应用名 → 落全局
                        // 分支放行 → 本路由就变成"无凭据铸造应用 token"的口子（全局鉴权关闭时
                        // 直接绕过应用密码；全局 token 持有者也能借此提权到任意独立密码应用）。
                        let guard_app = q
                            .get("app")
                            .or_else(|| q.get("appKey"))
                            .map(String::as_str);
                        if !auth::auth_guard_ok(
                            q.get("token").map(String::as_str).unwrap_or(""),
                            guard_app,
                            auth_cfg.as_deref(),
                            &app_cfgs,
                        ) {
                            return HttpResponse::Unauthorized()
                                .content_type("application/json")
                                .body(auth::DENIED_BODY);
                        }
                        let app_key = q.get("app").map(String::as_str);
                        let url = q.get("url").map(String::as_str);
                        if app_key.is_none() && url.is_none() {
                            return HttpResponse::BadRequest().finish();
                        }
                        let method = q.get("method").map(String::as_str).unwrap_or("GET");
                        let ep = url.and_then(|u| {
                            doc.endpoints.iter().find(|e| e.url == u && e.method == method)
                        });
                        let token = share::token_for(&share_cfg, app_key);
                        let base = q.get("base").map(String::as_str).unwrap_or("");
                        let body = serde_json::json!({
                            "url": share::build_url(base, app_key, ep, token.as_deref())
                        })
                        .to_string();
                        HttpResponse::Ok().content_type("application/json").body(body)
                    }
                }
            }),
        )
        // v2：GET /apidoc/generate?template=<name> → 渲染后的代码；未知模板 404。
        .route(
            "/generate",
            web::get().to({
                let auth_cfg = auth_cfg.clone();
                let app_cfgs = app_cfgs.clone();
                let doc = doc.clone();
                let codegen_cfgs = codegen_cfgs.clone();
                move |q: web::Query<HashMap<String, String>>| {
                    let auth_cfg = auth_cfg.clone();
                    let app_cfgs = app_cfgs.clone();
                    let doc = doc.clone();
                    let codegen_cfgs = codegen_cfgs.clone();
                    async move {
                        if !auth_guard_ok(&q, auth_cfg.as_deref(), &app_cfgs) {
                            return HttpResponse::Unauthorized()
                                .content_type("application/json")
                                .body(auth::DENIED_BODY);
                        }
                        let Some(name) = q.get("template").map(String::as_str) else {
                            return HttpResponse::BadRequest().finish();
                        };
                        let Some(tpl) = codegen::find(name, &codegen_cfgs) else {
                            return HttpResponse::NotFound().finish();
                        };
                        HttpResponse::Ok()
                            .content_type("text/plain; charset=utf-8")
                            .body(codegen::render(tpl, &doc))
                    }
                }
            }),
        )
}

/// M6a：数据路由守卫（对齐 axum 行为），auth 未启用恒放行。
fn auth_guard_ok(
    q: &HashMap<String, String>,
    auth_cfg: Option<&AuthConfig>,
    app_cfgs: &[AppConfig],
) -> bool {
    auth::auth_guard_ok(
        q.get("token").map(String::as_str).unwrap_or(""),
        q.get("appKey").map(String::as_str),
        auth_cfg,
        app_cfgs,
    )
}
