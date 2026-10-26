//! Port of `php/src/protocol/Adapter/JsonRpcAdapter.php`.

use serde_json::{json, Value};

use crate::protocol_defs::adapters::{message, refs_for, Adapter, Parsed};
use crate::protocol_defs::context::{Context, Loader};
use crate::protocol_defs::node::{parse_json, Node};
use crate::protocol_defs::schema::{self, RefCtx};
use crate::protocol_defs::token::ParseError;

pub struct JsonRpcAdapter;

impl Adapter for JsonRpcAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed, ParseError> {
        let document = match parse_json(content) {
            Ok(document) if document.is_array_like() => document,
            Ok(_) => {
                return Err(ParseError {
                    message: "JSON-RPC JSON parse error: No error".to_string(),
                    line: 0,
                    column: 0,
                })
            }
            Err(message) => {
                return Err(ParseError {
                    message: format!("JSON-RPC JSON parse error: {message}"),
                    line: 0,
                    column: 0,
                })
            }
        };
        let methods = document
            .get("methods")
            .filter(|node| node.is_array_like())
            .cloned()
            .unwrap_or_else(Node::object);
        let names = methods
            .keys()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if names.is_empty() {
            ctx.error("JSON-RPC methods are missing", 0, 0);
            return Ok(Parsed::empty());
        }
        let mut wanted = ctx.options().operation.clone();
        if wanted.is_empty() {
            if names.len() == 1 {
                wanted = names[0].clone();
            } else {
                ctx.error(
                    &format!("operation required; candidates: {}", names.join(", ")),
                    0,
                    0,
                );
                return Ok(Parsed::empty());
            }
        }
        let Some(method) = methods.get(&wanted).filter(|node| node.is_array_like()) else {
            ctx.error(
                &format!(
                    "operation not found: {wanted}; candidates: {}",
                    names.join(", ")
                ),
                0,
                0,
            );
            return Ok(Parsed::empty());
        };
        let params_schema = method
            .get("params")
            .filter(|node| node.is_array_like())
            .cloned()
            .unwrap_or_else(Node::object);
        let result_schema = method
            .get("result")
            .filter(|node| node.is_array_like())
            .cloned()
            .unwrap_or_else(Node::object);
        let params_schema = schema::expand(&params_schema, &document, ctx, Loader::Json, None);
        let result_schema = schema::expand(&result_schema, &document, ctx, Loader::Json, None);
        let refs = refs_for(&document, Loader::Json);
        let mut messages = vec![
            message(
                "Params",
                "request",
                schema::fields(&params_schema, ctx, &refs, 0),
            ),
            message(
                "Result",
                "response",
                schema::fields(&result_schema, ctx, &refs, 0),
            ),
        ];
        let sample = ctx.options().sample.clone();
        if !sample.is_empty() {
            validate_sample(
                &sample,
                &wanted,
                &params_schema,
                &result_schema,
                &mut messages,
                ctx,
                &refs,
            );
        }
        Ok(Parsed {
            operation: wanted,
            messages,
            meta: json!({}),
        })
    }
}

fn validate_sample(
    sample: &str,
    method: &str,
    params_schema: &Node,
    result_schema: &Node,
    messages: &mut [Value],
    ctx: &mut Context<'_>,
    refs: &RefCtx,
) {
    let data = match parse_json(sample) {
        Ok(data) if data.is_array_like() => data,
        Ok(_) => {
            messages[0]["sampleValidation"] = json!({
                "ok": false,
                "errors": [{
                    "path": "$",
                    "severity": "error",
                    "message": "JSON sample parse error: No error",
                }],
                "warnings": [],
            });
            return;
        }
        Err(message) => {
            messages[0]["sampleValidation"] = json!({
                "ok": false,
                "errors": [{
                    "path": "$",
                    "severity": "error",
                    "message": format!("JSON sample parse error: {message}"),
                }],
                "warnings": [],
            });
            return;
        }
    };
    let mut errors = Vec::new();
    let jsonrpc = data
        .get("jsonrpc")
        .filter(|node| **node != Node::Null)
        .cloned()
        .unwrap_or_else(|| Node::Str(String::new()));
    if !jsonrpc.strict_eq(&Node::Str("2.0".to_string())) {
        errors.push(json!({
            "path": "$.jsonrpc",
            "severity": "error",
            "message": "jsonrpc must be 2.0",
        }));
    }
    if let Some(actual) = data.get("method").filter(|node| **node != Node::Null) {
        if !actual.strict_eq(&Node::Str(method.to_string())) {
            errors.push(json!({
                "path": "$.method",
                "severity": "error",
                "message": "method mismatch",
            }));
        }
    }
    if data.has("params") {
        for (path, message) in schema::validate(
            params_schema,
            data.get("params").unwrap(),
            refs,
            ctx,
            "$",
            0,
        ) {
            errors.push(json!({
                "path": nested_path(&path),
                "severity": "error",
                "message": message,
            }));
        }
        messages[0]["sampleValidation"] = json!({
            "ok": errors.is_empty(),
            "errors": errors,
            "warnings": [],
        });
    }
    if data.has("result") {
        let mut result_errors = Vec::new();
        for (path, message) in schema::validate(
            result_schema,
            data.get("result").unwrap(),
            refs,
            ctx,
            "$",
            0,
        ) {
            result_errors.push(json!({
                "path": nested_path(&path),
                "severity": "error",
                "message": message,
            }));
        }
        messages[1]["sampleValidation"] = json!({
            "ok": result_errors.is_empty(),
            "errors": result_errors,
            "warnings": [],
        });
    }
}

/// `'$' . substr($path, 1)`
fn nested_path(path: &str) -> String {
    let rest: String = path.chars().skip(1).collect();
    format!("${rest}")
}
