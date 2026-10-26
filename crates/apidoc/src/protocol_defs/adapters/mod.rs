//! One adapter per external definition format, ported from the PHP reference.

pub mod asyncapi;
pub mod edmx;
pub mod graphql;
pub mod json_rpc;
pub mod json_schema;
pub mod openapi;
pub mod proto;
pub mod wsdl;
pub mod xml_rpc;

use serde_json::{json, Value};

use crate::protocol_defs::context::{Context, Loader};
use crate::protocol_defs::node::{parse_json, Node};
use crate::protocol_defs::schema::{self, RefCtx};
use crate::protocol_defs::token::ParseError;

pub struct Parsed {
    pub operation: String,
    pub messages: Vec<Value>,
    pub meta: Value,
}

impl Parsed {
    pub fn empty() -> Self {
        Self {
            operation: String::new(),
            messages: Vec::new(),
            meta: json!({}),
        }
    }
}

pub trait Adapter: Sync {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed, ParseError>;
}

pub fn message(name: &str, direction: &str, fields: Vec<Value>) -> Value {
    json!({
        "name": name,
        "direction": direction,
        "fields": fields,
    })
}

/// Loader used by OpenAPI/AsyncAPI (JSON when the content starts with `{`/`[`,
/// YAML otherwise) — same rule as the PHP adapters.
pub fn structured_loader(content: &str) -> Result<Node, ParseError> {
    let trimmed = content.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        parse_json(content).map_err(|message| ParseError {
            message,
            line: 0,
            column: 0,
        })
    } else {
        crate::protocol_defs::yaml::parse(content).map_err(|error| ParseError {
            message: error.message,
            line: error.line,
            column: error.column,
        })
    }
}

pub fn refs_for(document: &Node, loader: Loader) -> RefCtx {
    RefCtx::new(document.clone(), None, loader)
}

pub fn sample_validation(
    sample: &str,
    schema: Option<&Node>,
    fields: &[Value],
    refs: &RefCtx,
    ctx: &mut Context<'_>,
) -> Value {
    let data = match parse_json(sample) {
        Ok(data) => data,
        Err(message) => {
            return json!({
                "ok": false,
                "errors": [{
                    "path": "$",
                    "severity": "error",
                    "message": format!("JSON sample parse error: {message}"),
                }],
                "warnings": [],
            })
        }
    };
    if let Some(schema) = schema {
        let errors = schema::validate(schema, &data, refs, ctx, "$", 0);
        let errors: Vec<Value> = errors
            .into_iter()
            .map(|(path, message)| {
                json!({
                    "path": path,
                    "severity": "error",
                    "message": message,
                })
            })
            .collect();
        return json!({
            "ok": errors.is_empty(),
            "errors": errors,
            "warnings": [],
        });
    }
    let mut errors = Vec::new();
    if data.is_array_like() {
        for field in fields {
            let require = field.get("require").and_then(Value::as_bool) == Some(true);
            let name = field.get("name").and_then(Value::as_str).unwrap_or("");
            if require && data.get(name).is_none() {
                errors.push(json!({
                    "path": format!("$.{name}"),
                    "severity": "error",
                    "message": "required field missing",
                }));
            }
        }
    }
    json!({
        "ok": errors.is_empty(),
        "errors": errors,
        "warnings": [],
    })
}

/// `strpos($version, $prefix) === 0`
pub fn starts_with_bytes(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
}
