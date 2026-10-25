//! Swagger / OpenAPI 3.0.0 导出（README 定稿 OpenAPI3，不用 2.0）。
//! 返回 serde_json::Value 便于测试断言与直接序列化。
//! not_debug 端点不进输出；url 的 `:id` 占位符转 `{id}`。

use crate::{ApiDoc, DocEndpoint, DocExample, DocParam};
use serde_json::{json, Map, Value};

/// `version` 为项目版本（OpenAPI 3.0 info.version 必填），适配器传 Cargo 包版本
/// （`env!("CARGO_PKG_VERSION")`，见 src/route_core.rs）。
pub fn render(doc: &ApiDoc, version: &str) -> Value {
    let mut paths = Map::new();
    for ep in doc.endpoints.iter().filter(|e| !e.not_debug) {
        // 非 HTTP 语义的协议（WS/gRPC/TCP…）没有 OpenAPI 位置，直接跳过；
        // HTTP 族（SOAP/GraphQL/JSON-RPC…）照常进 paths，并带 x-apidoc-protocol。
        if !crate::protocol::is_http_compatible(&ep.protocol) {
            continue;
        }
        let path = convert_path(&ep.url);
        let op = render_operation(ep);
        let method = ep.method.to_lowercase();
        // 同一个 path 多方法：合并进同一个 object（serializer 路径不留可 panic 的 expect）
        if let Some(m) = paths.entry(path).or_insert_with(|| Value::Object(Map::new())).as_object_mut()
        {
            m.insert(method, op);
        }
    }
    let mut info = json!({ "title": doc.config.title, "version": version });
    if let Some(desc) = &doc.config.description {
        info["description"] = Value::String(desc.clone());
    }
    json!({
        "openapi": "3.0.0",
        "info": info,
        "paths": Value::Object(paths),
    })
}

/// `:id` → `{id}`（`{id}` 形式原样保留），逐段归一。
fn convert_path(url: &str) -> String {
    url.split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(name) => format!("{{{name}}}"),
            None => seg.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn render_operation(ep: &DocEndpoint) -> Value {
    let mut op = Map::new();
    let mut parameters = Vec::new();
    // path 参数以 URL 模板为准：OpenAPI 要求模板里的 `{x}` 必须声明，声明的
    // in:path 参数也必须出现在模板里。route_param 注解只提供类型/描述，名字对不上
    // 的丢弃并告警——照抄注解会让任何「注解与 URL 不逐字一致」的接口产出非法 spec。
    let spec = path_placeholders(&ep.url);
    for name in &spec {
        let declared = ep.route_params.iter().find(|p| p.name == name);
        let mut o = Map::new();
        o.insert("name".into(), Value::String(name.clone()));
        o.insert("in".into(), Value::String("path".into()));
        o.insert("required".into(), Value::Bool(true));
        o.insert(
            "schema".into(),
            json!({ "type": schema_type(declared.map_or("string", |p| p.ty)) }),
        );
        if let Some(desc) = declared.and_then(|p| p.desc) {
            o.insert("description".into(), Value::String(desc.to_string()));
        }
        parameters.push(Value::Object(o));
    }
    for p in &ep.route_params {
        if !spec.iter().any(|n| n == p.name) {
            eprintln!(
                "apidoc: route_param `{}` of `{}` is not in its url template, dropping it (declaring it as in:path would make the OpenAPI spec invalid)",
                p.name, ep.url
            );
        }
    }
    for p in &ep.querys {
        parameters.push(param_obj(p, "query", p.required));
    }
    for h in &ep.headers {
        let mut o = Map::new();
        o.insert("name".into(), Value::String(h.name.to_string()));
        o.insert("in".into(), Value::String("header".into()));
        o.insert("required".into(), Value::Bool(false));
        o.insert("schema".into(), json!({ "type": "string" }));
        parameters.push(Value::Object(o));
    }
    if !parameters.is_empty() {
        op.insert("parameters".into(), Value::Array(parameters));
    }
    if !ep.params.is_empty() {
        op.insert("requestBody".into(), json!({
            "content": { "application/json": { "schema": schema_for(&ep.params) } }
        }));
    }
    let mut responses = Map::new();
    for ex in &ep.success {
        let code = success_code(ex.code);
        if responses.insert(code.clone(), success_response(ex, &ep.returned)).is_some() {
            eprintln!("apidoc: duplicate response code {code} for `{}`, keeping the first", ep.url);
        }
    }
    for ex in &ep.error {
        let code = error_code(ex.code);
        if responses.insert(code.clone(), error_response(ex, &code)).is_some() {
            eprintln!("apidoc: duplicate response code {code} for `{}`, keeping the first", ep.url);
        }
    }
    if responses.is_empty() {
        responses.insert("200".into(), json!({ "description": "OK" }));
    }
    op.insert("responses".into(), Value::Object(responses));
    // 非默认协议用 vendor extension 保留信息（OpenAPI 无对应字段）。
    if ep.protocol != "http" {
        op.insert("x-apidoc-protocol".into(), Value::String(ep.protocol.clone()));
    }
    Value::Object(op)
}

fn param_obj(p: &DocParam, loc: &str, required: bool) -> Value {
    json!({
        "name": p.name,
        "in": loc,
        "required": required,
        "schema": { "type": schema_type(p.ty) },
    })
}

/// 供 AsyncAPI 复用：params/returned → JSON Schema object。
pub(crate) fn schema_for(params: &[DocParam]) -> Value {
    let mut props = Map::new();
    for p in params {
        props.insert(p.name.to_string(), prop_schema(p));
    }
    json!({ "type": "object", "properties": Value::Object(props) })
}

fn prop_schema(p: &DocParam) -> Value {
    if !p.children.is_empty() {
        if p.ty == "array" {
            json!({ "type": "array", "items": schema_for(p.children) })
        } else {
            schema_for(p.children)
        }
    } else {
        json!({ "type": schema_type(p.ty) })
    }
}

fn schema_type(ty: &str) -> &'static str {
    match ty {
        "int" => "integer",
        "float" => "number",
        "bool" => "boolean",
        "object" => "object",
        "array" => "array",
        _ => "string",
    }
}

fn success_response(ex: &DocExample, returned: &[DocParam]) -> Value {
    let mut content = Map::new();
    if !returned.is_empty() {
        content.insert("schema".into(), schema_for(returned));
    }
    content.insert("example".into(), parse_example(ex.example));
    json!({
        "description": status_text(ex.code),
        "content": { "application/json": Value::Object(content) },
    })
}

fn error_response(ex: &DocExample, code: &str) -> Value {
    json!({
        "description": status_text(code),
        "content": { "application/json": { "example": parse_example(ex.example) } },
    })
}

/// error 例子的状态码必须是 4xx/5xx，否则归 500。
fn error_code(code: &str) -> String {
    match code.parse::<u16>() {
        Ok(n) if (400..=599).contains(&n) => code.to_string(),
        _ => "500".to_string(),
    }
}

/// success 例子的状态码不合 OpenAPI（`^[1-5](\d{2}|XX)$`）时归 200。
/// （注解层已校验 100-599，这里兜住手工构造 DocExample 的路径。）
fn success_code(code: &str) -> String {
    match code.parse::<u16>() {
        Ok(n) if (100..=599).contains(&n) => code.to_string(),
        _ => "200".to_string(),
    }
}

/// URL 模板里的路径占位符（`:id` 与 `{id}` 两种写法），按出现顺序、去重。
fn path_placeholders(url: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in url.split('/') {
        let name = seg
            .strip_prefix(':')
            .or_else(|| seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')));
        if let Some(n) = name {
            if !n.is_empty() && !out.iter().any(|x| x == n) {
                out.push(n.to_string());
            }
        }
    }
    out
}

fn status_text(code: &str) -> &'static str {
    match code {
        "200" => "OK",
        "201" => "Created",
        "204" => "No Content",
        "400" => "Bad Request",
        "401" => "Unauthorized",
        "403" => "Forbidden",
        "404" => "Not Found",
        "409" => "Conflict",
        "422" => "Unprocessable Entity",
        "500" => "Internal Server Error",
        "502" => "Bad Gateway",
        "503" => "Service Unavailable",
        _ => "Response",
    }
}

/// example 字段按 JSON 解析，解析失败原样透出为字符串（注解不校验 example 合法性）。
fn parse_example(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}
