//! M6b 多应用/版本核心测试：app 注解挂载、嵌套版本树、未配置 key 回落默认应用、
//! 序列化省略规则与 collect() 兼容性（collect 恒为全集）。

use apidoc::{AppConfig, ApidocConfig, DocRegistry};
use serde_json::Value;

#[allow(dead_code)]
#[apidoc::title("默认应用接口")]
#[apidoc::url("/api/default")]
#[apidoc::method("GET")]
fn default_ep() {}

#[allow(dead_code)]
#[apidoc::app("api")]
#[apidoc::title("应用接口")]
#[apidoc::url("/api/app")]
#[apidoc::method("GET")]
fn app_ep() {}

#[allow(dead_code)]
#[apidoc::app("v2")]
#[apidoc::title("版本接口")]
#[apidoc::url("/api/v2")]
#[apidoc::method("GET")]
fn v2_ep() {}

#[allow(dead_code)]
#[apidoc::app("nope")]
#[apidoc::title("未配置应用")]
#[apidoc::url("/api/nope")]
#[apidoc::method("GET")]
fn nope_ep() {}

fn config() -> ApidocConfig {
    ApidocConfig {
        title: "t".into(),
        description: None,
        auth: None,
        apps: vec![AppConfig {
            key: "api".into(),
            title: "API".into(),
            items: vec![AppConfig { key: "v2".into(), title: "V2".into(), items: Vec::new(), password: None }],
            password: None,
        }],
    }
}

#[test]
fn collect_is_unchanged_and_contains_all_endpoints() {
    let eps = DocRegistry::collect();
    assert_eq!(eps.len(), 4);
    for title in ["默认应用接口", "应用接口", "版本接口", "未配置应用"] {
        assert!(eps.iter().any(|e| e.title == title), "collect() 缺 {title}");
    }
}

#[test]
fn app_annotations_attach_to_configured_tree() {
    let doc = DocRegistry::collect_doc(config());
    // 根 endpoints 恒为全集（默认应用视图）
    assert_eq!(doc.endpoints.len(), 4);
    assert_eq!(doc.apps.len(), 1);
    let api = &doc.apps[0];
    assert_eq!(api.key, "api");
    assert_eq!(api.title, "API");
    assert_eq!(api.endpoints.len(), 1);
    assert_eq!(api.endpoints[0].title, "应用接口");
    // 嵌套版本：v2 注解挂到 items 下
    assert_eq!(api.items.len(), 1);
    assert_eq!(api.items[0].key, "v2");
    assert_eq!(api.items[0].endpoints.len(), 1);
    assert_eq!(api.items[0].endpoints[0].title, "版本接口");
}

#[test]
fn unknown_app_key_falls_back_to_default_app() {
    // "nope" 未配置：警告 + 不进 apps 树（只在根 endpoints，默认应用可见）
    let doc = DocRegistry::collect_doc(config());
    let keys: Vec<&str> = doc.apps.iter().map(|a| a.key.as_str()).collect();
    assert_eq!(keys, ["api"]);
    assert!(doc.endpoints.iter().any(|e| e.title == "未配置应用"));
    // 应用树内没有误挂
    assert_eq!(doc.apps[0].items[0].endpoints.len(), 1);
}

#[test]
fn apps_tree_serializes_with_exact_keys() {
    let doc = DocRegistry::collect_doc(config());
    let v: Value = serde_json::to_value(&doc).unwrap();
    assert_eq!(v["apps"][0]["key"], "api");
    assert_eq!(v["apps"][0]["title"], "API");
    assert_eq!(v["apps"][0]["endpoints"][0]["title"], "应用接口");
    assert_eq!(v["apps"][0]["items"][0]["key"], "v2");
    assert_eq!(v["apps"][0]["items"][0]["endpoints"][0]["title"], "版本接口");
    // 端点字段与根级完全一致（复用同一模型）
    assert_eq!(v["apps"][0]["endpoints"][0]["method"], "GET");
}
