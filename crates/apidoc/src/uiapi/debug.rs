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

use super::{body_json, endpoint_app_key, error, find_endpoint, ok};

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
    #[serde(rename = "shareKey", default)]
    share_key: String,
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
    #[serde(rename = "shareKey", default)]
    share_key: String,
}

/// 单条客户端消息上限（与契约 §5.5 的 1 MiB 一致）。
const MAX_MESSAGE_BYTES: usize = 1_048_576;

fn payload_too_large(input: &UiDebugInput) -> bool {
    input
        .payload
        .as_ref()
        .is_some_and(|payload| payload.len() > MAX_MESSAGE_BYTES)
}

/// 分享请求的端点级校验：分享必须覆盖该端点；无 shareKey 时走普通 app/global 守卫。
fn check_scope(
    core: &Core,
    token: &str,
    app_key: &str,
    path: &str,
    body: &UiDebugBody,
) -> Result<(), R> {
    let share = super::access(core, token, app_key, &body.share_key)?;
    if share
        .as_ref()
        .is_some_and(|record| !record.api_allowed(app_key, path))
    {
        return Err(error(404, 404, "action not found"));
    }
    Ok(())
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
    if let Err(res) = check_scope(core, token, app_key, &input.path, &input) {
        return res;
    }
    if payload_too_large(&input.input) {
        return error(413, 413, "message too large");
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
const MAX_SESSIONS: usize = 4;
#[cfg(feature = "debug-session")]
const SESSION_TTL: std::time::Duration = std::time::Duration::from_secs(60);
/// 每个会话最多回传给客户端的消息条数（对齐原生流会话的 MAX_FRAMES）。
#[cfg(feature = "debug-session")]
const FRAME_CAP: usize = 1000;

#[cfg(feature = "debug-session")]
struct Session {
    rx: tokio::sync::mpsc::Receiver<String>,
    handle: Option<Box<dyn crate::debug::session::StreamSession>>,
    /// 最近一次 start/poll 时刻；超过 TTL 没有轮询即回收。
    created: std::time::Instant,
    /// 创建会话的端点所属应用；poll/stop 只认这个作用域，不认客户端 appKey。
    app_key: String,
    /// 创建会话的分享作用域（空 = 普通应用/全局会话）；poll/stop 必须匹配。
    share_key: String,
    /// 已回传的消息数；达到 FRAME_CAP 即关闭会话。
    frames: usize,
}

#[cfg(feature = "debug-session")]
fn purge_expired(
    all: &mut std::collections::HashMap<String, Session>,
    now: std::time::Instant,
) {
    // shortcut: 过期会话直接 drop，不优雅 close；执行器会话在 drop 时释放上游连接。
    // `created` 在每次 poll 刷新，所以 TTL 是"空闲"而非"总时长"。
    all.retain(|_, session| now.duration_since(session.created) <= SESSION_TTL);
}

#[cfg(feature = "debug-session")]
fn can_start(all: &std::collections::HashMap<String, Session>) -> bool {
    all.len() < MAX_SESSIONS
}

/// 会话作用域的鉴权：分享会话只认同一分享的令牌；普通会话走 app/global 守卫。
#[cfg(feature = "debug-session")]
fn session_auth_ok(
    core: &Core,
    token: &str,
    app_key: &str,
    session_share_key: &str,
    client_share_key: &str,
) -> Result<(), R> {
    if session_share_key.is_empty() {
        return if super::guard(core, token, app_key) {
            Ok(())
        } else {
            Err(super::unauthorized())
        };
    }
    if client_share_key != session_share_key {
        return Err(super::unauthorized());
    }
    match super::share_scope(core, token, session_share_key) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(super::unauthorized()),
        Err(res) => Err(res),
    }
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
    if let Err(res) = check_scope(core, token, app_key, &input.path, &input) {
        return res;
    }
    if payload_too_large(&input.input) {
        return error(413, 413, "message too large");
    }
    #[cfg(all(feature = "debug-http", feature = "debug-session"))]
    {
        let Some(cfg) = core.debug_config().filter(|cfg| cfg.enable_server_side) else {
            return ok(json!({"ok": false, "error": "server-side debug is disabled"}));
        };
        let request = build_request(ep, &input.input);
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);
        match crate::debug::stream_dispatch(cfg, &request, Some(ep), tx).await {
            Ok(mut handle) => {
                let id = session_id();
                let mut all = sessions().lock().await;
                purge_expired(&mut all, std::time::Instant::now());
                if !can_start(&all) {
                    drop(all);
                    handle.close().await;
                    return ok(json!({"ok": false, "error": "too many active sessions"}));
                }
                all.insert(
                    id.clone(),
                    Session {
                        rx,
                        handle: Some(handle),
                        created: std::time::Instant::now(),
                        app_key: app_key.to_string(),
                        share_key: input.share_key.clone(),
                        frames: 0,
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
        // 作用域取会话创建时记录的 app/share 作用域；客户端字段只在会话不存在时兜底。
        let (app_key, share_key, alive) = {
            let mut all = sessions().lock().await;
            purge_expired(&mut all, std::time::Instant::now());
            match all.get(&input.session_id) {
                Some(session) => (
                    session.app_key.clone(),
                    session.share_key.clone(),
                    true,
                ),
                None => (input.app_key.clone(), input.share_key.clone(), false),
            }
        };
        if let Err(res) = session_auth_ok(core, token, &app_key, &share_key, &input.share_key) {
            return res;
        }
        if !alive {
            return ok(json!({"ok": false, "error": "session not found or expired"}));
        }
        let (mut messages, handle, reached) = {
            let mut all = sessions().lock().await;
            let Some(session) = all.get_mut(&input.session_id) else {
                return ok(json!({"ok": false, "error": "session not found or expired"}));
            };
            // 活跃会话不断续期：TTL 只回收没人轮询的会话。
            session.created = std::time::Instant::now();
            let budget = FRAME_CAP.saturating_sub(session.frames);
            let mut messages = Vec::new();
            while messages.len() < 100 && messages.len() < budget {
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
            session.frames += messages.len();
            let reached = session.frames >= FRAME_CAP;
            let handle = if reached {
                all.remove(&input.session_id).and_then(|mut session| session.handle.take())
            } else {
                None
            };
            (messages, handle, reached)
        };
        if let Some(mut handle) = handle {
            handle.close().await;
        }
        if reached {
            messages.push(json!({"type": "error", "body": "max_frames reached"}));
        }
        ok(json!({"ok": true, "messages": messages}))
    }
    #[cfg(not(feature = "debug-session"))]
    {
        if let Err(res) = super::access(core, token, &input.app_key, &input.share_key) {
            return res;
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
        // 同 poll：已有会话按创建者的 app/share 作用域鉴权，避免跨应用停别人的会话。
        let (app_key, share_key) = sessions()
            .lock()
            .await
            .get(&input.session_id)
            .map(|session| (session.app_key.clone(), session.share_key.clone()))
            .unwrap_or_else(|| (input.app_key.clone(), input.share_key.clone()));
        if let Err(res) = session_auth_ok(core, token, &app_key, &share_key, &input.share_key) {
            return res;
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
        if let Err(res) = super::access(core, token, &input.app_key, &input.share_key) {
            return res;
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
        insert_session_with(id, app_key, "", 1).await;
    }

    async fn insert_session_with(
        id: &str,
        app_key: &str,
        share_key: &str,
        capacity: usize,
    ) -> tokio::sync::mpsc::Sender<String> {
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        sessions().lock().await.insert(
            id.to_string(),
            Session {
                rx,
                handle: None,
                created: std::time::Instant::now(),
                app_key: app_key.to_string(),
                share_key: share_key.to_string(),
                frames: 0,
            },
        );
        tx
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

    fn dummy_session(created: std::time::Instant) -> Session {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Session {
            rx,
            handle: None,
            created,
            app_key: String::new(),
            share_key: String::new(),
            frames: 0,
        }
    }

    // M1: 启动/轮询时清掉过期会话，并把并发会话数硬顶在 MAX_SESSIONS。
    #[test]
    fn purges_expired_sessions_and_caps_total() {
        let now = std::time::Instant::now();
        let mut all = std::collections::HashMap::new();
        all.insert(
            "old".to_string(),
            dummy_session(now - SESSION_TTL - std::time::Duration::from_secs(1)),
        );
        all.insert("fresh".to_string(), dummy_session(now));
        purge_expired(&mut all, now);
        assert!(!all.contains_key("old"), "expired session must be purged");
        assert!(all.contains_key("fresh"));
        all.clear();

        for index in 0..MAX_SESSIONS {
            all.insert(format!("s{index}"), dummy_session(now));
        }
        assert!(!can_start(&all), "cap is {MAX_SESSIONS}");
        all.remove("s0");
        assert!(can_start(&all));
    }

    // M1: 每会话最多 1000 条；到达后会话关闭并回报错误。
    #[tokio::test]
    async fn poll_caps_frames_and_drops_session() {
        let core = Core::build(ApidocConfig::default());
        let id = "test-frame-cap";
        let tx = insert_session_with(id, "", "", 1500).await;
        for _ in 0..1500 {
            tx.send(r#"{"type":"frame","data":"x"}"#.to_string())
                .await
                .unwrap();
        }
        drop(tx);
        let body = json!({"sessionId": id}).to_string();
        let mut frames = 0;
        let mut capped = false;
        for _ in 0..12 {
            let res = debug_poll(&core, body.as_bytes(), "").await;
            assert_eq!(res.status, 200);
            let value: Value = serde_json::from_slice(&res.body).unwrap();
            if value["data"]["ok"] == false {
                break;
            }
            for message in value["data"]["messages"].as_array().unwrap() {
                if message["type"] == "frame" {
                    frames += 1;
                }
                if message["body"] == "max_frames reached" {
                    capped = true;
                }
            }
        }
        assert_eq!(frames, FRAME_CAP, "delivered frames must stop at the cap");
        assert!(capped, "cap must be reported to the client");
        assert!(!sessions().lock().await.contains_key(id));
    }

    // M1: 1 MiB 上限只拒绝超出，不误伤恰好 1 MiB。
    #[test]
    fn rejects_messages_over_one_mib() {
        let input = |len: usize| UiDebugInput {
            payload: Some("x".repeat(len)),
            ..Default::default()
        };
        assert!(!payload_too_large(&input(MAX_MESSAGE_BYTES)));
        assert!(payload_too_large(&input(MAX_MESSAGE_BYTES + 1)));
        assert!(!payload_too_large(&UiDebugInput::default()));
    }

    // 分享会话不能靠客户端谎报 appKey：shareKey 必须与会话创建时一致。
    #[tokio::test]
    async fn share_session_requires_matching_share_key() {
        let core = Core::build(ApidocConfig::default());
        let id = "test-share-session";
        insert_session_with(id, "", "no-such-share-uic1-test", 1).await;
        let mismatch = json!({"sessionId": id}).to_string();
        let res = debug_poll(&core, mismatch.as_bytes(), "").await;
        assert_eq!(res.status, 401, "missing shareKey must be rejected");
        let missing = json!({"sessionId": id, "shareKey": "no-such-share-uic1-test"}).to_string();
        let res = debug_poll(&core, missing.as_bytes(), "").await;
        assert_eq!(res.status, 404, "unknown share record must be rejected");
        sessions().lock().await.remove(id);
    }
}
