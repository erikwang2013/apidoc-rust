//! apidoc runtime core: data model, distributed-slice fragment registry and
//! endpoint aggregation. Attribute macros are re-exported from apidoc-macros.
//!
//! Two things to know when consuming this crate:
//! - Registration happens via `linkme::distributed_slice`, so crates that only
//!   register documentation (no other use of this crate) must be linked, not
//!   merely built; call any exported item from the crate root to force it.
//! - The macros expand to paths like `apidoc::DocFragment`, so consumers must
//!   depend on `linkme` directly and re-export `distributed_slice` (this crate
//!   already re-exports it for convenience).

pub use apidoc_macros::*;
pub use linkme::distributed_slice;

/// M5: markdown / typescript / swagger(OpenAPI3) 三种导出格式。
pub mod export;

/// M6a: 密码鉴权（HMAC-SHA256 签名令牌 + 恒定时间比对 + 失败退避）。
pub mod auth;

/// M4 mock 引擎（feature "mock"；axum/actix feature 隐含启用）。
#[cfg(feature = "mock")]
pub mod mock;

/// axum 适配器（feature "axum"）。
#[cfg(feature = "axum")]
pub mod axum;

/// actix-web 适配器（feature "actix"）。
#[cfg(feature = "actix")]
pub mod actix;

/// 框架无关的 handler core：axum/actix 适配器共享的守卫、分支与响应构造。
#[cfg(any(feature = "axum", feature = "actix"))]
mod route_core;

/// 共享文档 UI（axum/actix 适配器 include_str! 自本 crate，发布打包安全）。
/// ui.html 为标记与样式，ui.js 为核心脚本，ui.codecs.js 为纯编解码，
/// ui.debug.js 为在线调试面板；编译期拼接为完整 HTML（同一 <script>，
/// 函数声明提升保证跨文件可见）。
pub const UI_HTML: &str = concat!(
    include_str!("ui.html"),
    include_str!("ui.js"),
    include_str!("ui.codecs.js"),
    include_str!("ui.protocols.js"),
    include_str!("ui.debug.js"),
    include_str!("ui.debug.stream.js"),
    include_str!("ui.debug.media.js")
);

/// 文档 UI 的宠物图标（favicon + 页头 logo），适配器挂在 `/apidoc/pet.svg`。
/// 与仓库 README 用的 docs/images/apidoc-pet.svg 同一份，包内自带以保证发布打包安全。
pub const PET_SVG: &str = include_str!("pet.svg");

// ---- v2 模块（接口已冻结，实现见各文件）----
/// v2 文档缓存：mock 输出的进程内记忆化（TTL 秒，0 = 永久）。
/// api.json / export 在构建期已物化，不经此模块。
pub mod cache;
/// v2 代码生成器：模板 + 文档模型 → 前端 Api 文件 / 接口骨架 / 建表 SQL。
pub mod codegen;
/// v2 分享链接：应用/接口深链 + 鉴权 token。
pub mod share;

use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};

pub mod protocol;

#[cfg(feature = "debug-http")]
mod debug;

/// Collects every `#[apidoc::*]` annotation from all linked crates.
#[distributed_slice]
pub static DOC_FRAGMENTS: [DocFragmentEntry];

/// One annotation: the endpoint id plus the annotated piece of documentation.
/// `seq` is assigned by the macro at expansion time (source order) and is used
/// by `DocRegistry::collect` to restore declaration order, since linkme's
/// linker-section iteration order is not source order.
pub struct DocFragmentEntry {
    pub id: &'static str,
    pub seq: u32,
    pub frag: DocFragment,
}

/// A single annotation payload.
/// `#[non_exhaustive]`：变体随版本增补（M3 → M6b → v2 已加过 3 轮），
/// 不标注的话每加一个都是下游 match 的破坏性变更。
#[non_exhaustive]
pub enum DocFragment {
    Title(&'static str),
    Desc(&'static str),
    Method(&'static str),
    Url(&'static str),
    Param(DocParam),
    Query(DocParam),
    Returned(DocParam),
    // M3: single-value fragments (later mounts win), list fragments (append),
    // flag fragments (OR), and the ref reference.
    Tag(&'static str),
    Group(&'static str),
    Author(&'static str),
    Header(DocHeader),
    RouteParam(DocParam),
    ResponseStatus(&'static str),
    Success(DocExample),
    Error(DocExample),
    NotDebug,
    Md(&'static str),
    Sort(i32),
    Ref(&'static str),
    // M6b: 挂到指定应用/版本 key 下（key 须在 ApidocConfig.apps 中配置，否则落默认应用）。
    App(&'static str),
    // v2: 数据表字段引用，key 须在 ApidocConfig.tables 中配置。
    Table(&'static str),
    // P0: 非 HTTP 协议及其连接/操作参数（target / service / method / topic …）。
    Protocol(DocProtocol),
    // P0: 长连接/流式协议的消息定义（WebSocket 帧、MQTT 消息、SSE 事件…）。
    Event(DocEvent),
}

/// `#[apidoc::protocol("grpc", target = "...", service = "...")]` 的原始载荷。
/// `opts` 走有序数组：宏产出静态切片，collect 时按声明顺序合并进 BTreeMap。
pub struct DocProtocol {
    pub kind: &'static str,
    pub opts: &'static [(&'static str, &'static str)],
}

/// 长连接/流式协议的一条消息定义。
#[derive(Clone, Serialize)]
pub struct DocEvent {
    /// "in"（服务端 → 客户端）或 "out"（客户端 → 服务端）。
    pub direction: &'static str,
    pub name: &'static str,
    #[serde(rename = "type")]
    pub ty: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mock: Option<&'static str>,
}

/// A documented example response body (shared by success / error).
#[derive(Clone, Serialize)]
pub struct DocExample {
    /// HTTP status code as written in the annotation ("200" / "404" …);
    /// validated by the macro, and normalized at export time when out of range.
    pub code: &'static str,
    /// Raw response body; stored verbatim, not validated as JSON.
    pub example: &'static str,
}

/// A documented parameter (body param, query string field or return field).
#[derive(Clone, Serialize)]
pub struct DocParam {
    pub name: &'static str,
    #[serde(rename = "type")]
    pub ty: &'static str,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mock: Option<&'static str>,
    /// 协议字段号：protobuf tag / Thrift field id / ISO 8583 域号。
    /// 仅非 HTTP 协议使用；未设置时省略，老文档输出不变。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<&'static str>,
    #[serde(skip_serializing_if = "slice_is_empty")]
    pub children: &'static [DocParam],
}

fn slice_is_empty<T>(s: &[T]) -> bool {
    s.is_empty()
}

/// One documented HTTP endpoint, built by merging fragments with the same id.
#[derive(Clone, Serialize)]
pub struct DocEndpoint {
    pub title: String,
    pub desc: String,
    pub url: String,
    pub method: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<DocHeader>,
    pub params: Vec<DocParam>,
    pub querys: Vec<DocParam>,
    pub returned: Vec<DocParam>,
    // M3 fields: all optional, all omitted from JSON when at their default
    // value, so api.json output is unchanged for endpoints that use none of
    // the new annotations.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub route_params: Vec<DocParam>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub response_status: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub success: Vec<DocExample>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub error: Vec<DocExample>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_debug: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub md: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub sort: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    // v2: `#[apidoc::table("key")]` 引用的数据表 key（未使用时省略，输出与 v1 字节级一致）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    // P0: 协议类型与连接参数。protocol 默认 "http" 时整体省略，老文档输出字节级不变。
    #[serde(skip_serializing_if = "protocol_is_http")]
    pub protocol: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub protocol_opts: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<DocEvent>,
    // 多应用归属（#[apidoc::app] 注解），仅 collect 用，不进任何序列化输出。
    #[serde(skip)]
    pub app_key: String,
}

fn protocol_is_http(p: &str) -> bool {
    p.is_empty() || p == "http"
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

impl Default for DocEndpoint {
    fn default() -> Self {
        DocEndpoint {
            title: String::new(),
            desc: String::new(),
            url: String::new(),
            method: "GET".to_string(),
            headers: Vec::new(),
            params: Vec::new(),
            querys: Vec::new(),
            returned: Vec::new(),
            group: String::new(),
            tags: Vec::new(),
            author: String::new(),
            route_params: Vec::new(),
            response_status: Vec::new(),
            success: Vec::new(),
            error: Vec::new(),
            not_debug: false,
            md: String::new(),
            sort: 0,
            r#ref: None,
            table: None,
            protocol: "http".to_string(),
            protocol_opts: BTreeMap::new(),
            events: Vec::new(),
            app_key: String::new(),
        }
    }
}

/// Request header documentation, e.g. `#[apidoc::header(name = "X-Token")]`.
#[derive(Clone, Serialize)]
pub struct DocHeader {
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<&'static str>,
}

/// 应用/版本配置树：key 为注解引用的唯一标识，title 为展示名，items 递归嵌套
/// 版本，password 为该应用的独立访问密码（优先级高于全局密码，永不序列化）。
#[derive(Clone)]
pub struct AppConfig {
    pub key: String,
    pub title: String,
    pub items: Vec<AppConfig>,
    pub password: Option<String>,
}

/// 按配置树在 ApidocConfig.apps 中递归查找应用配置。
pub fn find_app<'a>(apps: &'a [AppConfig], key: &str) -> Option<&'a AppConfig> {
    for app in apps {
        if app.key == key {
            return Some(app);
        }
        if let Some(found) = find_app(&app.items, key) {
            return Some(found);
        }
    }
    None
}

/// v2：数据表字段定义，`#[apidoc::table("key")]` 的数据来源。
/// Rust 侧不连数据库：表结构由配置提供，解析时按 key 取字段并入 returned。
#[derive(Clone, Default)]
pub struct TableDef {
    /// 注解引用的 key，如 `#[apidoc::table("user")]`。
    pub key: String,
    /// 展示名（进 codegen 模板，不进 api.json）。
    pub title: String,
    pub fields: Vec<TableField>,
}

/// v2：一个表字段（纯平铺，不支持 children —— 表字段是标量列）。
#[derive(Clone, Default)]
pub struct TableField {
    pub name: String,
    pub ty: String,
    pub required: bool,
    pub default: Option<String>,
    pub desc: Option<String>,
    pub mock: Option<String>,
}

/// v2：文档缓存配置。`ttl` 秒内命中即复用，0 表示永久有效。
#[derive(Clone, Default)]
pub struct CacheConfig {
    pub enable: bool,
    pub ttl: u64,
}

/// v2：代码生成器自定义模板。name 与内置模板同名时覆盖内置。
#[derive(Clone, Default)]
pub struct CodegenTemplate {
    pub name: String,
    pub template: String,
}

/// P0：服务端调试（协议代理）配置。默认关闭；开启后必须已配置鉴权，
/// 且默认只允许公网地址（内网/回环需显式 allow_private_networks）。
/// 敏感项 `#[serde(skip)]`，永不进 api.json。
#[derive(Clone, Serialize)]
pub struct DebugConfig {
    pub enable_server_side: bool,
    #[serde(skip)]
    pub allow_private_networks: bool,
    #[serde(skip)]
    pub allow_hosts: Vec<String>,
    #[serde(skip)]
    pub allow_unix_sockets: bool,
    pub timeout_ms: u64,
    pub max_bytes: usize,
}

impl Default for DebugConfig {
    fn default() -> Self {
        DebugConfig {
            enable_server_side: false,
            allow_private_networks: false,
            allow_hosts: Vec::new(),
            allow_unix_sockets: false,
            timeout_ms: 10_000,
            max_bytes: 1_048_576,
        }
    }
}

/// Project-level configuration, combined with endpoints into the final output.
#[derive(Serialize, Default)]
pub struct ApidocConfig {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    // 密码鉴权。None 时 api.json 输出与 M5 字节级一致（红线）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<auth::AuthConfig>,
    // P0 服务端调试配置。None 时输出与之前字节级一致；Some 时只序列化安全子集
    // （白名单/内网开关等敏感项 #[serde(skip)]），供 UI 决定是否显示「经服务端发送」。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebugConfig>,
    // 应用/版本配置树。仅服务端使用（校验注解 key、应用密码），不进任何输出。
    #[serde(skip)]
    pub apps: Vec<AppConfig>,
    // ---- v2 服务端配置：全部 #[serde(skip)]，未使用时 api.json 与 v1 字节级一致 ----
    /// 写进 OpenAPI `info.version` 的「被文档化的 API 版本」；缺省为 apidoc-rust 包版本。
    #[serde(skip)]
    pub version: Option<String>,
    #[serde(skip)]
    pub tables: Vec<TableDef>,
    #[serde(skip)]
    pub cache: Option<CacheConfig>,
    #[serde(skip)]
    pub codegen: Vec<CodegenTemplate>,
}

/// 一个应用/版本节点：注解挂载的 endpoints + 递归子版本。
#[derive(Serialize)]
pub struct AppDoc {
    pub key: String,
    pub title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<AppDoc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<DocEndpoint>,
}

/// Final aggregated document (the shape of api.json).
#[derive(Serialize)]
pub struct ApiDoc {
    pub config: ApidocConfig,
    pub endpoints: Vec<DocEndpoint>,
    // 应用/版本树。无 app 注解或未配置 apps 时省略，输出与 M5 字节级一致（红线）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<AppDoc>,
}

/// Collects and merges all registered fragments into per-endpoint documents.
pub struct DocRegistry;

impl DocRegistry {
    /// M1-M5 行为不变：仅返回合并后的端点列表。
    pub fn collect() -> Vec<DocEndpoint> {
        let (ids, mut endpoints) = Self::collect_fragments();
        Self::resolve_refs(&ids, &mut endpoints);
        endpoints
    }

    /// 构建完整文档（端点 + 应用/版本树）。apps 按 ApidocConfig.apps 配置树
    /// 挂载注解端点；app 注解引用未配置的 key 时 eprintln 警告并落默认应用（根层）。
    pub fn collect_doc(config: ApidocConfig) -> ApiDoc {
        let (ids, mut endpoints) = Self::collect_fragments();
        // 打表分两拨，让 ref 与 table 同时生效、且每个端点恰好打一次：
        // 1. 无 ref 的端点先打表 —— ref 目标由此带上表字段，供第 2 步复制继承；
        // 2. 解 ref —— 引用方复制到的是目标"打完表"的 returned；
        // 3. 有 ref 的端点再打自己的表 —— 追加在复制结果之后，不被覆盖。
        // 已知边界：目标自身同时带 ref 和 table 时，其自带表字段不再二次传播给引用方。
        for ep in endpoints.iter_mut().filter(|e| e.r#ref.is_none()) {
            append_table_fields(ep, &config.tables);
        }
        Self::resolve_refs(&ids, &mut endpoints);
        for ep in endpoints.iter_mut().filter(|e| e.r#ref.is_some()) {
            append_table_fields(ep, &config.tables);
        }
        let apps = build_apps(&config.apps, &endpoints);
        ApiDoc { config, endpoints, apps }
    }

    /// 收集所有片段并按 id 合并为端点（不含 ref / table 解析，两个入口共用）。
    fn collect_fragments() -> (Vec<&'static str>, Vec<DocEndpoint>) {
        // 排序键 (crate, seq)：linkme 的迭代顺序由链接器决定（不是源序），而 seq 是
        // 每个 crate 各自从 0 开始的进程内计数器（见 apidoc-macros 的 SEQ），所以
        // 只按 seq 排会让跨 crate 顺序随链接布局漂移（同一份源码两次构建产出不同
        // 的 api.json/markdown 顺序）。crate 名只用于让跨 crate 顺序可复现，
        // crate 内仍是源码声明序。
        let mut entries: Vec<&DocFragmentEntry> = DOC_FRAGMENTS.iter().collect();
        entries.sort_by_key(|e| {
            let base = id_base(e.id);
            (base.split("::").next().unwrap_or(base), e.seq)
        });
        let mut ids: Vec<&'static str> = Vec::new();
        let mut endpoints: Vec<DocEndpoint> = Vec::new();
        for entry in entries {
            // ponytail: O(n²) linear id lookup; fine for doc-sized inputs, swap
            // to a HashMap<&str, usize> if thousands of endpoints ever appear.
            let idx = match ids.iter().position(|id| *id == entry.id) {
                Some(i) => i,
                None => {
                    ids.push(entry.id);
                    endpoints.push(DocEndpoint::default());
                    endpoints.len() - 1
                }
            };
            let ep = &mut endpoints[idx];
            match &entry.frag {
                DocFragment::Title(t) => ep.title = t.to_string(),
                DocFragment::Desc(d) => ep.desc = d.to_string(),
                DocFragment::Method(m) => ep.method = m.to_string(),
                DocFragment::Url(u) => ep.url = u.to_string(),
                DocFragment::Param(p) => ep.params.push(p.clone()),
                DocFragment::Query(q) => ep.querys.push(q.clone()),
                DocFragment::Returned(r) => ep.returned.push(r.clone()),
                // M3: single-value fields overwrite (later mount wins), lists
                // append, response_status dedups, not_debug ORs, ref overwrites.
                DocFragment::Tag(t) => ep.tags.push(t.to_string()),
                DocFragment::Group(g) => ep.group = g.to_string(),
                DocFragment::Author(a) => ep.author = a.to_string(),
                DocFragment::Header(h) => ep.headers.push(h.clone()),
                DocFragment::RouteParam(p) => ep.route_params.push(p.clone()),
                DocFragment::ResponseStatus(s) => {
                    if !ep.response_status.iter().any(|x| x == s) {
                        ep.response_status.push(s.to_string());
                    }
                }
                DocFragment::Success(e) => ep.success.push(e.clone()),
                DocFragment::Error(e) => ep.error.push(e.clone()),
                DocFragment::NotDebug => ep.not_debug = true,
                DocFragment::Md(m) => ep.md = m.to_string(),
                DocFragment::Sort(n) => ep.sort = *n,
                DocFragment::Ref(r) => ep.r#ref = Some(r.to_string()),
                DocFragment::Table(t) => ep.table = Some(t.to_string()),
                DocFragment::App(a) => ep.app_key = a.to_string(),
                DocFragment::Protocol(p) => {
                    if protocol::spec(p.kind).is_none() {
                        eprintln!(
                            "apidoc: protocol `{}` is not in the runtime registry (macro/registry drift)",
                            p.kind
                        );
                    }
                    ep.protocol = p.kind.to_string();
                    for (k, v) in p.opts {
                        ep.protocol_opts.insert((*k).to_string(), (*v).to_string());
                    }
                }
                DocFragment::Event(e) => ep.events.push(e.clone()),
            }
        }
        (ids, endpoints)
    }

    /// 解 ref 链：把目标的 `returned` 复制进引用端点。须在所有端点建好之后调用
    /// （目标可任意位置声明），链式 A→B→C 递归解析，环由 visited 截断并警告。
    /// 只应调用一次：复制是整体覆盖，跑第二遍会抹掉其间追加的 table 字段。
    fn resolve_refs(ids: &[&'static str], endpoints: &mut [DocEndpoint]) {
        for i in 0..endpoints.len() {
            if endpoints[i].r#ref.is_some() {
                resolve_ref(i, &mut Vec::new(), ids, endpoints);
            }
        }
    }
}

/// 按配置树构建 AppDoc 树；未配置的注解 key 警告并留在根层（默认应用）。
fn build_apps(config_apps: &[AppConfig], endpoints: &[DocEndpoint]) -> Vec<AppDoc> {
    for ep in endpoints.iter().filter(|e| !e.app_key.is_empty()) {
        if find_app(config_apps, &ep.app_key).is_none() {
            eprintln!("apidoc: app `{}` not configured in ApidocConfig.apps, endpoints fall back to the default app", ep.app_key);
        }
    }
    config_apps.iter().map(|c| build_app_doc(c, endpoints)).collect()
}

fn build_app_doc(cfg: &AppConfig, endpoints: &[DocEndpoint]) -> AppDoc {
    let eps: Vec<DocEndpoint> = endpoints.iter().filter(|e| e.app_key == cfg.key).cloned().collect();
    AppDoc {
        key: cfg.key.clone(),
        title: cfg.title.clone(),
        items: cfg.items.iter().map(|c| build_app_doc(c, endpoints)).collect(),
        endpoints: eps,
    }
}

/// v2：把 `#[apidoc::table("key")]` 引用的数据表字段平铺追加进 `ep.returned`。
/// 与 `ref` 语义一致（平铺复制），区别是数据源为 `ApidocConfig.tables` 而非其他端点。
/// 未配置的 key 只警告不报错（与 ref 未命中同策略）。
fn append_table_fields(ep: &mut DocEndpoint, tables: &[TableDef]) {
    let Some(key) = ep.table.as_deref() else { return };
    let Some(def) = tables.iter().find(|t| t.key == key) else {
        eprintln!("apidoc: table `{key}` not found in config.tables for `{}`", ep.url);
        return;
    };
    for f in &def.fields {
        ep.returned.push(DocParam {
            name: intern(&f.name),
            ty: intern(&f.ty),
            required: f.required,
            default: f.default.as_deref().map(intern),
            desc: f.desc.as_deref().map(intern),
            mock: f.mock.as_deref().map(intern),
            field: None,
            // 表字段是标量列，不支持 children。
            children: &[],
        });
    }
}

/// 把配置里的运行时字符串提升为 `&'static str`（宏模型只收静态串）。
/// 进程内驻留：同一个串只泄漏一次，反复调用 `collect_doc` 不再线性涨内存。
/// ponytail: 驻留不回收（有界于"唯一字段串"的数量）；根治要把 DocParam 换成
/// `Cow<'static, str>`，那是公开数据模型的改动，等真有需要再做。
fn intern(s: &str) -> &'static str {
    static POOL: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(HashSet::new()));
    let mut g = pool.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = g.get(s) {
        return existing;
    }
    // 不变量：插入的就是即将 return 的同一个引用
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    g.insert(leaked);
    leaked
}

/// Copies the ref target's `returned` into `endpoints[idx].returned`.
/// Returns false (no copy) when the target is missing or a cycle is hit.
fn resolve_ref(
    idx: usize,
    visited: &mut Vec<usize>,
    ids: &[&'static str],
    endpoints: &mut [DocEndpoint],
) -> bool {
    if visited.contains(&idx) {
        eprintln!("apidoc: ref cycle at `{}`, skipping", ids[idx]);
        return false;
    }
    visited.push(idx);
    // 取出即拷贝：下面要可变借用 endpoints（含写 returned），不能继续持有 ref 的借用
    let Some(target) = endpoints[idx].r#ref.clone() else {
        return true;
    };
    let Some(j) = find_ref_target(&target, ids) else {
        eprintln!("apidoc: ref target `{target}` not found for `{}`", ids[idx]);
        return false;
    };
    if !resolve_ref(j, visited, ids, endpoints) {
        return false;
    }
    if !endpoints[idx].returned.is_empty() {
        // 复制是整体覆盖（有意如此），但引用方自己的 returned 会无声消失——没有
        // 这个警告就只能等字段在 UI 里不见了才发现。
        eprintln!(
            "apidoc: ref `{target}` overwrites {} own returned field(s) of `{}`",
            endpoints[idx].returned.len(),
            ids[idx]
        );
    }
    endpoints[idx].returned = endpoints[j].returned.clone();
    true
}

/// id 去掉 `#行号` 尾缀（宏生成，用于区分同模块重名项）后的基名。
fn id_base(id: &str) -> &str {
    id.split('#').next().unwrap_or(id)
}

/// Locates a ref target: exact id match first, then `::fn_name` suffix match
/// (referencing an endpoint by its function name). Both compare against the
/// line-stripped base, so `m3_ref::foo` and `foo` keep matching after the macro
/// started appending `#<item line>` to ids.
fn find_ref_target(target: &str, ids: &[&'static str]) -> Option<usize> {
    if let Some(j) = ids.iter().position(|id| id_base(id) == target) {
        return Some(j);
    }
    let suffix = format!("::{target}");
    let mut hits = ids.iter().enumerate().filter(|(_, id)| id_base(id).ends_with(&suffix));
    let (j, _) = hits.next()?;
    if hits.next().is_some() {
        eprintln!("apidoc: ref `{target}` matches multiple endpoints, using `{}`", ids[j]);
    }
    Some(j)
}
