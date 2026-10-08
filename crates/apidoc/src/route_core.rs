//! 框架无关的 handler core（feature "axum" / "actix"）：守卫判定、响应构造、
//! export / mock / share / generate 的全部分发逻辑收敛于此，并定义两适配器共享的
//! `CorsConfig`。适配器文件只留路由注册 + 状态注入（见 src/axum.rs / src/actix.rs）——
//! 两个框架行为 1:1 由单一实现结构性保证，不再靠两份复制维护。

use crate::auth::{self, AuthConfig};
use crate::mock::{generate_mock, mock_specs, MockEndpointSpec};
use crate::{cache, codegen, export, share};
use crate::{ApiDoc, ApidocConfig, AppConfig, CacheConfig, CodegenTemplate, DocRegistry};
use std::borrow::Cow;
use std::collections::HashMap;

/// 统一响应形状：状态码 + Content-Type + body，两个适配器各自映射为框架响应。
/// `ct: None` = 不设 Content-Type（对齐现状：share/generate 的 400 与 generate 的
/// 404 是无头空响应）。body 用 `Cow`：UI 页 / pet.svg 是 `Borrowed` 静态串（零拷贝），
/// 动态 JSON / 渲染产物是 `Owned`。
pub struct R {
    pub status: u16,
    pub ct: Option<&'static str>,
    pub body: Cow<'static, str>,
}

impl R {
    fn new(status: u16, ct: &'static str, body: impl Into<Cow<'static, str>>) -> R {
        R { status, ct: Some(ct), body: body.into() }
    }

    /// 无 Content-Type 的空响应（现状 share/generate 400 与 generate 404 的形状）。
    fn empty(status: u16) -> R {
        R { status, ct: None, body: Cow::Borrowed("") }
    }
}

/// UI 页与宠物图标（favicon + 页头 logo）：UI 内容的一部分，不设守卫。
pub const UI: R = R { status: 200, ct: Some("text/html; charset=utf-8"), body: Cow::Borrowed(crate::UI_HTML) };
pub const PET: R = R { status: 200, ct: Some("image/svg+xml"), body: Cow::Borrowed(crate::PET_SVG) };

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
///
/// 两适配器经 `pub use` 各自转发（`apidoc::axum::CorsConfig` / `apidoc::actix::CorsConfig`），
/// 仅 `cors_layer` 的返回类型因框架而异（tower-http / actix-cors）。
#[derive(Default)]
pub struct CorsConfig {
    pub allow_origins: Vec<String>,
}

/// 构建期物化的共享状态：适配器建路由时构造一次（Arc<Core>），请求期只读。
pub struct Core {
    doc: ApiDoc,
    api_json: String,
    md: String,
    ts: String,
    sw: String,
    mocks: Vec<MockEndpointSpec>,
    auth_cfg: Option<AuthConfig>,
    apps: Vec<AppConfig>,
    codegen_cfg: Vec<CodegenTemplate>,
    cache_cfg: Option<CacheConfig>,
    share_cfg: ApidocConfig,
}

impl Core {
    pub fn build(config: ApidocConfig) -> Core {
        let doc = DocRegistry::collect_doc(config);
        // 构建期序列化一次：ApiDoc 无 Clone（核心约束），且请求期不能借出捕获的
        // 自身引用 —— api.json / 三份导出全部预物化为 String 是唯一干净解。
        let api_json = serde_json::to_string(&doc).expect("ApiDoc must serialize");
        // M4 mock：只需可 Clone 的子集，不碰 ApiDoc/DocEndpoint，api.json 输出零变化。
        let mocks = mock_specs(&doc.endpoints);
        let md = export::markdown::render(&doc);
        let ts = export::typescript::render(&doc);
        // 版本号单一来源：Cargo 包版本（env! 编译期常量，随 Cargo.toml 走）
        let sw = serde_json::to_string(&export::swagger::render(&doc, env!("CARGO_PKG_VERSION")))
            .expect("swagger must serialize");
        // 鉴权配置与应用树按需捕获（password/secret_key 只在构建期内存中）
        let auth_cfg = doc.config.auth.clone();
        let apps = doc.config.apps.clone();
        let codegen_cfg = doc.config.codegen.clone();
        // v2 文档缓存：仅 mock 每请求有实际计算（api.json/export 构建期已物化一次，
        // 恒等重建无意义）——开启后按 (url,method) 记忆化 mock 输出。
        let cache_cfg = doc.config.cache.clone();
        // v2 分享 token 签发只看 auth + apps，构造轻量快照避免再 clone 整个 doc。
        let share_cfg = ApidocConfig {
            auth: doc.config.auth.clone(),
            apps: doc.config.apps.clone(),
            ..Default::default()
        };
        Core { doc, api_json, md, ts, sw, mocks, auth_cfg, apps, codegen_cfg, cache_cfg, share_cfg }
    }

    /// 数据路由守卫，auth 未启用恒放行。
    fn guard(&self, q: &HashMap<String, String>) -> bool {
        auth::auth_guard_ok(
            q.get("token").map(String::as_str).unwrap_or(""),
            q.get("appKey").map(String::as_str),
            self.auth_cfg.as_ref(),
            &self.apps,
        )
    }

    /// 守卫失败：401 + DENIED_BODY（body 是 JSON，Content-Type 同 actix 现状）。
    fn denied() -> R {
        R::new(401, "application/json", auth::DENIED_BODY)
    }

    /// GET /apidoc/auth?password=<md5>&appKey=...（appKey 应用密码优先）。
    /// auth 未启用 → 404 空 body（与原行为一致）。
    pub fn auth(&self, q: &HashMap<String, String>) -> R {
        let (status, body) = auth::auth_result_response(auth::auth_issue(
            q.get("password").map(String::as_str).unwrap_or(""),
            q.get("appKey").map(String::as_str),
            self.auth_cfg.as_ref(),
            &self.apps,
        ));
        R::new(status, "application/json", body)
    }

    /// GET /apidoc/api.json：守卫后原样返回构建期物化的文档。
    pub fn api_json(&self, q: &HashMap<String, String>) -> R {
        if !self.guard(q) {
            return Self::denied();
        }
        R::new(200, "application/json", self.api_json.clone())
    }

    /// GET /apidoc/export?format=md|ts|swagger，未知 format 400（无 Content-Type，对齐现状）。
    pub fn export(&self, q: &HashMap<String, String>) -> R {
        if !self.guard(q) {
            return Self::denied();
        }
        match q.get("format").map(String::as_str) {
            Some("md") => R::new(200, "text/markdown", self.md.clone()),
            Some("ts") => R::new(200, "application/typescript", self.ts.clone()),
            Some("swagger") => R::new(200, "application/json", self.sw.clone()),
            _ => R::empty(400),
        }
    }

    /// GET /apidoc/mock?url=&method=：url + method 精确匹配，未命中 404。
    pub fn mock(&self, q: &HashMap<String, String>) -> R {
        if !self.guard(q) {
            return Self::denied();
        }
        let url = q.get("url").map(String::as_str).unwrap_or("");
        let method = q.get("method").map(String::as_str).unwrap_or("");
        match self.mocks.iter().find(|s| s.url == url && s.method == method) {
            Some(spec) => {
                let build = || {
                    serde_json::to_string(&generate_mock(spec)).expect("mock must serialize")
                };
                // v2 文档缓存：开启时按 (url,method) 记忆化；未开启走原路径（字节级一致）
                let body = match &self.cache_cfg {
                    Some(c) if c.enable => cache::memo(&format!("mock:{url}:{method}"), c.ttl, build),
                    _ => build(),
                };
                R::new(200, "application/json", body)
            }
            None => R::new(404, "application/json", r#"{"error":"endpoint not found"}"#),
        }
    }

    /// GET /apidoc/share?app=&url=&method=&base= → {"url":"..."}
    /// 生成指定应用/接口的分享深链；鉴权开启时附带服务端签发的 token（打开免密）。
    pub fn share(&self, q: &HashMap<String, String>) -> R {
        // 守卫必须校验"被分享的那个应用"：本路由用 `app` 指定应用（与 UI 深链
        // 参数一致），若沿用其它路由的 `appKey` 守卫，守卫看不到应用名 → 落全局
        // 分支放行 → 本路由就变成"无凭据铸造应用 token"的口子（全局鉴权关闭时
        // 直接绕过应用密码；全局 token 持有者也能借此提权到任意独立密码应用）。
        let guard_app = q.get("app").or_else(|| q.get("appKey")).map(String::as_str);
        if !auth::auth_guard_ok(
            q.get("token").map(String::as_str).unwrap_or(""),
            guard_app,
            self.auth_cfg.as_ref(),
            &self.apps,
        ) {
            return Self::denied();
        }
        let app_key = q.get("app").map(String::as_str);
        let url = q.get("url").map(String::as_str);
        if app_key.is_none() && url.is_none() {
            return R::empty(400);
        }
        let method = q.get("method").map(String::as_str).unwrap_or("GET");
        let ep =
            url.and_then(|u| self.doc.endpoints.iter().find(|e| e.url == u && e.method == method));
        let token = share::token_for(&self.share_cfg, app_key);
        let base = q.get("base").map(String::as_str).unwrap_or("");
        let body = serde_json::json!({
            "url": share::build_url(base, app_key, ep, token.as_deref())
        })
        .to_string();
        R::new(200, "application/json", body)
    }

    /// GET /apidoc/generate?template=<name> → 渲染后的代码；未知模板 404（无 Content-Type）。
    pub fn generate(&self, q: &HashMap<String, String>) -> R {
        if !self.guard(q) {
            return Self::denied();
        }
        let Some(name) = q.get("template").map(String::as_str) else {
            return R::empty(400);
        };
        let Some(tpl) = codegen::find(name, &self.codegen_cfg) else {
            return R::empty(404);
        };
        R::new(200, "text/plain; charset=utf-8", codegen::render(tpl, &self.doc))
    }
}
