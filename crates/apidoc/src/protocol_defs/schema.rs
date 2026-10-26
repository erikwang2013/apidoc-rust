//! Ports of `RefResolver`, `SchemaMapper` and `JsonSchemaValidator`.

use std::collections::HashSet;

use serde_json::{json, Value};

use super::context::{Context, Loader};
use super::node::Node;

pub const MAX_REFS: usize = 64;
pub const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug)]
pub struct Resolved {
    pub node: Node,
    pub root: Node,
    pub file: String,
}

/// Root document plus a loader, used by the schema helpers.
pub struct RefCtx {
    pub root: Node,
    pub file: Option<String>,
    pub loader: Loader,
}

impl RefCtx {
    pub fn new(root: Node, file: Option<String>, loader: Loader) -> Self {
        Self { root, file, loader }
    }

    pub fn resolve(&self, refstr: &str, ctx: &mut Context) -> Option<Node> {
        let entered = self
            .file
            .as_deref()
            .is_some_and(|file| !file.is_empty() && file != ctx.file());
        if entered {
            ctx.enter_file(self.file.clone().unwrap());
        }
        let resolved = resolve(refstr, &self.root, ctx, self.loader).map(|r| r.node);
        if entered {
            ctx.leave_file();
        }
        resolved
    }
}

pub fn resolve(refstr: &str, root: &Node, ctx: &mut Context, loader: Loader) -> Option<Resolved> {
    let (file, fragment) = split(refstr, ctx)?;
    let mut root = root.clone();
    let mut resolved_file = ctx.file().to_string();
    if !file.is_empty() {
        let realpath = ctx.resolve(&file)?;
        resolved_file = realpath.clone();
        if let Some(document) = ctx.document(&realpath) {
            root = document.clone();
        } else {
            if !ctx.push(realpath.clone()) {
                return None;
            }
            let content = ctx.read();
            let document = content.and_then(|content| loader.load(&content).ok());
            ctx.pop();
            let Some(document) = document else {
                ctx.error(&format!("unable to parse $ref: {refstr}"), 0, 0);
                return None;
            };
            ctx.remember_document(realpath, document.clone());
            root = document;
        }
    }
    let node = pointer(&root, &fragment, ctx, refstr)?;
    Some(Resolved {
        node,
        root,
        file: resolved_file,
    })
}

pub fn expand(
    schema: &Node,
    root: &Node,
    ctx: &mut Context,
    loader: Loader,
    base_file: Option<&str>,
) -> Node {
    let entered = base_file.is_some_and(|file| !file.is_empty() && file != ctx.file());
    if entered {
        ctx.enter_file(base_file.unwrap().to_string());
    }
    let mut state = ExpandState::default();
    let result = expand_node(schema, root, ctx, loader, &mut state, 0);
    if entered {
        ctx.leave_file();
    }
    result
}

#[derive(Default)]
struct ExpandState {
    refs: usize,
    stack: Vec<String>,
}

fn expand_node(
    node: &Node,
    root: &Node,
    ctx: &mut Context,
    loader: Loader,
    state: &mut ExpandState,
    depth: usize,
) -> Node {
    if depth > MAX_DEPTH || state.refs > MAX_REFS {
        return stub(ctx, "$ref depth limit exceeded");
    }
    if let Some(refstr) = node.get("$ref").and_then(Node::as_str) {
        let mut siblings = node.clone();
        siblings.remove("$ref");
        let siblings = expand_node(&siblings, root, ctx, loader, state, depth);
        return expand_ref(refstr, &siblings, root, ctx, loader, state, depth);
    }
    let mut node = node.clone();
    for key in ["properties", "$defs", "definitions"] {
        let Some(value) = node.get(key).cloned() else {
            continue;
        };
        let expanded = expand_children(&value, root, ctx, loader, state, depth);
        node.set(key, expanded);
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        let Some(value) = node.get(key).cloned() else {
            continue;
        };
        let expanded = expand_children(&value, root, ctx, loader, state, depth);
        node.set(key, expanded);
    }
    if let Some(items) = node.get("items").cloned() {
        if items.is_list() {
            node.set(
                "items",
                expand_children(&items, root, ctx, loader, state, depth),
            );
        } else {
            node.set(
                "items",
                expand_node(&items, root, ctx, loader, state, depth),
            );
        }
    }
    node
}

fn expand_children(
    value: &Node,
    root: &Node,
    ctx: &mut Context,
    loader: Loader,
    state: &mut ExpandState,
    depth: usize,
) -> Node {
    match value {
        Node::Array(items) => Node::Array(
            items
                .iter()
                .map(|child| expand_node(child, root, ctx, loader, state, depth))
                .collect(),
        ),
        Node::Object(entries) => {
            let mut out = Node::object();
            for (name, child) in entries {
                out.set(
                    name.clone(),
                    expand_node(child, root, ctx, loader, state, depth),
                );
            }
            out
        }
        other => other.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_ref(
    refstr: &str,
    siblings: &Node,
    root: &Node,
    ctx: &mut Context,
    loader: Loader,
    state: &mut ExpandState,
    depth: usize,
) -> Node {
    let Some((file, fragment)) = split(refstr, ctx) else {
        return Node::merge(siblings, &empty_stub());
    };
    let mut realpath = String::new();
    let key = if !file.is_empty() {
        let Some(resolved) = ctx.resolve(&file) else {
            ctx.error(&format!("unable to resolve $ref: {refstr}"), 0, 0);
            return Node::merge(siblings, &empty_stub());
        };
        realpath = resolved;
        format!("{realpath}#{fragment}")
    } else {
        format!("{}#{fragment}", ctx.file())
    };
    if state.stack.iter().any(|entry| entry == &key) {
        return Node::merge(
            siblings,
            &stub(ctx, &format!("$ref cycle detected: {refstr}")),
        );
    }
    if state.refs >= MAX_REFS || depth >= MAX_DEPTH {
        return Node::merge(
            siblings,
            &stub(ctx, &format!("$ref depth limit exceeded: {refstr}")),
        );
    }
    state.refs += 1;
    state.stack.push(key);
    let document_root = if !realpath.is_empty() {
        if let Some(document) = ctx.expanded_document(&realpath) {
            document.clone()
        } else {
            if !ctx.push(realpath.clone()) {
                state.stack.pop();
                return Node::merge(siblings, &empty_stub());
            }
            let content = ctx.read();
            let document = content.and_then(|content| loader.load(&content).ok());
            let document_root = match &document {
                Some(document) => expand_node(document, document, ctx, loader, state, depth + 1),
                None => Node::array(),
            };
            ctx.pop();
            let Some(_) = document else {
                state.stack.pop();
                ctx.error(&format!("unable to parse $ref: {refstr}"), 0, 0);
                return Node::merge(siblings, &empty_stub());
            };
            ctx.remember_expanded(realpath, document_root.clone());
            document_root
        }
    } else {
        root.clone()
    };
    let Some(target) = pointer(&document_root, &fragment, ctx, refstr) else {
        state.stack.pop();
        return Node::merge(siblings, &empty_stub());
    };
    let expanded = expand_node(&target, &document_root, ctx, loader, state, depth + 1);
    state.stack.pop();
    Node::merge(siblings, &expanded)
}

fn split(refstr: &str, ctx: &mut Context) -> Option<(String, String)> {
    let refstr = refstr.trim();
    if refstr.is_empty() {
        ctx.error("empty $ref", 0, 0);
        return None;
    }
    if let Some(rest) = refstr.strip_prefix('#') {
        return Some((String::new(), rest.to_string()));
    }
    match refstr.find('#') {
        Some(index) => Some((refstr[..index].to_string(), refstr[index + 1..].to_string())),
        None => Some((refstr.to_string(), String::new())),
    }
}

fn pointer(root: &Node, fragment: &str, ctx: &mut Context, refstr: &str) -> Option<Node> {
    if fragment.is_empty() || fragment == "/" {
        return Some(root.clone());
    }
    let mut node = root.clone();
    for segment in fragment.trim_start_matches('/').split('/') {
        let segment = rawurldecode(&segment.replace("~1", "/").replace("~0", "~"));
        let child = match &node {
            Node::Object(_) => node.get(&segment).cloned(),
            Node::Array(items) => segment
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index).cloned()),
            _ => None,
        };
        let Some(child) = child else {
            ctx.error(&format!("$ref not found: {refstr}"), 0, 0);
            return None;
        };
        node = child;
    }
    if !node.is_array_like() {
        ctx.error(&format!("$ref target is not an object: {refstr}"), 0, 0);
        return None;
    }
    Some(node)
}

fn stub(ctx: &mut Context, message: &str) -> Node {
    ctx.warning(message, 0, 0);
    empty_stub()
}

pub fn empty_stub() -> Node {
    let mut node = Node::object();
    node.set("type", Node::Str("object".to_string()));
    node.set("properties", Node::object());
    node.set("children", Node::array());
    node
}

fn rawurldecode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// SchemaMapper
// ---------------------------------------------------------------------------

pub fn fields(schema: &Node, ctx: &mut Context, refs: &RefCtx, depth: usize) -> Vec<Value> {
    if depth > 32 {
        ctx.warning("schema nesting limit reached", 0, 0);
        return Vec::new();
    }
    let schema = flatten(schema, ctx, refs, &mut HashSet::new(), 0);
    let Some(properties) = schema.get("properties").filter(|node| node.is_array_like()) else {
        return Vec::new();
    };
    let properties = entries(properties);
    if properties.is_empty() {
        return Vec::new();
    }
    let required: Vec<String> = schema
        .get("required")
        .and_then(Node::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Node::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut result = Vec::new();
    for (name, prop) in properties {
        let prop = if prop.is_array_like() {
            flatten(&prop, ctx, refs, &mut HashSet::new(), 0)
        } else {
            Node::object()
        };
        let field_type = type_of(&prop);
        let mut field = json!({
            "name": name,
            "type": field_type,
            "require": required.iter().any(|entry| entry == &name),
        });
        if let Some(desc) = prop.get("description").filter(|node| node.truthy()) {
            field["desc"] = Value::String(desc.scalar_string());
        }
        if let Some(enum_values) = prop.get("enum").filter(|node| node.is_array_like()) {
            field["enum"] = Value::Array(values(enum_values));
        }
        if prop.get("nullable").is_some_and(Node::truthy)
            || prop.get("type").is_some_and(Node::is_array)
        {
            field["require"] = Value::Bool(false);
            ctx.warning(
                &format!("field {name} is nullable or a union type; marked optional"),
                0,
                0,
            );
        }
        if field["type"] == "array" {
            let items = prop
                .get("items")
                .filter(|node| node.is_array_like())
                .cloned()
                .unwrap_or_else(Node::object);
            let children = fields(&items, ctx, refs, depth + 1);
            if !children.is_empty() {
                field["children"] = Value::Array(children);
            }
        } else if field["type"] == "object" {
            let children = fields(&prop, ctx, refs, depth + 1);
            if !children.is_empty() {
                field["children"] = Value::Array(children);
            }
        }
        result.push(field);
    }
    result
}

pub fn type_of(schema: &Node) -> String {
    let mut type_node = schema.get("type").cloned();
    if let Some(Node::Array(items)) = &type_node {
        if let Some(found) = items.iter().find(|item| item.as_str() != Some("null")) {
            type_node = Some(found.clone());
        }
    }
    match type_node.as_ref().and_then(Node::as_str) {
        Some(kind @ ("string" | "integer" | "number" | "boolean" | "array" | "object")) => {
            kind.to_string()
        }
        Some("int") => "integer".to_string(),
        Some("bool") => "boolean".to_string(),
        _ => {
            if schema.get("properties").is_some_and(Node::truthy) {
                "object".to_string()
            } else {
                "string".to_string()
            }
        }
    }
}

fn flatten(
    schema: &Node,
    ctx: &mut Context,
    refs: &RefCtx,
    seen: &mut HashSet<String>,
    depth: usize,
) -> Node {
    let mut schema = schema.clone();
    while let Some(refstr) = schema.get("$ref").and_then(Node::as_str) {
        let refstr = refstr.to_string();
        if seen.contains(&refstr) || depth >= 32 {
            ctx.warning(
                &format!("$ref cycle or depth limit detected: {refstr}"),
                0,
                0,
            );
            return Node::object();
        }
        seen.insert(refstr.clone());
        let Some(resolved) = refs.resolve(&refstr, ctx) else {
            return Node::object();
        };
        let mut siblings = schema.clone();
        siblings.remove("$ref");
        schema = Node::merge(&siblings, &resolved);
    }
    if let Some(branches) = schema.get("allOf").filter(|node| node.truthy()) {
        let branches = entries(branches);
        for (_, branch) in branches {
            if !branch.is_array_like() {
                continue;
            }
            let mut branch_seen = seen.clone();
            let branch = flatten(&branch, ctx, refs, &mut branch_seen, depth);
            if branch.get("properties").is_some_and(Node::truthy) {
                let merged = Node::merge(
                    &schema
                        .get("properties")
                        .cloned()
                        .unwrap_or_else(Node::object),
                    &branch
                        .get("properties")
                        .cloned()
                        .unwrap_or_else(Node::object),
                );
                schema.set("properties", merged);
            }
            if branch.get("required").is_some_and(Node::truthy) {
                let base = schema
                    .get("required")
                    .and_then(Node::as_array)
                    .map(<[Node]>::to_vec)
                    .unwrap_or_default();
                let mut merged = Node::Array(base);
                for item in branch
                    .get("required")
                    .and_then(Node::as_array)
                    .unwrap_or(&[])
                {
                    merged.push(item.clone());
                }
                schema.set("required", merged);
            }
        }
        schema.set("required", unique_required(schema.get("required")));
        schema.remove("allOf");
    }
    for keyword in ["oneOf", "anyOf"] {
        if !schema.get(keyword).is_some_and(Node::truthy) {
            continue;
        }
        ctx.warning(
            &format!("{keyword} is not fully supported; using the first branch"),
            0,
            0,
        );
        let mut branch_seen = seen.clone();
        let branch = schema
            .get(keyword)
            .and_then(Node::as_array)
            .and_then(|items| items.first())
            .filter(|item| item.is_array_like())
            .cloned()
            .unwrap_or_else(Node::object);
        schema.remove(keyword);
        let branch = flatten(&branch, ctx, refs, &mut branch_seen, depth);
        schema = Node::merge(&schema, &branch);
        if schema.get("$ref").is_some() {
            schema = flatten(&schema, ctx, refs, &mut branch_seen, depth);
        }
    }
    schema
}

fn unique_required(required: Option<&Node>) -> Node {
    let mut out = Node::array();
    let mut seen: Vec<Node> = Vec::new();
    for item in required.and_then(Node::as_array).unwrap_or(&[]) {
        if !seen.iter().any(|entry| entry.strict_eq(item)) {
            seen.push(item.clone());
            out.push(item.clone());
        }
    }
    out
}

pub fn entries(value: &Node) -> Vec<(String, Node)> {
    match value {
        Node::Object(entries) => entries.clone(),
        Node::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| (index.to_string(), item.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

pub fn values(value: &Node) -> Vec<Value> {
    match value {
        Node::Array(items) => items.iter().map(Node::to_json).collect(),
        Node::Object(entries) => entries.iter().map(|(_, value)| value.to_json()).collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// JsonSchemaValidator
// ---------------------------------------------------------------------------

pub fn validate(
    schema: &Node,
    data: &Node,
    refs: &RefCtx,
    ctx: &mut Context,
    path: &str,
    depth: usize,
) -> Vec<(String, String)> {
    if depth > 32 {
        return Vec::new();
    }
    let mut errors = Vec::new();
    let mut schema = schema.clone();
    let mut seen = HashSet::new();
    let mut ref_depth = 0;
    while let Some(refstr) = schema.get("$ref").and_then(Node::as_str) {
        let refstr = refstr.to_string();
        if seen.contains(&refstr) || ref_depth >= 32 {
            break;
        }
        seen.insert(refstr.clone());
        let Some(resolved) = refs.resolve(&refstr, ctx) else {
            return errors;
        };
        let mut siblings = schema.clone();
        siblings.remove("$ref");
        schema = Node::merge(&siblings, &resolved);
        ref_depth += 1;
    }
    if let Some(branches) = schema.get("allOf").filter(|node| node.truthy()) {
        for (_, branch) in entries(branches) {
            if branch.is_array_like() {
                errors.extend(validate(&branch, data, refs, ctx, path, depth + 1));
            }
        }
    }
    if let Some(branches) = schema.get("anyOf").filter(|node| node.truthy()) {
        let branches = entries(branches);
        let matched = branches
            .iter()
            .any(|(_, branch)| validate(branch, data, refs, ctx, path, depth + 1).is_empty());
        if !matched {
            errors.push((
                path.to_string(),
                "does not match any of the allowed schemas".to_string(),
            ));
        }
    }
    if let Some(branches) = schema.get("oneOf").filter(|node| node.truthy()) {
        let matches = entries(branches)
            .iter()
            .filter(|(_, branch)| validate(branch, data, refs, ctx, path, depth + 1).is_empty())
            .count();
        if matches != 1 {
            errors.push((
                path.to_string(),
                "must match exactly one schema".to_string(),
            ));
        }
    }
    if let Some(enum_values) = schema.get("enum").filter(|node| node.is_array_like()) {
        let allowed = values(enum_values);
        let data_json = data.to_json();
        if !allowed
            .iter()
            .any(|value| strict_json_eq(value, &data_json))
        {
            errors.push((
                path.to_string(),
                "value is not one of the allowed values".to_string(),
            ));
        }
    }
    if let Some(expected) = schema.get("const") {
        if !data.loose_eq(expected) {
            errors.push((
                path.to_string(),
                "value must equal the const value".to_string(),
            ));
        }
    }
    if let Some(type_node) = schema.get("type").filter(|node| **node != Node::Null) {
        if !matches_type(type_node, data) {
            let expected = match type_node {
                Node::Array(items) => items
                    .iter()
                    .map(Node::scalar_string)
                    .collect::<Vec<_>>()
                    .join("|"),
                other => other.scalar_string(),
            };
            errors.push((path.to_string(), format!("expected type {expected}")));
            return errors;
        }
    }
    if !data.is_array_like() {
        return errors;
    }
    let is_object = match data {
        Node::Array(items) => items.is_empty(),
        Node::Object(_) => !data.is_list() || data.is_empty(),
        _ => false,
    };
    if is_object {
        if let Some(required) = schema.get("required").and_then(Node::as_array) {
            for name in required {
                let name = name.scalar_string();
                if data.get(&name).is_none() {
                    errors.push((child_path(path, &name), "is required".to_string()));
                }
            }
        }
    }
    if is_object {
        if let Some(properties) = schema.get("properties").filter(|node| node.truthy()) {
            for (name, prop) in entries(properties) {
                if prop.is_array_like() {
                    if let Some(value) = data.get(&name) {
                        errors.extend(validate(
                            &prop,
                            value,
                            refs,
                            ctx,
                            &child_path(path, &name),
                            depth + 1,
                        ));
                    }
                }
            }
        }
    }
    if let Some(items) = schema.get("items").filter(|node| node.is_array_like()) {
        if data.is_list() {
            let data_values = entries(data);
            for (index, value) in data_values {
                errors.extend(validate(
                    items,
                    &value,
                    refs,
                    ctx,
                    &format!("{path}[{index}]"),
                    depth + 1,
                ));
            }
        }
    }
    errors
}

fn matches_type(type_node: &Node, data: &Node) -> bool {
    match type_node {
        Node::Array(items) => items
            .iter()
            .any(|item| matches_one(&item.scalar_string(), data)),
        Node::Object(entries) => entries
            .iter()
            .any(|(_, item)| matches_one(&item.scalar_string(), data)),
        other => matches_one(&other.scalar_string(), data),
    }
}

fn matches_one(kind: &str, data: &Node) -> bool {
    match kind {
        "object" => data.is_array_like() && (data.is_empty() || !data.is_list()),
        "array" => data.is_array_like() && data.is_list(),
        "string" => matches!(data, Node::Str(_)),
        "integer" => matches!(data, Node::Int(_)),
        "number" => matches!(data, Node::Int(_) | Node::Float(_)),
        "boolean" => matches!(data, Node::Bool(_)),
        "null" => matches!(data, Node::Null),
        _ => true,
    }
}

fn strict_json_eq(left: &Value, right: &Value) -> bool {
    left == right
}

fn child_path(path: &str, name: &str) -> String {
    if path == "$" {
        format!("$.{name}")
    } else {
        format!("{path}.{name}")
    }
}
