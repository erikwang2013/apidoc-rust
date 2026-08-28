//! Swagger / OpenAPI 3.0.0 导出（README 定稿 OpenAPI3，不用 2.0）。
//! 返回 serde_json::Value 便于测试断言与直接序列化。
//! not_debug 端点不进输出；url 的 `:id` 占位符转 `{id}`。

use crate::{ApiDoc, DocEndpoint, DocExample, DocParam};
use serde_json::{json, Map, Value};

/// `version` 为项目版本（OpenAPI 3.0 info.version 必填），调用方从 VERSION 文件提供。
pub fn render(doc: &ApiDoc, version: &str) -> Value {
    let mut paths = Map::new();
    for ep in doc.endpoints.iter().filter(|e| !e.not_debug) {
        let path = convert_path(&ep.url);
        let op = render_operation(ep);
        let method = ep.method.to_lowercase();
        paths
            .entry(path)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("path must be object")
            .insert(method, op);
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
    for p in &ep.route_params {
        parameters.push(param_obj(p, "path", true));
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
        responses.insert(ex.code.to_string(), success_response(ex, &ep.returned));
    }
    for ex in &ep.error {
        let code = error_code(ex.code);
        responses.insert(code.clone(), error_response(ex, &code));
    }
    if responses.is_empty() {
        responses.insert("200".into(), json!({ "description": "OK" }));
    }
    op.insert("responses".into(), Value::Object(responses));
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

fn schema_for(params: &[DocParam]) -> Value {
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
