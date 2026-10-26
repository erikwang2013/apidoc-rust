//! Cross-backend golden comparison for the shared PHP protocol fixtures.
//!
//! `php.json` was produced by the PHP reference (`php/src/protocol`) with the
//! same fixtures/options; it is checked in so CI does not need PHP installed.

use std::path::Path;

use apidoc::protocol_defs::{parse, Limits, Options, Resolver};
use serde_json::{json, Value};

struct Fixtures;

impl Resolver for Fixtures {
    fn read(&mut self, path: &str) -> Option<(String, String)> {
        let full = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        std::fs::read_to_string(full)
            .ok()
            .map(|content| (path.to_string(), content))
    }
}

const ROOT: &str = "tests/fixtures/protocol/parity";

fn normalize_meta(value: &Value) -> Value {
    if value.as_array().is_some_and(Vec::is_empty) {
        json!({})
    } else {
        value.clone()
    }
}

#[test]
fn shared_fixtures_match_php_reference() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(ROOT);
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("cases.json")).unwrap()).unwrap();
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("php.json")).unwrap()).unwrap();
    let cases = cases.as_array().expect("cases array");
    assert_eq!(
        cases.len(),
        expected.as_object().expect("expected object").len()
    );

    for case in cases {
        let name = case["name"].as_str().expect("case name");
        let format = case["format"].as_str().expect("case format");
        let file = case["file"].as_str().expect("case file");
        let content = match case.get("content").and_then(Value::as_str) {
            Some(content) => content.to_string(),
            None => std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file))
                .unwrap_or_else(|error| panic!("{name}: read {file}: {error}")),
        };
        let options = Options {
            operation: case["operation"].as_str().unwrap_or("").to_string(),
            message: case["message"].as_str().unwrap_or("").to_string(),
            direction: case["direction"].as_str().unwrap_or("").to_string(),
            sample: case["sample"].as_str().unwrap_or("").to_string(),
            sample_encoding: case["sample_encoding"].as_str().unwrap_or("").to_string(),
            strict: case.get("strict").and_then(Value::as_bool).unwrap_or(true),
        };
        let definition = parse(
            format,
            file,
            &content,
            options,
            Limits::default(),
            &mut Fixtures,
        );
        let actual = json!({
            "operation": definition.operation,
            "messages": definition.messages,
            "meta": definition.meta,
            "errors": definition.validation.errors,
            "warnings": definition.validation.warnings,
        });
        let want = &expected[name];
        assert_eq!(
            actual,
            json!({
                "operation": want["operation"],
                "messages": want["messages"],
                "meta": normalize_meta(&want["meta"]),
                "errors": want["errors"],
                "warnings": want["warnings"],
            }),
            "{name}"
        );
    }
}
