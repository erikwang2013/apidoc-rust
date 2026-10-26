//! Port of `php/src/protocol/Adapter/ProtoAdapter.php`.

use serde_json::{json, Value};

use crate::protocol_defs::adapters::{Adapter, Parsed};
use crate::protocol_defs::context::Context;
use crate::protocol_defs::node::Node;
use crate::protocol_defs::schema;
use crate::protocol_defs::token::{ParseError, Result, TokenStream};

const SCALARS: [(&str, &str); 15] = [
    ("double", "number"),
    ("float", "number"),
    ("int32", "integer"),
    ("int64", "integer"),
    ("uint32", "integer"),
    ("uint64", "integer"),
    ("sint32", "integer"),
    ("sint64", "integer"),
    ("fixed32", "integer"),
    ("fixed64", "integer"),
    ("sfixed32", "integer"),
    ("sfixed64", "integer"),
    ("bool", "boolean"),
    ("string", "string"),
    ("bytes", "string"),
];

pub struct ProtoAdapter;

#[derive(Clone)]
struct RpcItem {
    service: String,
    operation: String,
    rpc: Node,
}

impl Adapter for ProtoAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed> {
        let symbols = parse_document(ctx, content, 0)?;
        let package = symbols
            .get("package")
            .map(Node::scalar_string)
            .unwrap_or_default();
        let mut rpcs = Vec::new();
        if let Some(services) = symbols.get("services").filter(|node| node.is_object()) {
            for (service_name, service) in schema::entries(services) {
                let service_package = service
                    .get("package")
                    .map(Node::scalar_string)
                    .unwrap_or_default();
                if let Some(entries) = service.get("rpcs").and_then(Node::as_array) {
                    for rpc in entries {
                        let rpc_name = rpc.get("name").map(Node::scalar_string).unwrap_or_default();
                        rpcs.push(RpcItem {
                            service: service_name.clone(),
                            operation: format!(
                                "{}{service_name}/{rpc_name}",
                                if service_package.is_empty() {
                                    String::new()
                                } else {
                                    format!("{service_package}.")
                                }
                            ),
                            rpc: rpc.clone(),
                        });
                    }
                }
            }
        }
        let wanted = ctx.options().operation.clone();
        let Some(selected) = select(&rpcs, &wanted, ctx) else {
            return Ok(Parsed::empty());
        };
        let rpc = &selected.rpc;
        let request_name = rpc
            .get("request")
            .map(Node::scalar_string)
            .unwrap_or_default();
        let response_name = rpc
            .get("response")
            .map(Node::scalar_string)
            .unwrap_or_default();
        let request = message_for_type(&request_name, &symbols, ctx, &[]);
        let response = message_for_type(&response_name, &symbols, ctx, &[]);
        let mut request_message = json!({
            "name": request_name,
            "direction": "request",
            "fields": request,
            "clientStream": rpc.get("client_stream").is_some_and(Node::truthy),
        });
        let response_message = json!({
            "name": response_name,
            "direction": "response",
            "fields": response,
            "serverStream": rpc.get("server_stream").is_some_and(Node::truthy),
        });
        if !ctx.options().sample.is_empty() {
            ctx.warning("protobuf sample validation is planned for P3", 0, 0);
        }
        let _ = &mut request_message;
        Ok(Parsed {
            operation: selected.operation.clone(),
            messages: vec![request_message, response_message],
            meta: json!({ "package": package }),
        })
    }
}

fn parse_document(ctx: &mut Context<'_>, content: &str, depth: usize) -> Result<Node> {
    if depth > 32 {
        return Err(ParseError {
            message: "proto nesting limit reached".to_string(),
            line: 0,
            column: 0,
        });
    }
    let mut symbols = new_symbols();
    let mut ts = TokenStream::new(content);
    while !ts.eof() {
        let line = ts.line();
        let column = ts.column();
        let keyword = ts.identifier()?;
        match keyword.as_str() {
            "syntax" => {
                ts.expect('=')?;
                ts.string()?;
                ts.expect(';')?;
            }
            "package" => {
                let package = ts.identifier()?;
                symbols.set("package", Node::Str(package));
                ts.expect(';')?;
            }
            "import" => {
                let import_line = ts.line();
                let next = ts.peek(0);
                if next == "p" || next == "w" {
                    ts.identifier()?;
                }
                let file = ts.string()?;
                ts.expect(';')?;
                import(ctx, &mut symbols, &file, import_line, depth)?;
            }
            "option" => skip_to_semicolon(&mut ts),
            "message" => parse_message(&mut ts, &mut symbols, ctx, depth + 1)?,
            "enum" => parse_enum(&mut ts, &mut symbols)?,
            "service" => {
                let package = symbols
                    .get("package")
                    .map(Node::scalar_string)
                    .unwrap_or_default();
                parse_service(&mut ts, &mut symbols, ctx, &package)?;
            }
            "extend" | "group" | "edition" => {
                ctx.unsupported(&format!("unsupported construct: {keyword}"), line, column);
                skip_definition(&mut ts);
            }
            "reserved" => skip_to_semicolon(&mut ts),
            _ => {
                let _ = line;
                let _ = column;
                ctx.warning(&format!("unsupported proto definition: {keyword}"), 0, 0);
                skip_definition(&mut ts);
            }
        }
    }
    alias_symbols(&mut symbols);
    Ok(symbols)
}

fn new_symbols() -> Node {
    let mut symbols = Node::object();
    symbols.set("messages", Node::object());
    symbols.set("enums", Node::object());
    symbols.set("services", Node::object());
    symbols.set("package", Node::Str(String::new()));
    symbols
}

fn import(
    ctx: &mut Context<'_>,
    symbols: &mut Node,
    file: &str,
    line: i64,
    depth: usize,
) -> Result<()> {
    let Some(realpath) = ctx.resolve(file) else {
        ctx.error(&format!("unable to resolve import: {file}"), line, 0);
        return Ok(());
    };
    if !ctx.push(realpath) {
        return Ok(());
    }
    let content = ctx.read();
    if let Some(content) = content {
        let imported = parse_document(ctx, &content, depth + 1)?;
        for kind in ["messages", "enums", "services"] {
            let base = symbols.get(kind).cloned().unwrap_or_else(Node::object);
            let overlay = imported.get(kind).cloned().unwrap_or_else(Node::object);
            symbols.set(kind, Node::merge(&base, &overlay));
        }
    }
    ctx.pop();
    Ok(())
}

fn alias_symbols(symbols: &mut Node) {
    let package = symbols
        .get("package")
        .map(Node::scalar_string)
        .unwrap_or_default();
    if package.is_empty() {
        return;
    }
    for kind in ["messages", "enums", "services"] {
        let entries = match symbols.get(kind).and_then(Node::as_object) {
            Some(entries) => entries.to_vec(),
            None => continue,
        };
        for (name, value) in entries {
            if let Some(target) = symbols.get_mut(kind) {
                target.set(format!("{package}.{name}"), value);
            }
        }
    }
}

fn parse_message(
    ts: &mut TokenStream,
    symbols: &mut Node,
    ctx: &mut Context<'_>,
    depth: usize,
) -> Result<()> {
    if depth > 128 {
        return Err(ts.error("proto nesting limit reached"));
    }
    let name = ts.identifier()?;
    ts.expect('{')?;
    let mut fields = Node::array();
    while !ts.eof() && ts.peek(0) != "}" {
        let word = ts.peek(0);
        let first = word.chars().next();
        if first.is_some_and(|ch| ch.is_ascii_alphabetic()) || word == "_" {
            let token = ts.identifier()?;
            if token == "message" {
                parse_message(ts, symbols, ctx, depth + 1)?;
                continue;
            }
            if token == "enum" {
                parse_enum(ts, symbols)?;
                continue;
            }
            if token == "oneof" {
                let oneof_name = ts.identifier()?;
                ts.expect('{')?;
                while !ts.eof() && ts.peek(0) != "}" {
                    if let Some(field) = parse_field(ts, true, &oneof_name, depth)? {
                        fields.push(field);
                    }
                }
                ts.expect('}')?;
                continue;
            }
            if ["option", "reserved", "extensions"].contains(&token.as_str()) {
                skip_to_semicolon(ts);
                continue;
            }
            let field = if ["repeated", "optional", "required"].contains(&token.as_str()) {
                parse_field_after_label(ts, "", false, "", &token, depth)?
            } else if token == "map" {
                parse_field_after_label(ts, "map", false, "", "", depth)?
            } else {
                parse_field_after_label(ts, &token, false, "", "", depth)?
            };
            if let Some(field) = field {
                fields.push(field);
            }
            continue;
        }
        ts.next_char();
    }
    ts.expect('}')?;
    let mut entry = Node::object();
    entry.set("name", Node::Str(name.clone()));
    entry.set("fields", fields);
    if let Some(messages) = symbols.get_mut("messages") {
        messages.set(name, entry);
    }
    Ok(())
}

fn parse_field(
    ts: &mut TokenStream,
    in_oneof: bool,
    oneof: &str,
    depth: usize,
) -> Result<Option<Node>> {
    let mut label = String::new();
    let peek = ts.peek(0);
    let first = peek.chars().next();
    if first.is_some_and(|ch| ch.is_ascii_alphabetic()) || peek == "_" {
        let word = ts.identifier()?;
        if ["repeated", "optional", "required"].contains(&word.as_str()) {
            label = word;
        } else {
            return parse_field_after_label(ts, &word, in_oneof, oneof, "", depth);
        }
    }
    parse_field_after_label(ts, "", in_oneof, oneof, &label, depth)
}

fn parse_field_after_label(
    ts: &mut TokenStream,
    first_type: &str,
    in_oneof: bool,
    oneof: &str,
    label: &str,
    depth: usize,
) -> Result<Option<Node>> {
    if depth > 128 {
        return Err(ts.error("proto nesting limit reached"));
    }
    let mut is_map = false;
    let mut map_key = String::new();
    let mut map_value = String::new();
    let field_type = if first_type == "map" {
        is_map = true;
        ts.expect('<')?;
        map_key = ts.identifier()?;
        ts.expect(',')?;
        map_value = ts.identifier()?;
        ts.expect('>')?;
        "map".to_string()
    } else if !first_type.is_empty() {
        first_type.to_string()
    } else {
        ts.identifier()?
    };
    let name = ts.identifier()?;
    let mut field_number = 0i64;
    if ts.consume("=").is_some() {
        field_number = ts
            .number()?
            .parse::<f64>()
            .map(|value| value as i64)
            .unwrap_or(0);
    }
    if ts.consume("[").is_some() {
        skip_balanced(ts, '[', ']');
    }
    ts.expect(';')?;
    let mut field = Node::object();
    field.set("name", Node::Str(name));
    field.set("type", Node::Str(type_for(&field_type)));
    field.set("require", Node::Bool(label == "required"));
    field.set("fieldNumber", Node::Int(field_number));
    if !is_scalar(&field_type) && field_type != "map" {
        field.set("typeName", Node::Str(field_type.clone()));
    }
    if field_type == "bytes" {
        field.set("format", Node::Str("bytes".to_string()));
    }
    if label == "repeated" {
        field.set("repeated", Node::Bool(true));
        field.set("childrenType", Node::Str(type_for(&field_type)));
        field.set("type", Node::Str("array".to_string()));
    }
    if is_map {
        field.set("type", Node::Str("object".to_string()));
        field.set("map", Node::Bool(true));
        let mut children = Node::array();
        for (child_name, child_type) in [("key", map_key), ("value", map_value)] {
            let mut child = Node::object();
            child.set("name", Node::Str(child_name.to_string()));
            child.set("type", Node::Str(type_for(&child_type)));
            child.set("typeName", Node::Str(child_type));
            children.push(child);
        }
        field.set("children", children);
    }
    if in_oneof {
        field.set("oneof", Node::Str(oneof.to_string()));
    }
    Ok(Some(field))
}

fn type_for(field_type: &str) -> String {
    SCALARS
        .iter()
        .find(|(name, _)| *name == field_type)
        .map_or_else(|| "object".to_string(), |(_, kind)| (*kind).to_string())
}

fn is_scalar(field_type: &str) -> bool {
    SCALARS.iter().any(|(name, _)| *name == field_type)
}

fn parse_enum(ts: &mut TokenStream, symbols: &mut Node) -> Result<()> {
    let name = ts.identifier()?;
    let mut values = Node::array();
    ts.expect('{')?;
    while !ts.eof() && ts.peek(0) != "}" {
        if ts.consume(";").is_some() {
            continue;
        }
        let value = ts.identifier()?;
        if ts.consume("=").is_some() {
            ts.number()?;
            if ts.consume("[").is_some() {
                skip_balanced(ts, '[', ']');
            }
        }
        ts.consume(";");
        values.push(Node::Str(value));
    }
    ts.expect('}')?;
    if let Some(enums) = symbols.get_mut("enums") {
        enums.set(name, values);
    }
    Ok(())
}

fn parse_service(
    ts: &mut TokenStream,
    symbols: &mut Node,
    ctx: &mut Context<'_>,
    package: &str,
) -> Result<()> {
    let name = ts.identifier()?;
    let mut rpcs = Node::array();
    ts.expect('{')?;
    while !ts.eof() && ts.peek(0) != "}" {
        let line = ts.line();
        let column = ts.column();
        let keyword = ts.identifier()?;
        if keyword == "option" {
            skip_to_semicolon(ts);
            continue;
        }
        if keyword != "rpc" {
            ctx.warning(
                &format!("unsupported service member: {keyword}"),
                line,
                column,
            );
            skip_definition(ts);
            continue;
        }
        let rpc_name = ts.identifier()?;
        ts.expect('(')?;
        let mut client_stream = false;
        if ts.peek_identifier() == "stream" {
            ts.identifier()?;
            client_stream = true;
        }
        let request = ts.identifier()?;
        ts.expect(')')?;
        let returns = ts.identifier()?;
        if returns != "returns" {
            ctx.error("expected returns", line, column);
            skip_definition(ts);
            continue;
        }
        ts.expect('(')?;
        let mut server_stream = false;
        if ts.peek_identifier() == "stream" {
            ts.identifier()?;
            server_stream = true;
        }
        let response = ts.identifier()?;
        ts.expect(')')?;
        if ts.consume("{").is_some() {
            skip_balanced(ts, '{', '}');
        }
        ts.consume(";");
        let mut rpc = Node::object();
        rpc.set("name", Node::Str(rpc_name));
        rpc.set("request", Node::Str(request));
        rpc.set("response", Node::Str(response));
        rpc.set("client_stream", Node::Bool(client_stream));
        rpc.set("server_stream", Node::Bool(server_stream));
        rpcs.push(rpc);
    }
    ts.expect('}')?;
    let mut service = Node::object();
    service.set("name", Node::Str(name.clone()));
    service.set("package", Node::Str(package.to_string()));
    service.set("rpcs", rpcs);
    if let Some(services) = symbols.get_mut("services") {
        services.set(name, service);
    }
    Ok(())
}

fn select<'a>(rpcs: &'a [RpcItem], wanted: &str, ctx: &mut Context<'_>) -> Option<&'a RpcItem> {
    if !wanted.is_empty() {
        for item in rpcs {
            let rpc_name = item
                .rpc
                .get("name")
                .map(Node::scalar_string)
                .unwrap_or_default();
            if item.operation == wanted || format!("{}/{rpc_name}", item.service) == wanted {
                return Some(item);
            }
        }
        ctx.error(
            &format!(
                "operation not found: {wanted}; candidates: {}",
                candidates(rpcs)
            ),
            0,
            0,
        );
        return None;
    }
    if rpcs.len() == 1 {
        return Some(&rpcs[0]);
    }
    ctx.error(
        &format!("operation required; candidates: {}", candidates(rpcs)),
        0,
        0,
    );
    None
}

fn candidates(rpcs: &[RpcItem]) -> String {
    rpcs.iter()
        .map(|item| item.operation.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn message_for_type(
    field_type: &str,
    symbols: &Node,
    ctx: &mut Context<'_>,
    visited: &[String],
) -> Value {
    if let Some(message) = find_message(field_type, symbols) {
        if visited.iter().any(|entry| entry == field_type) {
            ctx.warning(&format!("proto message cycle detected: {field_type}"), 0, 0);
            return Value::Array(Vec::new());
        }
        let mut next_visited = visited.to_vec();
        next_visited.push(field_type.to_string());
        let mut fields = Vec::new();
        for (_, field) in
            schema::entries(&message.get("fields").cloned().unwrap_or_else(Node::array))
        {
            fields.push(map_field(&field, symbols, ctx, &next_visited));
        }
        return Value::Array(fields);
    }
    if let Some(values) = find_enum(field_type, symbols) {
        let mut item = serde_json::Map::new();
        item.insert("name".to_string(), Value::String("value".to_string()));
        item.insert("type".to_string(), Value::String("string".to_string()));
        item.insert("enum".to_string(), values.to_json());
        return Value::Array(vec![Value::Object(item)]);
    }
    ctx.warning(&format!("unresolved proto type: {field_type}"), 0, 0);
    Value::Array(Vec::new())
}

fn map_field(field: &Node, symbols: &Node, ctx: &mut Context<'_>, visited: &[String]) -> Value {
    let mut result = serde_json::Map::new();
    result.insert(
        "name".to_string(),
        Value::String(
            field
                .get("name")
                .map(Node::scalar_string)
                .unwrap_or_default(),
        ),
    );
    result.insert(
        "type".to_string(),
        Value::String(
            field
                .get("type")
                .map(Node::scalar_string)
                .unwrap_or_default(),
        ),
    );
    result.insert(
        "require".to_string(),
        Value::Bool(field.get("require").is_some_and(Node::truthy)),
    );
    result.insert(
        "fieldNumber".to_string(),
        Value::Number(
            field
                .get("fieldNumber")
                .and_then(|node| match node {
                    Node::Int(value) => Some(*value),
                    Node::Float(value) => Some(*value as i64),
                    _ => None,
                })
                .unwrap_or(0)
                .into(),
        ),
    );
    if field.get("oneof").is_some_and(Node::truthy) {
        result.insert(
            "oneof".to_string(),
            Value::String(
                field
                    .get("oneof")
                    .map(Node::scalar_string)
                    .unwrap_or_default(),
            ),
        );
    }
    if field.get("repeated").is_some_and(Node::truthy) {
        result.insert("repeated".to_string(), Value::Bool(true));
        result.insert("type".to_string(), Value::String("array".to_string()));
    }
    if field.get("map").is_some_and(Node::truthy) {
        result.insert("map".to_string(), Value::Bool(true));
        result.insert("type".to_string(), Value::String("object".to_string()));
        result.insert(
            "children".to_string(),
            field
                .get("children")
                .cloned()
                .unwrap_or_else(Node::array)
                .to_json(),
        );
        return Value::Object(result);
    }
    let type_name = field
        .get("typeName")
        .map(Node::scalar_string)
        .unwrap_or_default();
    let repeated = field.get("repeated").is_some_and(Node::truthy);
    if !type_name.is_empty() {
        if find_message(&type_name, symbols).is_some() {
            let children = message_for_type(&type_name, symbols, ctx, visited);
            if repeated {
                result.insert("children".to_string(), children);
                result.insert(
                    "childrenType".to_string(),
                    Value::String("object".to_string()),
                );
            } else {
                result.insert("type".to_string(), Value::String("object".to_string()));
                result.insert("ref".to_string(), Value::String(type_name));
                result.insert("children".to_string(), children);
            }
            return Value::Object(result);
        }
        if let Some(values) = find_enum(&type_name, symbols) {
            result.insert("type".to_string(), Value::String("string".to_string()));
            result.insert("enum".to_string(), values.to_json());
            return Value::Object(result);
        }
        ctx.warning(&format!("unresolved proto type: {type_name}"), 0, 0);
        result.insert("type".to_string(), Value::String("object".to_string()));
        result.insert("ref".to_string(), Value::String(type_name));
        return Value::Object(result);
    }
    if repeated {
        result.insert(
            "childrenType".to_string(),
            field
                .get("childrenType")
                .cloned()
                .map(|node| Value::String(node.scalar_string()))
                .unwrap_or_else(|| Value::String("string".to_string())),
        );
    }
    Value::Object(result)
}

fn find_message<'a>(field_type: &str, symbols: &'a Node) -> Option<&'a Node> {
    let messages = symbols.get("messages")?;
    messages
        .get(field_type)
        .or_else(|| messages.get(last_segment(field_type)))
}

fn find_enum<'a>(field_type: &str, symbols: &'a Node) -> Option<&'a Node> {
    let enums = symbols.get("enums")?;
    enums
        .get(field_type)
        .or_else(|| enums.get(last_segment(field_type)))
}

fn last_segment(field_type: &str) -> &str {
    field_type
        .rfind('.')
        .map_or(field_type, |index| &field_type[index + 1..])
}

fn skip_to_semicolon(ts: &mut TokenStream) {
    while !ts.eof() {
        let ch = ts.peek(0);
        if ch == ";" {
            ts.next_char();
            return;
        }
        if ch == "{" {
            ts.next_char();
            skip_balanced(ts, '{', '}');
            continue;
        }
        if ch == "[" {
            ts.next_char();
            skip_balanced(ts, '[', ']');
            continue;
        }
        ts.next_char();
    }
}

fn skip_definition(ts: &mut TokenStream) {
    while !ts.eof() {
        let ch = ts.peek(0);
        if ch == "{" {
            ts.next_char();
            skip_balanced(ts, '{', '}');
            return;
        }
        if ch == ";" {
            ts.next_char();
            return;
        }
        ts.next_char();
    }
}

fn skip_balanced(ts: &mut TokenStream, open: char, close: char) {
    let mut depth = 1;
    while !ts.eof() && depth > 0 {
        let ch = ts.next_char();
        if ch == open.to_string() {
            depth += 1;
        } else if ch == close.to_string() {
            depth -= 1;
        }
    }
}
