//! 宏展开与多片段聚合：注解 → DocEndpoint 的合并正确性。

use apidoc::{DocEndpoint, DocRegistry};

#[allow(dead_code)]
#[apidoc::title("获取用户信息")]
#[apidoc::desc("根据用户 ID 查询用户详情")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, desc = "用户ID", mock = "1")]
#[apidoc::query(name = "lang", ty = "string", default = "zh-CN")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    desc = "用户数据",
    children = [
        { name = "id", ty = "int", required, desc = "用户ID" },
        { name = "name", ty = "string", required, desc = "用户名", mock = "erik" },
    ]
)]
fn get_user_info() {}

#[allow(dead_code)]
#[apidoc::title("创建用户")]
#[apidoc::desc("创建一个新用户")]
#[apidoc::url("/api/user")]
#[apidoc::method("POST")]
#[apidoc::param(name = "name", ty = "string", required, desc = "用户名")]
#[apidoc::returned(name = "user_id", ty = "int", required, desc = "新用户ID")]
fn create_user() {}

#[allow(dead_code)]
#[apidoc::title("缺省方法")]
#[apidoc::url("/api/default-method")]
fn default_method() {}

#[allow(dead_code)]
#[apidoc::param(name = "only_param", ty = "int", required)]
fn bare_param() {}

#[allow(dead_code)]
fn plain_fn() {}

fn find<'a>(eps: &'a [DocEndpoint], title: &str) -> &'a DocEndpoint {
    eps.iter().find(|e| e.title == title).expect("endpoint not found")
}

#[test]
fn seven_annotations_merge_into_one_complete_endpoint() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "获取用户信息");
    assert_eq!(ep.desc, "根据用户 ID 查询用户详情");
    assert_eq!(ep.url, "/api/user/info");
    assert_eq!(ep.method, "GET");
    assert!(ep.headers.is_empty());

    assert_eq!(ep.params.len(), 1);
    let p = &ep.params[0];
    assert_eq!(p.name, "user_id");
    assert_eq!(p.ty, "int");
    assert!(p.required);
    assert_eq!(p.desc, Some("用户ID"));
    assert_eq!(p.mock, Some("1"));
    assert_eq!(p.default, None);
    assert!(p.children.is_empty());

    assert_eq!(ep.querys.len(), 1);
    assert_eq!(ep.querys[0].name, "lang");
    assert_eq!(ep.querys[0].ty, "string");
    assert_eq!(ep.querys[0].default, Some("zh-CN"));

    assert_eq!(ep.returned.len(), 1);
    let r = &ep.returned[0];
    assert_eq!(r.name, "data");
    assert_eq!(r.ty, "object");
    assert_eq!(r.desc, Some("用户数据"));
    assert_eq!(r.children.len(), 2);
    assert_eq!(r.children[0].name, "id");
    assert!(r.children[0].required);
    assert_eq!(r.children[1].name, "name");
    assert_eq!(r.children[1].mock, Some("erik"));
    assert!(r.children[1].required);
}

#[test]
fn multiple_fns_become_multiple_endpoints_without_crosstalk() {
    let eps = DocRegistry::collect();
    let create = find(&eps, "创建用户");
    assert_eq!(create.desc, "创建一个新用户");
    assert_eq!(create.url, "/api/user");
    assert_eq!(create.method, "POST");
    assert_eq!(create.params.len(), 1);
    assert_eq!(create.params[0].name, "name");
    assert_eq!(create.returned.len(), 1);
    // 无串扰：创建用户的 endpoint 不含第一个 fn 的 query/children
    assert!(create.querys.is_empty());
    assert!(create.returned[0].children.is_empty());

    let get = find(&eps, "获取用户信息");
    assert_eq!(get.querys.len(), 1);
    assert_eq!(get.params.len(), 1);
}

#[test]
fn method_defaults_to_get() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "缺省方法");
    assert_eq!(ep.method, "GET");
    assert_eq!(ep.title, "缺省方法");
    assert_eq!(ep.url, "/api/default-method");
}

#[test]
fn param_only_fn_builds_endpoint_with_empty_strings() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "");
    assert_eq!(ep.title, "");
    assert_eq!(ep.desc, "");
    assert_eq!(ep.url, "");
    assert_eq!(ep.method, "GET");
    assert_eq!(ep.params.len(), 1);
    assert_eq!(ep.params[0].name, "only_param");
    assert!(ep.querys.is_empty());
    assert!(ep.returned.is_empty());
}

#[test]
fn plain_fn_is_not_collected() {
    // 只有 4 个注解过的 fn；plain_fn 不产生任何 endpoint
    assert_eq!(DocRegistry::collect().len(), 4);
}
