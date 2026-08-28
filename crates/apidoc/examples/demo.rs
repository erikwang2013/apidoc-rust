use apidoc::*;

#[allow(dead_code)]
#[apidoc::title("获取用户信息")]
#[apidoc::desc("根据用户 ID 查询用户详情")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::group("用户管理")]
#[apidoc::author("erik")]
#[apidoc::tag("user", "v1")]
#[apidoc::header(name = "X-Token", desc = "访问令牌")]
#[apidoc::route_param(name = "user_id", ty = "int", required, desc = "用户ID", mock = "1")]
#[apidoc::response_status("200", "404")]
#[apidoc::success(code = "200", example = "{\"code\":0,\"data\":{\"id\":1,\"name\":\"erik\"}}")]
#[apidoc::error(code = "404", example = "{\"code\":1,\"msg\":\"user not found\"}")]
#[apidoc::not_debug]
#[apidoc::md("### 备注\n调用前需登录")]
#[apidoc::sort(10)]
#[apidoc::param(name = "verbose", ty = "bool", desc = "是否返回扩展字段", default = "false")]
#[apidoc::query(name = "lang", ty = "string", desc = "语言", default = "zh-CN")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    desc = "用户数据",
    children = [
        { name = "id", ty = "int", required, desc = "用户ID" },
        { name = "name", ty = "string", required, desc = "用户名", mock = "erik" },
    ]
)]
fn get_user_info() -> String {
    unimplemented!()
}

#[allow(dead_code)]
#[apidoc::title("创建用户")]
#[apidoc::desc("创建一个新用户")]
#[apidoc::url("/api/user")]
#[apidoc::method("POST")]
#[apidoc::group("用户管理")]
#[apidoc::sort(20)]
#[apidoc::param(name = "name", ty = "string", required, desc = "用户名")]
#[apidoc::param(name = "age", ty = "int", desc = "年龄", mock = "18")]
#[apidoc::returned(name = "user_id", ty = "int", required, desc = "新用户ID")]
fn create_user() -> String {
    unimplemented!()
}

#[allow(dead_code)]
#[apidoc::title("用户列表")]
#[apidoc::url("/api/user/list")]
#[apidoc::method("GET")]
#[apidoc::group("用户管理")]
#[apidoc::sort(5)]
#[apidoc::returned(
    name = "list",
    ty = "array",
    desc = "用户列表",
    children = [
        { name = "id", ty = "int", required, desc = "用户ID" },
        { name = "name", ty = "string", required, desc = "用户名" },
    ]
)]
fn get_user_list() -> String {
    unimplemented!()
}

#[allow(dead_code)]
#[apidoc::title("用户详情(复用)")]
#[apidoc::url("/api/user/detail")]
#[apidoc::method("GET")]
#[apidoc::group("用户管理")]
// `ref` 是 Rust 关键字，属性路径写 raw identifier（与模型字段 r#ref 同理）
#[apidoc::r#ref("get_user_list")]
fn get_user_detail() -> String {
    unimplemented!()
}

fn main() {
    let endpoints = DocRegistry::collect();
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "demo api".to_string(),
            description: None, auth: None, apps: Vec::new(),
        },
        apps: Vec::new(), endpoints,
    };
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
