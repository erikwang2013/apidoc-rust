//! AsyncAPI 2.6 导出：只覆盖有 channel 语义的流式/事件协议。
//! HTTP 族与一元 RPC（grpc/thrift/dubbo…）没有 AsyncAPI 位置，直接跳过。
//!
//! 方向约定与文档模型一致：`events[].direction == "out"` = 客户端发送 →
//! AsyncAPI `publish`；`"in"` = 客户端接收 → `subscribe`。
//! 消息 payload 复用 swagger 的 `schema_for`（params 供 publish，returned 供 subscribe）。

use crate::export::{swagger, NameAllocator};
use crate::{ApiDoc, DocEndpoint, DocEvent, DocParam};
use serde_json::{json, Map, Value};

/// 有 AsyncAPI channel 语义的协议。其余协议（含未知 kind）不导出。
const ASYNC_PROTOCOLS: &[&str] = &[
    "websocket",
    "ws-binary",
    "sse",
    "socketio",
    "stomp",
    "stomp-binary",
    "mqtt",
    "amqp",
    "kafka",
    "webhook",
];

pub fn render(doc: &ApiDoc, version: &str) -> Value {
    let mut channels = Map::new();
    let mut names = NameAllocator::new();
    for ep in doc.endpoints.iter().filter(|e| !e.not_debug) {
        if crate::protocol::spec(&ep.protocol).is_none() {
            eprintln!("apidoc: asyncapi skipped unknown protocol kind `{}`", ep.protocol);
            continue;
        }
        if !ASYNC_PROTOCOLS.contains(&ep.protocol.as_str()) {
            continue;
        }
        let Some(raw) = channel_name(ep) else {
            eprintln!("apidoc: asyncapi skipped `{}`: no topic/target/url", ep.title);
            continue;
        };
        let name = names.alloc(raw);
        channels.insert(name.clone(), render_channel(ep, &name));
    }
    let mut info = json!({ "title": doc.config.title, "version": version });
    if let Some(desc) = &doc.config.description {
        info["description"] = Value::String(desc.clone());
    }
    json!({
        "asyncapi": "2.6.0",
        "info": info,
        "channels": Value::Object(channels),
    })
}

/// `topic` 优先（MQTT/Kafka），其次连接目标，最后退回文档 url。
fn channel_name(ep: &DocEndpoint) -> Option<String> {
    for key in ["topic", "target"] {
        if let Some(v) = ep.protocol_opts.get(key).filter(|v| !v.is_empty()) {
            return Some(v.clone());
        }
    }
    (!ep.url.is_empty()).then(|| ep.url.clone())
}

fn render_channel(ep: &DocEndpoint, channel: &str) -> Value {
    let mut ch = Map::new();
    if let Some(op) = direction_op(ep, channel, "out", "publish") {
        ch.insert("publish".into(), op);
    }
    if let Some(op) = direction_op(ep, channel, "in", "subscribe") {
        ch.insert("subscribe".into(), op);
    }
    if ch.is_empty() {
        // 没有事件声明的协议（如 webhook）：按「客户端接收」出一条 message。
        ch.insert("subscribe".into(), synthetic_subscribe(ep, channel));
    }
    Value::Object(ch)
}

fn direction_op(ep: &DocEndpoint, channel: &str, dir: &str, op_name: &str) -> Option<Value> {
    let events: Vec<&DocEvent> = ep.events.iter().filter(|e| e.direction == dir).collect();
    if events.is_empty() {
        return None;
    }
    let messages: Vec<Value> = events
        .iter()
        .map(|event| {
            let mut msg = Map::new();
            msg.insert("name".into(), Value::String(event.name.to_string()));
            if let Some(desc) = event.desc {
                msg.insert("description".into(), Value::String(desc.to_string()));
            }
            msg.insert("payload".into(), message_payload(ep, dir, event));
            Value::Object(msg)
        })
        .collect();
    let mut op = Map::new();
    op.insert("operationId".into(), Value::String(operation_id(channel, op_name)));
    // AsyncAPI 2.6: 单个用 `message`，多个用 `messages` 数组。
    if messages.len() == 1 {
        op.insert("message".into(), messages.into_iter().next().expect("len == 1"));
    } else {
        op.insert("messages".into(), Value::Array(messages));
    }
    Some(Value::Object(op))
}

fn synthetic_subscribe(ep: &DocEndpoint, channel: &str) -> Value {
    let fields: &[DocParam] =
        if ep.returned.is_empty() { &ep.params } else { &ep.returned };
    let payload = if fields.is_empty() {
        json!({ "type": "object" })
    } else {
        swagger::schema_for(fields)
    };
    json!({
        "operationId": operation_id(channel, "subscribe"),
        "message": { "name": ep.title, "payload": payload },
    })
}

fn message_payload(ep: &DocEndpoint, dir: &str, event: &DocEvent) -> Value {
    let fields: &[DocParam] = if dir == "out" { &ep.params } else { &ep.returned };
    if fields.is_empty() {
        // 没有字段模型时按事件类型给最小 schema。
        match event.ty {
            "json" => json!({ "type": "object" }),
            "binary" => json!({ "type": "string", "format": "binary" }),
            _ => json!({ "type": "string" }),
        }
    } else {
        swagger::schema_for(fields)
    }
}

/// channel 名 → 合法 operationId（非字母数字折叠成单个 `_`）。
fn operation_id(channel: &str, direction: &str) -> String {
    let mut out = String::new();
    for c in channel.chars() {
        let ch = if c.is_ascii_alphanumeric() { c } else { '_' };
        if ch == '_' && out.ends_with('_') {
            continue;
        }
        out.push(ch);
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        format!("channel_{direction}")
    } else {
        format!("{out}_{direction}")
    }
}

#[cfg(test)]
mod tests {
    use super::operation_id;

    #[test]
    fn operation_ids_are_stable_and_legal() {
        assert_eq!(operation_id("wss://example.com/chat", "publish"), "wss_example_com_chat_publish");
        assert_eq!(operation_id("sensors/temp", "subscribe"), "sensors_temp_subscribe");
        assert_eq!(operation_id("///", "publish"), "channel_publish");
    }
}
