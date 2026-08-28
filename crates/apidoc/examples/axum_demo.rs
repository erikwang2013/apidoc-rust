use apidoc::axum::{apidoc_routes, cors_layer, CorsConfig};
use apidoc::ApidocConfig;
use axum::Router;

#[allow(dead_code)]
#[apidoc::title("获取用户信息")]
#[apidoc::desc("根据用户 ID 查询用户详情")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, desc = "用户ID", mock = "1")]
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

#[allow(dead_code)]
#[apidoc::title("健康检查")]
#[apidoc::url("/health")]
#[apidoc::method("GET")]
fn health() -> String {
    unimplemented!()
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .merge(apidoc_routes(ApidocConfig {
            title: "demo api".to_string(),
            description: Some("apidoc::axum 演示".to_string()),
            auth: None,
            apps: Vec::new(),
        }))
        // 收紧模式演示：改为 CorsConfig { allow_origins: vec!["http://localhost:3000".into()] }
        .layer(cors_layer(CorsConfig::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
