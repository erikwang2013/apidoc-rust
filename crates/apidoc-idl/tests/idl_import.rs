use apidoc_idl::{proto, thrift_idl, MAX_INPUT};

#[test]
fn proto_snapshot_is_stable() {
    let generated = proto::generate(include_str!("fixtures/helloworld.proto"), None).unwrap();
    assert_eq!(generated, include_str!("fixtures/helloworld.rs"));
}

#[test]
fn proto_package_override_only_changes_the_service_name() {
    let generated =
        proto::generate(include_str!("fixtures/helloworld.proto"), Some("acme.api")).unwrap();
    assert!(generated.contains(r#"service = "acme.api.Greeter""#), "{generated}");
    assert!(generated.contains(r#"ty = "string", field = "1", required"#));
}

#[test]
fn proto_maps_and_oneofs_become_todo_comments() {
    let source = r#"
syntax = "proto3";
package demo;
message Child { string id = 1; }
message Req {
  map<string, string> labels = 1;
  oneof choice {
    string a = 2;
    int32 b = 3;
  }
}
service S { rpc Call (Req) returns (Child); }
"#;
    let generated = proto::generate(source, None).unwrap();
    assert!(generated.contains("// TODO: map unsupported: field `labels`"), "{generated}");
    assert!(generated.contains("// TODO: oneof unsupported: field `a`"), "{generated}");
    assert!(generated.contains("// TODO: oneof unsupported: field `b`"), "{generated}");
    assert!(generated.contains("fn call() {"), "{generated}");
}

#[test]
fn proto_allows_builtin_well_known_imports() {
    let source = r#"
syntax = "proto3";
package demo;
import "google/protobuf/timestamp.proto";
message M { google.protobuf.Timestamp at = 1; }
service S { rpc Call (M) returns (M); }
"#;
    let generated = proto::generate(source, None).unwrap();
    assert!(generated.contains(r#"name = "at", ty = "object", field = "1""#), "{generated}");
}

#[test]
fn proto_rejects_non_builtin_imports_without_reading_them() {
    for path in ["http://evil.example/x.proto", "../shared.proto"] {
        let source = format!("syntax = \"proto3\";\nimport \"{path}\";\n");
        let err = proto::generate(&source, None).unwrap_err();
        assert!(err.contains("line 2") && err.contains("not supported"), "{err}");
    }
}

#[test]
fn deeply_nested_proto_messages_stay_bounded() {
    let mut source = String::from("syntax = \"proto3\";\npackage demo;\n");
    for i in 0..70 {
        source.push_str(&format!("message M{i} {{ "));
    }
    source.push_str("string leaf = 1;");
    for _ in 0..70 {
        source.push_str(" }");
    }
    source.push_str("\nservice S { rpc Call (M0) returns (M0); }\n");
    let generated = proto::generate(&source, None).unwrap();
    assert!(generated.contains("fn call()"), "{generated}");
}

#[test]
fn thrift_snapshot_is_stable() {
    let generated = thrift_idl::generate(include_str!("fixtures/service.thrift")).unwrap();
    assert_eq!(generated, include_str!("fixtures/service.rs"));
}

#[test]
fn thrift_errors_carry_line_numbers() {
    let err = thrift_idl::generate("struct X {\n  1: string @\n}").unwrap_err();
    assert!(err.contains("line 2"), "{err}");
}

#[test]
fn oversized_input_is_rejected_before_parsing() {
    let huge = " ".repeat(MAX_INPUT + 1);
    assert!(proto::generate(&huge, None).unwrap_err().contains("limit"));
    assert!(thrift_idl::generate(&huge).unwrap_err().contains("limit"));
}

#[test]
fn cli_writes_the_generated_file() {
    let dir = std::env::temp_dir().join(format!("apidoc-idl-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("helloworld.proto");
    let output = dir.join("generated.rs");
    std::fs::write(&input, include_str!("fixtures/helloworld.proto")).unwrap();

    let bin = env!("CARGO_BIN_EXE_apidoc-idl");
    let status = std::process::Command::new(bin)
        .args(["proto", "--in"])
        .arg(&input)
        .arg("--out")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(&output).unwrap(), include_str!("fixtures/helloworld.rs"));

    // Missing --out is a usage error, not a panic.
    let status =
        std::process::Command::new(bin).arg("proto").arg("--in").arg(&input).status().unwrap();
    assert!(!status.success());
    std::fs::remove_dir_all(&dir).ok();
}
