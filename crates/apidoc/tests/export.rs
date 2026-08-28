//! M5 导出测试：markdown 分组与参数表、typescript 类型名去重与 children
//! 嵌套、swagger path 占位符与 requestBody/parameters/responses 映射。

use apidoc::{ApiDoc, ApidocConfig, DocRegistry};
use serde_json::json;

// fixture：用户组，URL 带 :id 占位符，children 两级嵌套
#[allow(dead_code)]
#[apidoc::title("用户详情")]
#[apidoc::desc("获取单个用户信息")]
#[apidoc::url("/api/user/:id")]
#[apidoc::method("GET")]
#[apidoc::group("用户")]
#[apidoc::author("erik")]
#[apidoc::tag("用户管理")]
#[apidoc::route_param(name = "id", ty = "int", required, desc = "用户 ID")]
#[apidoc::query(name = "verbose", ty = "bool", default = "false", desc = "是否返回详情")]
#[apidoc::param(name = "name", ty = "string", required, desc = "用户名")]
#[apidoc::param(name = "user", ty = "object", children = [
    { name = "id", ty = "int", required },
    { name = "profile", ty = "object", children = [{ name = "bio", ty = "string" }] }
])]
#[apidoc::returned(name = "id", ty = "int", required)]
#[apidoc::returned(name = "name", ty = "string")]
#[apidoc::success(code = "200", example = "{\"id\":1,\"name\":\"alice\"}")]
#[apidoc::error(code = "404", example = "{\"error\":\"not found\"}")]
fn get_user() {}

// fixture：两个「GET + 末段 orders」端点 → TS 类型名去重（GetOrders / GetOrders2）
#[allow(dead_code)]
#[apidoc::title("订单列表")]
#[apidoc::url("/api/orders")]
#[apidoc::method("GET")]
#[apidoc::group("订单")]
#[apidoc::query(name = "page", ty = "int")]
fn list_orders() {}

#[allow(dead_code)]
#[apidoc::title("后台订单")]
#[apidoc::url("/admin/orders")]
#[apidoc::method("GET")]
#[apidoc::group("后台")]
#[apidoc::returned(name = "total", ty = "int", required)]
fn admin_list_orders() {}

// fixture：not_debug 端点，swagger 必须排除、markdown/ts 照常输出
#[allow(dead_code)]
#[apidoc::title("内部接口")]
#[apidoc::url("/internal/secret")]
#[apidoc::method("POST")]
#[apidoc::not_debug]
fn internal() {}

// fixture：无 group → markdown 落「未分组」
#[allow(dead_code)]
#[apidoc::title("健康检查")]
#[apidoc::url("/health")]
#[apidoc::method("GET")]
fn health() {}

fn doc() -> ApiDoc {
    ApiDoc {
        config: ApidocConfig {
            title: "测试文档".into(),
            description: Some("导出测试".into()), auth: None, apps: Vec::new(),
        },
        apps: Vec::new(), endpoints: DocRegistry::collect(),
    }
}

#[test]
fn markdown_groups_and_param_table() {
    let md = apidoc::export::markdown::render(&doc());
    // 分节：用户 / 订单 / 后台 / 未分组
    assert!(md.contains("## 用户\n"), "缺用户分组");
    assert!(md.contains("## 订单\n"), "缺订单分组");
    assert!(md.contains("## 后台\n"), "缺后台分组");
    assert!(md.contains("## 未分组\n"), "无 group 端点应落未分组");
    // 接口块：标题 + method/url 代码块
    assert!(md.contains("### 用户详情\n"));
    assert!(md.contains("```\nGET /api/user/:id\n```\n"));
    // desc + 作者/标签行
    assert!(md.contains("获取单个用户信息"));
    assert!(md.contains("作者：erik　标签：用户管理"));
    // 参数表：route_param + param + query 合并，children 点路径展平
    let table = "| 名称 | 类型 | 必填 | 默认 | 描述 |\n\
                 |---|---|---|---|---|\n\
                 | id | int | 是 | - | 用户 ID |\n\
                 | name | string | 是 | - | 用户名 |\n\
                 | user | object | 否 | - | - |\n\
                 | user.id | int | 是 | - | - |\n\
                 | user.profile | object | 否 | - | - |\n\
                 | user.profile.bio | string | 否 | - | - |\n\
                 | verbose | bool | 否 | false | 是否返回详情 |\n";
    assert!(md.contains(table), "参数表不匹配:\n{md}");
    // 响应代码块
    assert!(md.contains("**成功响应 200**\n\n```json\n{\"id\":1,\"name\":\"alice\"}\n```\n"));
    assert!(md.contains("**错误响应 404**\n\n```json\n{\"error\":\"not found\"}\n```\n"));
}

#[test]
fn typescript_namespaces_interfaces_and_dedup() {
    let ts = apidoc::export::typescript::render(&doc());
    // 路径常量按 group 命名空间（中文 group 原样，未分组落 defaultGroup）
    assert!(ts.contains("export const 用户 = {\n  getUser: '/api/user/:id' as const,\n}\n"));
    assert!(ts.contains("export const 订单 = {\n  getOrders: '/api/orders' as const,\n}\n"));
    assert!(ts.contains("export const 后台 = {\n  getOrders: '/admin/orders' as const,\n}\n"));
    assert!(ts.contains(
        "export const defaultGroup = {\n  postSecret: '/internal/secret' as const,\n  getHealth: '/health' as const,\n}\n"
    ));
    // 同名派生类型去重：GetOrdersParams / GetOrders2Result
    assert!(ts.contains("export interface GetOrdersParams {\n  page?: number\n}\n"), "缺 GetOrdersParams");
    assert!(ts.contains("export interface GetOrders2Result {\n  total: number\n}\n"), "重名类型应加数字后缀");
    // Params：params+querys 合并，children 嵌套对象，必填不加 ?
    assert!(ts.contains(
        "export interface GetUserParams {\n  name: string\n  user?: { id: number; profile?: { bio?: string } }\n  verbose?: boolean\n}\n"
    ));
    // Result：returned 映射
    assert!(ts.contains("export interface GetUserResult {\n  id: number\n  name?: string\n}\n"));
    // 无参数端点不产接口
    assert!(!ts.contains("GetHealthParams"), "空参数不应产 Params 接口");
}

#[test]
fn swagger_paths_params_body_responses_and_not_debug_exclusion() {
    let v = apidoc::export::swagger::render(&doc(), "1.0.0");
    assert_eq!(v["openapi"], "3.0.0");
    assert_eq!(v["info"]["title"], "测试文档");
    assert_eq!(v["info"]["description"], "导出测试");
    // :id 占位符 → {id}
    let op = &v["paths"]["/api/user/{id}"]["get"];
    // 参数：route_param → in:path required、query → in:query
    assert_eq!(
        op["parameters"],
        json!([
            { "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } },
            { "name": "verbose", "in": "query", "required": false, "schema": { "type": "boolean" } },
        ])
    );
    // requestBody：params → object properties，children 递归
    let schema = &op["requestBody"]["content"]["application/json"]["schema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["user"]["type"], "object");
    assert_eq!(schema["properties"]["user"]["properties"]["id"]["type"], "integer");
    assert_eq!(
        schema["properties"]["user"]["properties"]["profile"]["properties"]["bio"]["type"],
        "string"
    );
    // responses：success → 200 + example + schema(returned)；error → 404
    let r200 = &op["responses"]["200"];
    assert_eq!(r200["description"], "OK");
    assert_eq!(r200["content"]["application/json"]["example"], json!({"id": 1, "name": "alice"}));
    assert_eq!(r200["content"]["application/json"]["schema"]["properties"]["id"]["type"], "integer");
    let r404 = &op["responses"]["404"];
    assert_eq!(r404["description"], "Not Found");
    assert_eq!(r404["content"]["application/json"]["example"], json!({"error": "not found"}));
    // 无 example 的端点也有默认 200
    assert_eq!(v["paths"]["/health"]["get"]["responses"]["200"]["description"], "OK");
    // not_debug 端点排除，其余端点照常
    assert!(v["paths"].get("/internal/secret").is_none(), "not_debug 不应进 swagger");
    assert!(v["paths"].get("/api/orders").is_some());
}

// —— tester 补充：M5 边界 ——

fn doc_param(name: &'static str, ty: &'static str, required: bool, desc: Option<&'static str>) -> apidoc::DocParam {
    apidoc::DocParam { name, ty, required, default: None, desc, mock: None, children: &[] }
}

#[test]
fn boundary_empty_doc_renders_without_panic() {
    let d = ApiDoc { config: ApidocConfig { title: "空文档".into(), description: None, auth: None, apps: Vec::new() }, apps: Vec::new(), endpoints: vec![] };
    assert!(apidoc::export::markdown::render(&d).contains("# 空文档"));
    assert!(apidoc::export::typescript::render(&d).is_empty());
    let v = apidoc::export::swagger::render(&d, "1.0.0");
    assert_eq!(v["openapi"], "3.0.0");
    assert_eq!(v["paths"], json!({}));
}

#[test]
fn boundary_swagger_roundtrip_and_mixed_placeholders() {
    let mut ep = apidoc::DocEndpoint {
        title: "混合占位符".into(),
        url: "/api/users/:uid/posts/{pid}/latest".into(),
        method: "GET".into(),
        ..Default::default()
    };
    ep.route_params.push(doc_param("uid", "int", true, None));
    ep.route_params.push(doc_param("pid", "int", true, None));
    let d = ApiDoc { config: ApidocConfig { title: "t".into(), description: None, auth: None, apps: Vec::new() }, apps: Vec::new(), endpoints: vec![ep] };
    let v = apidoc::export::swagger::render(&d, "1.0.0");
    // axum route 依赖 to_string 预序列化：round-trip 必须成功且是合法 JSON
    let s = serde_json::to_string(&v).unwrap();
    let back: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(back["openapi"], "3.0.0");
    // :uid 与 {pid} 混合归一，无占位符段原样保留
    let path = "/api/users/{uid}/posts/{pid}/latest";
    assert!(back["paths"].get(path).is_some(), "混合占位符路径应为 {path}: {s}");
    assert_eq!(back["paths"][path]["get"]["parameters"][0]["name"], "uid");
}

#[test]
fn swagger_error_code_outside_4xx_5xx_maps_to_500() {
    let mut ep = apidoc::DocEndpoint {
        title: "错误映射".into(),
        url: "/api/bad".into(),
        method: "POST".into(),
        ..Default::default()
    };
    ep.error.push(apidoc::DocExample { code: "200", example: "{}" });
    let doc = ApiDoc {
        config: ApidocConfig { title: "t".into(), description: None, auth: None, apps: Vec::new() },
        apps: Vec::new(), endpoints: vec![ep],
    };
    let v = apidoc::export::swagger::render(&doc, "1.0.0");
    let op = &v["paths"]["/api/bad"]["post"];
    assert_eq!(op["responses"]["500"]["description"], "Internal Server Error");
    assert!(op["responses"].get("200").is_none(), "2xx 不应出现在 error 响应");
}
