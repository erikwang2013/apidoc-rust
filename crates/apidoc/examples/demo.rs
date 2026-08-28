use apidoc::*;

#[allow(dead_code)]
#[apidoc::title("获取用户信息")]
#[apidoc::desc("根据用户 ID 查询用户详情")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, desc = "用户ID", mock = "1")]
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
#[apidoc::param(name = "name", ty = "string", required, desc = "用户名")]
#[apidoc::param(name = "age", ty = "int", desc = "年龄", mock = "18")]
#[apidoc::returned(name = "user_id", ty = "int", required, desc = "新用户ID")]
fn create_user() -> String {
    unimplemented!()
}

fn main() {
    let endpoints = DocRegistry::collect();
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "demo api".to_string(),
            description: None,
        },
        endpoints,
    };
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
