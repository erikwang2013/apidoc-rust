#![cfg(feature = "axum")]
//! M4 apidoc::axum 集成测试：/apidoc/mock 命中/未命中、not_debug 端点不过滤、
//! api.json 回归哨兵（api.json 零变化证明）。

use apidoc::ApidocConfig;
use apidoc::axum::{apidoc_routes, cors_layer, CorsConfig};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

// fixture：三类参数齐备，mock 注解覆盖显式值 / fake: 规则 / ty 默认
#[allow(dead_code)]
#[apidoc::title("用户详情")]
#[apidoc::url("/api/user/{id}")]
#[apidoc::method("GET")]
#[apidoc::route_param(name = "id", ty = "int", mock = "fake:int")]
#[apidoc::param(name = "name", ty = "string", mock = "alice")]
#[apidoc::param(name = "role", ty = "string")]
#[apidoc::query(name = "page", ty = "int")]
fn get_user() {}

// fixture：not_debug 端点 —— 服务端不过滤契约，mock 路由照常服务
#[allow(dead_code)]
#[apidoc::title("内部接口")]
#[apidoc::url("/internal/secret")]
#[apidoc::method("POST")]
#[apidoc::not_debug]
fn internal() {}

fn app() -> Router {
    Router::new()
        .merge(apidoc_routes(ApidocConfig {
            title: "mock test".into(),
            description: None, auth: None, apps: Vec::new(),
        }))
        .layer(cors_layer(CorsConfig::default()))
}

async fn get(uri: &str) -> (StatusCode, String) {
    let res = app()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn mock_route_hit_returns_three_keys_and_mock_values() {
    let (status, body) = get("/apidoc/mock?url=%2Fapi%2Fuser%2F%7Bid%7D&method=GET").await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("mock 响应必须是合法 JSON");
    assert!(v.get("route_params").is_some(), "缺 route_params 键");
    assert!(v.get("params").is_some(), "缺 params 键");
    assert!(v.get("querys").is_some(), "缺 querys 键");
    // 显式 mock 原样、fake: 规则产出非空、ty 默认落 string
    assert_eq!(v["params"]["name"], "alice");
    let fake_id = v["route_params"]["id"].as_str().unwrap();
    assert!(!fake_id.is_empty() && fake_id != "fake:int");
    assert_eq!(v["params"]["role"], "string");
}

#[tokio::test]
async fn mock_route_miss_returns_404() {
    let (status, body) = get("/apidoc/mock?url=%2Fnope&method=GET").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v: Value = serde_json::from_str(&body).expect("404 响应必须是合法 JSON");
    assert_eq!(v["error"], "endpoint not found");
}

// 匹配必须 url+method 双键精确：同 url 只存在 POST，请求 GET 应 404，
// 小写 method 也应 404（精确匹配，不归一化）
#[tokio::test]
async fn mock_route_requires_exact_method_match() {
    let (status, _) = get("/apidoc/mock?url=%2Finternal%2Fsecret&method=GET").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "同 url 不同 method 不应命中");
    let (status, _) = get("/apidoc/mock?url=%2Finternal%2Fsecret&method=post").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "method 应区分大小写");
    // 反方向：/api/user/{id} 只有 GET，请求 POST 应 404
    let (status, _) = get("/apidoc/mock?url=%2Fapi%2Fuser%2F%7Bid%7D&method=POST").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "同 url 不同 method 不应命中");
}

#[tokio::test]
async fn mock_route_without_params_returns_404() {
    let (status, body) = get("/apidoc/mock").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v: Value = serde_json::from_str(&body).expect("404 响应必须是合法 JSON");
    assert_eq!(v["error"], "endpoint not found");
}

#[tokio::test]
async fn mock_route_serves_not_debug_endpoints() {
    let (status, body) = get("/apidoc/mock?url=%2Finternal%2Fsecret&method=POST").await;
    assert_eq!(status, StatusCode::OK, "服务端不过滤 not_debug 端点");
    let v: Value = serde_json::from_str(&body).expect("mock 响应必须是合法 JSON");
    assert!(v.get("params").is_some());
}

#[tokio::test]
async fn api_json_regression_sentinel() {
    let (status, body) = get("/apidoc/api.json").await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("api.json 必须是合法 JSON");
    assert!(v["endpoints"].is_array(), "endpoints 必须是数组");
    // M4 零变化哨兵：两个 fixture 端点原样序列化，mock 引擎不得改动形状
    assert_eq!(v["endpoints"].as_array().unwrap().len(), 2, "endpoints 数量不应变");
    assert_eq!(v["endpoints"][0]["method"].as_str().unwrap(), "GET");
    assert_eq!(v["endpoints"][0]["url"].as_str().unwrap(), "/api/user/{id}");
    assert_eq!(v["endpoints"][1]["method"].as_str().unwrap(), "POST");
    assert_eq!(v["endpoints"][1]["not_debug"], true, "not_debug 标记应原样出现在 api.json");
    // mock 引擎的产物不得渗入 api.json
    assert!(v.get("route_params").is_none(), "api.json 不应出现 mock 专用键");
    assert!(v.get("params").is_none(), "api.json 不应出现 mock 专用键");
    assert!(v.get("querys").is_none(), "api.json 不应出现 mock 专用键");
}
