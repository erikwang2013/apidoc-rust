//! Port of `php/src/protocol/Adapter/JsonSchemaAdapter.php`.

use serde_json::{json, Value};

use crate::protocol_defs::adapters::{refs_for, Adapter, Parsed};
use crate::protocol_defs::context::{Context, Loader};
use crate::protocol_defs::node::{parse_json, Node};
use crate::protocol_defs::schema;
use crate::protocol_defs::token::ParseError;

pub struct JsonSchemaAdapter;

impl Adapter for JsonSchemaAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed, ParseError> {
        let root = parse_json(content).map_err(|message| ParseError {
            message: format!("JSON parse error: {message}"),
            line: 0,
            column: 0,
        })?;
        if !root.is_array_like() {
            return Err(ParseError {
                message: "JSON parse error: root must be an object".to_string(),
                line: 0,
                column: 0,
            });
        }
        let message_name = ctx.options().message.clone();
        let mut schema = root.clone();
        if !message_name.is_empty() {
            let selected = root
                .get("$defs")
                .and_then(|defs| defs.get(&message_name))
                .filter(|node| node.is_array_like())
                .or_else(|| {
                    root.get("definitions")
                        .and_then(|defs| defs.get(&message_name))
                        .filter(|node| node.is_array_like())
                });
            match selected {
                Some(selected) => schema = selected.clone(),
                None => ctx.error(&format!("message not found: {message_name}"), 0, 0),
            }
        }
        let refs = refs_for(&root, Loader::Json);
        let expanded = schema::expand(&schema, &root, ctx, Loader::Json, None);
        let name = if !message_name.is_empty() {
            message_name.clone()
        } else {
            schema
                .get("title")
                .map(Node::scalar_string)
                .unwrap_or_else(|| "Body".to_string())
        };
        let direction = if ctx.options().direction.is_empty() {
            "request".to_string()
        } else {
            ctx.options().direction.clone()
        };
        let sample = ctx.options().sample.clone();
        let item = json!({
            "name": name,
            "direction": direction,
            "fields": schema::fields(&expanded, ctx, &refs, 0),
            "sample": sample,
            "sampleValidation": validate_sample(&sample, &expanded, &refs, ctx),
        });
        Ok(Parsed {
            operation: String::new(),
            messages: vec![item],
            meta: schema
                .get("x-webhook-signature")
                .filter(|node| node.is_array_like())
                .map(Node::to_json)
                .unwrap_or_else(|| json!({})),
        })
    }
}

fn validate_sample(
    sample: &str,
    schema: &Node,
    refs: &schema::RefCtx,
    ctx: &mut Context<'_>,
) -> Value {
    if sample.is_empty() {
        return json!({ "ok": true, "errors": [], "warnings": [] });
    }
    let data = match parse_json(sample) {
        Ok(data) => data,
        Err(message) => {
            return json!({
                "ok": false,
                "errors": [{
                    "path": "$",
                    "severity": "error",
                    "message": format!("JSON parse error: {message}"),
                }],
                "warnings": [],
            })
        }
    };
    let errors: Vec<Value> = schema::validate(schema, &data, refs, ctx, "$", 0)
        .into_iter()
        .map(|(path, message)| {
            json!({
                "path": path,
                "severity": "error",
                "message": message,
            })
        })
        .collect();
    json!({
        "ok": errors.is_empty(),
        "errors": errors,
        "warnings": [],
    })
}
