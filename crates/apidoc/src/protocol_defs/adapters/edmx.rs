//! Port of `php/src/protocol/Adapter/EdmxAdapter.php`.

use serde_json::{json, Value};

use crate::protocol_defs::adapters::{message, Adapter, Parsed};
use crate::protocol_defs::context::{Context, Loader};
use crate::protocol_defs::node::{parse_json, Node};
use crate::protocol_defs::schema::{self, RefCtx};
use crate::protocol_defs::token::ParseError;
use crate::protocol_defs::xml::{self, XmlNode};

const TYPES: [(&str, &str); 11] = [
    ("Edm.Int32", "integer"),
    ("Edm.Int64", "integer"),
    ("Edm.Int16", "integer"),
    ("Edm.Byte", "integer"),
    ("Edm.String", "string"),
    ("Edm.Boolean", "boolean"),
    ("Edm.Decimal", "number"),
    ("Edm.Double", "number"),
    ("Edm.Single", "number"),
    ("Edm.DateTimeOffset", "string"),
    ("Edm.Guid", "string"),
];

pub struct EdmxAdapter;

impl Adapter for EdmxAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed, ParseError> {
        let document = xml::load(content).map_err(|error| ParseError {
            message: error.message,
            line: error.line,
            column: error.column,
        })?;
        let mut entities = Node::object();
        let mut complex_types = Node::object();
        let mut entity_sets = Node::object();
        let mut functions = Node::object();

        for entity_type in descendants(&document.root, "EntityType") {
            let name = entity_type.attr("Name").to_string();
            entities.set(name, properties(entity_type));
        }
        for complex_type in descendants(&document.root, "ComplexType") {
            let name = complex_type.attr("Name").to_string();
            complex_types.set(name, properties(complex_type));
        }
        for entity_set in descendants(&document.root, "EntitySet") {
            entity_sets.set(
                entity_set.attr("Name").to_string(),
                Node::Str(local_name(entity_set.attr("EntityType"))),
            );
        }
        for function in descendants(&document.root, "FunctionImport") {
            let name = function.attr("Name").to_string();
            let mut fields = Node::array();
            for parameter in function.all("Parameter") {
                let mut field = Node::object();
                field.set("name", Node::Str(parameter.attr("Name").to_string()));
                field.set("type", Node::Str(edm_type(parameter.attr("Type"))));
                field.set("require", Node::Bool(true));
                fields.push(field);
            }
            functions.set(name, fields);
        }

        let mut candidates: Vec<String> = schema::entries(&entity_sets)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        candidates.extend(
            schema::entries(&functions)
                .into_iter()
                .map(|(name, _)| name),
        );
        let mut wanted = ctx.options().operation.clone();
        if !wanted.is_empty() && !candidates.contains(&wanted) {
            ctx.error(
                &format!(
                    "operation not found: {wanted}; candidates: {}",
                    candidates.join(", ")
                ),
                0,
                0,
            );
            return Ok(Parsed::empty());
        }
        if wanted.is_empty() {
            if candidates.len() == 1 {
                wanted = candidates[0].clone();
            } else {
                ctx.error(
                    &format!("operation required; candidates: {}", candidates.join(", ")),
                    0,
                    0,
                );
                return Ok(Parsed::empty());
            }
        }

        let (fields, direction) = if let Some(entity_name) = entity_sets.get(&wanted) {
            let entity_name = entity_name.scalar_string();
            let empty = Node::array();
            let source = entities.get(&entity_name).unwrap_or(&empty);
            (
                resolve_fields(source, &entities, &complex_types, &[]),
                "response",
            )
        } else {
            let empty = Node::array();
            let source = functions.get(&wanted).unwrap_or(&empty);
            let fields = schema::entries(source)
                .into_iter()
                .map(|(_, value)| value)
                .collect::<Vec<_>>();
            (fields, "request")
        };
        let mut messages = vec![message(
            &wanted,
            direction,
            fields.into_iter().map(|field| field.to_json()).collect(),
        )];
        let sample = ctx.options().sample.clone();
        if !sample.is_empty() {
            let fields = messages[0]
                .get("fields")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            messages[0]["sampleValidation"] = validate_sample(&sample, &fields, ctx);
        }
        Ok(Parsed {
            operation: wanted,
            messages,
            meta: json!({ "direction": direction }),
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

fn properties(node: &XmlNode) -> Node {
    let mut fields = Node::array();
    for property in node.all("Property") {
        let mut field = Node::object();
        field.set("name", Node::Str(property.attr("Name").to_string()));
        field.set("type", Node::Str(edm_type(property.attr("Type"))));
        field.set("require", Node::Bool(property.attr("Nullable") == "false"));
        fields.push(field);
    }
    for navigation in node.all("NavigationProperty") {
        let mut field = Node::object();
        field.set("name", Node::Str(navigation.attr("Name").to_string()));
        field.set("type", Node::Str("object".to_string()));
        field.set("require", Node::Bool(false));
        field.set("ref", Node::Str(local_name(navigation.attr("Type"))));
        fields.push(field);
    }
    fields
}

fn resolve_fields(
    fields: &Node,
    entities: &Node,
    complex_types: &Node,
    visited: &[String],
) -> Vec<Node> {
    let mut result = Vec::new();
    for (_, field) in schema::entries(fields) {
        let mut field = field;
        let reference = field
            .get("ref")
            .map(Node::scalar_string)
            .unwrap_or_default();
        if !reference.is_empty() && !visited.iter().any(|entry| entry == &reference) {
            let mut next_visited = visited.to_vec();
            next_visited.push(reference.clone());
            let empty = Node::array();
            let target = entities
                .get(&reference)
                .or_else(|| complex_types.get(&reference))
                .unwrap_or(&empty);
            field.set(
                "children",
                Node::Array(resolve_fields(
                    target,
                    entities,
                    complex_types,
                    &next_visited,
                )),
            );
        }
        result.push(field);
    }
    result
}

fn validate_sample(sample: &str, fields: &[Value], ctx: &mut Context<'_>) -> Value {
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
    let schema = schema_for_fields(fields);
    let refs = RefCtx::new(Node::object(), None, Loader::JsonOrYaml);
    let errors: Vec<Value> = schema::validate(&schema, &data, &refs, ctx, "$", 0)
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

fn schema_for_fields(fields: &[Value]) -> Node {
    let mut properties = Node::object();
    let mut required = Node::array();
    for field in fields {
        let name = field.get("name").and_then(Value::as_str).unwrap_or("");
        let field_type = field
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("string");
        let mut property = Node::object();
        property.set("type", Node::Str(json_type(field_type)));
        if let Some(enum_values) = field.get("enum").and_then(Value::as_array) {
            if !enum_values.is_empty() {
                property.set(
                    "enum",
                    Node::Array(
                        enum_values
                            .iter()
                            .filter_map(|value| serde_json::from_value::<Node>(value.clone()).ok())
                            .collect(),
                    ),
                );
            }
        }
        if field_type == "array" {
            let mut items = Node::object();
            items.set(
                "type",
                Node::Str(json_type(
                    field
                        .get("childrenType")
                        .and_then(Value::as_str)
                        .unwrap_or("string"),
                )),
            );
            property.set("items", items);
        }
        if field_type == "object" {
            property = Node::object();
            property.set("type", Node::Str("object".to_string()));
            property.set("properties", Node::object());
        }
        properties.set(name, property);
        if field.get("require").and_then(Value::as_bool) == Some(true) {
            required.push(Node::Str(name.to_string()));
        }
    }
    let mut schema = Node::object();
    schema.set("type", Node::Str("object".to_string()));
    schema.set("properties", properties);
    schema.set("required", required);
    schema
}

fn json_type(field_type: &str) -> String {
    match field_type {
        "integer" | "number" | "boolean" | "array" | "object" | "string" => field_type.to_string(),
        _ => "string".to_string(),
    }
}

fn edm_type(field_type: &str) -> String {
    if field_type.starts_with("Collection(") {
        return "array".to_string();
    }
    TYPES
        .iter()
        .find(|(name, _)| *name == field_type)
        .map_or_else(|| "string".to_string(), |(_, kind)| (*kind).to_string())
}

fn local_name(qualified: &str) -> String {
    qualified
        .rsplit('.')
        .next()
        .unwrap_or(qualified)
        .to_string()
}
