//! M2 适配器集成测试的共享断言体（axum 侧 / actix 侧逐条 1:1 共用）：路由可达性、
//! api.json 结构、CORS 头、UI 数据流一致性。
//! driver（tests/axum_m2_routes.rs / tests/actix_m2_routes.rs）只提供取响应的 get。
#![allow(dead_code)]

use std::future::Future;

use apidoc::{ApidocConfig, DocRegistry};
use serde_json::Value;

use crate::common::Resp;

// 与 examples/demo.rs 同构：公共前缀 /api + 混合前缀 /health，覆盖分组启发式输入形态
#[allow(dead_code)]
#[apidoc::title("获取用户信息")]
#[apidoc::desc("根据用户 ID 查询用户详情")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, mock = "1")]
#[apidoc::query(name = "lang", ty = "string", default = "zh-CN")]
#[apidoc::returned(name = "data", ty = "object", children = [{ name = "id", ty = "int", required }])]
fn get_user_info() {}

#[allow(dead_code)]
#[apidoc::title("创建用户")]
#[apidoc::url("/api/user")]
#[apidoc::method("POST")]
fn create_user() {}

#[allow(dead_code)]
#[apidoc::title("健康检查")]
#[apidoc::url("/health")]
#[apidoc::method("GET")]
fn health() {}

// M6b fixture：挂到应用 key "api" 下，支撑 apps 树在 api.json 中可见
#[allow(dead_code)]
#[apidoc::app("api")]
#[apidoc::title("应用接口")]
#[apidoc::url("/api/app/ep")]
#[apidoc::method("GET")]
fn app_ep() {}

// M3 fixture：ui.html 新增的 M3 字段引用（ep.group/sort/author/ref/...）
// 需要真实数据支撑，白名单测试的 path_exists 才会命中。
#[allow(dead_code)]
#[apidoc::title("M3 分组接口")]
#[apidoc::url("/api/m3/grouped")]
#[apidoc::method("GET")]
#[apidoc::group("M3组")]
#[apidoc::sort(99)]
#[apidoc::author("tester")]
#[apidoc::tag("m3", "ui")]
#[apidoc::response_status("200")]
#[apidoc::success(code = "200", example = "{}")]
#[apidoc::error(code = "500", example = "{}")]
#[apidoc::md("补充说明")]
#[apidoc::r#ref("get_user_info")]
fn m3_grouped() {}

/// M2 基线配置（两适配器共用同一份固定配置）。
fn config() -> ApidocConfig {
    ApidocConfig {
        title: "test api".into(),
        description: Some("集成测试".into()),
        auth: None,
        tables: Vec::new(),
        cache: None,
        codegen: Vec::new(),
        apps: Vec::new(),
    }
}

pub async fn assert_apidoc_route_returns_html_page<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String, Option<String>) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc".to_string(), None).await;
    assert_eq!(r.status, 200, "[{adapter}] /apidoc 期望 200，实际 {}", r.status);
    assert!(
        r.header("content-type").expect("缺少 Content-Type").starts_with("text/html"),
        "[{adapter}] /apidoc Content-Type 应为 text/html"
    );
    assert!(r.body.contains("<script>"), "[{adapter}] HTML 缺少内嵌脚本");
    assert!(r.body.contains("<title>API Documentation</title>"), "[{adapter}] HTML 缺少标题");
}

pub async fn assert_pet_route_serves_svg_and_ui_references_it<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String, Option<String>) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/pet.svg".to_string(), None).await;
    assert_eq!(r.status, 200, "[{adapter}] /apidoc/pet.svg 期望 200，实际 {}", r.status);
    assert!(
        r.header("content-type").expect("缺少 Content-Type").starts_with("image/svg+xml"),
        "[{adapter}] 宠物 Content-Type 错误"
    );
    assert!(r.body.contains("<svg"), "[{adapter}] 宠物必须是 SVG 源码");
    // UI 页确实引用了它（favicon + 页头 logo），否则图挂了也没人发现
    let ui = get(config(), "/apidoc".to_string(), None).await.body;
    assert!(ui.contains(r#"href="/apidoc/pet.svg""#), "[{adapter}] UI 缺少 favicon 引用");
    assert!(ui.contains(r#"src="/apidoc/pet.svg""#), "[{adapter}] UI 缺少页头 logo 引用");
}

pub async fn assert_api_json_route_returns_valid_doc<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String, Option<String>) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/api.json".to_string(), None).await;
    assert_eq!(r.status, 200, "[{adapter}] api.json 期望 200，实际 {}", r.status);
    assert_eq!(
        r.header("content-type").unwrap(),
        "application/json",
        "[{adapter}] api.json Content-Type 错误"
    );
    let v: Value = serde_json::from_str(&r.body).expect("api.json 必须是合法 JSON");
    assert_eq!(v["config"]["title"], "test api", "[{adapter}] config.title 错误");
    assert_eq!(v["config"]["description"], "集成测试", "[{adapter}] config.description 错误");
    let eps = v["endpoints"].as_array().expect("endpoints 必须是数组");
    assert!(
        eps.iter().any(|e| e["method"] == "GET" && e["url"] == "/api/user/info"),
        "[{adapter}] 缺 GET /api/user/info"
    );
    assert!(
        eps.iter().any(|e| e["method"] == "POST" && e["url"] == "/api/user"),
        "[{adapter}] 缺 POST /api/user"
    );
    assert!(
        eps.iter().any(|e| e["method"] == "GET" && e["url"] == "/health"),
        "[{adapter}] 缺 GET /health"
    );
    // 嵌套 children 保留
    let info = eps.iter().find(|e| e["url"] == "/api/user/info").unwrap();
    assert_eq!(
        info["returned"][0]["children"][0]["name"], "id",
        "[{adapter}] 嵌套 children 丢失"
    );
}

// M5：/apidoc/export?format=md|ts|swagger 分发与 Content-Type
pub async fn assert_export_route_dispatches_by_format_and_rejects_unknown<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String, Option<String>) -> Fut,
    Fut: Future<Output = Resp>,
{
    for (format, ct, marker) in [
        ("md", "text/markdown", "# test api"),
        ("ts", "application/typescript", "export const"),
        ("swagger", "application/json", "\"openapi\":\"3.0.0\""),
    ] {
        let r = get(config(), format!("/apidoc/export?format={format}"), None).await;
        assert_eq!(r.status, 200, "[{adapter}] format={format} 应返回 200，实际 {}", r.status);
        assert_eq!(
            r.header("content-type").unwrap(),
            ct,
            "[{adapter}] format={format} Content-Type 错误"
        );
        assert!(
            r.body.contains(marker),
            "[{adapter}] format={format} 输出缺标记 {marker}"
        );
    }
    // 未知 / 缺失 format → 400
    let r = get(config(), "/apidoc/export?format=pdf".to_string(), None).await;
    assert_eq!(r.status, 400, "[{adapter}] 未知 format 应 400，实际 {}", r.status);
    let r = get(config(), "/apidoc/export".to_string(), None).await;
    assert_eq!(r.status, 400, "[{adapter}] 缺 format 应 400，实际 {}", r.status);
}

pub async fn assert_cors_header_present_on_origin_request<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String, Option<String>) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/api.json".to_string(), Some("http://example.com".to_string())).await;
    assert_eq!(r.status, 200, "[{adapter}] 带 Origin 的 api.json 期望 200，实际 {}", r.status);
    assert_eq!(
        r.header("access-control-allow-origin").unwrap(),
        "*",
        "[{adapter}] 默认 CorsConfig 应放行任意 Origin"
    );
    assert!(
        r.header("access-control-allow-credentials").is_none(),
        "[{adapter}] 永不携带 allow-credentials"
    );
    // 不带 Origin（curl/同源）也应正常返回
    let r = get(config(), "/apidoc".to_string(), None).await;
    assert_eq!(r.status, 200, "[{adapter}] 不带 Origin 的 /apidoc 期望 200，实际 {}", r.status);
}

pub fn assert_ui_html_has_grouping_markers(adapter: &str) {
    // 分组启发式在 ui.js 的 JS 内（Rust 侧无可测函数），验证分组代码存在
    let js = concat!(include_str!("../../src/ui.js"), include_str!("../../src/ui.debug.js"));
    assert!(js.contains("function groupOf"), "[{adapter}] 缺少 groupOf");
    assert!(js.contains("allShare"), "[{adapter}] 缺少公共前缀判断");
    assert!(js.contains("location.hash"), "[{adapter}] 缺少 hash 恢复选中");
    assert!(js.contains("#g' + gi + '/e' + ei"), "[{adapter}] 缺少分组/端点 hash 标记");
    assert!(js.contains("textContent"), "[{adapter}] 缺少 textContent 安全注入");
    // M6a：密码遮罩与 md5 提交；M6b：应用/版本选择器
    assert!(js.contains("function md5"), "[{adapter}] 缺少前端 md5");
    assert!(js.contains("apidoc_token"), "[{adapter}] 缺少 token 存取");
    assert!(js.contains("apps-sel"), "[{adapter}] 缺少应用选择器");
    assert!(!js.contains("innerHTML"), "[{adapter}] 禁用 innerHTML");
    let html = include_str!("../../src/ui.html");
    assert!(html.contains("<title>API Documentation</title>"), "[{adapter}] HTML 缺少标题");
}

pub fn assert_ui_html_fields_all_present_in_api_json(adapter: &str) {
    // 交叉一致性：ui.html 的 JS 引用的每个数据路径都能在 api.json 中找到对应键
    let doc = DocRegistry::collect_doc(ApidocConfig {
        title: "t".into(),
        description: Some("d".into()),
        // M6a/M6b：auth.enable 与 apps 树进 api.json（password/secret_key 永不）
        auth: Some(apidoc::auth::AuthConfig {
            enable: true,
            password: "secret".into(),
            secret_key: "k".into(),
            expire: 0,
        }),
        tables: Vec::new(),
        cache: None,
        codegen: Vec::new(),
        apps: vec![apidoc::AppConfig {
            key: "api".into(),
            title: "API".into(),
            items: vec![apidoc::AppConfig { key: "v2".into(), title: "V2".into(), items: vec![], password: None }],
            password: None,
        }],
    });
    let v: Value = serde_json::to_value(&doc).unwrap();
    // 红线：password / secret_key 永不进任何输出
    assert!(!v.to_string().contains("secret"), "[{adapter}] 密码/密钥泄漏进 api.json");
    let html = include_str!("../../src/ui.js");
    for (js_ref, json_path) in [
        ("doc.config.title", "config.title"),
        ("doc.config.description", "config.description"),
        ("doc.endpoints", "endpoints"),
        ("ep.method", "endpoints[].method"),
        ("ep.url", "endpoints[].url"),
        ("ep.title", "endpoints[].title"),
        ("ep.params", "endpoints[].params"),
        ("ep.querys", "endpoints[].querys"),
        ("ep.returned", "endpoints[].returned"),
        ("p.children", "endpoints[].returned[].children"),
        ("p.mock", "endpoints[].params[].mock"),
        ("p.default", "endpoints[].querys[].default"),
        // M3：ui.html 新增引用，数据由 m3_grouped fixture 提供
        ("ep.group", "endpoints[].group"),
        ("b.sort", "endpoints[].sort"),
        ("ep.author", "endpoints[].author"),
        ("ep.ref", "endpoints[].ref"),
        ("ep.response_status", "endpoints[].response_status"),
        ("ep.tags", "endpoints[].tags"),
        ("ep.success", "endpoints[].success"),
        ("ep.error", "endpoints[].error"),
        ("ep.md", "endpoints[].md"),
        ("ex.code", "endpoints[].success[].code"),
        ("ex.example", "endpoints[].success[].example"),
        // M6：多应用/版本树（JS 引用即白名单；auth.enable 在核心测试断言）
        ("doc.apps", "apps"),
        ("app.title", "apps[].title"),
        ("node.items", "apps[].items"),
        ("node.key", "apps[].key"),
        ("node.endpoints", "apps[].endpoints"),
    ] {
        assert!(html.contains(js_ref), "[{adapter}] ui.html 缺少引用: {js_ref}");
        assert!(
            path_exists(&v, json_path),
            "[{adapter}] api.json 缺少 {json_path}（ui.html 引用了它）"
        );
    }
}

/// 键路径存在性：`a.b[]` 表示 a.b 是数组且至少一个元素满足后续路径。
fn path_exists(v: &Value, path: &str) -> bool {
    let segs: Vec<&str> = path.split('.').collect();
    fn walk(cur: &Value, segs: &[&str]) -> bool {
        if segs.is_empty() {
            return true;
        }
        let raw = segs[0];
        match raw.strip_suffix("[]") {
            Some(name) => match cur.get(name) {
                Some(Value::Array(arr)) => {
                    !arr.is_empty() && arr.iter().any(|e| walk(e, &segs[1..]))
                }
                _ => false,
            },
            None => match cur.get(raw) {
                Some(next) => walk(next, &segs[1..]),
                None => false,
            },
        }
    }
    walk(v, &segs)
}
