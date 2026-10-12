//! 框架无关的 handler core（feature "axum" / "actix"）：守卫判定、响应构造、
//! export / mock / share / generate 的全部分发逻辑收敛于此，并定义两适配器共享的
//! `CorsConfig`。适配器文件只留路由注册 + 状态注入（见 src/axum.rs / src/actix.rs）——
//! 两个框架行为 1:1 由单一实现结构性保证，不再靠两份复制维护。

use crate::auth::{self, AuthConfig};
use crate::mock::{generate_mock, mock_specs, MockEndpointSpec};
use crate::{cache, codegen, export, share};
use crate::{ApiDoc, ApidocConfig, AppConfig, CacheConfig, DocRegistry};
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// 统一响应形状：状态码 + Content-Type + body，两个适配器各自映射为框架响应。
/// `ct: None` = 不设 Content-Type（对齐现状：share/generate 的 400 与 generate 的
/// 404 是无头空响应）。body 用 `Bytes`：UI 页 / pet.svg 是 `from_static`（零拷贝），
/// 构建期物化的 JSON / 导出产物克隆是引用计数自增——请求期不再复制整份文档。
pub struct R {
    pub status: u16,
    pub ct: Option<&'static str>,
    pub body: Bytes,
}

impl R {
    fn new(status: u16, ct: &'static str, body: impl Into<Bytes>) -> R {
        R { status, ct: Some(ct), body: body.into() }
    }

    /// 无 Content-Type 的空响应（现状 share/generate 400 与 generate 404 的形状）。
    fn empty(status: u16) -> R {
        R { status, ct: None, body: Bytes::new() }
    }
}

/// UI 页与宠物图标（favicon + 页头 logo）：UI 内容的一部分，不设守卫。
pub const UI: R = R { status: 200, ct: Some("text/html; charset=utf-8"), body: Bytes::from_static(crate::UI_HTML.as_bytes()) };
pub const PET: R = R { status: 200, ct: Some("image/svg+xml"), body: Bytes::from_static(crate::PET_SVG.as_bytes()) };

/// CORS 策略（为 M4 在线调试跨域直连目标接口做准备）。
///
/// `allow_origins` 为空（默认）或含 `"*"`：`Access-Control-Allow-Origin: *`，不携带凭据。
/// 宽松但安全：`*` + 无凭据只允许跨域读响应，这是调试工具的意图。
/// （`"*"` 走通配分支而不是进白名单——两个框架的白名单实现都拿 `*` 没辙：
/// tower-http 直接 panic，actix-cors 把错误推迟到 wrap 时变成莫名的 Transform error。）
///
/// `allow_origins` 非空且不含 `"*"`：精确匹配白名单（安全收紧模式，如 ["http://localhost:3000"]）。
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
    api_json: Bytes,
    md: Bytes,
    ts: Bytes,
    sw: Bytes,
    mocks: Vec<MockEndpointSpec>,
    /// 已渲染模板（内置 + config.codegen 同名覆盖），请求期只查表。
    generated: HashMap<String, Bytes>,
    auth_cfg: Option<AuthConfig>,
    apps: Vec<AppConfig>,
    cache_cfg: Option<CacheConfig>,
    /// 进程内唯一序号：mock 缓存是全局的，键里带上它以隔离同一进程里的多个 Core
    /// （同一进程挂多套 /apidoc 时，两边同 url+method 的 mock 会互相顶掉）。
    id: u64,
}

/// Core 的进程内序号源（见 `Core::id`）。
static CORE_SEQ: AtomicU64 = AtomicU64::new(0);

impl Core {
    pub fn build(config: ApidocConfig) -> Core {
        let doc = DocRegistry::collect_doc(config);
        // 构建期序列化一次：ApiDoc 无 Clone（核心约束），且请求期不能借出捕获的
        // 自身引用 —— api.json / 三份导出全部预物化为 Bytes 是唯一干净解。
        let api_json = Bytes::from(serde_json::to_string(&doc).expect("ApiDoc must serialize"));
        // M4 mock：只需可 Clone 的子集，不碰 ApiDoc/DocEndpoint，api.json 输出零变化。
        let mocks = mock_specs(&doc.endpoints);
        let md = Bytes::from(export::markdown::render(&doc));
        let ts = Bytes::from(export::typescript::render(&doc));
        // info.version：显式配了 ApidocConfig.version 就用它（被文档化的 API 版本），
        // 否则回落到 apidoc-rust 自身的包版本
        let api_version =
            doc.config.version.clone().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
        let sw = Bytes::from(
            serde_json::to_string(&export::swagger::render(&doc, &api_version))
                .expect("swagger must serialize"),
        );
        // 鉴权配置与应用树按需捕获（password/secret_key 只在构建期内存中）
        let auth_cfg = doc.config.auth.clone();
        let apps = doc.config.apps.clone();
        // secret_key 缺省时在构建期就取好随机密钥：熵源故障要在启动时暴露，
        // 而不是等到第一个登录/分享请求才 panic（OnceLock 失败后每次都会再 panic）
        if let Some(cfg) = auth_cfg.as_ref() {
            auth::warm_secret_key(cfg);
        } else if apps.iter().any(|a| a.password.as_deref().is_some_and(|p| !p.is_empty())) {
            auth::warm_secret_key(&AuthConfig::default());
        }
        // v2 文档缓存：仅 mock 每请求有实际计算（api.json/export 构建期已物化一次，
        // 恒等重建无意义）——开启后按 (url,method) 记忆化 mock 输出。
        let cache_cfg = doc.config.cache.clone();
        // /apidoc/generate：全部模板（内置 + config.codegen 同名覆盖）构建期渲染一次，
        // 请求期只查表——"请求期零重算" 由此对 7 条路由全部成立，模板渲染的深递归/
        // 展开预算也只在构建期需要。
        let names = codegen::TEMPLATES
            .iter()
            .map(|(n, _)| *n)
            .chain(doc.config.codegen.iter().map(|t| t.name.as_str()));
        let mut generated: HashMap<String, Bytes> = HashMap::new();
        for name in names {
            if let Some(tpl) = codegen::find(name, &doc.config.codegen) {
                generated.entry(name.to_string()).or_insert_with(|| Bytes::from(codegen::render(tpl, &doc)));
            }
        }
        let id = CORE_SEQ.fetch_add(1, Ordering::Relaxed);
        Core { doc, api_json, md, ts, sw, mocks, generated, auth_cfg, apps, cache_cfg, id }
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

    /// POST /apidoc/auth，body `{"password": "...", "appKey": "..."}`（appKey 应用密码优先）。
    /// 密码走请求体而不是 URL：不进访问日志/浏览器历史/Referer。非法 body → 400，
    /// 未配置任何密码 → 404 空 body。
    pub fn auth(&self, body: &[u8]) -> R {
        let Some(req) = auth::parse_login(body) else {
            return R::empty(400);
        };
        let (status, body) = auth::auth_result_response(auth::auth_issue(
            &req.password,
            req.app_key.as_deref(),
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
                // v2 文档缓存：开启时按 (Core, url, method) 记忆化；未开启走原路径（字节级一致）
                let body = match &self.cache_cfg {
                    Some(c) if c.enable => {
                        cache::memo(&format!("mock:{}:{url}:{method}", self.id), c.ttl, build)
                    }
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
        // 签发必须用与守卫同一个应用绑定：曾用 q.get("app") 单独取，`appKey`
        // 形式的请求守卫看得到应用、签发看不到 → 回落全局分支 → 应用密码持有者
        // 拿到全局 token（越权）。
        let app_key = guard_app;
        let url = q.get("url").map(String::as_str);
        if app_key.is_none() && url.is_none() {
            return R::empty(400);
        }
        let method = q.get("method").map(String::as_str).unwrap_or("GET");
        let ep =
            url.and_then(|u| self.doc.endpoints.iter().find(|e| e.url == u && e.method == method));
        let token = share::token_for(self.auth_cfg.as_ref(), &self.apps, app_key);
        let base = q.get("base").map(String::as_str).unwrap_or("");
        let body = serde_json::json!({
            "url": share::build_url(base, app_key, ep, token.as_deref())
        })
        .to_string();
        R::new(200, "application/json", body)
    }

    /// GET /apidoc/generate?template=<name> → 构建期渲染好的代码；未知模板 404（无 Content-Type）。
    pub fn generate(&self, q: &HashMap<String, String>) -> R {
        if !self.guard(q) {
            return Self::denied();
        }
        let Some(name) = q.get("template").map(String::as_str) else {
            return R::empty(400);
        };
        match self.generated.get(name) {
            Some(body) => R::new(200, "text/plain; charset=utf-8", body.clone()),
            None => R::empty(404),
        }
    }
}
