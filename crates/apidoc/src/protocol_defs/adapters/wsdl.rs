//! Port of `php/src/protocol/Adapter/WsdlAdapter.php`.

use serde_json::{json, Value};

use crate::protocol_defs::adapters::{message, Adapter, Parsed};
use crate::protocol_defs::context::Context;
use crate::protocol_defs::node::Node;
use crate::protocol_defs::schema;
use crate::protocol_defs::token::ParseError;
use crate::protocol_defs::xml::{self, XmlDoc, XmlNode};

const XSD_SCALARS: [(&str, &str); 20] = [
    ("string", "string"),
    ("normalizedString", "string"),
    ("token", "string"),
    ("int", "integer"),
    ("integer", "integer"),
    ("long", "integer"),
    ("short", "integer"),
    ("byte", "integer"),
    ("unsignedInt", "integer"),
    ("unsignedLong", "integer"),
    ("unsignedShort", "integer"),
    ("decimal", "number"),
    ("float", "number"),
    ("double", "number"),
    ("boolean", "boolean"),
    ("dateTime", "string"),
    ("date", "string"),
    ("time", "string"),
    ("base64Binary", "string"),
    ("anyType", "string"),
];

pub struct WsdlAdapter;

#[derive(Clone)]
struct Operation {
    name: String,
    input: String,
    output: String,
}

#[derive(Clone)]
struct Candidate {
    label: String,
    service: String,
    port: String,
    port_type: String,
    operation: Operation,
}

impl Adapter for WsdlAdapter {
    fn parse(&self, ctx: &mut Context<'_>, content: &str) -> Result<Parsed, ParseError> {
        let document = load(content)?;
        if document.root.name != "definitions" {
            ctx.error("WSDL definitions element is missing", 0, 0);
            return Ok(Parsed::empty());
        }
        let definitions = &document.root;
        let mut elements = Node::object();
        let mut complex_types = Node::object();
        let mut simple_types = Node::object();
        let mut messages = Node::object();
        let mut port_types = Node::object();
        let mut bindings = Node::object();
        let mut services = Node::object();

        if let Some(types) = definitions.first("types") {
            for schema_node in types.all("schema") {
                parse_schema(
                    schema_node,
                    ctx,
                    &mut elements,
                    &mut complex_types,
                    &mut simple_types,
                )?;
            }
        }
        for message_node in definitions.all("message") {
            let name = message_node.attr("name").to_string();
            let mut parts = Node::array();
            for part in message_node.all("part") {
                let mut entry = Node::object();
                entry.set("name", Node::Str(part.attr("name").to_string()));
                entry.set("element", Node::Str(local_name(part.attr("element"))));
                entry.set("type", Node::Str(local_name(part.attr("type"))));
                parts.push(entry);
            }
            messages.set(name, parts);
        }
        for port_type in definitions.all("portType") {
            let name = port_type.attr("name").to_string();
            let mut operations = Node::array();
            for operation in port_type.all("operation") {
                let input = operation.first("input");
                let output = operation.first("output");
                let mut entry = Node::object();
                entry.set("name", Node::Str(operation.attr("name").to_string()));
                entry.set(
                    "input",
                    Node::Str(
                        input.map_or_else(String::new, |node| local_name(node.attr("message"))),
                    ),
                );
                entry.set(
                    "output",
                    Node::Str(
                        output.map_or_else(String::new, |node| local_name(node.attr("message"))),
                    ),
                );
                operations.push(entry);
            }
            port_types.set(name, operations);
        }
        for binding in definitions.all("binding") {
            bindings.set(
                binding.attr("name").to_string(),
                Node::Str(local_name(binding.attr("type"))),
            );
        }
        for service in definitions.all("service") {
            let mut ports = Node::array();
            for port in service.all("port") {
                let mut entry = Node::object();
                entry.set("name", Node::Str(port.attr("name").to_string()));
                entry.set("binding", Node::Str(local_name(port.attr("binding"))));
                ports.push(entry);
            }
            services.set(service.attr("name").to_string(), ports);
        }

        let candidate_list = candidates(&services, &bindings, &port_types);
        let wanted = ctx.options().operation.clone();
        let Some(selected) = select(&candidate_list, &wanted, ctx) else {
            return Ok(Parsed::empty());
        };
        let request_fields = message_fields(
            &selected.operation.input,
            &messages,
            &elements,
            &complex_types,
            &simple_types,
        );
        let response_fields = message_fields(
            &selected.operation.output,
            &messages,
            &elements,
            &complex_types,
            &simple_types,
        );
        let request_name = if selected.operation.input.is_empty() {
            "Request"
        } else {
            &selected.operation.input
        };
        let response_name = if selected.operation.output.is_empty() {
            "Response"
        } else {
            &selected.operation.output
        };
        let mut messages_out = vec![
            message(request_name, "request", request_fields),
            message(response_name, "response", response_fields),
        ];
        let sample = ctx.options().sample.clone();
        if !sample.is_empty() {
            messages_out[0]["sampleValidation"] = validate_sample(&sample, &selected.operation);
        }
        Ok(Parsed {
            operation: selected.label.clone(),
            messages: messages_out,
            meta: json!({
                "service": selected.service,
                "port": selected.port,
            }),
        })
    }
}

fn load(content: &str) -> Result<XmlDoc, ParseError> {
    xml::load(content).map_err(|error| ParseError {
        message: error.message,
        line: error.line,
        column: error.column,
    })
}

fn local_name(qualified: &str) -> String {
    qualified
        .rsplit(':')
        .next()
        .unwrap_or(qualified)
        .to_string()
}

fn parse_schema(
    schema_node: &XmlNode,
    ctx: &mut Context<'_>,
    elements: &mut Node,
    complex_types: &mut Node,
    simple_types: &mut Node,
) -> Result<(), ParseError> {
    for tag in ["include", "import"] {
        for include in schema_node.all(tag) {
            let location = include.attr("schemaLocation");
            if location.is_empty() {
                continue;
            }
            let Some(realpath) = ctx.resolve(location) else {
                ctx.warning(&format!("unable to resolve XSD {tag}: {location}"), 0, 0);
                continue;
            };
            if !ctx.push(realpath) {
                continue;
            }
            let content = ctx.read();
            if let Some(content) = content {
                let included = load(&content)?;
                parse_schema(&included.root, ctx, elements, complex_types, simple_types)?;
            }
            ctx.pop();
        }
    }
    for element in schema_node.all("element") {
        let name = element.attr("name");
        if name.is_empty() {
            continue;
        }
        let field_type = local_name(element.attr("type"));
        let inline = element.first("complexType");
        let simple = element.first("simpleType");
        let mut entry = Node::object();
        entry.set("name", Node::Str(name.to_string()));
        entry.set("type", Node::Str(field_type));
        entry.set(
            "fields",
            inline.map_or_else(Node::array, parse_complex_type),
        );
        entry.set("enum", simple.map_or_else(Node::array, parse_simple_type));
        elements.set(name, entry);
    }
    for complex_type in schema_node.all("complexType") {
        let name = complex_type.attr("name");
        if !name.is_empty() {
            complex_types.set(name, parse_complex_type(complex_type));
        }
    }
    for simple_type in schema_node.all("simpleType") {
        let name = simple_type.attr("name");
        if !name.is_empty() {
            simple_types.set(name, parse_simple_type(simple_type));
        }
    }
    Ok(())
}

fn parse_complex_type(complex_type: &XmlNode) -> Node {
    let mut fields = Node::array();
    let container = complex_type
        .first("sequence")
        .or_else(|| complex_type.first("all"));
    if let Some(container) = container {
        for element in container.all("element") {
            fields.push(parse_xsd_element(element));
        }
    }
    for attribute in complex_type.all("attribute") {
        let name = attribute.attr("name");
        if !name.is_empty() {
            let mut field = Node::object();
            field.set("name", Node::Str(name.to_string()));
            field.set(
                "type",
                Node::Str(xsd_type(&local_name(attribute.attr("type")))),
            );
            field.set("require", Node::Bool(attribute.attr("use") == "required"));
            fields.push(field);
        }
    }
    fields
}

fn parse_xsd_element(element: &XmlNode) -> Node {
    let name = element.attr("name");
    let field_type = local_name(element.attr("type"));
    let inline = element.first("complexType");
    let simple = element.first("simpleType");
    let min = element.attr("minOccurs");
    let max = element.attr("maxOccurs");
    let mut field = Node::object();
    field.set("name", Node::Str(name.to_string()));
    field.set(
        "type",
        Node::Str(if inline.is_some() {
            "object".to_string()
        } else {
            xsd_type(&field_type)
        }),
    );
    field.set("require", Node::Bool(min != "0"));
    if let Some(inline) = inline {
        field.set("children", parse_complex_type(inline));
    } else if !field_type.is_empty() {
        field.set("typeName", Node::Str(field_type));
    }
    if let Some(simple) = simple {
        field.set("enum", parse_simple_type(simple));
        field.set("type", Node::Str("string".to_string()));
    }
    if max == "unbounded"
        || (!max.is_empty()
            && max.bytes().all(|b| b.is_ascii_digit())
            && max.parse::<i64>().unwrap_or(0) > 1)
    {
        let current = field
            .get("type")
            .map(Node::scalar_string)
            .unwrap_or_else(|| "string".to_string());
        field.set("repeated", Node::Bool(true));
        field.set("childrenType", Node::Str(current));
        field.set("type", Node::Str("array".to_string()));
    }
    field
}

fn parse_simple_type(simple_type: &XmlNode) -> Node {
    let mut values = Node::array();
    for enumeration in simple_type.all("enumeration") {
        values.push(Node::Str(enumeration.attr("value").to_string()));
    }
    values
}

fn message_fields(
    message_name: &str,
    messages: &Node,
    elements: &Node,
    complex_types: &Node,
    simple_types: &Node,
) -> Vec<Value> {
    if message_name.is_empty() {
        return Vec::new();
    }
    let Some(parts) = messages.get(message_name).and_then(Node::as_array) else {
        return Vec::new();
    };
    let mut fields: Vec<Value> = Vec::new();
    for part in parts {
        let element_name = part
            .get("element")
            .map(Node::scalar_string)
            .unwrap_or_default();
        let type_name = part
            .get("type")
            .map(Node::scalar_string)
            .unwrap_or_default();
        if !element_name.is_empty() {
            if let Some(element) = elements.get(&element_name) {
                let element_fields = element.get("fields").cloned().unwrap_or_else(Node::array);
                fields.extend(
                    resolve_fields(&element_fields, elements, complex_types, simple_types, &[])
                        .into_iter()
                        .map(|field| field.to_json()),
                );
                continue;
            }
        }
        // PHP falls through to `type` when `element` is missing or unresolved.
        if !type_name.is_empty() {
            let name = part
                .get("name")
                .map(Node::scalar_string)
                .unwrap_or_default();
            fields.push(json!({
                "name": if name.is_empty() { "value" } else { &name },
                "type": xsd_type(&type_name),
            }));
        }
    }
    fields
}

fn resolve_fields(
    fields: &Node,
    elements: &Node,
    complex_types: &Node,
    simple_types: &Node,
    visited: &[String],
) -> Vec<Node> {
    let mut result = Vec::new();
    for field in schema::entries(fields).into_iter().map(|(_, value)| value) {
        let mut field = field;
        let type_name = field
            .get("typeName")
            .map(Node::scalar_string)
            .unwrap_or_default();
        if !type_name.is_empty()
            && complex_types.get(&type_name).is_some()
            && !visited.iter().any(|entry| entry == &type_name)
        {
            let mut next_visited = visited.to_vec();
            next_visited.push(type_name.clone());
            field.set("type", Node::Str("object".to_string()));
            let children = complex_types
                .get(&type_name)
                .map(|value| {
                    resolve_fields(value, elements, complex_types, simple_types, &next_visited)
                })
                .unwrap_or_default();
            field.set("children", Node::Array(children));
        } else if !type_name.is_empty() {
            if let Some(element) = elements.get(&type_name) {
                field.set("type", Node::Str("object".to_string()));
                let children = element
                    .get("fields")
                    .map(|value| {
                        resolve_fields(value, elements, complex_types, simple_types, visited)
                    })
                    .unwrap_or_default();
                field.set("children", Node::Array(children));
            }
        }
        if field.get("children").is_some_and(Node::truthy) {
            let children = field.get("children").cloned().unwrap_or_else(Node::array);
            let resolved =
                resolve_fields(&children, elements, complex_types, simple_types, visited);
            field.set("children", Node::Array(resolved));
        }
        result.push(field);
    }
    result
}

fn candidates(services: &Node, bindings: &Node, port_types: &Node) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for (service_name, ports) in schema::entries(services) {
        for port in schema::entries(&ports).into_iter().map(|(_, value)| value) {
            let binding = port
                .get("binding")
                .map(Node::scalar_string)
                .unwrap_or_default();
            let port_type = bindings
                .get(&binding)
                .map(Node::scalar_string)
                .unwrap_or_default();
            if let Some(operations) = port_types.get(&port_type).and_then(Node::as_array) {
                for operation in operations {
                    let operation = operation_from(operation);
                    candidates.push(Candidate {
                        label: format!(
                            "{service_name}/{}/{name}",
                            port.get("name")
                                .map(Node::scalar_string)
                                .unwrap_or_default(),
                            name = operation.name
                        ),
                        service: service_name.clone(),
                        port: port
                            .get("name")
                            .map(Node::scalar_string)
                            .unwrap_or_default(),
                        port_type: port_type.clone(),
                        operation,
                    });
                }
            }
        }
    }
    if candidates.is_empty() {
        for (port_type_name, operations) in schema::entries(port_types) {
            for operation in schema::entries(&operations)
                .into_iter()
                .map(|(_, value)| value)
            {
                let operation = operation_from(&operation);
                candidates.push(Candidate {
                    label: format!("{port_type_name}/{}", operation.name),
                    service: String::new(),
                    port: String::new(),
                    port_type: port_type_name.clone(),
                    operation,
                });
            }
        }
    }
    candidates
}

fn operation_from(operation: &Node) -> Operation {
    Operation {
        name: operation
            .get("name")
            .map(Node::scalar_string)
            .unwrap_or_default(),
        input: operation
            .get("input")
            .map(Node::scalar_string)
            .unwrap_or_default(),
        output: operation
            .get("output")
            .map(Node::scalar_string)
            .unwrap_or_default(),
    }
}

fn select<'a>(
    candidates: &'a [Candidate],
    wanted: &str,
    ctx: &mut Context<'_>,
) -> Option<&'a Candidate> {
    if !wanted.is_empty() {
        for candidate in candidates {
            if candidate.label == wanted
                || format!("{}/{}", candidate.port_type, candidate.operation.name) == wanted
                || candidate.operation.name == wanted
            {
                return Some(candidate);
            }
        }
        ctx.error(
            &format!(
                "operation not found: {wanted}; candidates: {}",
                candidate_labels(candidates)
            ),
            0,
            0,
        );
        return None;
    }
    if candidates.len() == 1 {
        return Some(&candidates[0]);
    }
    ctx.error(
        &format!(
            "operation required; candidates: {}",
            candidate_labels(candidates)
        ),
        0,
        0,
    );
    None
}

fn candidate_labels(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .map(|candidate| candidate.label.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn xsd_type(field_type: &str) -> String {
    if field_type.is_empty() {
        return "string".to_string();
    }
    XSD_SCALARS
        .iter()
        .find(|(name, _)| *name == field_type)
        .map_or_else(|| "object".to_string(), |(_, kind)| (*kind).to_string())
}

fn validate_sample(sample: &str, operation: &Operation) -> Value {
    let document = match xml::load(sample) {
        Ok(document) => document,
        Err(error) => {
            return json!({
                "ok": false,
                "errors": [{
                    "path": "$",
                    "severity": "error",
                    "message": error.message,
                }],
                "warnings": [],
            })
        }
    };
    let mut errors = Vec::new();
    if document.root.name == "Envelope" {
        let body = document.root.first("Body");
        let operation_element = body.and_then(XmlNode::first_element);
        if let Some(operation_element) = operation_element {
            let expected = operation.input.as_str();
            if !expected.is_empty() && operation_element.name != expected {
                errors.push(json!({
                    "path": "$",
                    "severity": "error",
                    "message": "SOAP Body operation mismatch",
                }));
            }
        } else {
            errors.push(json!({
                "path": "$",
                "severity": "error",
                "message": "SOAP Body is empty",
            }));
        }
    }
    json!({
        "ok": errors.is_empty(),
        "errors": errors,
        "warnings": [],
    })
}
