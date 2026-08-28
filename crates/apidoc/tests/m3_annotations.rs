//! M3 注解：13 个新注解的聚合、去重、覆盖语义、ref 解析与序列化省略规则。

use apidoc::{ApiDoc, ApidocConfig, DocEndpoint, DocRegistry};
use serde_json::{json, Value};

#[allow(dead_code)]
#[apidoc::title("完整新注解")]
#[apidoc::url("/api/user/info")]
#[apidoc::group("用户管理")]
#[apidoc::author("erik")]
#[apidoc::tag("user", "auth")]
#[apidoc::tag("v2")]
#[apidoc::header(name = "X-Token", desc = "访问令牌")]
#[apidoc::route_param(name = "user_id", ty = "int", desc = "用户ID", mock = "1")]
#[apidoc::response_status("200", "404", "500")]
#[apidoc::response_status("200")]
#[apidoc::success(code = "200", example = "{\"code\":0,\"data\":{}}")]
#[apidoc::error(code = "500", example = "{\"code\":1,\"msg\":\"err\"}")]
#[apidoc::not_debug]
#[apidoc::md("### 备注\n调用前需登录")]
#[apidoc::sort(10)]
fn full_endpoint() {}

#[allow(dead_code)]
#[apidoc::title("覆盖语义")]
#[apidoc::group("第一组")]
#[apidoc::group("第二组")]
#[apidoc::author("a")]
#[apidoc::author("b")]
fn overwrite() {}

#[allow(dead_code)]
#[apidoc::title("排序权重")]
#[apidoc::sort(-1)]
fn neg_sort() {}

#[allow(dead_code)]
#[apidoc::title("参考接口")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },
        { name = "name", ty = "string", required },
    ]
)]
fn get_user_list() {}

#[allow(dead_code)]
#[apidoc::title("引用接口")]
// `ref` 是 Rust 关键字，属性路径须写 raw identifier（与模型字段 r#ref 同理）
#[apidoc::r#ref("get_user_list")]
fn ref_user_list() {}

fn find<'a>(eps: &'a [DocEndpoint], title: &str) -> &'a DocEndpoint {
    eps.iter().find(|e| e.title == title).expect("endpoint not found")
}

#[test]
fn m3_annotations_aggregate_into_fields() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "完整新注解");
    assert_eq!(ep.group, "用户管理");
    assert_eq!(ep.author, "erik");
    // 变参一次挂多个 + 可重复挂载追加
    assert_eq!(ep.tags, ["user", "auth", "v2"]);
    assert_eq!(ep.headers.len(), 1);
    assert_eq!(ep.headers[0].name, "X-Token");
    assert_eq!(ep.headers[0].desc, Some("访问令牌"));
    assert_eq!(ep.route_params.len(), 1);
    assert_eq!(ep.route_params[0].name, "user_id");
    assert_eq!(ep.route_params[0].ty, "int");
    assert_eq!(ep.route_params[0].mock, Some("1"));
    // response_status 追加 + 去重："200" 挂两次只保留一个
    assert_eq!(ep.response_status, ["200", "404", "500"]);
    assert_eq!(ep.success.len(), 1);
    assert_eq!(ep.success[0].code, "200");
    assert_eq!(ep.success[0].example, "{\"code\":0,\"data\":{}}");
    assert_eq!(ep.error.len(), 1);
    assert_eq!(ep.error[0].code, "500");
    assert_eq!(ep.error[0].example, "{\"code\":1,\"msg\":\"err\"}");
    assert!(ep.not_debug);
    assert_eq!(ep.md, "### 备注\n调用前需登录");
    assert_eq!(ep.sort, 10);
    assert_eq!(ep.r#ref, None);
}

#[test]
fn single_value_fields_overwrite_later_mount_wins() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "覆盖语义");
    assert_eq!(ep.group, "第二组");
    assert_eq!(ep.author, "b");
    assert_eq!(find(&eps, "排序权重").sort, -1);
}

#[test]
fn ref_copies_target_returned_by_suffix_match() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "引用接口");
    assert_eq!(ep.r#ref.as_deref(), Some("get_user_list"));
    assert_eq!(ep.returned.len(), 1);
    assert_eq!(ep.returned[0].name, "data");
    assert_eq!(ep.returned[0].children.len(), 2);
    assert_eq!(ep.returned[0].children[0].name, "id");
    assert_eq!(ep.returned[0].children[1].name, "name");
    // 目标接口自身不受影响
    assert_eq!(find(&eps, "参考接口").returned.len(), 1);
}

#[allow(dead_code)]
// 校验边界：100/599 是合法 HTTP 状态码（编译期校验不误报）
#[apidoc::title("边界值")]
#[apidoc::response_status("100", "599")]
#[apidoc::success(code = "599", example = "{}")]
#[apidoc::header(name = "X-NoDesc")]
fn boundary_status() {}

#[test]
fn new_fields_serialize_with_exact_json_names() {
    let doc = ApiDoc {
        config: ApidocConfig { title: "t".into(), description: None },
        endpoints: DocRegistry::collect(),
    };
    let v: Value = serde_json::to_value(&doc).unwrap();
    let eps = v["endpoints"].as_array().unwrap();
    let full = eps.iter().find(|e| e["title"] == "完整新注解").unwrap();
    // headers：name/desc 直出；route_params 复用 DocParam 的 type rename
    assert_eq!(full["headers"][0], json!({"name": "X-Token", "desc": "访问令牌"}));
    assert_eq!(full["route_params"][0]["type"], "int");
    assert!(full["route_params"][0].get("ty").is_none());
    assert_eq!(
        full["success"][0],
        json!({"code": "200", "example": "{\"code\":0,\"data\":{}}"}),
    );
    assert_eq!(full["error"][0], json!({"code": "500", "example": "{\"code\":1,\"msg\":\"err\"}"}));
    assert_eq!(full["sort"], 10);
    assert_eq!(full["not_debug"], true);
    // r#ref 字段在 JSON 中的键名是 "ref"
    let refep = eps.iter().find(|e| e["title"] == "引用接口").unwrap();
    assert_eq!(refep["ref"], "get_user_list");
    assert!(refep.get("group").is_none());
    // 边界值通过编译期校验；header 未提供 desc 时省略（与 DocParam 一致）
    let boundary = eps.iter().find(|e| e["title"] == "边界值").unwrap();
    assert_eq!(boundary["response_status"], json!(["100", "599"]));
    assert_eq!(boundary["headers"][0], json!({"name": "X-NoDesc"}));
    assert_eq!(boundary["success"][0]["code"], "599");
}

#[test]
fn new_fields_omitted_from_json_when_default() {
    let doc = ApiDoc {
        config: ApidocConfig { title: "t".into(), description: None },
        endpoints: DocRegistry::collect(),
    };
    let v: Value = serde_json::to_value(&doc).unwrap();
    let eps = v["endpoints"].as_array().unwrap();
    // 未使用任何新注解的 endpoint：全部新字段省略（api.json 零变化保证）
    let bare = eps.iter().find(|e| e["title"] == "参考接口").unwrap();
    for key in [
        "group", "tags", "author", "headers", "route_params", "response_status",
        "success", "error", "not_debug", "md", "sort", "ref",
    ] {
        assert!(bare.get(key).is_none(), "unexpected key {key}");
    }
    // 使用新注解的 endpoint：对应字段原样输出
    let full = eps.iter().find(|e| e["title"] == "完整新注解").unwrap();
    assert_eq!(full["group"], "用户管理");
    assert_eq!(full["tags"], json!(["user", "auth", "v2"]));
    assert_eq!(full["response_status"], json!(["200", "404", "500"]));
    assert_eq!(full["success"][0]["code"], "200");
    assert_eq!(full["success"][0]["example"], "{\"code\":0,\"data\":{}}");
    assert_eq!(full["not_debug"], true);
    assert_eq!(full["md"], "### 备注\n调用前需登录");
    assert_eq!(full["sort"], 10);
    assert_eq!(eps.iter().find(|e| e["title"] == "排序权重").unwrap()["sort"], -1);
}
