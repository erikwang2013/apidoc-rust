//! Pure external protocol definition parsing (UIC-1 Phase 4).
//!
//! Mirrors `php/src/protocol`: detection by path/content, one adapter per
//! format, diagnostics with `{path,line,column,severity,message}`. Imported
//! documents are supplied by a caller-provided [`Resolver`]; this module never
//! touches the filesystem or network.

pub mod adapters;
pub mod context;
pub mod node;
pub mod schema;
pub mod token;
pub mod xml;
pub mod yaml;

use serde::Serialize;
use serde_json::Value;

pub use context::{Diagnostic, Limits, NoResolver, Options, Resolver, Validation};

use adapters::{
    asyncapi::AsyncApiAdapter, edmx::EdmxAdapter, graphql::GraphqlAdapter,
    json_rpc::JsonRpcAdapter, json_schema::JsonSchemaAdapter, openapi::OpenApiAdapter,
    proto::ProtoAdapter, wsdl::WsdlAdapter, xml_rpc::XmlRpcAdapter, Adapter,
};
use context::Context;

/// Formats implemented by this module.
pub const FORMATS: [&str; 9] = [
    "openapi",
    "asyncapi",
    "graphql",
    "proto",
    "wsdl",
    "edmx",
    "json-schema",
    "json-rpc",
    "xml-rpc",
];

/// UIC-1 definition: `{format,source,operation,meta,messages,validation}`.
#[derive(Serialize)]
pub struct Definition {
    pub format: String,
    pub source: String,
    pub operation: String,
    pub meta: Value,
    pub messages: Vec<Value>,
    pub validation: Validation,
}

/// Detect a definition format from the declared path and (for structured
/// formats) the content — same rules as `ProtocolRegistry::detect`.
pub fn detect(relative_path: &str, content: &str) -> String {
    let basename = relative_path.rsplit('/').next().unwrap_or(relative_path);
    let extension = basename
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "proto" => return "proto".to_string(),
        "thrift" => return "thrift".to_string(),
        "graphql" | "gql" => return "graphql".to_string(),
        "wsdl" => return "wsdl".to_string(),
        "json" | "yaml" | "yml" => return detect_structured(content),
        "xml" => return detect_xml(content),
        _ => return String::new(),
    }
}

fn detect_structured(content: &str) -> String {
    for (key, format) in [
        ("openapi", "openapi"),
        ("swagger", "openapi"),
        ("asyncapi", "asyncapi"),
        ("methods", "json-rpc"),
    ] {
        if has_key_colon(content, key) {
            return format.to_string();
        }
    }
    for key in ["$schema", "$ref", "type", "properties"] {
        if has_key_colon(content, key) {
            return "json-schema".to_string();
        }
    }
    String::new()
}

/// `["']?key["']?\s*:` — the quote before the key is optional in the PHP regex,
/// so a plain occurrence of the key followed by a colon is enough.
fn has_key_colon(content: &str, key: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    let key = key.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(index) = lowered[offset..].find(&key) {
        let after = offset + index + key.len();
        let rest = &content[after..];
        let rest = rest
            .trim_start_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c' | '\x0b'));
        let rest = rest
            .strip_prefix('"')
            .or_else(|| rest.strip_prefix('\''))
            .unwrap_or(rest);
        let rest = rest
            .trim_start_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c' | '\x0b'));
        if rest.starts_with(':') {
            return true;
        }
        offset = after;
    }
    false
}

fn detect_xml(content: &str) -> String {
    let lowered = content.to_ascii_lowercase();
    if lowered.contains("<edmx:") {
        return "edmx".to_string();
    }
    if lowered.contains("<definitions") && lowered.contains("wsdl") {
        return "wsdl".to_string();
    }
    if lowered.contains("<methodcall") || lowered.contains("<methodresponse") {
        return "xml-rpc".to_string();
    }
    if lowered.contains("<envelope") || lowered.contains("<definitions") {
        return "wsdl".to_string();
    }
    String::new()
}

/// Parse `content` as `format`. `source` is the declared path (used for
/// relative imports and reported as `source`). Unknown formats record
/// `unsupported definition format: <format>` (error in strict mode).
pub fn parse(
    format: &str,
    source: &str,
    content: &str,
    options: Options,
    limits: Limits,
    resolver: &mut dyn Resolver,
) -> Definition {
    let mut ctx = Context::new(
        source.to_string(),
        content.to_string(),
        options,
        limits,
        resolver,
    );
    let mut operation = String::new();
    let mut meta = Value::Object(Default::default());
    let mut messages = Vec::new();
    if let Some(read) = ctx.read() {
        let parsed = match format {
            "openapi" => Some(OpenApiAdapter.parse(&mut ctx, &read)),
            "asyncapi" => Some(AsyncApiAdapter.parse(&mut ctx, &read)),
            "graphql" => Some(GraphqlAdapter.parse(&mut ctx, &read)),
            "proto" => Some(ProtoAdapter.parse(&mut ctx, &read)),
            "wsdl" => Some(WsdlAdapter.parse(&mut ctx, &read)),
            "edmx" => Some(EdmxAdapter.parse(&mut ctx, &read)),
            "json-schema" => Some(JsonSchemaAdapter.parse(&mut ctx, &read)),
            "json-rpc" => Some(JsonRpcAdapter.parse(&mut ctx, &read)),
            "xml-rpc" => Some(XmlRpcAdapter.parse(&mut ctx, &read)),
            _ => {
                ctx.unsupported(&format!("unsupported definition format: {format}"), 0, 0);
                None
            }
        };
        if let Some(parsed) = parsed {
            match parsed {
                Ok(parsed) => {
                    operation = parsed.operation;
                    meta = parsed.meta;
                    messages = parsed.messages;
                }
                Err(error) => ctx.error(&error.message, error.line, error.column),
            }
        }
        if !ctx.options().operation.is_empty() {
            operation = ctx.options().operation.clone();
        }
    }
    let errors = ctx.errors().to_vec();
    let warnings = ctx.warnings().to_vec();
    Definition {
        format: format.to_string(),
        source: source.to_string(),
        operation,
        meta,
        messages,
        validation: Validation {
            ok: errors.is_empty(),
            errors,
            warnings,
        },
    }
}
