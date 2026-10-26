//! UIC-1 Phase 4: pure protocol definition adapters, with the PHP fixtures.

use std::path::Path;

use apidoc::protocol_defs::{detect, parse, Definition, Limits, Options, Resolver, Validation};
use serde_json::Value;

struct Fixtures;

impl Resolver for Fixtures {
    fn read(&mut self, path: &str) -> Option<(String, String)> {
        let full = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        std::fs::read_to_string(full)
            .ok()
            .map(|content| (path.to_string(), content))
    }
}

const ROOT: &str = "tests/fixtures/protocol";

fn parse_file(path: &str, format: &str, options: Options) -> Definition {
    let content = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .expect("fixture must exist");
    parse(
        format,
        path,
        &content,
        options,
        Limits::default(),
        &mut Fixtures,
    )
}

fn with_operation(operation: &str) -> Options {
    Options {
        operation: operation.to_string(),
        ..Options::default()
    }
}

fn fields(message: &Value) -> &Vec<Value> {
    message["fields"].as_array().expect("fields array")
}

fn field<'a>(message: &'a Value, index: usize) -> &'a Value {
    &fields(message)[index]
}

fn assert_diagnostic(
    value: &Value,
    path: &str,
    line: i64,
    column: i64,
    severity: &str,
    message: &str,
) {
    assert_eq!(
        value,
        &serde_json::json!({
            "path": path,
            "line": line,
            "column": column,
            "severity": severity,
            "message": message,
        }),
        "unexpected diagnostic: {value}"
    );
}

fn assert_error(validation: &Validation, message: &str) -> Value {
    assert!(!validation.ok, "definition must fail: {message}");
    let error = serde_json::to_value(&validation.errors[0]).unwrap();
    assert_eq!(error["path"], "");
    assert_eq!(error["severity"], "error");
    assert_eq!(error["message"], message);
    error
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn detects_all_formats() {
    assert_eq!(detect("x.proto", ""), "proto");
    assert_eq!(detect("x.thrift", ""), "thrift");
    assert_eq!(detect("x.graphql", ""), "graphql");
    assert_eq!(detect("x.gql", ""), "graphql");
    assert_eq!(detect("x.wsdl", ""), "wsdl");
    assert_eq!(detect("x.yaml", "openapi: 3.0.0"), "openapi");
    assert_eq!(detect("x.yaml", "\"swagger\": \"2.0\""), "openapi");
    assert_eq!(detect("x.json", "\"asyncapi\": \"2.6.0\""), "asyncapi");
    assert_eq!(detect("x.json", "\"methods\": {}"), "json-rpc");
    assert_eq!(detect("x.json", "\"$schema\": \"x\""), "json-schema");
    assert_eq!(detect("x.json", "\"type\": \"object\""), "json-schema");
    assert_eq!(detect("x.xml", "<edmx:Edmx/>"), "edmx");
    assert_eq!(detect("x.xml", "<definitions xmlns=\"wsdl\"/>"), "wsdl");
    assert_eq!(detect("x.xml", "<methodCall/>"), "xml-rpc");
    assert_eq!(detect("x.xml", "<methodResponse/>"), "xml-rpc");
    assert_eq!(detect("x.xml", "<Envelope/>"), "wsdl");
    assert_eq!(detect("x.txt", ""), "");
}

// ---------------------------------------------------------------------------
// One success + one error per format
// ---------------------------------------------------------------------------

#[test]
fn openapi_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/openapi/api.yaml"),
        "openapi",
        with_operation("getUser"),
    );
    assert_eq!(definition.operation, "getUser");
    assert!(definition.validation.ok);
    assert_eq!(field(&definition.messages[0], 0)["name"], "id");
    assert_eq!(field(&definition.messages[0], 0)["type"], "integer");
    assert_eq!(field(&definition.messages[0], 0)["require"], true);
    assert_eq!(definition.messages[1]["name"], "Response 200");
    assert_eq!(field(&definition.messages[1], 1)["name"], "name");

    let bad = parse_file(
        &format!("{ROOT}/openapi/api.yaml"),
        "openapi",
        with_operation("nope"),
    );
    assert_error(
        &bad.validation,
        "operation not found: nope; candidates: getUser, createUser",
    );
}

#[test]
fn asyncapi_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/asyncapi/asyncapi.yaml"),
        "asyncapi",
        with_operation("sensors/+/temp:publish"),
    );
    assert_eq!(definition.operation, "sensors/+/temp:publish");
    assert!(definition.validation.ok);
    assert_eq!(definition.messages[0]["name"], "Temperature");
    assert_eq!(definition.messages[0]["direction"], "event");
    assert_eq!(field(&definition.messages[0], 0)["type"], "number");
    assert_eq!(field(&definition.messages[0], 1)["type"], "string");

    let bad = parse_file(
        &format!("{ROOT}/asyncapi/asyncapi.yaml"),
        "asyncapi",
        with_operation("nope"),
    );
    assert_error(
        &bad.validation,
        "operation not found: nope; candidates: sensors/+/temp:publish",
    );
}

#[test]
fn graphql_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/graphql/schema.graphql"),
        "graphql",
        with_operation("Query.user"),
    );
    assert_eq!(definition.operation, "Query.user");
    assert!(definition.validation.ok);
    assert_eq!(field(&definition.messages[0], 0)["require"], true);
    assert_eq!(field(&definition.messages[1], 2)["type"], "array");
    assert_eq!(field(&definition.messages[1], 2)["childrenType"], "string");

    let bad = parse_file(
        &format!("{ROOT}/graphql/schema.graphql"),
        "graphql",
        with_operation("Query.nope"),
    );
    assert_error(
        &bad.validation,
        "operation not found: Query.nope; candidates: Query.user",
    );
}

#[test]
fn proto_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/proto/helloworld.proto"),
        "proto",
        with_operation("helloworld.Greeter/SayHello"),
    );
    assert_eq!(definition.operation, "helloworld.Greeter/SayHello");
    assert_eq!(definition.meta["package"], "helloworld");
    assert!(definition.validation.ok);
    assert_eq!(field(&definition.messages[0], 0)["fieldNumber"], 1);
    assert_eq!(
        field(&definition.messages[0], 1)["children"][0]["name"],
        "trace_id"
    );
    assert_eq!(field(&definition.messages[0], 2)["type"], "array");

    let bad = parse_file(
        &format!("{ROOT}/proto/unsupported.proto"),
        "proto",
        Options::default(),
    );
    assert!(!bad.validation.ok);
    assert_eq!(bad.validation.errors.len(), 2);
    assert_diagnostic(
        &serde_json::to_value(&bad.validation.errors[0]).unwrap(),
        "",
        2,
        1,
        "error",
        "unsupported construct: extend",
    );
    assert_diagnostic(
        &serde_json::to_value(&bad.validation.errors[1]).unwrap(),
        "",
        0,
        0,
        "error",
        "operation required; candidates: ",
    );
}

#[test]
fn wsdl_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/wsdl/calculator.wsdl"),
        "wsdl",
        with_operation("CalculatorService/CalculatorPort/Add"),
    );
    assert_eq!(definition.operation, "CalculatorService/CalculatorPort/Add");
    assert_eq!(definition.meta["service"], "CalculatorService");
    assert_eq!(definition.meta["port"], "CalculatorPort");
    assert!(definition.validation.ok);
    assert_eq!(field(&definition.messages[0], 0)["type"], "integer");
    assert_eq!(field(&definition.messages[0], 0)["typeName"], "int");
    assert_eq!(field(&definition.messages[1], 0)["name"], "result");

    let bad = parse_file(
        &format!("{ROOT}/wsdl/calculator.wsdl"),
        "wsdl",
        with_operation("Nope"),
    );
    assert_error(
        &bad.validation,
        "operation not found: Nope; candidates: CalculatorService/CalculatorPort/Add",
    );
}

#[test]
fn edmx_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/edmx/metadata.xml"),
        "edmx",
        with_operation("Products"),
    );
    assert_eq!(definition.operation, "Products");
    assert_eq!(definition.meta["direction"], "response");
    assert!(definition.validation.ok);
    assert_eq!(field(&definition.messages[0], 0)["require"], true);
    assert_eq!(field(&definition.messages[0], 2)["type"], "number");

    let bad = parse_file(
        &format!("{ROOT}/edmx/metadata.xml"),
        "edmx",
        with_operation("Nope"),
    );
    assert_error(
        &bad.validation,
        "operation not found: Nope; candidates: Products",
    );
}

#[test]
fn json_schema_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/json-schema/user.json"),
        "json-schema",
        Options {
            sample: r#"{"id":1,"name":"a"}"#.to_string(),
            ..Options::default()
        },
    );
    assert_eq!(definition.messages[0]["name"], "User");
    assert_eq!(definition.messages[0]["sampleValidation"]["ok"], true);
    assert_eq!(field(&definition.messages[0], 0)["require"], true);
    assert_eq!(field(&definition.messages[0], 2)["type"], "array");

    let bad = parse_file(
        &format!("{ROOT}/json-schema/invalid.json"),
        "json-schema",
        Options::default(),
    );
    assert_error(&bad.validation, "JSON parse error: Syntax error");
    assert!(bad.messages.is_empty());
}

#[test]
fn json_rpc_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/json-rpc/methods.json"),
        "json-rpc",
        Options {
            operation: "user.get".to_string(),
            sample: r#"{"jsonrpc":"2.0","method":"user.get","params":{"id":1},"id":1}"#.to_string(),
            ..Options::default()
        },
    );
    assert_eq!(definition.operation, "user.get");
    assert_eq!(definition.messages[0]["sampleValidation"]["ok"], true);
    assert_eq!(field(&definition.messages[1], 1)["name"], "name");

    let bad = parse_file(
        &format!("{ROOT}/json-rpc/methods.json"),
        "json-rpc",
        Options {
            operation: "user.get".to_string(),
            sample: r#"{"jsonrpc":"2.0","method":"user.get","params":{},"id":1}"#.to_string(),
            ..Options::default()
        },
    );
    let validation = &bad.messages[0]["sampleValidation"];
    assert_eq!(validation["ok"], false);
    assert_eq!(validation["errors"][0]["path"], "$.id");
    assert_eq!(validation["errors"][0]["severity"], "error");
    assert_eq!(validation["errors"][0]["message"], "is required");
}

#[test]
fn xml_rpc_success_and_error() {
    let definition = parse_file(
        &format!("{ROOT}/xml-rpc/methods.xml"),
        "xml-rpc",
        with_operation("add"),
    );
    assert_eq!(definition.operation, "add");
    assert!(definition.validation.ok);
    assert_eq!(field(&definition.messages[0], 0)["name"], "a");
    assert_eq!(field(&definition.messages[1], 0)["type"], "integer");

    let bad_sample = parse_file(
        &format!("{ROOT}/xml-rpc/methods.xml"),
        "xml-rpc",
        Options {
            operation: "add".to_string(),
            sample: "<methodCall><methodName>other</methodName></methodCall>".to_string(),
            ..Options::default()
        },
    );
    assert_eq!(bad_sample.messages[0]["sampleValidation"]["ok"], false);
    assert_eq!(
        bad_sample.messages[0]["sampleValidation"]["errors"][0]["message"],
        "methodName mismatch"
    );

    let bad_operation = parse_file(
        &format!("{ROOT}/xml-rpc/methods.xml"),
        "xml-rpc",
        with_operation("nope"),
    );
    assert_error(
        &bad_operation.validation,
        "operation not found: nope; candidates: add",
    );
}

// ---------------------------------------------------------------------------
// Safety and caps
// ---------------------------------------------------------------------------

#[test]
fn unknown_format_has_stable_error() {
    let definition = parse(
        "unknown",
        "x.unknown",
        "",
        Options::default(),
        Limits::default(),
        &mut Fixtures,
    );
    assert_eq!(definition.format, "unknown");
    assert_error(
        &definition.validation,
        "unsupported definition format: unknown",
    );
    assert!(definition.messages.is_empty());
}

#[test]
fn size_and_line_limits_are_enforced() {
    let content = "{\n\"type\":\"object\"\n}";
    let by_bytes = parse(
        "json-schema",
        "x.json",
        content,
        Options::default(),
        Limits {
            max_file_bytes: 1,
            ..Limits::default()
        },
        &mut Fixtures,
    );
    assert_error(&by_bytes.validation, "file exceeds max_file_bytes");

    let by_lines = parse(
        "json-schema",
        "x.json",
        content,
        Options::default(),
        Limits {
            max_lines: 1,
            ..Limits::default()
        },
        &mut Fixtures,
    );
    assert_error(&by_lines.validation, "file exceeds max_lines");
}

#[test]
fn xml_doctype_is_rejected() {
    let definition = parse(
        "edmx",
        "x.xml",
        r#"<!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><edmx:Edmx/>"#,
        Options::default(),
        Limits::default(),
        &mut Fixtures,
    );
    assert_error(&definition.validation, "DOCTYPE is not allowed");
}
