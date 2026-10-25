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

use super::{body_json, error, find_endpoint, guard, ok, unauthorized};

#[derive(Deserialize, Default)]
struct UiDebugInput {
    target: Option<String>,
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
    let _ = &input.query; // query fields are folded into `target` by the UI for relay requests
    DebugRequest {
        kind,
        target: input
            .target
            .clone()
            .or_else(|| ep.protocol_opts.get("target").cloned())
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
    if !guard(core, token, &input.app_key) {
        return unauthorized();
    }
    let Some(ep) = find_endpoint(core, &input.path) else {
        return error(404, 404, "action not found");
    };
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
        let _ = (core, ep);
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
    if !guard(core, token, &input.app_key) {
        return unauthorized();
    }
    let Some(ep) = find_endpoint(core, &input.path) else {
        return error(404, 404, "action not found");
    };
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
    if !guard(core, token, &input.app_key) {
        return unauthorized();
    }
    #[cfg(feature = "debug-session")]
    {
        let mut guard = sessions().lock().await;
        let expired = guard
            .get(&input.session_id)
            .is_none_or(|session| session.created.elapsed().as_secs() > 60);
        if expired {
            guard.remove(&input.session_id);
            return ok(json!({"ok": false, "error": "session not found or expired"}));
        }
        let mut messages = Vec::new();
        if let Some(session) = guard.get_mut(&input.session_id) {
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
                        let time = value["ts"]
                            .as_u64()
                            .map(|n| n.to_string())
                            .unwrap_or_default();
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
        }
        ok(json!({"ok": true, "messages": messages}))
    }
    #[cfg(not(feature = "debug-session"))]
    {
        let _ = (core, input);
        ok(
            json!({"ok": false, "error": "streaming debug is not compiled in (enable feature `debug-session`)"}),
        )
    }
}

pub async fn debug_stop(core: &Core, body: &[u8], token: &str) -> R {
    let Some(input) = body_json::<SessionBody>(body) else {
        return error(400, 400, "invalid body");
    };
    if !guard(core, token, &input.app_key) {
        return unauthorized();
    }
    #[cfg(feature = "debug-session")]
    {
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
        let _ = (core, input);
        ok(
            json!({"ok": false, "error": "streaming debug is not compiled in (enable feature `debug-session`)"}),
        )
    }
}
