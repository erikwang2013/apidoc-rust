//! P0 协议注解：protocol / event / field 的聚合、别名归一与序列化省略规则。

use apidoc::{ApiDoc, ApidocConfig, DebugConfig, DocRegistry};
use serde_json::Value;

#[allow(dead_code)]
#[apidoc::title("聊天")]
#[apidoc::protocol("websocket", target = "wss://example.com/chat", room = "lobby")]
#[apidoc::event(direction = "out", name = "join", ty = "json", desc = "加入房间")]
#[apidoc::event(direction = "in", name = "message", ty = "json", desc = "收到消息", mock = "{\"text\":\"hi\"}")]
fn chat() {}

#[allow(dead_code)]
#[apidoc::title("SayHello")]
#[apidoc::protocol("grpc", target = "http://127.0.0.1:50051", service = "helloworld.Greeter", method = "SayHello")]
#[apidoc::param(name = "name", ty = "string", field = "1", required)]
#[apidoc::returned(name = "message", ty = "string", field = "1")]
fn say_hello() {}

#[allow(dead_code)]
#[apidoc::title("REST 别名")]
#[apidoc::protocol("REST")]
#[apidoc::url("/api/rest")]
fn rest_alias() {}

#[allow(dead_code)]
#[apidoc::title("普通 HTTP")]
#[apidoc::url("/api/plain")]
#[apidoc::param(name = "q", ty = "string")]
fn plain_http() {}

fn ep<'a>(doc: &'a ApiDoc, title: &str) -> &'a apidoc::DocEndpoint {
    doc.endpoints.iter().find(|e| e.title == title).expect("endpoint not found")
}

#[test]
fn protocol_opts_and_events_aggregate() {
    let doc = DocRegistry::collect_doc(ApidocConfig::default());
    let chat = ep(&doc, "聊天");
    assert_eq!(chat.protocol, "websocket");
    assert_eq!(chat.protocol_opts.get("target").map(String::as_str), Some("wss://example.com/chat"));
    assert_eq!(chat.protocol_opts.get("room").map(String::as_str), Some("lobby"));
    assert_eq!(chat.events.len(), 2);
    assert_eq!(chat.events[0].direction, "out");
    assert_eq!(chat.events[0].name, "join");
    assert_eq!(chat.events[0].ty, "json");
    assert_eq!(chat.events[0].desc, Some("加入房间"));
    assert_eq!(chat.events[1].direction, "in");
    assert_eq!(chat.events[1].mock, Some("{\"text\":\"hi\"}"));
}

#[test]
fn field_number_lands_on_param_and_returned() {
    let doc = DocRegistry::collect_doc(ApidocConfig::default());
    let hello = ep(&doc, "SayHello");
    assert_eq!(hello.protocol, "grpc");
    assert_eq!(hello.protocol_opts.get("service").map(String::as_str), Some("helloworld.Greeter"));
    assert_eq!(hello.params[0].field, Some("1"));
    assert_eq!(hello.returned[0].field, Some("1"));
}

#[test]
fn alias_normalizes_to_canonical_kind() {
    let doc = DocRegistry::collect_doc(ApidocConfig::default());
    assert_eq!(ep(&doc, "REST 别名").protocol, "http");
}

#[test]
fn default_http_output_omits_protocol_fields() {
    let doc = DocRegistry::collect_doc(ApidocConfig::default());
    let plain = serde_json::to_value(ep(&doc, "普通 HTTP")).unwrap();
    assert!(plain.get("protocol").is_none(), "http 默认协议不应出现 protocol 字段");
    assert!(plain.get("protocol_opts").is_none());
    assert!(plain.get("events").is_none());
    assert!(plain["params"][0].get("field").is_none(), "未设置 field 时不应出现字段");

    let chat = serde_json::to_value(ep(&doc, "聊天")).unwrap();
    assert_eq!(chat["protocol"], "websocket");
    assert_eq!(chat["protocol_opts"]["target"], "wss://example.com/chat");
    assert_eq!(chat["events"][1]["name"], "message");
}

#[test]
fn debug_config_serializes_only_safe_subset() {
    let cfg = ApidocConfig {
        title: "t".into(),
        debug: Some(DebugConfig {
            enable_server_side: true,
            allow_private_networks: true,
            allow_hosts: vec!["internal.example".into()],
            allow_unix_sockets: true,
            timeout_ms: 1234,
            max_bytes: 99,
        }),
        ..Default::default()
    };
    let v: Value = serde_json::to_value(&cfg).unwrap();
    assert_eq!(v["debug"]["enable_server_side"], true);
    assert_eq!(v["debug"]["timeout_ms"], 1234);
    assert_eq!(v["debug"]["max_bytes"], 99);
    // 白名单/内网开关是攻击面信息，绝不进 api.json
    assert!(v["debug"].get("allow_hosts").is_none());
    assert!(v["debug"].get("allow_private_networks").is_none());
    assert!(v["debug"].get("allow_unix_sockets").is_none());
}
