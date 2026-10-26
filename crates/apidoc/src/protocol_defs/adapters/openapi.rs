//! Port of `php/src/protocol/Adapter/OpenApiAdapter.php`.

use serde_json::{json, Value};

use crate::protocol_defs::adapters::{message, refs_for, sample_validation, Adapter, Parsed};
use crate::protocol_defs::context::{Context, Loader};
use crate::protocol_defs::node::{parse_json, Node};
use crate::protocol_defs::schema::{self, RefCtx};
use crate::protocol_defs::token::ParseError;
use crate::protocol_defs::yaml;

const METHODS: [&str; 8] = [
    "get", "post", "put", "patch", "delete", "options", "head", "trace",
];

const CONTENT_TYPES: [&str; 3] = ["application/json", "*/*", "application/*"];

pub struct OpenApiAdapter;

#[derive(Clone)]
struct Operation {
    path: String,
    method: String,
    operation_id: String,
    operation: Node,
    path_item: Node,
    root: Node,
    file: String,
}

impl Operation {
    fn display_name(&self) -> String {
        if self.operation_id.is_empty() {
            self.signature()
        } else {
            self.operation_id.clone()
        }
    }

    fn signature(&self) -> String {
        format!("{} {}", self.method.to_uppercase(), self.path)
    }
}

impl Adapter for OpenApiAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed, ParseError> {
        let document = decode(content)?;
        let version = document
            .get("openapi")
            .map(Node::version_string)
            .unwrap_or_default();
        let swagger = document
            .get("swagger")
            .map(Node::version_string)
            .unwrap_or_default();
        if version.is_empty() && swagger.is_empty() {
            ctx.error("OpenAPI version is missing", 0, 0);
        } else if !version.is_empty() && !version.starts_with("3.") {
            ctx.unsupported(&format!("unsupported OpenAPI version: {version}"), 0, 0);
        } else if !swagger.is_empty() && !swagger.starts_with("2.") {
            ctx.unsupported(&format!("unsupported Swagger version: {swagger}"), 0, 0);
        }
        let paths = match document.get("paths").filter(|node| node.is_array_like()) {
            Some(paths) if !paths.is_empty() => paths.clone(),
            _ => {
                ctx.error("OpenAPI paths are missing", 0, 0);
                return Ok(Parsed::empty());
            }
        };
        let operations = collect_operations(&paths, &document, ctx);
        if operations.is_empty() {
            ctx.error("OpenAPI operations are missing", 0, 0);
            return Ok(Parsed::empty());
        }
        let wanted = ctx.options().operation.clone();
        let selected = select(&operations, &wanted, ctx);
        let operation = selected.operation.clone();
        let operation_root = selected.root.clone();
        let operation_file = selected.file.clone();
        let refs = refs_for(&document, Loader::JsonOrYaml);

        let mut request_fields: Vec<Value> = Vec::new();
        let mut parameters: Vec<Node> = Vec::new();
        if let Some(list) = selected
            .path_item
            .get("parameters")
            .filter(|node| node.is_array_like())
        {
            parameters.extend(schema::entries(list).into_iter().map(|(_, value)| value));
        }
        if let Some(list) = operation
            .get("parameters")
            .filter(|node| node.is_array_like())
        {
            parameters.extend(schema::entries(list).into_iter().map(|(_, value)| value));
        }
        for parameter in parameters {
            if !parameter.is_array_like() {
                continue;
            }
            let mut parameter = parameter;
            let mut parameter_root = operation_root.clone();
            let mut parameter_file = operation_file.clone();
            if let Some(refstr) = parameter
                .get("$ref")
                .and_then(Node::as_str)
                .map(str::to_string)
            {
                let Some(resolved) = schema::resolve(&refstr, &document, ctx, Loader::JsonOrYaml)
                else {
                    continue;
                };
                parameter = resolved.node;
                parameter_root = resolved.root;
                parameter_file = resolved.file;
            }
            request_fields.extend(parameter_fields(
                &parameter,
                ctx,
                &parameter_root,
                &parameter_file,
                &refs,
            ));
        }
        if let Some(request_body) = operation
            .get("requestBody")
            .filter(|node| **node != Node::Null)
        {
            let mut request_body = Some(request_body.clone());
            let mut body_root = operation_root.clone();
            let mut body_file = operation_file.clone();
            if let Some(refstr) = request_body
                .as_ref()
                .and_then(|value| value.get("$ref"))
                .and_then(Node::as_str)
                .map(str::to_string)
            {
                request_body =
                    schema::resolve(&refstr, &document, ctx, Loader::JsonOrYaml).map(|resolved| {
                        body_root = resolved.root;
                        body_file = resolved.file;
                        resolved.node
                    });
            }
            if let Some(request_body) = request_body.filter(Node::is_array_like) {
                request_fields.extend(body_fields(
                    &request_body,
                    ctx,
                    &body_root,
                    &body_file,
                    &refs,
                ));
            }
        }

        let mut status = String::new();
        let mut response_schema = Node::object();
        let mut response_root = operation_root.clone();
        let mut response_file = operation_file.clone();
        if let Some(responses) = operation
            .get("responses")
            .filter(|node| node.is_array_like())
        {
            for (code, response) in schema::entries(responses) {
                if !code.starts_with('2') {
                    continue;
                }
                let mut response = Some(response);
                let mut current_root = operation_root.clone();
                let mut current_file = operation_file.clone();
                if let Some(refstr) = response
                    .as_ref()
                    .and_then(|value| value.get("$ref"))
                    .and_then(Node::as_str)
                    .map(str::to_string)
                {
                    response = schema::resolve(&refstr, &document, ctx, Loader::JsonOrYaml).map(
                        |resolved| {
                            current_root = resolved.root;
                            current_file = resolved.file;
                            resolved.node
                        },
                    );
                }
                if let Some(response) = response.filter(Node::is_array_like) {
                    status = code;
                    response_schema = response_schema_of(&response);
                    response_root = current_root;
                    response_file = current_file;
                    break;
                }
            }
        }
        if response_schema.is_array_like() && !response_schema.is_empty() {
            response_schema = schema::expand(
                &response_schema,
                &response_root,
                ctx,
                Loader::JsonOrYaml,
                Some(&response_file),
            );
        }
        let response_fields = schema::fields(&response_schema, ctx, &refs, 0);
        let request_message = message("Request", "request", request_fields);
        let response_name = if status.is_empty() {
            "Response".to_string()
        } else {
            format!("Response {status}")
        };
        let response_message = message(&response_name, "response", response_fields);
        let mut messages = vec![request_message, response_message];

        let sample = ctx.options().sample.clone();
        if !sample.is_empty() {
            let direction = if ctx.options().direction.is_empty() {
                "response"
            } else {
                ctx.options().direction.as_str()
            };
            let target = if direction == "request" { 0 } else { 1 };
            let schema = if target == 1 && !response_schema.is_empty() {
                Some(&response_schema)
            } else {
                None
            };
            let fields: Vec<Value> = messages[target]
                .get("fields")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            messages[target]["sampleValidation"] =
                sample_validation(&sample, schema, &fields, &refs, ctx);
        }

        Ok(Parsed {
            operation: if selected.operation_id.is_empty() {
                format!("{} {}", selected.method.to_uppercase(), selected.path)
            } else {
                selected.operation_id.clone()
            },
            messages,
            meta: json!({}),
        })
    }
}

fn decode(content: &str) -> Result<Node, ParseError> {
    let trimmed = content.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        parse_json(content).map_err(|message| ParseError {
            message: format!("OpenAPI JSON parse error: {message}"),
            line: 0,
            column: 0,
        })
    } else {
        yaml::parse(content).map_err(|error| ParseError {
            message: error.message,
            line: error.line,
            column: error.column,
        })
    }
}

fn collect_operations(paths: &Node, document: &Node, ctx: &mut Context<'_>) -> Vec<Operation> {
    let mut result = Vec::new();
    for (path, path_item) in schema::entries(paths) {
        if !path_item.is_array_like() {
            continue;
        }
        let mut path_item = path_item;
        let mut path_item_root = document.clone();
        let mut path_item_file = ctx.file().to_string();
        if let Some(refstr) = path_item
            .get("$ref")
            .and_then(Node::as_str)
            .map(str::to_string)
        {
            if let Some(resolved) = schema::resolve(&refstr, document, ctx, Loader::JsonOrYaml) {
                path_item = resolved.node;
                path_item_root = resolved.root;
                path_item_file = resolved.file;
            }
        }
        for method in METHODS {
            let Some(operation) = path_item
                .get(method)
                .filter(|node| node.is_array_like())
                .cloned()
            else {
                continue;
            };
            let operation_id = operation
                .get("operationId")
                .map(Node::scalar_string)
                .unwrap_or_default();
            result.push(Operation {
                path: path.clone(),
                method: method.to_string(),
                operation_id,
                operation,
                path_item: path_item.clone(),
                root: path_item_root.clone(),
                file: path_item_file.clone(),
            });
        }
    }
    result
}

fn select<'a>(operations: &'a [Operation], wanted: &str, ctx: &mut Context<'_>) -> &'a Operation {
    if !wanted.is_empty() {
        for operation in operations {
            if !operation.operation_id.is_empty() && operation.operation_id == wanted {
                return operation;
            }
            // PHP only case-folds the "METHOD path" signature, never operationId.
            if operation.signature().eq_ignore_ascii_case(wanted) {
                return operation;
            }
        }
        ctx.error(
            &format!(
                "operation not found: {wanted}; candidates: {}",
                candidates(operations)
            ),
            0,
            0,
        );
        return &operations[0];
    }
    if operations.len() == 1 {
        return &operations[0];
    }
    ctx.error(
        &format!("operation required; candidates: {}", candidates(operations)),
        0,
        0,
    );
    &operations[0]
}

fn candidates(operations: &[Operation]) -> String {
    operations
        .iter()
        .map(Operation::display_name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn parameter_fields(
    parameter: &Node,
    ctx: &mut Context<'_>,
    root: &Node,
    file: &str,
    refs: &RefCtx,
) -> Vec<Value> {
    let name = parameter
        .get("name")
        .map(Node::scalar_string)
        .unwrap_or_default();
    let in_ = parameter
        .get("in")
        .filter(|node| **node != Node::Null)
        .map(Node::scalar_string)
        .unwrap_or_else(|| "query".to_string());
    if name.is_empty() {
        return Vec::new();
    }
    let schema = parameter
        .get("schema")
        .filter(|node| node.is_array_like())
        .cloned()
        .unwrap_or_else(|| parameter.clone());
    let schema = schema::expand(&schema, root, ctx, Loader::JsonOrYaml, Some(file));
    let mut field = json!({
        "name": name,
        "type": schema::type_of(&schema),
        "require": parameter.get("required").is_some_and(Node::truthy),
        "desc": parameter.get("description").filter(|node| **node != Node::Null).map(Node::scalar_string).unwrap_or_default(),
        "in": in_,
    });
    let children = schema::fields(&schema, ctx, refs, 0);
    if in_ == "body" {
        return if children.is_empty() {
            vec![field]
        } else {
            children
        };
    }
    if !children.is_empty() {
        field["children"] = Value::Array(children);
    }
    vec![field]
}

fn body_fields(
    request_body: &Node,
    ctx: &mut Context<'_>,
    root: &Node,
    file: &str,
    refs: &RefCtx,
) -> Vec<Value> {
    let content = match request_body
        .get("content")
        .filter(|node| node.is_array_like())
    {
        Some(content) => content,
        None => return Vec::new(),
    };
    let mut schema = Node::object();
    for media_type in CONTENT_TYPES {
        if let Some(candidate) = content
            .get(media_type)
            .and_then(|item| item.get("schema"))
            .filter(|node| node.is_array_like())
        {
            schema = candidate.clone();
            break;
        }
    }
    if !schema.is_array_like() || schema.is_empty() {
        return Vec::new();
    }
    let schema = schema::expand(&schema, root, ctx, Loader::JsonOrYaml, Some(file));
    let mut fields = schema::fields(&schema, ctx, refs, 0);
    for field in &mut fields {
        if let Value::Object(map) = field {
            map.entry("in".to_string())
                .or_insert_with(|| Value::String("body".to_string()));
        }
    }
    fields
}

fn response_schema_of(response: &Node) -> Node {
    if let Some(content) = response.get("content").filter(|node| node.is_array_like()) {
        for media_type in CONTENT_TYPES {
            if let Some(schema) = content
                .get(media_type)
                .and_then(|item| item.get("schema"))
                .filter(|node| node.is_array_like())
            {
                return schema.clone();
            }
        }
    }
    response
        .get("schema")
        .filter(|node| node.is_array_like())
        .cloned()
        .unwrap_or_else(Node::object)
}
