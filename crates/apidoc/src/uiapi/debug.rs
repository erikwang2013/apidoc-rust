//! UIC-1 debug bridge. Unary requests go to `debug::execute`; stream sessions
//! reuse `debug::stream_dispatch` directly (no loopback WebSocket).

use std::collections::BTreeMap;

#[cfg(feature = "debug-session")]
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
#[cfg(feature = "debug-session")]
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::route_core::{Core, R};
#[cfg(feature = "debug-http")]
use crate::DocEndpoint;

use super::{body_json, endpoint_app_key, error, find_endpoint, guard, ok, unauthorized};

/// 客户端输入。注意没有 `target`：转发目标只能来自服务端端点配置
/// （`protocol_opts["target"]` 或 `ep.url`），客户端不可改 host/port。
#[derive(Deserialize, Default)]
struct UiDebugInput {
    headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    query: Option<BTreeMap<String, String>>,
    fields: Option<BTreeMap<String, Value>>,
    payload: Option<String>,
    encoding: Option<String>,
    timeout: Option<u64>,
}

#[derive(Deserialize)]
struct UiDebugBody {
    #[serde(rename = "appKey", default)]
    app_key: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    input: UiDebugInput,
}

#[derive(Deserialize)]
struct SessionBody {
    #[serde(rename = "sessionId", default)]
    session_id: String,
    #[serde(rename = "appKey", default)]
    app_key: String,
}

fn normalize_encoding(value: &str) -> &'static str {
    match value {
        "base64" => "base64",
        "hex" => "hex",
        _ => "text",
    }
}

#[cfg(feature = "debug-http")]
fn build_request(ep: &DocEndpoint, input: &UiDebugInput) -> crate::debug::DebugRequest {
    use crate::debug::{DebugRequest, EndpointRef, Payload};

    let kind = if ep.protocol.is_empty() {
        "http".to_string()
    } else {
        ep.protocol.clone()
    };
    let mut opts = ep.protocol_opts.clone();
    opts.remove("target");
    if let Some(fields) = &input.fields {
        for (key, value) in fields {
            // 保留键只认服务端注解：dubbo 读 opts.target、rtsp 读 opts.url，
            // 客户端 fields 不得改写已确定的 relay 目标/地址。
            if key == "target" || (kind == "rtsp" && key == "url") {
                continue;
            }
            opts.insert(
                key.clone(),
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
    }
    let body = input
        .payload
        .as_ref()
        .filter(|payload| !payload.is_empty())
        .map(|payload| Payload {
            encoding: input.encoding.clone().unwrap_or_else(|| "utf8".to_string()),
            data: payload.clone(),
        });
    // client query is intentionally unused: the target (including its query) is server-derived
    let _ = &input.query;
    DebugRequest {
        kind,
        // 服务端注解/配置是目标地址的唯一来源（spec: client may not change host/port）。
        target: ep
            .protocol_opts
            .get("target")
            .cloned()
            .unwrap_or_else(|| ep.url.clone()),
        method: Some(ep.method.clone()),
        headers: input.headers.clone().unwrap_or_default(),
        body,
        opts,
        endpoint: Some(EndpointRef {
            url: ep.url.clone(),
            method: ep.method.clone(),
            protocol: ep.protocol.clone(),
        }),
        timeout_ms: input.timeout.map(|seconds| seconds.saturating_mul(1000)),
    }
}

pub async fn debug_send(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<UiDebugBody>(body) else {
        return error(400, 400, "invalid body");
    };
    // 先解析端点，再用端点自己的 appKey 鉴权：客户端 appKey 只做一致性校验，
    // 不能让 A 应用的令牌打 B 应用的端点（find_endpoint 覆盖全文档）。
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
    #[cfg(feature = "debug-http")]
    {
        let Some(cfg) = core.debug_config().filter(|cfg| cfg.enable_server_side) else {
            return ok(json!({"ok": false, "error": "server-side debug is disabled"}));
        };
        let request = build_request(ep, &input.input);
        match crate::debug::execute(cfg, &request, Some(ep)).await {
            Ok(out) => {
                let mut data = json!({
                    "ok": true,
                    "status": out.status,
                    "headers": out.headers,
                    "body": out.body.data,
                    "encoding": normalize_encoding(&out.body.encoding),
                    "meta": {"ms": out.ms},
                });
                if !out.frames.is_empty() {
                    data["frames"] = json!(out.frames);
                }
                ok(data)
            }
            Err(message) => ok(json!({"ok": false, "error": message})),
        }
    }
    #[cfg(not(feature = "debug-http"))]
    {
        let _ = ep;
        ok(
            json!({"ok": false, "error": "server-side debug is not compiled in (enable feature `debug-http`)"}),
        )
    }
}

#[cfg(feature = "debug-session")]
struct Session {
    rx: tokio::sync::mpsc::Receiver<String>,
    handle: Option<Box<dyn crate::debug::session::StreamSession>>,
    created: std::time::Instant,
    /// 创建会话的端点所属应用；poll/stop 只认这个作用域，不认客户端 appKey。
    app_key: String,
}

#[cfg(feature = "debug-session")]
fn sessions() -> &'static tokio::sync::Mutex<std::collections::HashMap<String, Session>> {
    static SESSIONS: std::sync::OnceLock<
        tokio::sync::Mutex<std::collections::HashMap<String, Session>>,
    > = std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| tokio::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(feature = "debug-session")]
fn session_id() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::getrandom(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub async fn debug_start(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<UiDebugBody>(body) else {
        return error(400, 400, "invalid body");
    };
    // 同 debug_send：鉴权与会话作用域都绑定端点自己的 appKey。
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
    #[cfg(all(feature = "debug-http", feature = "debug-session"))]
    {
        let Some(cfg) = core.debug_config().filter(|cfg| cfg.enable_server_side) else {
            return ok(json!({"ok": false, "error": "server-side debug is disabled"}));
        };
        let request = build_request(ep, &input.input);
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);
        match crate::debug::stream_dispatch(cfg, &request, Some(ep), tx).await {
            Ok(handle) => {
                let id = session_id();
                sessions().lock().await.insert(
                    id.clone(),
                    Session {
                        rx,
                        handle: Some(handle),
                        created: std::time::Instant::now(),
                        app_key: app_key.to_string(),
                    },
                );
                ok(json!({"ok": true, "sessionId": id}))
            }
            Err(message) => ok(json!({"ok": false, "error": message})),
        }
    }
    #[cfg(not(all(feature = "debug-http", feature = "debug-session")))]
    {
        let _ = (core, ep);
        ok(
            json!({"ok": false, "error": "streaming debug is not compiled in (enable feature `debug-session`)"}),
        )
    }
}

pub async fn debug_poll(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<SessionBody>(body) else {
        return error(400, 400, "invalid body");
    };
    #[cfg(feature = "debug-session")]
    {
        // 作用域取会话创建时记录的 appKey；客户端 appKey 只在会话不存在时兜底。
        let (app_key, alive) = {
            let mut all = sessions().lock().await;
            if all
                .get(&input.session_id)
                .is_none_or(|session| session.created.elapsed().as_secs() > 60)
            {
                all.remove(&input.session_id);
                (input.app_key.clone(), false)
            } else {
                let app_key = all
                    .get(&input.session_id)
                    .map(|session| session.app_key.clone())
                    .unwrap_or_else(|| input.app_key.clone());
                (app_key, true)
            }
        };
        if !guard(core, token, &app_key) {
            return unauthorized();
        }
        if !alive {
            return ok(json!({"ok": false, "error": "session not found or expired"}));
        }
        let mut all = sessions().lock().await;
        let Some(session) = all.get_mut(&input.session_id) else {
            return ok(json!({"ok": false, "error": "session not found or expired"}));
        };
        let mut messages = Vec::new();
        while messages.len() < 100 {
            let Ok(text) = session.rx.try_recv() else {
                break;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                messages.push(json!({"type": "frame", "body": text, "encoding": "text"}));
                continue;
            };
            match value["type"].as_str().unwrap_or("") {
                "frame" => {
                    let time = value["ts"].as_u64().map(|n| n.to_string()).unwrap_or_default();
                    messages.push(json!({
                        "time": time,
                        "type": "frame",
                        "body": value["data"].as_str().unwrap_or(""),
                        "encoding": normalize_encoding(value["encoding"].as_str().unwrap_or("utf8")),
                    }));
                }
                "error" => messages.push(json!({"type": "error", "body": value["error"]})),
                "close" => messages.push(json!({"type": "close", "body": value["reason"]})),
                _ => {}
            }
        }
        ok(json!({"ok": true, "messages": messages}))
    }
    #[cfg(not(feature = "debug-session"))]
    {
        if !guard(core, token, &input.app_key) {
            return unauthorized();
        }
        ok(
            json!({"ok": false, "error": "streaming debug is not compiled in (enable feature `debug-session`)"}),
        )
    }
}

pub async fn debug_stop(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<SessionBody>(body) else {
        return error(400, 400, "invalid body");
    };
    #[cfg(feature = "debug-session")]
    {
        // 同 poll：已有会话按创建者的 appKey 鉴权，避免跨应用停别人的会话。
        let app_key = sessions()
            .lock()
            .await
            .get(&input.session_id)
            .map(|session| session.app_key.clone())
            .unwrap_or_else(|| input.app_key.clone());
        if !guard(core, token, &app_key) {
            return unauthorized();
        }
        let session = sessions().lock().await.remove(&input.session_id);
        if let Some(mut session) = session {
            if let Some(mut handle) = session.handle.take() {
                handle.close().await;
            }
        }
        ok(json!({"ok": true}))
    }
    #[cfg(not(feature = "debug-session"))]
    {
        if !guard(core, token, &input.app_key) {
            return unauthorized();
        }
        ok(
            json!({"ok": false, "error": "streaming debug is not compiled in (enable feature `debug-session`)"}),
        )
    }
}

#[cfg(all(test, feature = "debug-http"))]
mod build_request_tests {
    use super::*;
    use std::collections::BTreeMap;

    // I2: 客户端 input.target 必须被忽略，目标只来自服务端注解配置。
    #[test]
    fn relay_target_comes_from_server_config_only() {
        let ep = DocEndpoint {
            protocol: "websocket".into(),
            url: "wss://configured.example/ws".into(),
            protocol_opts: BTreeMap::from([(
                "target".to_string(),
                "wss://annotated.example/ws".to_string(),
            )]),
            ..Default::default()
        };
        let body: UiDebugBody = serde_json::from_value(json!({
            "appKey": "",
            "path": "",
            "input": {"target": "https://attacker.example/steal"}
        }))
        .unwrap();
        let req = build_request(&ep, &body.input);
        assert_eq!(req.target, "wss://annotated.example/ws");
    }

    #[test]
    fn relay_target_falls_back_to_endpoint_url() {
        let ep = DocEndpoint {
            protocol: "websocket".into(),
            url: "wss://configured.example/ws".into(),
            ..Default::default()
        };
        let req = build_request(&ep, &UiDebugInput::default());
        assert_eq!(req.target, "wss://configured.example/ws");
    }

    // R1: reserved opts read by feature-gated executors must not come from client fields.
    #[test]
    fn client_fields_cannot_inject_reserved_relay_opts() {
        for (kind, url, reserved) in [
            ("dubbo", "dubbo://configured.example:20880", "target"),
            ("rtsp", "rtsp://configured.example/live", "url"),
        ] {
            let ep = DocEndpoint { protocol: kind.into(), url: url.into(), ..Default::default() };
            let body: UiDebugBody = serde_json::from_value(json!({
                "input": {"fields": {
                    "target": "tcp://attacker.example:1",
                    "url": "rtsp://attacker.example/live",
                    "service": "svc"
                }}
            }))
            .unwrap();
            let req = build_request(&ep, &body.input);
            assert_eq!(req.target, url, "{kind}: server target must win");
            assert!(
                !req.opts.contains_key(reserved),
                "{kind}: client field `{reserved}` leaked into opts"
            );
            assert_eq!(
                req.opts.get("service").map(String::as_str),
                Some("svc"),
                "{kind}: non-reserved field kept"
            );
        }
    }
}

#[cfg(all(test, feature = "debug-session"))]
mod session_scope_tests {
    use super::*;
    use crate::auth::{auth_token, AuthConfig};
    use crate::{ApidocConfig, AppConfig};

    fn core() -> Core {
        Core::build(ApidocConfig {
            apps: vec![
                AppConfig {
                    key: "A".into(),
                    title: "A".into(),
                    items: Vec::new(),
                    password: Some("pwA".into()),
                },
                AppConfig {
                    key: "B".into(),
                    title: "B".into(),
                    items: Vec::new(),
                    password: Some("pwB".into()),
                },
            ],
            ..Default::default()
        })
    }

    async fn insert_session(id: &str, app_key: &str) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        sessions().lock().await.insert(
            id.to_string(),
            Session {
                rx,
                handle: None,
                created: std::time::Instant::now(),
                app_key: app_key.to_string(),
            },
        );
    }

    // I1: poll/stop 必须按会话创建时的 appKey 鉴权，客户端 appKey 不作数。
    #[tokio::test]
    async fn poll_and_stop_bind_to_session_app_not_client_app() {
        let core = core();
        let cfg = AuthConfig::default();
        let token_a = auth_token("pwA", &cfg);
        let token_b = auth_token("pwB", &cfg);
        let id = "test-session-cross-app";
        insert_session(id, "A").await;

        // 客户端谎报 appKey=B 也应按会话的 A 作用域拒绝 B 令牌。
        let body = json!({"sessionId": id, "appKey": "B"}).to_string();
        let res = debug_poll(&core, body.as_bytes(), &token_b).await;
        assert_eq!(res.status, 401, "app B token must not poll app A's session");
        let res = debug_stop(&core, body.as_bytes(), &token_b).await;
        assert_eq!(res.status, 401, "app B token must not stop app A's session");
        assert!(sessions().lock().await.contains_key(id), "rejected stop must keep session");

        let res = debug_poll(&core, body.as_bytes(), &token_a).await;
        assert_eq!(res.status, 200);
        let value: Value = serde_json::from_slice(&res.body).unwrap();
        assert_eq!(value["data"]["ok"], true);
        let res = debug_stop(&core, body.as_bytes(), &token_a).await;
        assert_eq!(res.status, 200);
        assert!(!sessions().lock().await.contains_key(id));
    }
}
