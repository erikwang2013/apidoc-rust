//! Port of `php/src/protocol/Adapter/AsyncApiAdapter.php`.

use serde_json::json;

use crate::protocol_defs::adapters::{message, refs_for, sample_validation, Adapter, Parsed};
use crate::protocol_defs::context::{Context, Loader};
use crate::protocol_defs::node::{parse_json, Node};
use crate::protocol_defs::schema;
use crate::protocol_defs::token::ParseError;
use crate::protocol_defs::yaml;

pub struct AsyncApiAdapter;

#[derive(Clone)]
struct Candidate {
    channel: String,
    action: String,
    operation: Node,
    root: Node,
    file: String,
}

struct Selection {
    operation: String,
    messages: Vec<Node>,
    roots: Vec<Node>,
    files: Vec<String>,
}

impl Adapter for AsyncApiAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed, ParseError> {
        let document = decode(content)?;
        let version = document
            .get("asyncapi")
            .filter(|node| **node != Node::Null)
            .map(Node::scalar_string)
            .unwrap_or_default();
        if version.is_empty() {
            ctx.error("AsyncAPI version is missing", 0, 0);
        } else if !version.starts_with("2.") && !version.starts_with("3.") {
            ctx.unsupported(&format!("unsupported AsyncAPI version: {version}"), 0, 0);
        }
        let wanted = ctx.options().operation.clone();
        let selection = if version.starts_with("3.") {
            select_v3(&document, &wanted, ctx)
        } else {
            select_v2(&document, &wanted, ctx)
        };
        let Some(selection) = selection else {
            return Ok(Parsed::empty());
        };
        let direction = if ctx.options().direction.is_empty() {
            "event".to_string()
        } else {
            ctx.options().direction.clone()
        };
        let sample = ctx.options().sample.clone();
        let refs = refs_for(&document, Loader::JsonOrYaml);
        let mut messages = Vec::new();
        for (index, message_node) in selection.messages.iter().enumerate() {
            if !message_node.is_array_like() {
                continue;
            }
            let message_root = selection
                .roots
                .get(index)
                .cloned()
                .unwrap_or_else(|| document.clone());
            let message_file = selection
                .files
                .get(index)
                .cloned()
                .unwrap_or_else(|| ctx.file().to_string());
            let payload = message_node
                .get("payload")
                .filter(|node| node.is_array_like())
                .cloned()
                .unwrap_or_else(Node::object);
            let payload = schema::expand(
                &payload,
                &message_root,
                ctx,
                Loader::JsonOrYaml,
                Some(&message_file),
            );
            let fields = schema::fields(&payload, ctx, &refs, 0);
            let name = message_node
                .get("name")
                .filter(|node| **node != Node::Null)
                .map(Node::scalar_string)
                .or_else(|| {
                    payload
                        .get("title")
                        .filter(|node| **node != Node::Null)
                        .map(Node::scalar_string)
                })
                .unwrap_or_else(|| "Message".to_string());
            let mut item = message(&name, &direction, fields);
            if !sample.is_empty() {
                item["sampleValidation"] =
                    sample_validation(&sample, Some(&payload), &[], &refs, ctx);
            }
            messages.push(item);
        }
        Ok(Parsed {
            operation: selection.operation,
            messages,
            meta: json!({}),
        })
    }
}

fn decode(content: &str) -> Result<Node, ParseError> {
    let trimmed = content.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        parse_json(content).map_err(|message| ParseError {
            message: format!("AsyncAPI JSON parse error: {message}"),
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

fn select_v2(document: &Node, wanted: &str, ctx: &mut Context<'_>) -> Option<Selection> {
    let channels = document
        .get("channels")
        .filter(|node| node.is_array_like())
        .cloned()
        .unwrap_or_else(Node::object);
    let mut candidates = Vec::new();
    for (channel_name, channel) in schema::entries(&channels) {
        if !channel.is_array_like() {
            continue;
        }
        let mut channel = channel;
        let mut root = document.clone();
        let mut file = ctx.file().to_string();
        if let Some(refstr) = channel
            .get("$ref")
            .and_then(Node::as_str)
            .map(str::to_string)
        {
            if let Some(resolved) = schema::resolve(&refstr, document, ctx, Loader::JsonOrYaml) {
                channel = resolved.node;
                root = resolved.root;
                file = resolved.file;
            }
        }
        for action in ["publish", "subscribe"] {
            if let Some(operation) = channel
                .get(action)
                .filter(|node| node.is_array_like())
                .cloned()
            {
                candidates.push(Candidate {
                    channel: channel_name.clone(),
                    action: action.to_string(),
                    operation,
                    root: root.clone(),
                    file: file.clone(),
                });
            }
        }
    }
    let candidates_names = || {
        candidates
            .iter()
            .map(|candidate| format!("{}:{}", candidate.channel, candidate.action))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !wanted.is_empty() {
        let (channel_wanted, action_wanted) = match wanted.rfind(':') {
            Some(index) => (&wanted[..index], &wanted[index + 1..]),
            None => (wanted, ""),
        };
        for candidate in &candidates {
            if candidate.channel == channel_wanted
                && (action_wanted.is_empty() || candidate.action == action_wanted)
            {
                return Some(v2_messages(candidate, document, ctx));
            }
        }
        ctx.error(
            &format!(
                "operation not found: {wanted}; candidates: {}",
                candidates_names()
            ),
            0,
            0,
        );
        return None;
    }
    if candidates.len() == 1 {
        return Some(v2_messages(&candidates[0], document, ctx));
    }
    ctx.error(
        &format!("operation required; candidates: {}", candidates_names()),
        0,
        0,
    );
    None
}

fn v2_messages(candidate: &Candidate, document: &Node, ctx: &mut Context<'_>) -> Selection {
    let operation = format!("{}:{}", candidate.channel, candidate.action);
    let mut message = candidate
        .operation
        .get("message")
        .cloned()
        .unwrap_or_else(Node::object);
    let mut message_root = candidate.root.clone();
    let mut message_file = candidate.file.clone();
    if let Some(refstr) = message
        .get("$ref")
        .and_then(Node::as_str)
        .map(str::to_string)
    {
        if let Some(resolved) = schema::resolve(&refstr, document, ctx, Loader::JsonOrYaml) {
            message = resolved.node;
            message_root = resolved.root;
            message_file = resolved.file;
        } else {
            message = Node::Null;
        }
    }
    let messages = if message.is_array_like() {
        vec![message]
    } else {
        Vec::new()
    };
    Selection {
        operation,
        messages,
        roots: vec![message_root],
        files: vec![message_file],
    }
}

fn select_v3(document: &Node, wanted: &str, ctx: &mut Context<'_>) -> Option<Selection> {
    let operations = document
        .get("operations")
        .filter(|node| node.is_array_like())
        .cloned()
        .unwrap_or_else(Node::object);
    let mut candidates = Vec::new();
    for (id, operation) in schema::entries(&operations) {
        if operation.is_array_like() {
            candidates.push((id, operation));
        }
    }
    let names = candidates
        .iter()
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>()
        .join(", ");
    if !wanted.is_empty() {
        for (id, operation) in &candidates {
            if id == wanted {
                return Some(v3_messages(id, operation, document, ctx));
            }
        }
        ctx.error(
            &format!("operation not found: {wanted}; candidates: {names}"),
            0,
            0,
        );
        return None;
    }
    if candidates.len() == 1 {
        let (id, operation) = &candidates[0];
        return Some(v3_messages(id, operation, document, ctx));
    }
    ctx.error(&format!("operation required; candidates: {names}"), 0, 0);
    None
}

fn v3_messages(id: &str, operation: &Node, document: &Node, ctx: &mut Context<'_>) -> Selection {
    let mut messages = Vec::new();
    let mut roots = Vec::new();
    let mut files = Vec::new();
    if let Some(message_refs) = operation
        .get("messages")
        .filter(|node| node.is_array_like())
    {
        for (_, message_ref) in schema::entries(message_refs) {
            let mut message_ref = Some(message_ref);
            let message_root = document.clone();
            let message_file = ctx.file().to_string();
            if let Some(refstr) = message_ref
                .as_ref()
                .and_then(|node| node.get("$ref"))
                .and_then(Node::as_str)
                .map(str::to_string)
            {
                message_ref = schema::resolve(&refstr, document, ctx, Loader::JsonOrYaml)
                    .map(|resolved| resolved.node);
            }
            if let Some(message) = message_ref.filter(Node::is_array_like) {
                messages.push(message);
                roots.push(message_root);
                files.push(message_file);
            }
        }
    }
    Selection {
        operation: id.to_string(),
        messages,
        roots,
        files,
    }
}
