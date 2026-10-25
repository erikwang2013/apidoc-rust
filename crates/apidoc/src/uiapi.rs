//! UIC-1: the shared contract consumed by apidoc-ui.
//!
//! This module is intentionally a thin mapping over the existing `Core`:
//! it does not change `api.json`, the native UI, or any executor semantics.

mod debug;

use std::collections::{BTreeMap, HashMap};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::route_core::{Core, R};
use crate::{auth as crate_auth, find_app, protocol, AppDoc, DocEndpoint, DocParam};

const CONTRACT: &str = "1.0";

#[derive(Deserialize)]
struct ScopeBody {
    #[serde(rename = "appKey", default)]
    app_key: String,
}

#[derive(Deserialize)]
struct DetailBody {
    #[serde(rename = "appKey", default)]
    app_key: String,
    #[serde(default)]
    path: String,
}

#[derive(Deserialize)]
struct DocDetailBody {
    #[serde(rename = "appKey", default)]
    app_key: String,
    #[serde(default)]
    path: String,
}

fn response(value: Value) -> R {
    R {
        status: 200,
        ct: Some("application/json"),
        body: Bytes::from(value.to_string()),
    }
}

fn ok(data: Value) -> R {
    response(json!({"code": 0, "message": "ok", "data": data}))
}

fn error(status: u16, code: i64, message: &str) -> R {
    R {
        status,
        ct: Some("application/json"),
        body: Bytes::from(json!({"code": code, "message": message}).to_string()),
    }
}

fn unauthorized() -> R {
    error(401, 401, "token error")
}

fn body_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Option<T> {
    serde_json::from_slice(body).ok()
}

fn guard(core: &Core, token: &str, app_key: &str) -> bool {
    if core.auth_config().is_none()
        && core
            .app_configs()
            .iter()
            .all(|a| a.password.as_deref().is_none_or(str::is_empty))
    {
        return true;
    }
    let mut q = HashMap::new();
    if !token.is_empty() {
        q.insert("token".to_string(), token.to_string());
    }
    if !app_key.is_empty() {
        q.insert("appKey".to_string(), app_key.to_string());
    }
    core.guard(&q)
}

/// 每应用对外 host：`APIDOC_PUBLIC_HOSTS` 为 JSON 对象 `{appKey: host}`，
/// 未命中时回落到 `APIDOC_PUBLIC_HOST`。
/// 偏离 Phase 1 计划的 `AppConfig.public_host` 字段：新增公共字段会破坏大量
/// `AppConfig` 字面量；env map 保持公共 API 与 `api.json` 字节稳定。
fn public_hosts() -> BTreeMap<String, String> {
    match std::env::var("APIDOC_PUBLIC_HOSTS") {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            eprintln!("apidoc: ignoring invalid APIDOC_PUBLIC_HOSTS JSON: {e}");
            BTreeMap::new()
        }),
        Err(_) => BTreeMap::new(),
    }
}

fn public_host() -> String {
    std::env::var("APIDOC_PUBLIC_HOST").unwrap_or_default()
}

/// 纯函数：先查 appKey 专属 host（空串视为未配置），再回落全局 host。
fn host_for(hosts: &BTreeMap<String, String>, fallback: &str, app_key: &str) -> String {
    hosts
        .get(app_key)
        .filter(|host| !host.is_empty())
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

/// 端点所属应用 key：未标注 app 的端点属于默认应用。
fn endpoint_app_key(ep: &DocEndpoint) -> &str {
    if ep.app_key.is_empty() { "default" } else { ep.app_key.as_str() }
}

fn endpoint_id(ep: &DocEndpoint) -> String {
    let raw = format!("{}\0{}\0{}\0{}", ep.protocol, ep.method, ep.url, ep.title);
    URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

fn find_endpoint<'a>(core: &'a Core, id: &str) -> Option<&'a DocEndpoint> {
    core.doc().endpoints.iter().find(|ep| endpoint_id(ep) == id)
}

#[derive(Clone)]
struct AppLeaf<'a> {
    key: String,
    title: String,
    endpoints: &'a [DocEndpoint],
    has_password: bool,
}

fn flatten_apps<'a>(core: &'a Core) -> Vec<AppLeaf<'a>> {
    fn walk<'a>(core: &'a Core, app: &'a AppDoc, out: &mut Vec<AppLeaf<'a>>) {
        if !app.endpoints.is_empty() || app.items.is_empty() {
            let has_password = find_app(core.app_configs(), &app.key)
                .and_then(|a| a.password.as_deref())
                .is_some_and(|p| !p.is_empty());
            out.push(AppLeaf {
                key: app.key.clone(),
                title: app.title.clone(),
                endpoints: &app.endpoints,
                has_password,
            });
        }
        for child in &app.items {
            walk(core, child, out);
        }
    }

    let doc = core.doc();
    if doc.apps.is_empty() {
        return vec![AppLeaf {
            key: "default".to_string(),
            title: doc.config.title.clone(),
            endpoints: &doc.endpoints,
            has_password: false,
        }];
    }
    let mut out = Vec::new();
    for app in &doc.apps {
        walk(core, app, &mut out);
    }
    out
}

fn app_leaf_json(leaf: &AppLeaf<'_>, hosts: &BTreeMap<String, String>, fallback: &str) -> Value {
    json!({
        "key": leaf.key,
        "title": leaf.title,
        "path": leaf.key,
        "host": host_for(hosts, fallback, &leaf.key),
        "hasPassword": leaf.has_password,
        "groups": [],
    })
}

fn pick_app<'a, 'b>(apps: &'a [AppLeaf<'b>], app_key: &str) -> Option<&'a AppLeaf<'b>> {
    if app_key.is_empty() {
        return apps.first();
    }
    apps.iter().find(|leaf| leaf.key == app_key)
}

/// 该协议的服务端执行器是否编进本构建。
/// 与 `debug::execute` / `debug::stream_dispatch` 的 feature gate 一一对应；
/// capabilities 与 detail 必须共用这一判断，避免"能选但一发就报未编译"。
fn executor_compiled(kind: &str) -> bool {
    match kind {
        "http" => cfg!(feature = "debug-http"),
        "websocket" | "ws-binary" => cfg!(feature = "debug-session"),
        "grpc" => cfg!(feature = "debug-grpc"),
        "thrift" => cfg!(feature = "debug-thrift"),
        "dubbo" | "hessian" => cfg!(feature = "debug-dubbo"),
        "iso8583" => cfg!(feature = "debug-iso8583"),
        "mqtt" => cfg!(feature = "debug-mqtt"),
        "amqp" => cfg!(feature = "debug-amqp"),
        "kafka" => cfg!(feature = "debug-kafka"),
        "xmpp" => cfg!(feature = "debug-xmpp"),
        "tcp" | "udp" | "raw" | "unix" | "stomp" => cfg!(feature = "debug-sockets"),
        "coap" | "lwm2m" => cfg!(feature = "debug-coap"),
        "modbus" | "dnp3" | "iec104" | "bacnet" | "opcua" => cfg!(feature = "debug-industrial"),
        "rtp" | "rtcp" | "rtsp" | "sip" | "rtmp" => cfg!(feature = "debug-media"),
        "http3" => cfg!(feature = "debug-http3"),
        _ => false,
    }
}

fn relay_protocols() -> Vec<&'static str> {
    ["http", "websocket", "grpc", "thrift", "dubbo", "iso8583", "mqtt", "tcp", "udp", "unix"]
        .into_iter()
        .filter(|kind| executor_compiled(kind))
        .collect()
}

fn capabilities_data(core: &Core) -> Value {
    let global_auth = core
        .auth_config()
        .is_some_and(|c| c.enable && !c.password.is_empty());
    let app_auth = core
        .app_configs()
        .iter()
        .any(|a| a.password.as_deref().is_some_and(|p| !p.is_empty()));
    json!({
        "contract": CONTRACT,
        "backend": "rust",
        "version": env!("CARGO_PKG_VERSION"),
        "features": {
            "menus": true,
            "detail": true,
            "docs": true,
            "protocolDefinitions": true,
            "debug": {
                "browser": ["http", "websocket", "sse"],
                "relay": relay_protocols(),
                "modes": ["browser-http", "browser-websocket", "browser-sse", "relay-unary", "relay-stream"],
            },
            "share": {"link": true, "records": false, "actions": false},
            "cache": {"manage": false, "mode": "build"},
            "generator": {"forms": false, "templates": true, "sideEffects": false},
            "exports": [],
            "auth": {
                "global": global_auth,
                "app": app_auth,
                "shareToken": false,
            },
        },
    })
}

pub fn capabilities(core: &Core) -> R {
    ok(capabilities_data(core))
}

pub fn config(core: &Core, q: &HashMap<String, String>, token: &str) -> R {
    let app_key = q.get("appKey").map(String::as_str).unwrap_or("");
    if !guard(core, token, app_key) {
        return unauthorized();
    }
    let hosts = public_hosts();
    let fallback = public_host();
    let apps = flatten_apps(core)
        .iter()
        .map(|leaf| app_leaf_json(leaf, &hosts, &fallback))
        .collect::<Vec<_>>();
    ok(json!({
        "title": core.doc().config.title,
        "desc": core.doc().config.description.clone().unwrap_or_default(),
        "apps": apps,
        "cache": {"enable": false},
        "params": {"header": [], "query": [], "body": []},
        "responses": {"success": [], "error": []},
        "generator": [],
        "code_template": [],
        "share": {"enable": false},
        "export_config": {"enable": false},
        "capabilities": capabilities_data(core),
    }))
}

pub fn menus(core: &Core, body: &[u8], token: &str) -> R {
    let Some(scope) = body_json::<ScopeBody>(body) else {
        return error(400, 400, "invalid body");
    };
    if !guard(core, token, &scope.app_key) {
        return unauthorized();
    }
    let apps = flatten_apps(core);
    let Some(leaf) = pick_app(&apps, &scope.app_key) else {
        return error(404, 404, "app not found");
    };
    let hosts = public_hosts();
    let fallback = public_host();
    let mut groups: BTreeMap<String, Vec<&DocEndpoint>> = BTreeMap::new();
    for ep in leaf.endpoints {
        let group = if ep.group.is_empty() {
            "default"
        } else {
            ep.group.as_str()
        };
        groups.entry(group.to_string()).or_default().push(ep);
    }
    let mut data = Vec::new();
    let mut tags = Vec::new();
    for (group, mut eps) in groups {
        eps.sort_by_key(|ep| ep.sort);
        let children = eps
            .iter()
            .map(|ep| {
                tags.extend(ep.tags.iter().cloned());
                let url = if ep.url.is_empty() { channel_address(ep) } else { ep.url.clone() };
                let mut item = json!({
                    "name": ep.title,
                    "title": ep.title,
                    "menuKey": endpoint_id(ep),
                    "method": ep.method,
                    "url": url,
                    "controller": group,
                    "author": ep.author,
                    "tag": ep.tags,
                    "notDebug": ep.not_debug,
                });
                if !ep.protocol.is_empty() && ep.protocol != "http" {
                    item["protocolKey"] = json!(ep.protocol);
                    item["protocolLabel"] = json!(protocol_label(&ep.protocol));
                    item["channelAddress"] = json!(channel_address(ep));
                }
                item
            })
            .collect::<Vec<_>>();
        data.push(json!({
            "name": group,
            "title": group,
            "menuKey": format!("controller:{group}"),
            "controller": group,
            "path": group,
            "appKey": leaf.key,
            "children": children,
        }));
    }
    tags.sort();
    tags.dedup();
    ok(json!({
        "app": app_leaf_json(leaf, &hosts, &fallback),
        "data": data,
        "tags": tags,
        "groups": [],
    }))
}

pub fn detail(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<DetailBody>(body) else {
        return error(400, 400, "invalid body");
    };
    let Some(ep) = find_endpoint(core, &input.path) else {
        return error(404, 404, "action not found");
    };
    let app_key = endpoint_app_key(ep);
    if !input.app_key.is_empty() && input.app_key != app_key {
        return error(404, 404, "action not found");
    }
    if !guard(core, token, app_key) {
        return unauthorized();
    }
    ok(detail_data(core, ep, app_key))
}

fn detail_data(core: &Core, ep: &DocEndpoint, app_key: &str) -> Value {
    let header = ep
        .headers
        .iter()
        .map(|h| json!({"name": h.name, "type": "string", "desc": h.desc.unwrap_or(""), "require": false}))
        .collect::<Vec<_>>();
    let query = ep.querys.iter().map(doc_param).collect::<Vec<_>>();
    let route = ep.route_params.iter().map(doc_param).collect::<Vec<_>>();
    let params = ep.params.iter().map(doc_param).collect::<Vec<_>>();
    let success = ep
        .success
        .iter()
        .map(|e| json!({"name": e.code, "type": "object", "desc": e.example, "require": true, "mock": e.example}))
        .collect::<Vec<_>>();
    let failure = ep
        .error
        .iter()
        .map(|e| json!({"name": e.code, "type": "object", "desc": e.example, "require": true, "mock": e.example}))
        .collect::<Vec<_>>();
    let status = ep
        .response_status
        .iter()
        .map(|s| json!({"name": s, "desc": s}))
        .collect::<Vec<_>>();
    let kind = if ep.protocol.is_empty() {
        "http"
    } else {
        ep.protocol.as_str()
    };
    let spec = protocol::spec(kind);
    let debug_enabled = core.debug_config().is_some_and(|c| c.enable_server_side);
    let (mode, debuggable, reason) = debug_mode(kind, spec, debug_enabled);
    let address = channel_address(ep);
    let (fields, defaults) = debug_fields(ep, &address);
    json!({
        "name": ep.title,
        "title": ep.title,
        "menuKey": endpoint_id(ep),
        "method": ep.method,
        "url": ep.url,
        "appKey": app_key,
        "author": ep.author,
        "tag": ep.tags,
        "desc": ep.desc,
        "md": ep.md,
        "contentType": "application/json",
        "paramType": "json",
        "notDebug": ep.not_debug,
        "header": header,
        "query": query,
        "routeParam": route,
        "param": params,
        "responseSuccess": success,
        "responseError": failure,
        "responseStatus": status,
        "protocol": {
            "name": kind,
            "label": protocol_label(kind),
            "transport": protocol_transport(kind),
            "mode": mode,
            "http": spec.is_some_and(|s| s.http_compatible),
            "debuggable": debuggable,
            "debugReason": reason,
        },
        "channel": {
            "address": address,
            "action": ep.method,
            "meta": ep.protocol_opts,
        },
        "debug": {
            "mode": mode,
            "driver": kind,
            "relayEnabled": debug_enabled,
            "debuggable": debuggable,
            "debugReason": reason,
            "fields": fields,
            "defaults": defaults,
            "meta": {},
            "responseEncoding": if spec.is_some_and(|s| s.http_compatible) { "json" } else { "text" },
            "hints": "",
        },
        "definitions": definitions(ep),
        "validation": {"ok": true, "errors": [], "warnings": []},
    })
}

fn doc_param(p: &DocParam) -> Value {
    let mut item = json!({
        "name": p.name,
        "type": p.ty,
        "desc": p.desc.unwrap_or(""),
        "require": p.required,
    });
    if let Some(default) = p.default {
        item["default"] = json!(default);
    }
    if let Some(mock) = p.mock {
        item["mock"] = json!(mock);
    }
    if let Some(field) = p.field {
        item["field"] = json!(field);
    }
    if !p.children.is_empty() {
        item["children"] = Value::Array(p.children.iter().map(doc_param).collect());
    }
    item
}

fn protocol_label(kind: &str) -> &'static str {
    match kind {
        "" | "http" => "HTTP",
        "http2" => "HTTP/2",
        "http3" => "HTTP/3",
        "soap" => "SOAP",
        "graphql" => "GraphQL",
        "jsonrpc" => "JSON-RPC",
        "xmlrpc" => "XML-RPC",
        "odata" => "OData",
        "websocket" => "WebSocket",
        "sse" => "SSE",
        "grpc" => "gRPC",
        "thrift" => "Thrift",
        "dubbo" => "Dubbo",
        "mqtt" => "MQTT",
        "stomp" => "STOMP",
        "kafka" => "Kafka",
        "amqp" => "AMQP",
        "tcp" => "TCP",
        "udp" => "UDP",
        "iso8583" => "ISO 8583",
        _ => "Protocol",
    }
}

fn protocol_transport(kind: &str) -> &'static str {
    match kind {
        "" | "http" | "http2" | "http3" | "soap" | "graphql" | "jsonrpc" | "xmlrpc" | "odata" => {
            "http"
        }
        "websocket" | "socketio" => "ws",
        "sse" => "sse",
        _ => "tcp",
    }
}

fn channel_address(ep: &DocEndpoint) -> String {
    ep.protocol_opts
        .get("target")
        .cloned()
        .unwrap_or_else(|| ep.url.clone())
}

fn debug_mode(
    kind: &str,
    spec: Option<&protocol::ProtocolSpec>,
    server_enabled: bool,
) -> (&'static str, bool, String) {
    if spec.is_some_and(|s| s.http_compatible) {
        return ("browser-http", true, String::new());
    }
    match kind {
        "websocket" => return ("browser-websocket", true, String::new()),
        "sse" => return ("browser-sse", true, String::new()),
        _ => {}
    }
    if server_enabled && spec.is_some_and(|s| s.execution != protocol::Execution::Browser) {
        // Phase 1: stream_dispatch only supports WebSocket; other server
        // protocols use the unary executor until their stream bridges land.
        if executor_compiled(kind) {
            return ("relay-unary", true, String::new());
        }
        return (
            "relay-unary",
            false,
            format!("server executor for `{kind}` is not compiled in this build"),
        );
    }
    (
        "relay-unary",
        false,
        "server-side debug is disabled or not compiled in".to_string(),
    )
}

fn debug_fields(ep: &DocEndpoint, address: &str) -> (Vec<Value>, BTreeMap<String, Value>) {
    let mut fields = Vec::new();
    let mut defaults = BTreeMap::new();
    let kind = if ep.protocol.is_empty() {
        "http"
    } else {
        ep.protocol.as_str()
    };
    if protocol::spec(kind).is_some_and(|s| s.http_compatible) {
        return (fields, defaults);
    }
    fields.push(json!({"name": "target", "type": "string", "in": "target", "require": true, "default": address}));
    defaults.insert("target".to_string(), json!(address));
    for (key, value) in &ep.protocol_opts {
        if key == "target" {
            continue;
        }
        fields.push(json!({"name": key, "type": "string", "in": "meta", "default": value}));
        defaults.insert(key.clone(), json!(value));
    }
    (fields, defaults)
}

fn definitions(ep: &DocEndpoint) -> Vec<Value> {
    if ep.events.is_empty() {
        return Vec::new();
    }
    let messages = ep
        .events
        .iter()
        .map(|event| {
            let direction = if event.direction == "in" { "response" } else { "request" };
            let mut message = json!({
                "name": event.name,
                "direction": direction,
                "fields": [{"name": "body", "type": event.ty, "desc": event.desc.unwrap_or(""), "require": true}],
                "sampleValidation": {"ok": true, "errors": [], "warnings": []},
            });
            if let Some(mock) = event.mock {
                message["sample"] = json!(mock);
            }
            message
        })
        .collect::<Vec<_>>();
    vec![json!({
        "format": "asyncapi",
        "source": "inline",
        "operation": ep.method,
        "meta": {"protocol": ep.protocol},
        "messages": messages,
        "validation": {"ok": true, "errors": [], "warnings": []},
    })]
}

pub fn doc_menus(core: &Core, q: &HashMap<String, String>, token: &str) -> R {
    let app_key = q.get("appKey").map(String::as_str).unwrap_or("");
    if !guard(core, token, app_key) {
        return unauthorized();
    }
    let apps = flatten_apps(core);
    let leaves: Vec<&AppLeaf<'_>> = if app_key.is_empty() {
        apps.iter().collect()
    } else {
        apps.iter().filter(|leaf| leaf.key == app_key).collect()
    };
    let mut out = Vec::new();
    for leaf in leaves {
        let mut groups: BTreeMap<String, Vec<&DocEndpoint>> = BTreeMap::new();
        for ep in leaf.endpoints {
            if ep.md.is_empty() {
                continue;
            }
            let group = if ep.group.is_empty() {
                "default"
            } else {
                ep.group.as_str()
            };
            groups.entry(group.to_string()).or_default().push(ep);
        }
        for (group, eps) in groups {
            let children = eps
                .iter()
                .map(|ep| {
                    json!({
                        "name": ep.title,
                        "title": ep.title,
                        "menuKey": format!("doc:{}", endpoint_id(ep)),
                        "path": endpoint_id(ep),
                    })
                })
                .collect::<Vec<_>>();
            out.push(json!({
                "name": group,
                "title": group,
                "menuKey": format!("doc-controller:{}:{group}", leaf.key),
                "children": children,
            }));
        }
    }
    ok(Value::Array(out))
}

pub fn doc_detail(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<DocDetailBody>(body) else {
        return error(400, 400, "invalid body");
    };
    let Some(ep) = find_endpoint(core, &input.path) else {
        return error(404, 404, "doc not found");
    };
    let app_key = endpoint_app_key(ep);
    if !input.app_key.is_empty() && input.app_key != app_key {
        return error(404, 404, "doc not found");
    }
    if ep.md.is_empty() {
        return error(404, 404, "doc not found");
    }
    if !guard(core, token, app_key) {
        return unauthorized();
    }
    ok(json!({"content": ep.md}))
}

pub fn verify_auth(core: &Core, body: &[u8]) -> R {
    // Phase 1 does not implement share scopes: answer 501 instead of ignoring
    // the field and minting a normal credential (which would over-grant).
    let share_key = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("shareKey").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    if !share_key.is_empty() {
        return error(501, 501, "shareKey is not supported in Phase 1");
    }
    let Some(req) = crate_auth::parse_login(body) else {
        return error(400, 400, "invalid body");
    };
    let cfg = core.auth_config();
    if cfg.is_none()
        && core
            .app_configs()
            .iter()
            .all(|a| a.password.as_deref().is_none_or(str::is_empty))
    {
        return error(401, 401, "password error");
    }
    match crate_auth::auth_issue(
        &req.password,
        req.app_key.as_deref(),
        cfg,
        core.app_configs(),
    ) {
        crate_auth::AuthResult::Token(token) => ok(json!({"token": token})),
        crate_auth::AuthResult::Error | crate_auth::AuthResult::Disabled => {
            error(401, 401, "password error")
        }
    }
}

pub use debug::{debug_poll, debug_send, debug_start, debug_stop};

#[cfg(test)]
mod tests {
    use super::*;

    // I5: per-app host from APIDOC_PUBLIC_HOSTS json map, fallback to APIDOC_PUBLIC_HOST.
    #[test]
    fn per_app_host_map_overrides_fallback() {
        let hosts = BTreeMap::from([
            ("A".to_string(), "https://a.example".to_string()),
            ("empty".to_string(), String::new()),
        ]);
        assert_eq!(host_for(&hosts, "https://fallback.example", "A"), "https://a.example");
        assert_eq!(host_for(&hosts, "https://fallback.example", "B"), "https://fallback.example");
        assert_eq!(
            host_for(&hosts, "https://fallback.example", "empty"),
            "https://fallback.example"
        );
        assert_eq!(host_for(&BTreeMap::new(), "", "A"), "");
    }

    // M3: 未编译执行器的协议不得在 detail 里标 debuggable。
    #[test]
    fn debug_mode_hides_uncompiled_executor() {
        // socketio has a server execution policy but no executor in any feature set.
        let spec = crate::protocol::spec("socketio");
        assert!(spec.is_some());
        let (mode, debuggable, reason) = debug_mode("socketio", spec, true);
        assert_eq!(mode, "relay-unary");
        assert!(!debuggable);
        assert!(reason.contains("not compiled"), "{reason}");
    }

    #[test]
    fn debug_mode_keeps_browser_protocols() {
        let (mode, debuggable, _) = debug_mode("http", crate::protocol::spec("http"), false);
        assert_eq!(mode, "browser-http");
        assert!(debuggable);
        let (mode, debuggable, _) =
            debug_mode("websocket", crate::protocol::spec("websocket"), false);
        assert_eq!(mode, "browser-websocket");
        assert!(debuggable);
    }
}
