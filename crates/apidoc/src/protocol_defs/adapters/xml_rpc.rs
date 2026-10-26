//! Port of `php/src/protocol/Adapter/XmlRpcAdapter.php`.

use serde_json::{json, Value};

use crate::protocol_defs::adapters::{message, Adapter, Parsed};
use crate::protocol_defs::context::Context;
use crate::protocol_defs::node::Node;
use crate::protocol_defs::token::ParseError;
use crate::protocol_defs::xml::{self, XmlNode};

const TYPES: [(&str, &str); 10] = [
    ("int", "integer"),
    ("i4", "integer"),
    ("i8", "integer"),
    ("double", "number"),
    ("boolean", "boolean"),
    ("string", "string"),
    ("dateTime.iso8601", "string"),
    ("base64", "string"),
    ("struct", "object"),
    ("array", "array"),
];

pub struct XmlRpcAdapter;

impl Adapter for XmlRpcAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed, ParseError> {
        let document = xml::load(content).map_err(|error| ParseError {
            message: error.message,
            line: error.line,
            column: error.column,
        })?;
        let mut methods = Node::object();
        for method in descendants(&document.root, "method") {
            let name = method.attr("name").to_string();
            let mut params = Node::array();
            for param in descendants(method, "param") {
                let field_type = param.attr("type");
                let mut field = Node::object();
                let param_name = param.attr("name");
                field.set(
                    "name",
                    Node::Str(if param_name.is_empty() {
                        format!("param{}", params.len() + 1)
                    } else {
                        param_name.to_string()
                    }),
                );
                field.set("type", Node::Str(type_of(field_type)));
                field.set("require", Node::Bool(true));
                if field_type == "struct" {
                    let mut children = Node::array();
                    for member in param.all("member") {
                        let mut child = Node::object();
                        child.set("name", Node::Str(member.attr("name").to_string()));
                        child.set("type", Node::Str(type_of(member.attr("type"))));
                        children.push(child);
                    }
                    field.set("children", children);
                }
                params.push(field);
            }
            let result_type = method
                .first("result")
                .map(|result| type_of(result.attr("type")))
                .unwrap_or_else(|| "string".to_string());
            let mut entry = Node::object();
            entry.set("params", params);
            entry.set("result", Node::Str(result_type));
            methods.set(name, entry);
        }
        let names = methods
            .keys()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
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
        let Some(method) = methods.get(&wanted) else {
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
        let params = method.get("params").cloned().unwrap_or_else(Node::array);
        let result_type = method
            .get("result")
            .map(Node::scalar_string)
            .unwrap_or_else(|| "string".to_string());
        let mut result_field = Node::object();
        result_field.set("name", Node::Str("result".to_string()));
        result_field.set("type", Node::Str(result_type));
        result_field.set("require", Node::Bool(true));
        let mut messages = vec![
            message(
                "Params",
                "request",
                params
                    .as_array()
                    .unwrap_or(&[])
                    .iter()
                    .map(Node::to_json)
                    .collect(),
            ),
            message("Result", "response", vec![result_field.to_json()]),
        ];
        let sample = ctx.options().sample.clone();
        if !sample.is_empty() {
            messages[0]["sampleValidation"] = validate_sample(&sample, &wanted, &params);
        }
        Ok(Parsed {
            operation: wanted,
            messages,
            meta: json!({}),
        })
    }
}

fn descendants<'a>(node: &'a XmlNode, local: &str) -> Vec<&'a XmlNode> {
    let mut result = Vec::new();
    for child in &node.children {
        if child.name == local {
            result.push(child);
        }
        result.extend(descendants(child, local));
    }
    result
}

fn type_of(field_type: &str) -> String {
    TYPES
        .iter()
        .find(|(name, _)| *name == field_type)
        .map_or_else(|| "string".to_string(), |(_, kind)| (*kind).to_string())
}

fn validate_sample(sample: &str, method: &str, params: &Node) -> Value {
    let error = |message: &str| {
        json!({
            "ok": false,
            "errors": [{
                "path": "$",
                "severity": "error",
                "message": message,
            }],
            "warnings": [],
        })
    };
    let document = match xml::load(sample) {
        Ok(document) => document,
        Err(err) => return error(&err.message),
    };
    let root = &document.root;
    if root.name != "methodCall" && root.name != "methodResponse" {
        return error("root must be methodCall or methodResponse");
    }
    if root.name == "methodResponse" {
        if root.first("params").is_none() && root.first("fault").is_none() {
            return error("methodResponse requires params or fault");
        }
        return json!({ "ok": true, "errors": [], "warnings": [] });
    }
    let matches = root
        .first("methodName")
        .is_some_and(|node| php_trim(&node.text_content()) == method);
    if !matches {
        return error("methodName mismatch");
    }
    let actual = root
        .first("params")
        .map(|node| node.all("param").len())
        .unwrap_or(0);
    let expected = params.as_array().map_or(0, |fields| fields.len());
    if actual != expected {
        return error("param count mismatch");
    }
    json!({ "ok": true, "errors": [], "warnings": [] })
}

fn php_trim(value: &str) -> &str {
    value.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\0' | '\u{0B}'))
}
