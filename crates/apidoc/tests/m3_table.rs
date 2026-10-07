//! v2 `#[apidoc::table("key")]` 注解语义：配置表字段平铺并入 returned（顺序保持）、
//! JSON 形状（type 键名 / 可选字段省略）、未使用注解时 api.json 无 table 键
//! （v1 字节级兼容红线）、未配置 key 的安全降级、与 ref 的共存与继承。

use apidoc::{ApidocConfig, DocEndpoint, DocRegistry, TableDef, TableField};
use serde_json::{json, Value};

#[allow(dead_code)]
#[apidoc::title("表引用")]
#[apidoc::url("/api/table/user")]
#[apidoc::table("user")]
fn table_user() {}

#[allow(dead_code)]
#[apidoc::title("无表引用")]
#[apidoc::url("/api/table/plain")]
#[apidoc::returned(name = "own", ty = "string", required)]
fn table_plain() {}

#[allow(dead_code)]
#[apidoc::title("未配置表")]
#[apidoc::url("/api/table/missing")]
#[apidoc::table("missing_table")]
fn table_missing() {}

#[allow(dead_code)]
#[apidoc::title("ref目标")]
#[apidoc::returned(name = "base", ty = "string", required)]
fn table_ref_target() {}

#[allow(dead_code)]
#[apidoc::title("表+ref")]
#[apidoc::url("/api/table/both")]
#[apidoc::r#ref("table_ref_target")]
#[apidoc::table("user")]
fn table_both() {}

#[allow(dead_code)]
#[apidoc::title("表目标")]
#[apidoc::url("/api/table/target")]
#[apidoc::table("user")]
fn table_target() {}

#[allow(dead_code)]
#[apidoc::title("ref表目标")]
#[apidoc::url("/api/table/ref_table")]
#[apidoc::r#ref("table_target")]
fn table_ref_table() {}

/// 配置用数据表：2 个必填 + 1 个带 default 的可选字段，覆盖 required/default/desc/mock。
fn user_table() -> TableDef {
    TableDef {
        key: "user".into(),
        title: "用户表".into(),
        fields: vec![
            TableField {
                name: "id".into(),
                ty: "int".into(),
                required: true,
                default: None,
                desc: Some("用户ID".into()),
                mock: Some("1".into()),
            },
            TableField {
                name: "name".into(),
                ty: "string".into(),
                required: true,
                default: None,
                desc: Some("用户名".into()),
                mock: None,
            },
            TableField {
                name: "status".into(),
                ty: "int".into(),
                required: false,
                default: Some("1".into()),
                desc: None,
                mock: None,
            },
        ],
    }
}

fn cfg(tables: Vec<TableDef>) -> ApidocConfig {
    ApidocConfig { title: "t".into(), tables, ..Default::default() }
}

fn find<'a>(eps: &'a [DocEndpoint], url: &str) -> &'a DocEndpoint {
    eps.iter().find(|e| e.url == url).expect("endpoint not found")
}

fn find_json<'a>(eps: &'a Value, url: &str) -> &'a Value {
    eps.as_array()
        .unwrap()
        .iter()
        .find(|e| e["url"] == json!(url))
        .expect("endpoint not found in json")
}

#[test]
fn configured_table_fields_merge_into_returned_in_order() {
    let doc = DocRegistry::collect_doc(cfg(vec![user_table()]));
    let ep = find(&doc.endpoints, "/api/table/user");
    assert_eq!(ep.table.as_deref(), Some("user"));
    let names: Vec<&str> = ep.returned.iter().map(|r| r.name).collect();
    assert_eq!(names, ["id", "name", "status"], "字段顺序应与配置一致");
    assert!(ep.returned[0].required);
    assert_eq!(ep.returned[0].desc, Some("用户ID"));
    assert_eq!(ep.returned[0].mock, Some("1"));
    assert!(!ep.returned[2].required);
    assert_eq!(ep.returned[2].default, Some("1"));
    assert_eq!(ep.returned[2].desc, None);
    // 表字段是标量列，children 恒空
    assert!(ep.returned.iter().all(|r| r.children.is_empty()));
}

#[test]
fn table_fields_serialize_with_type_key_and_omit_defaults() {
    let doc = DocRegistry::collect_doc(cfg(vec![user_table()]));
    let v = serde_json::to_value(&doc).unwrap();
    let ep = find_json(&v["endpoints"], "/api/table/user");
    assert_eq!(ep["table"], json!("user"));
    let ret = ep["returned"].as_array().unwrap();
    // id: desc/mock 有值，default/children 省略
    assert_eq!(
        ret[0],
        json!({"name": "id", "type": "int", "required": true, "desc": "用户ID", "mock": "1"})
    );
    assert!(ret[0].get("ty").is_none(), "键名必须是 type 而非 ty");
    // status: default 有值照常输出，desc/mock/children 省略
    assert_eq!(
        ret[2],
        json!({"name": "status", "type": "int", "required": false, "default": "1"})
    );
}

#[test]
fn endpoint_without_table_annotation_has_no_table_key() {
    let doc = DocRegistry::collect_doc(cfg(vec![user_table()]));
    let v = serde_json::to_value(&doc).unwrap();
    let ep = find_json(&v["endpoints"], "/api/table/plain");
    assert!(ep.get("table").is_none(), "未用注解的端点不应出现 table 键");
    let s = serde_json::to_string(ep).unwrap();
    assert!(!s.contains("\"table\""), "v1 字节级兼容被破坏: {s}");
    // tables 配置本身也 #[serde(skip)]，不进 api.json
    assert_eq!(v["config"], json!({"title": "t"}));
}

#[test]
fn unconfigured_table_key_degrades_without_panic() {
    let doc = DocRegistry::collect_doc(cfg(vec![user_table()]));
    let ep = find(&doc.endpoints, "/api/table/missing");
    assert_eq!(ep.table.as_deref(), Some("missing_table"));
    assert!(ep.returned.is_empty(), "未配置 key 不产生字段");
    // 完全未配置 tables（v1 形态）走同一降级分支
    let doc = DocRegistry::collect_doc(cfg(Vec::new()));
    let ep = find(&doc.endpoints, "/api/table/missing");
    assert_eq!(ep.table.as_deref(), Some("missing_table"));
    assert!(ep.returned.is_empty());
}

#[test]
fn table_and_ref_compose() {
    let doc = DocRegistry::collect_doc(cfg(vec![user_table()]));
    let ep = find(&doc.endpoints, "/api/table/both");
    assert_eq!(ep.r#ref.as_deref(), Some("table_ref_target"));
    assert_eq!(ep.table.as_deref(), Some("user"));
    // 本端点先复制 ref 目标（该目标无表字段），再追加自己的表字段，两者都可见
    let names: Vec<&str> = ep.returned.iter().map(|r| r.name).collect();
    assert_eq!(names, ["base", "id", "name", "status"]);
}

#[test]
fn ref_inherits_target_table_fields() {
    let doc = DocRegistry::collect_doc(cfg(vec![user_table()]));
    let names = |ep: &DocEndpoint| -> Vec<&str> { ep.returned.iter().map(|r| r.name).collect() };
    let target = find(&doc.endpoints, "/api/table/target");
    assert_eq!(names(target), ["id", "name", "status"]);
    // A(ref B) 复制到的是 B"打完表"的 returned，字段与细节一并继承
    let referrer = find(&doc.endpoints, "/api/table/ref_table");
    assert_eq!(names(referrer), names(target));
    assert_eq!(referrer.returned[0].ty, "int");
    assert_eq!(referrer.returned[2].default, Some("1"));
    assert_eq!(referrer.table, None);
    // JSON 侧同样可见，且不重复
    let v = serde_json::to_value(&doc).unwrap();
    let json_ep = find_json(&v["endpoints"], "/api/table/ref_table");
    assert_eq!(json_ep["returned"].as_array().unwrap().len(), 3);
}
