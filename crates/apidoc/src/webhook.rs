//! P6: webhook inbound capture — a bounded, rate-limited ring buffer per configured
//! webhook id. `POST /apidoc/webhook/{id}` is public (optional HMAC-SHA256), the
//! two GET routes go through the normal auth guard.
//!
//! Bounds are not optional: this is an unauthenticated write endpoint. Every
//! stored event is capped by `max_body`, each ring by `capacity`/total bytes, and
//! writes are throttled per webhook id. Request bodies are never logged.

use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_MAX_BODY: usize = 64 * 1024;
pub const DEFAULT_CAPACITY: usize = 50;
/// Absolute cap for `max_body` regardless of configuration (1 MiB).
pub const MAX_BODY: usize = 1_048_576;
/// Absolute cap for `capacity` regardless of configuration.
const MAX_CAPACITY: usize = 200;
/// Total stored bytes per webhook (oldest events are evicted first).
const MAX_STORE_BYTES: usize = 8 * 1024 * 1024;
/// 单次 GET 响应上限：保证序列化 + 传输本地耗时远低于 10ms。
const MAX_EVENTS_PER_RESPONSE: usize = 100;
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

// Fixed-window throttles for unauthenticated writes: per webhook id and a
// registry-wide total so a burst across many ids cannot exhaust memory/CPU.
// shortcut: the per-id bucket is keyed only by the public id, so a leaked id can
// still starve its own capture — put a per-IP gateway limit in front when that matters.
const RATE_LIMIT: u32 = 100;
const GLOBAL_RATE_LIMIT: u32 = 1000;
const RATE_WINDOW: Duration = Duration::from_secs(60);

// Header storage bounds (attacker-controlled input).
const MAX_HEADERS: usize = 32;
const MAX_HEADER_NAME: usize = 64;
const MAX_HEADER_VALUE: usize = 1024;

/// `ApidocConfig.webhooks` entry. `id` must be unguessable (use [`generate_id`]);
/// `secret` enables HMAC-SHA256 verification of `X-Apidoc-Signature`.
/// `max_body`/`capacity` of 0 mean the defaults (64 KiB / 50).
/// No `Debug` on purpose: it would print the secret into logs.
#[derive(Clone, Default)]
pub struct WebhookConfig {
    pub id: String,
    pub title: String,
    pub secret: Option<String>,
    pub max_body: usize,
    pub capacity: usize,
}

/// 32 hex chars from the OS entropy source.
pub fn generate_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("OS entropy source must be available");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Serialize)]
struct StoredEvent {
    seq: u64,
    ts: u64,
    headers: BTreeMap<String, String>,
    body: StoredBody,
}

#[derive(Clone, Serialize)]
struct StoredBody {
    encoding: &'static str,
    data: String,
}

#[derive(Default)]
struct Store {
    events: VecDeque<StoredEvent>,
    bytes: usize,
}

struct RateWindow {
    start: Instant,
    count: u32,
}

struct WebhookRuntime {
    cfg: WebhookConfig,
    store: Mutex<Store>,
    seq: AtomicU64,
    rate: Mutex<RateWindow>,
}

pub struct WebhookRegistry {
    hooks: Vec<WebhookRuntime>,
    /// Cross-id write budget for this registry (unknown ids never reach it).
    gate: Mutex<RateWindow>,
}

impl WebhookRuntime {
    fn new(mut cfg: WebhookConfig) -> Self {
        if cfg.max_body == 0 {
            cfg.max_body = DEFAULT_MAX_BODY;
        }
        cfg.max_body = cfg.max_body.min(MAX_BODY);
        if cfg.capacity == 0 {
            cfg.capacity = DEFAULT_CAPACITY;
        }
        cfg.capacity = cfg.capacity.min(MAX_CAPACITY);
        if cfg.secret.as_deref().is_some_and(str::is_empty) {
            eprintln!(
                "apidoc: webhook `{}` has an empty secret — HMAC verification disabled",
                cfg.id
            );
            cfg.secret = None;
        }
        WebhookRuntime {
            cfg,
            store: Mutex::new(Store::default()),
            seq: AtomicU64::new(0),
            rate: Mutex::new(RateWindow { start: Instant::now(), count: 0 }),
        }
    }

    fn push(&self, event: StoredEvent) {
        let size = event_size(&event);
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        while !store.events.is_empty()
            && (store.events.len() >= self.cfg.capacity || store.bytes + size > MAX_STORE_BYTES)
        {
            if let Some(old) = store.events.pop_front() {
                store.bytes = store.bytes.saturating_sub(event_size(&old));
            }
        }
        if size > MAX_STORE_BYTES {
            return;
        }
        store.bytes += size;
        store.events.push_back(event);
    }
}

impl WebhookRegistry {
    pub fn new(configs: Vec<WebhookConfig>) -> Self {
        let mut hooks: Vec<WebhookRuntime> = Vec::new();
        for cfg in configs {
            if cfg.id.is_empty() {
                eprintln!("apidoc: ignoring webhook with empty id (use webhook::generate_id())");
                continue;
            }
            if hooks.iter().any(|h| h.cfg.id == cfg.id) {
                eprintln!("apidoc: duplicate webhook id `{}`, keeping the first", cfg.id);
                continue;
            }
            hooks.push(WebhookRuntime::new(cfg));
        }
        WebhookRegistry { hooks, gate: Mutex::new(RateWindow { start: Instant::now(), count: 0 }) }
    }

    /// Charge a write against both the per-id bucket and the registry-wide cap.
    /// Lock order is gate → hook.rate; no other path locks both.
    fn allow_post(&self, hook: &WebhookRuntime) -> bool {
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut window = hook.rate.lock().unwrap_or_else(|e| e.into_inner());
        if gate.start.elapsed() >= RATE_WINDOW {
            *gate = RateWindow { start: Instant::now(), count: 0 };
        }
        if window.start.elapsed() >= RATE_WINDOW {
            *window = RateWindow { start: Instant::now(), count: 0 };
        }
        if gate.count >= GLOBAL_RATE_LIMIT || window.count >= RATE_LIMIT {
            return false;
        }
        gate.count += 1;
        window.count += 1;
        true
    }

    /// Public write path. Returns (status, JSON body).
    pub fn post(
        &self,
        id: &str,
        signature: Option<&str>,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> (u16, String) {
        let Some(hook) = self.hooks.iter().find(|h| h.cfg.id == id) else {
            return (404, r#"{"error":"unknown webhook"}"#.to_string());
        };
        if !self.allow_post(hook) {
            return (429, r#"{"error":"rate limit exceeded"}"#.to_string());
        }
        if body.len() > hook.cfg.max_body {
            return (413, r#"{"error":"body too large"}"#.to_string());
        }
        if let Some(secret) = hook.cfg.secret.as_deref() {
            // HMAC-SHA256 hex is exactly 64 chars; reject longer input before allocating.
            let Some(given) = signature.filter(|s| s.trim().len() == 64).and_then(decode_hex)
            else {
                return (401, r#"{"error":"invalid signature"}"#.to_string());
            };
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
                .expect("HMAC accepts any key length");
            mac.update(body);
            if !ct_eq(&given, &mac.finalize().into_bytes()) {
                return (401, r#"{"error":"invalid signature"}"#.to_string());
            }
        }
        let event = StoredEvent {
            seq: hook.seq.fetch_add(1, Ordering::Relaxed) + 1,
            ts: now_ms(),
            headers: bounded_headers(headers),
            body: stored_body(body),
        };
        let seq = event.seq;
        hook.push(event);
        (202, format!("{{\"seq\":{seq}}}"))
    }

    /// Guarded event read. Returns (status, JSON body).
    pub fn events(&self, id: &str, since: u64) -> (u16, String) {
        let Some(hook) = self.hooks.iter().find(|h| h.cfg.id == id) else {
            return (404, r#"{"error":"unknown webhook"}"#.to_string());
        };
        let store = hook.store.lock().unwrap_or_else(|e| e.into_inner());
        let mut events: Vec<&StoredEvent> = Vec::new();
        let mut bytes = 0usize;
        let mut truncated = false;
        for event in store.events.iter().filter(|e| e.seq > since) {
            let size = event_size(event);
            if events.len() >= MAX_EVENTS_PER_RESPONSE || bytes + size > MAX_RESPONSE_BYTES {
                truncated = true;
                break;
            }
            bytes += size;
            events.push(event);
        }
        (
            200,
            serde_json::json!({ "events": events, "truncated": truncated }).to_string(),
        )
    }

    /// Guarded configuration list — id/title only, never the secret.
    pub fn list(&self) -> String {
        let hooks: Vec<serde_json::Value> = self
            .hooks
            .iter()
            .map(|h| serde_json::json!({ "id": h.cfg.id, "title": h.cfg.title }))
            .collect();
        serde_json::json!({ "webhooks": hooks }).to_string()
    }
}

fn stored_body(body: &[u8]) -> StoredBody {
    match std::str::from_utf8(body) {
        Ok(text) => StoredBody { encoding: "utf8", data: text.to_string() },
        Err(_) => {
            StoredBody { encoding: "hex", data: body.iter().map(|b| format!("{b:02x}")).collect() }
        }
    }
}

fn bounded_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(k, v)| k.len() <= MAX_HEADER_NAME && v.len() <= MAX_HEADER_VALUE)
        .take(MAX_HEADERS)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn event_size(e: &StoredEvent) -> usize {
    let headers: usize = e.headers.iter().map(|(k, v)| k.len() + v.len()).sum();
    64 + headers + e.body.data.len()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push((hex_val(pair[0])? << 4) | hex_val(pair[1])?);
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_decoder_accepts_both_cases_and_rejects_junk() {
        assert_eq!(decode_hex("ff00AB").as_deref(), Some(&[0xff, 0x00, 0xab][..]));
        assert_eq!(decode_hex(" zz "), None);
        assert_eq!(decode_hex("abc"), None);
    }

    #[test]
    fn config_defaults_and_clamps() {
        let r = WebhookRegistry::new(vec![
            WebhookConfig { id: "a".into(), ..Default::default() },
            WebhookConfig { id: "a".into(), ..Default::default() },
            WebhookConfig { id: String::new(), ..Default::default() },
            WebhookConfig {
                id: "b".into(),
                max_body: 10 * MAX_BODY,
                capacity: 10_000,
                ..Default::default()
            },
        ]);
        assert_eq!(r.hooks.len(), 2, "duplicate and empty ids are dropped");
        assert_eq!(r.hooks[0].cfg.max_body, DEFAULT_MAX_BODY);
        assert_eq!(r.hooks[0].cfg.capacity, DEFAULT_CAPACITY);
        assert_eq!(r.hooks[1].cfg.max_body, MAX_BODY);
        assert_eq!(r.hooks[1].cfg.capacity, MAX_CAPACITY);
    }

    #[test]
    fn list_never_contains_secret() {
        let r = WebhookRegistry::new(vec![WebhookConfig {
            id: "a".into(),
            title: "t".into(),
            secret: Some("super-secret".into()),
            ..Default::default()
        }]);
        assert!(!r.list().contains("super-secret"));
    }
}
