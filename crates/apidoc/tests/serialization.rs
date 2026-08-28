//! JSON 序列化：字段名（serde rename/skip_serializing_if）与省略规则。

use apidoc::{ApiDoc, ApidocConfig, DocRegistry};
use serde_json::{json, Value};

#[allow(dead_code)]
#[apidoc::title("序列化")]
#[apidoc::desc("desc")]
#[apidoc::url("/api/ser")]
#[apidoc::method("PUT")]
#[apidoc::param(name = "id", ty = "int", required, desc = "ID", mock = "1")]
#[apidoc::param(name = "extra", ty = "string")]
#[apidoc::returned(name = "obj", ty = "object", children = [{ name = "x", ty = "string", required }])]
fn ser_target() {}

#[test]
fn endpoint_json_field_names_are_exact() {
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "t".into(),
            description: None, auth: None, apps: Vec::new(),
        },
        apps: Vec::new(), endpoints: DocRegistry::collect(),
    };
    let v: Value = serde_json::to_value(&doc).unwrap();

    // config.description 为 None → 省略
    assert_eq!(v["config"], json!({"title": "t"}));

    let ep = v["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["url"] == json!("/api/ser"))
        .expect("endpoint /api/ser");

    // headers 为空 → 省略；其余固定字段恒存在
    assert!(ep.get("headers").is_none());
    assert_eq!(ep["method"], "PUT");
    for key in ["title", "desc", "url", "method", "params", "querys", "returned"] {
        assert!(ep.get(key).is_some(), "missing key {key}");
    }

    let params = ep["params"].as_array().unwrap();
    let id = &params[0];
    assert_eq!(id["name"], "id");
    // ty 被 rename 为 type，且不出现 ty
    assert_eq!(id["type"], "int");
    assert!(id.get("ty").is_none());
    assert_eq!(id["required"], true);
    assert_eq!(id["desc"], "ID");
    assert_eq!(id["mock"], "1");
    // 未提供的 default 省略
    assert!(id.get("default").is_none());

    // extra 无 desc/mock/default/children → 全部省略；required=false 照常输出
    let extra = &params[1];
    assert_eq!(extra["required"], false);
    for key in ["desc", "mock", "default", "children"] {
        assert!(extra.get(key).is_none(), "unexpected key {key}");
    }

    // children 嵌套序列化
    let ret = &ep["returned"][0];
    assert_eq!(ret["type"], "object");
    assert_eq!(ret["children"][0]["name"], "x");
    assert_eq!(ret["children"][0]["required"], true);
}

#[test]
fn config_description_serializes_when_present() {
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "t".into(),
            description: Some("d".into()),
            auth: None,
            apps: Vec::new(),
        },
        apps: Vec::new(), endpoints: Vec::new(),
    };
    let v = serde_json::to_value(doc).unwrap();
    assert_eq!(v["config"], json!({"title": "t", "description": "d"}));
    assert_eq!(v["endpoints"], json!([]));
}
