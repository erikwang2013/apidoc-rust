//! UIC-1: the shared contract consumed by apidoc-ui.
//!
//! This module is intentionally a thin mapping over the existing `Core`:
//! it does not change `api.json`, the native UI, or any executor semantics.

mod debug;
mod share;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::protocol_defs::{self, Diagnostic, Limits, Options, Resolver, Validation};
use crate::route_core::{Core, R};
use crate::{auth as crate_auth, protocol, AppDoc, DocDefinition, DocEndpoint, DocParam};

const CONTRACT: &str = "1.0";

#[derive(Deserialize)]
struct ScopeBody {
    #[serde(rename = "appKey", default)]
    app_key: String,
    #[serde(rename = "shareKey", default)]
    share_key: String,
}

#[derive(Deserialize)]
struct DetailBody {
    #[serde(rename = "appKey", default)]
    app_key: String,
    #[serde(rename = "shareKey", default)]
    share_key: String,
    #[serde(default)]
    path: String,
}

#[derive(Deserialize)]
struct DocDetailBody {
    #[serde(rename = "appKey", default)]
    app_key: String,
    #[serde(rename = "shareKey", default)]
    share_key: String,
    #[serde(default)]
    path: String,
}

fn response(value: Value) -> R {
    R {
        status: 200,
        ct: Some("application/json"),
        body: Bytes::from(value.to_string()),
        location: None,
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
        location: None,
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

fn share_error(e: share::Error) -> R {
    match e {
        share::Error::NotFound => error(404, 404, "share not found"),
        share::Error::Invalid(message) => error(400, 400, message),
        share::Error::Store => error(500, 500, "share store error"),
    }
}

/// 分享令牌校验：签名密钥 = 记录 secret，天然与全局/应用令牌隔离。
fn share_token_ok(core: &Core, record: &share::Record, token: &str) -> bool {
    let default = crate_auth::AuthConfig::default();
    share::check_token(record, token, core.auth_config().unwrap_or(&default))
}

/// 全局凭据状态：`None` = 未启用全局鉴权（无全局凭据可校验）；
/// `Some(ok)` = 已启用，`ok` 表示令牌是否有效。`enable` 但密码为空按失败关闭处理，
/// 不能当成"未启用"而放行。
fn global_admin(core: &Core, token: &str) -> Option<bool> {
    let cfg = core.auth_config()?;
    if !cfg.enable {
        return None;
    }
    Some(!cfg.password.is_empty() && crate_auth::auth_check(token, cfg))
}

/// Resolve a shareKey into its record. `Ok(None)` = no shareKey, normal auth
/// applies. A password-protected share requires its own token; a public share
/// needs none (the shareKey itself is the capability).
fn share_scope(core: &Core, token: &str, share_key: &str) -> Result<Option<share::Record>, R> {
    if share_key.is_empty() {
        return Ok(None);
    }
    let record = share::find(share_key).map_err(share_error)?;
    if !record.password_hash.is_empty() && !share_token_ok(core, &record, token) {
        return Err(unauthorized());
    }
    Ok(Some(record))
}

/// Share scope when `shareKey` is present, otherwise the ordinary app/global
/// guard. A share token never passes the guard and vice versa.
fn access(
    core: &Core,
    token: &str,
    app_key: &str,
    share_key: &str,
) -> Result<Option<share::Record>, R> {
    let record = share_scope(core, token, share_key)?;
    if record.is_none() && !guard(core, token, app_key) {
        return Err(unauthorized());
    }
    Ok(record)
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

/// 应用是否配置了非空独立密码（未配置的应用公开可读）。
fn app_has_password(core: &Core, key: &str) -> bool {
    crate::find_app(core.app_configs(), key)
        .and_then(|app| app.password.as_deref())
        .is_some_and(|password| !password.is_empty())
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
            let has_password = app_has_password(core, &app.key);
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
    [
        "http", "http2", "http3", "websocket", "grpc", "thrift", "dubbo",
        "hessian", "iso8583", "mqtt", "stomp", "amqp", "kafka", "xmpp", "tcp", "udp", "raw",
        "unix", "coap", "lwm2m", "modbus", "dnp3", "iec104", "bacnet", "opcua", "rtp", "rtcp",
        "rtsp", "sip", "rtmp",
    ]
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
            // Rust has no declarative share-action config yet; records/links only.
            "share": {"link": true, "records": true, "actions": false},
            "cache": {"manage": true, "mode": "build"},
            "generator": {"forms": false, "templates": true, "sideEffects": false},
            "exports": ["md", "pdf", "openapi", "postman", "asyncapi", "typescript"],
            "auth": {
                "global": global_auth,
                "app": app_auth,
                "shareToken": true,
            },
        },
    })
}

pub fn capabilities(core: &Core) -> R {
    ok(capabilities_data(core))
}

pub fn config(core: &Core, q: &HashMap<String, String>, token: &str) -> R {
    let app_key = q.get("appKey").map(String::as_str).unwrap_or("");
    let share_key = q.get("shareKey").map(String::as_str).unwrap_or("");
    let share = match access(core, token, app_key, share_key) {
        Ok(share) => share,
        Err(res) => return res,
    };
    let hosts = public_hosts();
    let fallback = public_host();
    let apps = flatten_apps(core)
        .iter()
        .filter(|leaf| share.as_ref().is_none_or(|s| s.app_allowed(&leaf.key)))
        .map(|leaf| app_leaf_json(leaf, &hosts, &fallback))
        .collect::<Vec<_>>();
    ok(json!({
        "title": core.doc().config.title,
        "desc": core.doc().config.description.clone().unwrap_or_default(),
        "apps": apps,
        "cache": {"enable": true},
        "params": {"header": [], "query": [], "body": []},
        "responses": {"success": [], "error": []},
        "generator": [],
        "code_template": code_templates(core),
        "share": {"enable": true},
        "export_config": {"enable": true},
        "capabilities": capabilities_data(core),
    }))
}

/// `config.codegen` 模板即声明式代码模板：`template` 是模板名，渲染时回查同一份配置。
fn code_templates(core: &Core) -> Vec<Value> {
    core.doc()
        .config
        .codegen
        .iter()
        .map(|t| {
            json!({
                "title": t.name,
                "template": t.name,
                "select_mode": "api",
                "multiple": true,
                "form": {"items": []},
                "language": language_for(&t.name),
            })
        })
        .collect()
}

fn language_for(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "php" => "php",
        "py" => "python",
        "sql" => "sql",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        _ => "text",
    }
}

pub fn menus(core: &Core, body: &[u8], token: &str) -> R {
    let Some(scope) = body_json::<ScopeBody>(body) else {
        return error(400, 400, "invalid body");
    };
    let share = match access(core, token, &scope.app_key, &scope.share_key) {
        Ok(share) => share,
        Err(res) => return res,
    };
    let apps: Vec<AppLeaf<'_>> = flatten_apps(core)
        .into_iter()
        .filter(|leaf| share.as_ref().is_none_or(|s| s.app_allowed(&leaf.key)))
        .collect();
    let Some(leaf) = pick_app(&apps, &scope.app_key) else {
        return error(404, 404, "app not found");
    };
    let hosts = public_hosts();
    let fallback = public_host();
    let mut groups: BTreeMap<String, Vec<&DocEndpoint>> = BTreeMap::new();
    for ep in leaf.endpoints.iter().filter(|ep| {
        share
            .as_ref()
            .is_none_or(|s| s.api_allowed(&leaf.key, &endpoint_id(ep)))
    }) {
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
    let share = match access(core, token, app_key, &input.share_key) {
        Ok(share) => share,
        Err(res) => return res,
    };
    if share
        .as_ref()
        .is_some_and(|s| !s.api_allowed(app_key, &input.path))
    {
        return error(404, 404, "action not found");
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
    let (definitions, validation) = definitions(ep);
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
        "definitions": definitions,
        "validation": validation,
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

/// 端点定义 + 所有定义的诊断聚合。无 `#[apidoc::definition]` 时保持注解内联
/// 输出（事件协议）或空列表，与 Phase 4 之前一致。
fn definitions(ep: &DocEndpoint) -> (Vec<Value>, Validation) {
    if ep.definitions.is_empty() {
        return (
            event_definitions(ep),
            Validation {
                ok: true,
                errors: Vec::new(),
                warnings: Vec::new(),
            },
        );
    }
    let mut rendered = Vec::with_capacity(ep.definitions.len());
    let mut validation = Validation {
        ok: true,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    for declaration in &ep.definitions {
        let (definition, item) = render_definition(ep, declaration);
        rendered.push(definition);
        validation.errors.extend(item.errors);
        validation.warnings.extend(item.warnings);
    }
    validation.ok = validation.errors.is_empty();
    (rendered, validation)
}

fn render_definition(ep: &DocEndpoint, declaration: &DocDefinition) -> (Value, Validation) {
    let limits = Limits::default();
    let root = std::fs::canonicalize(declaration.root).ok();
    let resolved = root
        .as_deref()
        .and_then(|root| resolve_definition_file(root, declaration.file));
    let Some(path) = resolved else {
        let format = if declaration.format.is_empty() {
            let fallback = default_definition_format(&ep.protocol);
            if fallback.is_empty() {
                "auto"
            } else {
                fallback
            }
            .to_string()
        } else {
            declaration.format.to_string()
        };
        return failed_definition(
            format,
            declaration.file,
            "",
            diagnostic("definition file not found or outside allowed roots"),
        );
    };

    let mut options = Options {
        operation: declaration.operation.to_string(),
        message: declaration.message.to_string(),
        direction: declaration.direction.to_string(),
        sample: declaration.sample.to_string(),
        sample_encoding: declaration.sample_encoding.to_string(),
        strict: declaration.strict,
    };
    let mut sample_errors = Vec::new();
    if !declaration.sample_file.is_empty() {
        let sample = root
            .as_deref()
            .and_then(|root| resolve_definition_file(root, declaration.sample_file));
        match sample {
            Some(sample) => match read_limited(&sample, &limits) {
                Ok(content) => options.sample = content,
                Err(error) => sample_errors.push(error),
            },
            None => {
                sample_errors.push(diagnostic("sample file not found or outside allowed roots"))
            }
        }
    }

    let content = match read_limited(&path, &limits) {
        Ok(content) => content,
        Err(error) => {
            // PHP 在读取失败后仍按「空内容」检测格式，再回落到协议默认格式。
            let mut format = declaration.format.to_string();
            if format.is_empty() {
                format = protocol_defs::detect(declaration.file, "");
                if format.is_empty() {
                    format = default_definition_format(&ep.protocol).to_string();
                }
            }
            sample_errors.push(error);
            let validation = Validation {
                ok: false,
                errors: sample_errors,
                warnings: Vec::new(),
            };
            return failed_definition_with(
                format,
                declaration.file,
                declaration.operation,
                validation,
            );
        }
    };

    let mut format = declaration.format.to_string();
    if format.is_empty() {
        format = protocol_defs::detect(declaration.file, &content);
        if format.is_empty() {
            format = default_definition_format(&ep.protocol).to_string();
        }
    }
    let source = path.to_string_lossy().to_string();
    let mut resolver = DefinitionResolver {
        root: root.expect("resolved definition implies a canonical root"),
        failure: None,
    };
    let mut definition =
        protocol_defs::parse(&format, &source, &content, options, limits, &mut resolver);
    // 对外 source 是注解里的声明路径；内部 canonical path 只用于解析导入。
    definition.source = declaration.file.to_string();
    if !sample_errors.is_empty() {
        sample_errors.extend(definition.validation.errors);
        definition.validation.errors = sample_errors;
        definition.validation.ok = false;
    }
    let validation = definition.validation.clone();
    let value = serde_json::to_value(definition).expect("definition must serialize");
    (value, validation)
}

fn failed_definition(
    format: String,
    source: &str,
    operation: &str,
    error: Diagnostic,
) -> (Value, Validation) {
    let validation = Validation {
        ok: false,
        errors: vec![error],
        warnings: Vec::new(),
    };
    failed_definition_with(format, source, operation, validation)
}

fn failed_definition_with(
    format: String,
    source: &str,
    operation: &str,
    validation: Validation,
) -> (Value, Validation) {
    let definition = protocol_defs::Definition {
        format,
        source: source.to_string(),
        operation: operation.to_string(),
        meta: json!({}),
        messages: Vec::new(),
        validation: validation.clone(),
    };
    let value = serde_json::to_value(definition).expect("definition must serialize");
    (value, validation)
}

fn diagnostic(message: &str) -> Diagnostic {
    Diagnostic {
        path: String::new(),
        line: 0,
        column: 0,
        severity: "error".to_string(),
        message: message.to_string(),
    }
}

/// 注解路径相对声明方 crate 根；软链 realpath 后必须仍在根内。
fn resolve_definition_file(root: &Path, file: &str) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(root.join(file)).ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(resolved)
}

struct DefinitionResolver {
    root: PathBuf,
    failure: Option<String>,
}

impl Resolver for DefinitionResolver {
    fn read(&mut self, path: &str) -> Option<(String, String)> {
        self.failure = None;
        let resolved = resolve_definition_file(&self.root, path)?;
        // Stat before reading so an oversized import is never allocated.
        let content = match read_limited(&resolved, &Limits::default()) {
            Ok(content) => content,
            Err(error) => {
                self.failure = Some(error.message);
                return None;
            }
        };
        Some((resolved.to_string_lossy().to_string(), content))
    }

    fn failure(&mut self) -> Option<String> {
        self.failure.take()
    }
}

/// 与 `ProtocolContext::read` 同序：先 stat/字节上限，再读取，最后行数上限。
fn read_limited(path: &Path, limits: &Limits) -> Result<String, Diagnostic> {
    let metadata = std::fs::metadata(path).map_err(|_| diagnostic("unable to stat file"))?;
    if metadata.len() > limits.max_file_bytes as u64 {
        return Err(diagnostic("file exceeds max_file_bytes"));
    }
    let content = std::fs::read_to_string(path).map_err(|_| diagnostic("unable to read file"))?;
    if content.bytes().filter(|b| *b == b'\n').count() + 1 > limits.max_lines {
        return Err(diagnostic("file exceeds max_lines"));
    }
    Ok(content)
}

/// PHP `ProtocolCatalog::$formats` 的 Rust 协议 id 映射。
fn default_definition_format(kind: &str) -> &'static str {
    match kind {
        "soap" => "wsdl",
        "graphql" => "graphql",
        "grpc" | "dubbo" => "proto",
        "thrift" => "thrift",
        "hessian" => "hessian",
        "jsonrpc" => "json-rpc",
        "xmlrpc" => "xml-rpc",
        "webhook" | "mcp" | "a2a" => "json-schema",
        "odata" => "edmx",
        _ => "",
    }
}

fn event_definitions(ep: &DocEndpoint) -> Vec<Value> {
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
    let share_key = q.get("shareKey").map(String::as_str).unwrap_or("");
    let share = match access(core, token, app_key, share_key) {
        Ok(share) => share,
        Err(res) => return res,
    };
    if share
        .as_ref()
        .is_some_and(|s| !app_key.is_empty() && !s.app_allowed(app_key))
    {
        return error(404, 404, "app not found");
    }
    let apps = flatten_apps(core);
    let leaves: Vec<&AppLeaf<'_>> = apps
        .iter()
        .filter(|leaf| app_key.is_empty() || leaf.key == app_key)
        .filter(|leaf| share.as_ref().is_none_or(|s| s.app_allowed(&leaf.key)))
        .collect();
    let mut out = Vec::new();
    for leaf in leaves {
        let mut groups: BTreeMap<String, Vec<&DocEndpoint>> = BTreeMap::new();
        for ep in leaf.endpoints.iter().filter(|ep| {
            !ep.md.is_empty()
                && share
                    .as_ref()
                    .is_none_or(|s| s.api_allowed(&leaf.key, &endpoint_id(ep)))
        }) {
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
    let share = match access(core, token, app_key, &input.share_key) {
        Ok(share) => share,
        Err(res) => return res,
    };
    if share
        .as_ref()
        .is_some_and(|s| !s.api_allowed(app_key, &input.path))
    {
        return error(404, 404, "doc not found");
    }
    ok(json!({"content": ep.md}))
}

#[derive(Deserialize)]
struct VerifyBody {
    #[serde(default)]
    password: String,
    #[serde(rename = "appKey")]
    app_key: Option<String>,
    #[serde(rename = "shareKey")]
    share_key: Option<String>,
}

pub fn verify_auth(core: &Core, body: &[u8]) -> R {
    let Some(req) = body_json::<VerifyBody>(body) else {
        return error(400, 400, "invalid body");
    };
    // 分享登录：密码比对分享记录哈希，令牌用分享 secret 签名（作用域天然隔离）。
    // 公开分享没有密码可校验，登录一律拒绝（401，与 PHP 一致）；读接口无需令牌。
    if let Some(share_key) = req.share_key.as_deref().filter(|k| !k.is_empty()) {
        let record = match share::find(share_key) {
            Ok(record) => record,
            Err(e) => return share_error(e),
        };
        if !record.password_ok(&req.password) {
            return error(401, 401, "password error");
        }
        let default = crate_auth::AuthConfig::default();
        let cfg = core.auth_config().unwrap_or(&default);
        return ok(json!({"token": share::issue_token(&record, cfg)}));
    }
    if req.password.is_empty() {
        return error(400, 400, "invalid body");
    }
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

// ---------- 分享记录 CRUD ----------

#[derive(Deserialize)]
struct ShareAddBody {
    #[serde(default)]
    key: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "type", default = "share::default_kind")]
    kind: String,
    #[serde(rename = "appKeys", default)]
    app_keys: Vec<String>,
    #[serde(rename = "apiKeys", default)]
    api_keys: Vec<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Deserialize)]
struct ShareKeyBody {
    #[serde(default)]
    key: String,
}

#[derive(Deserialize)]
struct ShareListBody {
    #[serde(rename = "pageIndex", default = "one")]
    page_index: u64,
}

fn one() -> u64 {
    1
}

#[derive(Deserialize)]
struct ShareActionBody {
    #[serde(default)]
    key: String,
}

pub fn add_api_share(core: &Core, body: &[u8], token: &str) -> R {
    if !guard(core, token, "") {
        return unauthorized();
    }
    let Some(input) = body_json::<ShareAddBody>(body) else {
        return error(400, 400, "invalid body");
    };
    let result = share::add(&share::NewShare {
        key: input.key,
        name: input.name,
        kind: input.kind,
        app_keys: input.app_keys,
        api_keys: input.api_keys,
        password: input.password,
    });
    match result {
        Ok(()) => ok(json!(true)),
        Err(e) => share_error(e),
    }
}

pub fn get_api_share_list(core: &Core, body: &[u8], token: &str) -> R {
    if !guard(core, token, "") {
        return unauthorized();
    }
    let input = body_json::<ShareListBody>(body).unwrap_or(ShareListBody { page_index: 1 });
    match share::list(input.page_index) {
        Ok((records, total)) => ok(json!({
            "total": total,
            "data": records.iter().map(share::record_json).collect::<Vec<_>>(),
        })),
        Err(e) => share_error(e),
    }
}

pub fn get_api_share_detail(core: &Core, body: &[u8], token: &str) -> R {
    if !guard(core, token, "") {
        return unauthorized();
    }
    let Some(input) = body_json::<ShareKeyBody>(body) else {
        return error(400, 400, "invalid body");
    };
    match share::find(&input.key) {
        Ok(record) => ok(share::record_json(&record)),
        Err(e) => share_error(e),
    }
}

pub fn delete_api_share(core: &Core, body: &[u8], token: &str) -> R {
    if !guard(core, token, "") {
        return unauthorized();
    }
    let Some(input) = body_json::<ShareKeyBody>(body) else {
        return error(400, 400, "invalid body");
    };
    match share::delete(&input.key) {
        Ok(()) => ok(json!(true)),
        Err(e) => share_error(e),
    }
}

/// 分享 action 由服务端配置声明；Rust 无 action 配置，故恒返回 false。
pub fn handle_api_share_action(core: &Core, body: &[u8], token: &str) -> R {
    if !guard(core, token, "") {
        return unauthorized();
    }
    let Some(input) = body_json::<ShareActionBody>(body) else {
        return error(400, 400, "invalid body");
    };
    match share::find(&input.key) {
        Ok(_) => ok(json!(false)),
        Err(e) => share_error(e),
    }
}

// ---------- 缓存（构建期物化） ----------

pub fn cache_manage(core: &Core, token: &str) -> R {
    if !guard(core, token, "") {
        return unauthorized();
    }
    ok(json!({"ok": true, "mode": "build", "message": "build-time materialized"}))
}

// ---------- 声明式代码模板 ----------

#[derive(Deserialize)]
struct CodeTemplateBody {
    #[serde(rename = "appKey", default)]
    app_key: String,
    #[serde(default)]
    template: Value,
    #[serde(default)]
    selected: Vec<String>,
    #[serde(default)]
    form: Value,
}

pub fn render_code_template(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<CodeTemplateBody>(body) else {
        return error(400, 400, "invalid body");
    };
    if !guard(core, token, &input.app_key) {
        return unauthorized();
    }
    // UI 传整个 ConfigCodeTemplateItem；夹具传模板名字符串。两种都接受。
    let name = match &input.template {
        Value::String(name) => name.clone(),
        Value::Object(map) => map
            .get("template")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    if name.is_empty() {
        return error(400, 400, "invalid body");
    }
    let Some(template) = crate::codegen::find(&name, &core.doc().config.codegen) else {
        return error(404, 404, "template not found");
    };
    // form 不参与渲染：声明式模板没有副作用，也不执行宿主代码。
    let _ = &input.form;
    // 先按被鉴权的 appKey 裁剪，再应用客户端勾选：勾选里的其它应用端点一律忽略
    // （与 Go 的 actionInApp 一致）；空勾选 = 该应用的全部端点，不是整份文档。
    let endpoints: Vec<&DocEndpoint> = core
        .doc()
        .endpoints
        .iter()
        .filter(|ep| input.app_key.is_empty() || endpoint_app_key(ep) == input.app_key)
        .filter(|ep| {
            input.selected.is_empty()
                || input.selected.iter().any(|id| id == &endpoint_id(ep))
        })
        .collect();
    let code = crate::codegen::render_selected(template, core.doc(), &endpoints);
    ok(json!({"code": code}))
}

// ---------- exportAllData ----------

#[derive(Deserialize)]
struct ExportAllBody {
    #[serde(default)]
    key: String,
    #[serde(rename = "appKey", default)]
    app_key: String,
    #[serde(rename = "shareKey", default)]
    share_key: String,
}

pub fn export_all_data(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<ExportAllBody>(body) else {
        return error(400, 400, "invalid body");
    };
    let admin = global_admin(core, token);
    // `key` 是属主接口（UI 分享管理页），导出范围按分享记录裁剪；
    // `shareKey` 走分享 token，供分享页直接批量导出。
    let share = if !input.key.is_empty() {
        let record = match share::find(&input.key) {
            Ok(record) => record,
            Err(e) => return share_error(e),
        };
        if record.password_hash.is_empty() {
            // 公开分享记录：全局鉴权启用时只认全局令牌；未启用时无凭据可校验。
            if admin == Some(false) {
                return unauthorized();
            }
        } else if admin != Some(true) && !share_token_ok(core, &record, token) {
            // 有密码的记录：属主（全局令牌）或该分享自己的令牌。
            return unauthorized();
        }
        Some(record)
    } else {
        match access(core, token, &input.app_key, &input.share_key) {
            Ok(share) => share,
            Err(res) => return res,
        }
    };
    // 无全局凭据的裸导出（空 body）只给未设应用密码的应用：与会话内 `menus`
    // 的 app 密码守卫一致，不能让匿名导出绕过应用密码。
    let hide_protected = input.key.is_empty()
        && input.share_key.is_empty()
        && input.app_key.is_empty()
        && admin != Some(true);

    let doc = core.doc();
    let synthetic;
    let apps: &[AppDoc] = if doc.apps.is_empty() {
        synthetic = AppDoc {
            key: "default".to_string(),
            title: doc.config.title.clone(),
            items: Vec::new(),
            endpoints: doc.endpoints.clone(),
        };
        std::slice::from_ref(&synthetic)
    } else {
        &doc.apps
    };
    if !input.app_key.is_empty() && !apps.iter().any(|app| app_matches(app, &input.app_key)) {
        return error(404, 404, "app not found");
    }
    let nodes = export_nodes(core, apps, share.as_ref(), &input.app_key, hide_protected);
    ok(json!({"apiData": nodes}))
}

pub(crate) fn app_matches(app: &AppDoc, key: &str) -> bool {
    app.key == key || app.items.iter().any(|child| app_matches(child, key))
}

fn export_nodes(
    core: &Core,
    apps: &[AppDoc],
    share: Option<&share::Record>,
    only: &str,
    hide_protected: bool,
) -> Vec<Value> {
    apps.iter()
        .filter(|app| only.is_empty() || app_matches(app, only))
        // 叶子应用的独立密码与会话内守卫同源；容器本身不承载端点。
        .filter(|app| !(hide_protected && app.items.is_empty() && app_has_password(core, &app.key)))
        .filter_map(|app| {
            let children = export_nodes(core, &app.items, share, only, hide_protected);
            if !app.items.is_empty() {
                // 容器节点不带 appKey：UI 递归进入子应用；与 PHP 的目录语义一致。
                if (share.is_some() || hide_protected) && children.is_empty() {
                    return None;
                }
                return Some(json!({
                    "key": app.key,
                    "title": app.title,
                    "menuKey": app.key,
                    "children": children,
                }));
            }
            let endpoints: Vec<Value> = app
                .endpoints
                .iter()
                .filter(|ep| {
                    share.is_none_or(|s| s.api_allowed(&app.key, &endpoint_id(ep)))
                })
                .map(|ep| detail_data(core, ep, &app.key))
                .collect();
            if share.is_some() && endpoints.is_empty() {
                return None;
            }
            Some(json!({
                "key": app.key,
                "title": app.title,
                "appKey": app.key,
                "menuKey": app.key,
                "children": endpoints,
            }))
        })
        .collect()
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
