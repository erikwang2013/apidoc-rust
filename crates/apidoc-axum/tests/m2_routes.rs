//! M2 apidoc-axum 集成测试：路由可达性、api.json 结构、CORS 头、UI 数据流一致性。

use apidoc::{ApiDoc, ApidocConfig, DocRegistry};
use apidoc_axum::{apidoc_routes, cors_layer, CorsConfig};
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

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

fn app() -> Router {
    Router::new()
        .merge(apidoc_routes(ApidocConfig {
            title: "test api".into(),
            description: Some("集成测试".into()),
        }))
        .layer(cors_layer(CorsConfig::default()))
}

async fn get(uri: &str, origin: Option<&str>) -> (StatusCode, HeaderMap, String) {
    let mut req = Request::builder().uri(uri);
    if let Some(o) = origin {
        req = req.header(header::ORIGIN, o);
    }
    let res = app().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn apidoc_route_returns_html_page() {
    let (status, headers, body) = get("/apidoc", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(body.contains("<script>"), "HTML 缺少内嵌脚本");
    assert!(body.contains("<title>API Documentation</title>"), "HTML 缺少标题");
}

#[tokio::test]
async fn api_json_route_returns_valid_doc() {
    let (status, headers, body) = get("/apidoc/api.json", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json",
        "api.json Content-Type 错误"
    );
    let v: Value = serde_json::from_str(&body).expect("api.json 必须是合法 JSON");
    assert_eq!(v["config"]["title"], "test api");
    assert_eq!(v["config"]["description"], "集成测试");
    let eps = v["endpoints"].as_array().expect("endpoints 必须是数组");
    assert!(eps.iter().any(|e| e["method"] == "GET" && e["url"] == "/api/user/info"));
    assert!(eps.iter().any(|e| e["method"] == "POST" && e["url"] == "/api/user"));
    assert!(eps.iter().any(|e| e["method"] == "GET" && e["url"] == "/health"));
    // 嵌套 children 保留
    let info = eps.iter().find(|e| e["url"] == "/api/user/info").unwrap();
    assert_eq!(info["returned"][0]["children"][0]["name"], "id");
}

#[tokio::test]
async fn cors_header_present_on_origin_request() {
    let (status, headers, _) = get("/apidoc/api.json", Some("http://example.com")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("access-control-allow-origin").unwrap(),
        "*",
        "默认 CorsConfig 应放行任意 Origin"
    );
    // 不带 Origin（curl/同源）也应正常返回
    let (status, _, _) = get("/apidoc", None).await;
    assert_eq!(status, StatusCode::OK);
}

#[test]
fn ui_html_has_grouping_markers() {
    // 分组启发式在 ui.html 的 JS 内（Rust 侧无可测函数），验证分组代码存在
    let html = include_str!("../src/ui.html");
    assert!(html.contains("function groupOf"), "缺少 groupOf");
    assert!(html.contains("allShare"), "缺少公共前缀判断");
    assert!(html.contains("location.hash"), "缺少 hash 恢复选中");
    assert!(html.contains("#g' + gi + '/e' + ei"), "缺少分组/端点 hash 标记");
    assert!(html.contains("textContent"), "缺少 textContent 安全注入");
}

#[test]
fn ui_html_fields_all_present_in_api_json() {
    // 交叉一致性：ui.html 的 JS 引用的每个数据路径都能在 api.json 中找到对应键
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "t".into(),
            description: Some("d".into()),
        },
        endpoints: DocRegistry::collect(),
    };
    let v: Value = serde_json::to_value(&doc).unwrap();
    let html = include_str!("../src/ui.html");
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
    ] {
        assert!(html.contains(js_ref), "ui.html 缺少引用: {js_ref}");
        assert!(
            path_exists(&v, json_path),
            "api.json 缺少 {json_path}（ui.html 引用了它）"
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
