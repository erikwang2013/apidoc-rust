//! M4 适配器集成测试的共享断言体（axum 侧 / actix 侧逐条 1:1 共用）：
//! /apidoc/mock 命中/未命中、not_debug 端点不过滤、api.json 回归哨兵（api.json 零变化证明）。
//! driver（tests/axum_mock_route.rs / tests/actix_mock_route.rs）只提供取响应的 get。
#![allow(dead_code)]

use std::future::Future;

use apidoc::ApidocConfig;
use serde_json::Value;

use crate::common::Resp;

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

/// 两适配器共用的基线配置。
fn config() -> ApidocConfig {
    ApidocConfig {
        title: "mock test".into(),
        description: None, auth: None, apps: Vec::new(),
        ..Default::default()
    }
}

pub async fn assert_mock_route_hit_returns_three_keys_and_mock_values<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/mock?url=%2Fapi%2Fuser%2F%7Bid%7D&method=GET".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] mock 命中期望 200，实际 {}", r.status);
    let v: Value = serde_json::from_str(&r.body).expect("mock 响应必须是合法 JSON");
    assert!(v.get("route_params").is_some(), "[{adapter}] 缺 route_params 键");
    assert!(v.get("params").is_some(), "[{adapter}] 缺 params 键");
    assert!(v.get("querys").is_some(), "[{adapter}] 缺 querys 键");
    // 显式 mock 原样、fake: 规则产出非空、ty 默认落 string
    assert_eq!(v["params"]["name"], "alice", "[{adapter}] 显式 mock 应原样");
    let fake_id = v["route_params"]["id"].as_str().unwrap();
    assert!(
        !fake_id.is_empty() && fake_id != "fake:int",
        "[{adapter}] fake: 规则应产出非空值，实际 {fake_id}"
    );
    assert_eq!(v["params"]["role"], "string", "[{adapter}] ty 默认应落 string");
}

pub async fn assert_mock_route_miss_returns_404<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/mock?url=%2Fnope&method=GET".to_string()).await;
    assert_eq!(r.status, 404, "[{adapter}] 未命中期望 404，实际 {}", r.status);
    let v: Value = serde_json::from_str(&r.body).expect("404 响应必须是合法 JSON");
    assert_eq!(v["error"], "endpoint not found", "[{adapter}] 404 错误体错误");
}

// 匹配必须 url+method 双键精确：同 url 只存在 POST，请求 GET 应 404，
// 小写 method 也应 404（精确匹配，不归一化）
pub async fn assert_mock_route_requires_exact_method_match<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/mock?url=%2Finternal%2Fsecret&method=GET".to_string()).await;
    assert_eq!(r.status, 404, "[{adapter}] 同 url 不同 method 不应命中，实际 {}", r.status);
    let r = get(config(), "/apidoc/mock?url=%2Finternal%2Fsecret&method=post".to_string()).await;
    assert_eq!(r.status, 404, "[{adapter}] method 应区分大小写，实际 {}", r.status);
    // 反方向：/api/user/{id} 只有 GET，请求 POST 应 404
    let r = get(config(), "/apidoc/mock?url=%2Fapi%2Fuser%2F%7Bid%7D&method=POST".to_string()).await;
    assert_eq!(r.status, 404, "[{adapter}] 同 url 不同 method 不应命中，实际 {}", r.status);
}

pub async fn assert_mock_route_without_params_returns_404<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/mock".to_string()).await;
    assert_eq!(r.status, 404, "[{adapter}] 缺 url/method 期望 404，实际 {}", r.status);
    let v: Value = serde_json::from_str(&r.body).expect("404 响应必须是合法 JSON");
    assert_eq!(v["error"], "endpoint not found", "[{adapter}] 404 错误体错误");
}

pub async fn assert_mock_route_serves_not_debug_endpoints<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/mock?url=%2Finternal%2Fsecret&method=POST".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] 服务端不过滤 not_debug 端点，实际 {}", r.status);
    let v: Value = serde_json::from_str(&r.body).expect("mock 响应必须是合法 JSON");
    assert!(v.get("params").is_some(), "[{adapter}] not_debug 端点响应缺 params 键");
}

pub async fn assert_api_json_regression_sentinel<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/api.json".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] api.json 期望 200，实际 {}", r.status);
    let v: Value = serde_json::from_str(&r.body).expect("api.json 必须是合法 JSON");
    assert!(v["endpoints"].is_array(), "[{adapter}] endpoints 必须是数组");
    // M4 零变化哨兵：两个 fixture 端点原样序列化，mock 引擎不得改动形状
    assert_eq!(
        v["endpoints"].as_array().unwrap().len(), 2,
        "[{adapter}] endpoints 数量不应变，实际 {}",
        v["endpoints"]
    );
    assert_eq!(v["endpoints"][0]["method"].as_str().unwrap(), "GET", "[{adapter}] endpoints[0] method 错误");
    assert_eq!(
        v["endpoints"][0]["url"].as_str().unwrap(), "/api/user/{id}",
        "[{adapter}] endpoints[0] url 错误"
    );
    assert_eq!(v["endpoints"][1]["method"].as_str().unwrap(), "POST", "[{adapter}] endpoints[1] method 错误");
    assert_eq!(
        v["endpoints"][1]["not_debug"], true,
        "[{adapter}] not_debug 标记应原样出现在 api.json"
    );
    // mock 引擎的产物不得渗入 api.json
    assert!(v.get("route_params").is_none(), "[{adapter}] api.json 不应出现 mock 专用键");
    assert!(v.get("params").is_none(), "[{adapter}] api.json 不应出现 mock 专用键");
    assert!(v.get("querys").is_none(), "[{adapter}] api.json 不应出现 mock 专用键");
}
