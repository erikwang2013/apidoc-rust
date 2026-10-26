//! Port of `php/src/protocol/Adapter/GraphqlAdapter.php`.

use serde_json::{json, Value};

use crate::protocol_defs::adapters::{message, Adapter, Parsed};
use crate::protocol_defs::context::Context;
use crate::protocol_defs::node::{parse_json, Node};
use crate::protocol_defs::schema;
use crate::protocol_defs::token::{Result, TokenStream};

const DEFINITIONS: [&str; 9] = [
    "schema",
    "type",
    "input",
    "interface",
    "enum",
    "scalar",
    "union",
    "extend",
    "directive",
];

const SCALARS: [(&str, &str); 5] = [
    ("String", "string"),
    ("ID", "string"),
    ("Int", "integer"),
    ("Float", "number"),
    ("Boolean", "boolean"),
];

pub struct GraphqlAdapter;

impl Adapter for GraphqlAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed> {
        let mut types = Node::object();
        parse_document(ctx, content, &mut types)?;
        let query_type = types
            .get("__schema")
            .and_then(|schema| schema.get("query"))
            .map(Node::scalar_string)
            .unwrap_or_else(|| "Query".to_string());
        let query_fields = types
            .get(&query_type)
            .and_then(|kind| kind.get("fields"))
            .filter(|node| node.is_array_like())
            .cloned()
            .unwrap_or_else(Node::object);
        let wanted = ctx.options().operation.clone();
        let Some((type_name, field_name, field)) =
            select(&types, &query_type, &query_fields, &wanted, ctx)
        else {
            return Ok(Parsed::empty());
        };
        let mut request = Vec::new();
        if let Some(args) = field.get("args").and_then(Node::as_array) {
            for arg in args {
                let Some(type_ref) = arg.get("type") else {
                    continue;
                };
                let mut descriptor = describe_type(type_ref, &types, &[], 0);
                descriptor.set(
                    "name",
                    Node::Str(arg.get("name").map(Node::scalar_string).unwrap_or_default()),
                );
                descriptor.set(
                    "require",
                    Node::Bool(type_ref.get("require").is_some_and(Node::truthy)),
                );
                request.push(descriptor.to_json());
            }
        }
        let response = field
            .get("type")
            .map(|type_ref| response_fields(type_ref, &types))
            .unwrap_or_else(Node::array)
            .to_json();
        let response = match response {
            Value::Array(items) => items,
            _ => Vec::new(),
        };
        let mut messages = vec![
            message("Request", "request", request),
            message("Response", "response", response),
        ];
        let sample = ctx.options().sample.clone();
        if !sample.is_empty() {
            let target = if ctx.options().direction == "request" {
                0
            } else {
                1
            };
            let fields = messages[target]
                .get("fields")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            messages[target]["sampleValidation"] = validate_sample(&sample, &fields);
        }
        Ok(Parsed {
            operation: format!("{type_name}.{field_name}"),
            messages,
            meta: json!({}),
        })
    }
}

fn parse_document(ctx: &mut Context<'_>, content: &str, types: &mut Node) -> Result<()> {
    for import in imports(content) {
        let Some(realpath) = ctx.resolve(&import) else {
            ctx.warning(&format!("unable to resolve GraphQL import: {import}"), 0, 0);
            continue;
        };
        if !ctx.push(realpath) {
            continue;
        }
        let imported = ctx.read();
        if let Some(imported) = imported {
            let mut imported_types = Node::object();
            parse_document(ctx, &imported, &mut imported_types)?;
            if let Node::Object(entries) = imported_types {
                for (name, value) in entries {
                    types.set(name, value);
                }
            }
        }
        ctx.pop();
    }
    let mut parser = Parser {
        ts: TokenStream::new(content),
    };
    while !parser.ts.eof() {
        skip_description(&mut parser.ts);
        if parser.ts.eof() {
            break;
        }
        let keyword = parser.ts.identifier()?;
        match keyword.as_str() {
            "schema" => parse_schema(&mut parser.ts, types)?,
            "type" | "interface" | "input" => {
                parse_type_definition(&mut parser.ts, types, &keyword, false)?
            }
            "enum" => parse_enum(&mut parser.ts, types)?,
            "scalar" => {
                let name = parser.ts.identifier()?;
                let mut entry = Node::object();
                entry.set("kind", Node::Str("scalar".to_string()));
                types.set(name, entry);
            }
            "union" => parse_union(&mut parser.ts, types)?,
            "extend" => {
                let kind = parser.ts.identifier()?;
                if kind == "type" || kind == "interface" || kind == "input" {
                    parse_type_definition(&mut parser.ts, types, &kind, true)?;
                } else {
                    skip_definition(&mut parser.ts);
                }
            }
            "directive" => skip_directive(&mut parser.ts),
            _ => {
                ctx.warning(&format!("unsupported GraphQL definition: {keyword}"), 0, 0);
                skip_definition(&mut parser.ts);
            }
        }
    }
    Ok(())
}

fn imports(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("#import") else {
            continue;
        };
        if !rest.starts_with([' ', '\t']) {
            continue;
        }
        let rest = rest.trim_start();
        let Some(quote) = rest.chars().next().filter(|ch| *ch == '"' || *ch == '\'') else {
            continue;
        };
        let Some(end) = rest[1..].find(quote) else {
            continue;
        };
        result.push(rest[1..1 + end].to_string());
    }
    result
}

struct Parser {
    ts: TokenStream,
}

fn parse_schema(ts: &mut TokenStream, types: &mut Node) -> Result<()> {
    ts.expect('{')?;
    while !ts.eof() && ts.peek(0) != "}" {
        skip_description(ts);
        if ts.peek(0) == "}" {
            break;
        }
        let kind = ts.identifier()?;
        ts.expect(':')?;
        let value = ts.identifier()?;
        if !types.get("__schema").is_some_and(Node::is_object) {
            types.set("__schema", Node::object());
        }
        if let Some(schema) = types.get_mut("__schema") {
            schema.set(kind, Node::Str(value));
        }
        ts.consume(",");
    }
    ts.expect('}')?;
    Ok(())
}

fn parse_type_definition(
    ts: &mut TokenStream,
    types: &mut Node,
    kind: &str,
    extend: bool,
) -> Result<()> {
    let name = ts.identifier()?;
    while !ts.eof() && ts.peek(0) != "{" {
        if ts.peek(0) == "}" {
            break;
        }
        ts.next_char();
    }
    let fields = parse_fields(ts)?;
    if extend
        && types
            .get(&name)
            .is_some_and(|node| node.get("fields").is_some())
    {
        let existing = types
            .get(&name)
            .and_then(|node| node.get("fields"))
            .cloned()
            .unwrap_or_else(Node::object);
        types
            .get_mut(&name)
            .unwrap()
            .set("fields", Node::merge(&existing, &fields));
    } else {
        let mut entry = Node::object();
        entry.set("kind", Node::Str(kind.to_string()));
        entry.set("fields", fields);
        types.set(name, entry);
    }
    Ok(())
}

fn parse_fields(ts: &mut TokenStream) -> Result<Node> {
    let mut fields = Node::object();
    ts.expect('{')?;
    while !ts.eof() && ts.peek(0) != "}" {
        skip_description(ts);
        if ts.peek(0) == "}" {
            break;
        }
        let name = ts.identifier()?;
        let args = if ts.consume("(").is_some() {
            parse_arguments(ts)?
        } else {
            Node::array()
        };
        ts.expect(':')?;
        let type_ref = parse_type_ref(ts, 0)?;
        skip_directives(ts);
        let mut field = Node::object();
        field.set("name", Node::Str(name.clone()));
        field.set("type", type_ref);
        field.set("args", args);
        fields.set(name, field);
    }
    ts.expect('}')?;
    Ok(fields)
}

fn parse_arguments(ts: &mut TokenStream) -> Result<Node> {
    let mut args = Node::array();
    while !ts.eof() && ts.peek(0) != ")" {
        skip_description(ts);
        if ts.peek(0) == ")" {
            break;
        }
        let name = ts.identifier()?;
        ts.expect(':')?;
        let type_ref = parse_type_ref(ts, 0)?;
        if ts.consume("=").is_some() {
            skip_default_value(ts);
        }
        let mut arg = Node::object();
        arg.set("name", Node::Str(name));
        arg.set("type", type_ref);
        args.push(arg);
        ts.consume(",");
    }
    ts.expect(')')?;
    Ok(args)
}

fn parse_type_ref(ts: &mut TokenStream, depth: usize) -> Result<Node> {
    if depth > 256 {
        return Err(ts.error("graphql nesting limit reached"));
    }
    let mut type_ref = if ts.consume("[").is_some() {
        let inner = parse_type_ref(ts, depth + 1)?;
        ts.expect(']')?;
        let mut node = Node::object();
        node.set("kind", Node::Str("array".to_string()));
        node.set("inner", inner);
        node
    } else {
        let mut node = Node::object();
        node.set("kind", Node::Str("named".to_string()));
        node.set("name", Node::Str(ts.identifier()?));
        node
    };
    type_ref.set("require", Node::Bool(ts.consume("!").is_some()));
    Ok(type_ref)
}

fn parse_enum(ts: &mut TokenStream, types: &mut Node) -> Result<()> {
    let name = ts.identifier()?;
    let mut values = Node::array();
    ts.expect('{')?;
    while !ts.eof() && ts.peek(0) != "}" {
        skip_description(ts);
        if ts.peek(0) == "}" {
            break;
        }
        values.push(Node::Str(ts.identifier()?));
        ts.consume(",");
    }
    ts.expect('}')?;
    let mut entry = Node::object();
    entry.set("kind", Node::Str("enum".to_string()));
    entry.set("values", values);
    types.set(name, entry);
    Ok(())
}

fn parse_union(ts: &mut TokenStream, types: &mut Node) -> Result<()> {
    let name = ts.identifier()?;
    let mut members = Node::array();
    if ts.consume("=").is_some() {
        while !ts.eof() {
            members.push(Node::Str(ts.identifier()?));
            if ts.consume("|").is_none() {
                break;
            }
        }
    }
    let mut entry = Node::object();
    entry.set("kind", Node::Str("union".to_string()));
    entry.set("members", members);
    types.set(name, entry);
    Ok(())
}

fn select(
    types: &Node,
    query_type: &str,
    query_fields: &Node,
    wanted: &str,
    ctx: &mut Context<'_>,
) -> Option<(String, String, Node)> {
    if !wanted.is_empty() {
        let (type_name, field_name) = match wanted.rfind('.') {
            Some(index) => (&wanted[..index], &wanted[index + 1..]),
            None => (query_type, wanted),
        };
        if let Some(definition) = types
            .get(type_name)
            .and_then(|kind| kind.get("fields"))
            .and_then(|fields| fields.get(field_name))
        {
            return Some((
                type_name.to_string(),
                field_name.to_string(),
                definition.clone(),
            ));
        }
        ctx.error(
            &format!(
                "operation not found: {wanted}; candidates: {}",
                candidates(query_type, query_fields)
            ),
            0,
            0,
        );
        return None;
    }
    let entries = schema::entries(query_fields);
    if entries.len() == 1 {
        let (field_name, definition) = entries.into_iter().next().unwrap();
        return Some((query_type.to_string(), field_name, definition));
    }
    ctx.error(
        &format!(
            "operation required; candidates: {}",
            candidates(query_type, query_fields)
        ),
        0,
        0,
    );
    None
}

fn candidates(type_name: &str, fields: &Node) -> String {
    schema::entries(fields)
        .into_iter()
        .map(|(name, _)| format!("{type_name}.{name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn response_fields(type_ref: &Node, types: &Node) -> Node {
    let kind = type_ref
        .get("kind")
        .map(Node::scalar_string)
        .unwrap_or_default();
    if kind == "array" {
        let inner = type_ref
            .get("inner")
            .map(|inner| describe_type(inner, types, &[], 0))
            .unwrap_or_else(Node::object);
        let mut field = Node::object();
        field.set("name", Node::Str("items".to_string()));
        field.set("type", Node::Str("array".to_string()));
        field.set(
            "require",
            Node::Bool(type_ref.get("require").is_some_and(Node::truthy)),
        );
        field.set(
            "children",
            inner.get("children").cloned().unwrap_or_else(Node::array),
        );
        let mut result = Node::array();
        result.push(field);
        return result;
    }
    if kind == "named" {
        let descriptor = describe_type(type_ref, types, &[], 0);
        let descriptor_type = descriptor
            .get("type")
            .map(Node::scalar_string)
            .unwrap_or_default();
        if descriptor_type == "object" {
            return descriptor
                .get("children")
                .cloned()
                .unwrap_or_else(Node::array);
        }
        let mut field = Node::object();
        field.set("name", Node::Str("result".to_string()));
        field.set("type", Node::Str(descriptor_type));
        field.set(
            "require",
            Node::Bool(type_ref.get("require").is_some_and(Node::truthy)),
        );
        let mut result = Node::array();
        result.push(field);
        return result;
    }
    Node::array()
}

fn describe_type(type_ref: &Node, types: &Node, visited: &[String], depth: usize) -> Node {
    if depth > 256 {
        let mut node = Node::object();
        node.set("type", Node::Str("object".to_string()));
        node.set("children", Node::array());
        return node;
    }
    let kind = type_ref
        .get("kind")
        .map(Node::scalar_string)
        .unwrap_or_default();
    let require = type_ref.get("require").is_some_and(Node::truthy);
    if kind == "array" {
        let inner = type_ref
            .get("inner")
            .map(|inner| describe_type(inner, types, visited, depth + 1))
            .unwrap_or_else(Node::object);
        let mut result = Node::object();
        result.set("type", Node::Str("array".to_string()));
        result.set("require", Node::Bool(require));
        if inner.get("type").map(Node::scalar_string).as_deref() == Some("object") {
            result.set(
                "children",
                inner.get("children").cloned().unwrap_or_else(Node::array),
            );
            result.set("childrenType", Node::Str("object".to_string()));
        } else {
            result.set(
                "childrenType",
                Node::Str(
                    inner
                        .get("type")
                        .map(Node::scalar_string)
                        .unwrap_or_else(|| "string".to_string()),
                ),
            );
        }
        return result;
    }
    let name = type_ref
        .get("name")
        .map(Node::scalar_string)
        .unwrap_or_default();
    if let Some((_, scalar)) = SCALARS.iter().find(|(scalar_name, _)| *scalar_name == name) {
        let mut result = Node::object();
        result.set("type", Node::Str(scalar.to_string()));
        result.set("require", Node::Bool(require));
        return result;
    }
    if types
        .get(&name)
        .and_then(|entry| entry.get("kind"))
        .map(Node::scalar_string)
        .as_deref()
        == Some("enum")
    {
        let mut result = Node::object();
        result.set("type", Node::Str("string".to_string()));
        result.set("require", Node::Bool(require));
        result.set(
            "enum",
            types
                .get(&name)
                .and_then(|entry| entry.get("values"))
                .cloned()
                .unwrap_or_else(Node::array),
        );
        return result;
    }
    let has_fields = types
        .get(&name)
        .and_then(|entry| entry.get("fields"))
        .is_some();
    if has_fields && !visited.iter().any(|entry| entry == &name) {
        let mut next_visited = visited.to_vec();
        next_visited.push(name.clone());
        let children = types
            .get(&name)
            .and_then(|entry| entry.get("fields"))
            .map(|fields| map_fields(fields, types, &next_visited, depth + 1))
            .unwrap_or_else(Node::array);
        let mut result = Node::object();
        result.set("type", Node::Str("object".to_string()));
        result.set("require", Node::Bool(require));
        result.set("ref", Node::Str(name));
        result.set("children", children);
        return result;
    }
    let mut result = Node::object();
    result.set("type", Node::Str("object".to_string()));
    result.set("require", Node::Bool(require));
    result.set("ref", Node::Str(name));
    result.set("children", Node::array());
    result
}

fn map_fields(fields: &Node, types: &Node, visited: &[String], depth: usize) -> Node {
    let mut result = Node::array();
    for (_, field) in schema::entries(fields) {
        let Some(type_ref) = field.get("type") else {
            continue;
        };
        let mut descriptor = describe_type(type_ref, types, visited, depth);
        descriptor.set(
            "name",
            Node::Str(
                field
                    .get("name")
                    .map(Node::scalar_string)
                    .unwrap_or_default(),
            ),
        );
        result.push(descriptor);
    }
    result
}

fn validate_sample(sample: &str, fields: &[Value]) -> Value {
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
    let mut errors = Vec::new();
    validate_fields(fields, &data, "$", &mut errors);
    json!({
        "ok": errors.is_empty(),
        "errors": errors,
        "warnings": [],
    })
}

fn validate_fields(fields: &[Value], data: &Node, path: &str, errors: &mut Vec<Value>) {
    if !data.is_array_like() {
        errors.push(json!({
            "path": path,
            "severity": "error",
            "message": "expected object",
        }));
        return;
    }
    for field in fields {
        let name = field.get("name").and_then(Value::as_str).unwrap_or("");
        let Some(value) = data.get(name) else {
            if field.get("require").and_then(Value::as_bool) == Some(true) {
                errors.push(json!({
                    "path": format!("{path}.{name}"),
                    "severity": "error",
                    "message": "required field missing",
                }));
            }
            continue;
        };
        let field_type = field.get("type").and_then(Value::as_str).unwrap_or("");
        let children: &[Value] = field
            .get("children")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let field_path = format!("{path}.{name}");
        if field_type == "object" && !children.is_empty() {
            validate_fields(children, value, &field_path, errors);
        } else if field_type == "array" && !children.is_empty() {
            if !value.is_array_like() {
                errors.push(json!({
                    "path": field_path,
                    "severity": "error",
                    "message": "expected array",
                }));
                continue;
            }
            for (index, item) in schema::entries(value) {
                validate_fields(children, &item, &format!("{field_path}[{index}]"), errors);
            }
        } else if field_type == "integer" && !matches!(value, Node::Int(_)) {
            errors.push(json!({
                "path": field_path,
                "severity": "error",
                "message": "expected integer",
            }));
        } else if field_type == "number" && !matches!(value, Node::Int(_) | Node::Float(_)) {
            errors.push(json!({
                "path": field_path,
                "severity": "error",
                "message": "expected number",
            }));
        } else if field_type == "boolean" && !matches!(value, Node::Bool(_)) {
            errors.push(json!({
                "path": field_path,
                "severity": "error",
                "message": "expected boolean",
            }));
        } else if field_type == "string" && !matches!(value, Node::Str(_)) {
            errors.push(json!({
                "path": field_path,
                "severity": "error",
                "message": "expected string",
            }));
        }
    }
}

fn skip_description(ts: &mut TokenStream) {
    if ts.peek(0) != "\"" {
        return;
    }
    if ts.peek(1) == "\"" && ts.peek(2) == "\"" {
        ts.next_char();
        ts.next_char();
        ts.next_char();
        while !ts.eof() {
            if ts.peek(0) == "\"" && ts.peek(1) == "\"" && ts.peek(2) == "\"" {
                ts.next_char();
                ts.next_char();
                ts.next_char();
                return;
            }
            ts.next_char();
        }
        return;
    }
    let _ = ts.string();
}

fn skip_directives(ts: &mut TokenStream) {
    while ts.consume("@").is_some() {
        let _ = ts.identifier();
        if ts.consume("(").is_some() {
            skip_balanced(ts, '(', ')');
        }
    }
}

fn skip_default_value(ts: &mut TokenStream) {
    let ch = ts.peek(0);
    if ch == "[" {
        ts.next_char();
        skip_balanced(ts, '[', ']');
        return;
    }
    if ch == "{" {
        ts.next_char();
        skip_balanced(ts, '{', '}');
        return;
    }
    if ch == "\"" || ch == "'" {
        let _ = ts.string();
        return;
    }
    while !ts.eof() {
        let ch = ts.peek(0);
        if ch == "," || ch == ")" || ch == "}" {
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

fn skip_directive(ts: &mut TokenStream) {
    ts.consume("@");
    if !ts.eof() {
        let _ = ts.identifier();
    }
    if ts.consume("(").is_some() {
        skip_balanced(ts, '(', ')');
    }
    if !ts.eof() {
        let _ = ts.identifier();
    }
    while !ts.eof() {
        let ch = ts.peek(0);
        let first = ch.chars().next();
        if ch == "|" || first.is_some_and(|ch| ch.is_ascii_alphabetic()) || ch == "_" {
            ts.next_char();
            continue;
        }
        break;
    }
}

fn skip_definition(ts: &mut TokenStream) {
    while !ts.eof() {
        let ch = ts.peek(0);
        if ch == "}" {
            ts.next_char();
            return;
        }
        let first = ch.chars().next();
        if first.is_some_and(|ch| ch.is_ascii_alphabetic()) || ch == "_" {
            let Ok(word) = ts.identifier() else {
                return;
            };
            if DEFINITIONS.contains(&word.as_str()) {
                return;
            }
            continue;
        }
        ts.next_char();
    }
}
