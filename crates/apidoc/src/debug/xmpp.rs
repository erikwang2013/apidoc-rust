//! P3 XMPP 执行器（tokio-xmpp，STARTTLS + SASL）。
//!
//! 一元语义：连接（STARTTLS 强制）→ 等 Online →（有 body 时）发一条
//! message → 收集 `messages` 条或到 `duration_ms` → `</stream:stream>`。
//! 凭据只从 `opts` 读取；错误文本中的密码会被替换。
//!
//! TLS 由 tokio-xmpp 的 starttls-rust 提供（webpki 根证书，证书校验对
//! JID 域名），无跳过校验开关。target 只决定 TCP 地址；guard 校验后的 IP
//! 用于拨号，TLS SNI 仍是 JID 域名。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint};
use futures_util::StreamExt;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio_xmpp::parsers::message::Message;
use tokio_xmpp::{starttls::ServerConfig, AsyncClient as XmppClient, AsyncConfig};

const MAX_FRAMES: usize = 1000;

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    _ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let jid_raw = req
        .opts
        .get("jid")
        .filter(|s| !s.is_empty())
        .ok_or("xmpp requires opts.jid")?;
    let jid = jid_raw
        .parse::<tokio_xmpp::jid::Jid>()
        .map_err(|e| format!("invalid xmpp jid: {e}"))?;
    let password = req
        .opts
        .get("password")
        .filter(|s| !s.is_empty())
        .ok_or("xmpp requires opts.password")?
        .clone();
    let message_type = match req.opts.get("type").map(String::as_str) {
        None | Some("chat") => "chat",
        Some("groupchat") => "groupchat",
        Some(other) => {
            return Err(format!(
                "invalid xmpp message type `{other}`; use chat or groupchat"
            ))
        }
    };
    let to = match req.opts.get("to").filter(|s| !s.is_empty()) {
        Some(raw) => Some(
            raw.parse::<tokio_xmpp::jid::Jid>()
                .map_err(|e| format!("invalid xmpp recipient jid: {e}"))?,
        ),
        None => None,
    };
    let body = match req.body.as_ref() {
        Some(payload) => {
            let bytes = payload.decode()?;
            if bytes.len() > cfg.max_bytes {
                return Err(format!(
                    "request body exceeds max_bytes ({})",
                    cfg.max_bytes
                ));
            }
            Some(String::from_utf8(bytes).map_err(|_| "xmpp body must be valid utf8")?)
        }
        None => None,
    };
    if body.is_some() && to.is_none() {
        return Err("xmpp send requires opts.to".into());
    }

    let (host, port) = if req.target.trim().is_empty() {
        (jid.domain().to_string(), 5222)
    } else {
        parse_target(&req.target)?
    };
    let addrs = guard::check_host(cfg, &host, port).await?;
    let mut client = XmppClient::new_with_config(AsyncConfig {
        jid,
        password: password.clone(),
        server: ServerConfig::Manual {
            host: addrs[0].ip().to_string(),
            port: addrs[0].port(),
        },
    });

    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    match tokio::time::timeout(connect_timeout, wait_online(&mut client, &password)).await {
        Err(_) => {
            // 连接任务在 client 内部 spawn；drop client 会关闭已建立的 socket。
            return Err("xmpp did not come online in time".into());
        }
        Ok(Err(e)) => return Err(e),
        Ok(Ok(())) => {}
    }

    if let (Some(body), Some(to)) = (body, to) {
        let message = match message_type {
            "groupchat" => Message::groupchat(to),
            _ => Message::chat(to),
        }
        .with_body("en".to_string(), body);
        client
            .send_stanza(message.into())
            .await
            .map_err(|e| redact(&format!("xmpp send failed: {e}"), &password))?;
    }

    let want = opts_usize(req, "messages")
        .unwrap_or(1)
        .clamp(1, MAX_FRAMES);
    let deadline = Instant::now() + Duration::from_millis(duration_ms(cfg, req));
    let mut messages = Vec::new();
    let mut total = 0usize;
    while messages.len() < want {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let event = match tokio::time::timeout(remaining, client.next()).await {
            Err(_) => break,
            Ok(None) => break,
            Ok(Some(event)) => event,
        };
        if let tokio_xmpp::Event::Disconnected(e) = event {
            return Err(redact(&format!("xmpp disconnected: {e}"), &password));
        }
        let Some(element) = event.into_stanza() else {
            continue;
        };
        let Ok(message) = Message::try_from(element) else {
            continue;
        };
        let Some((_lang, text)) = message.get_best_body(vec!["en"]) else {
            continue;
        };
        total += text.0.len();
        if total > cfg.max_bytes {
            return Err(format!(
                "xmpp messages exceed max_bytes ({})",
                cfg.max_bytes
            ));
        }
        messages.push(json!({
            "from": message.from.as_ref().map(|from| from.to_string()).unwrap_or_default(),
            "body": text.0,
        }));
    }

    let _ = client.send_end().await;
    drop(client);

    let mut headers = BTreeMap::new();
    headers.insert("x-messages".to_string(), messages.len().to_string());
    let body = json!({ "messages": messages });
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload {
            encoding: "utf8".into(),
            data: body.to_string(),
        },
        frames: Vec::new(),
    })
}

async fn wait_online(client: &mut XmppClient<ServerConfig>, password: &str) -> Result<(), String> {
    loop {
        match client.next().await {
            None => return Err("xmpp connection closed before online".into()),
            Some(tokio_xmpp::Event::Online { .. }) => return Ok(()),
            Some(tokio_xmpp::Event::Disconnected(e)) => {
                return Err(redact(&format!("xmpp connect failed: {e}"), password))
            }
            Some(_) => {}
        }
    }
}

fn parse_target(target: &str) -> Result<(String, u16), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("xmpp://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid xmpp target: {e}"))?;
    match url.scheme() {
        "xmpp" => {}
        "xmpps" => {
            return Err(
                "xmpps (direct TLS) is not supported; use xmpp:// which upgrades via STARTTLS"
                    .into(),
            )
        }
        other => return Err(format!("unsupported xmpp scheme `{other}`; use xmpp://")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("xmpp credentials must be provided via opts, not the target URL".into());
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() {
        return Err("invalid xmpp target; use xmpp://host[:port]".into());
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    Ok((host, url.port().unwrap_or(5222)))
}

fn duration_ms(cfg: &DebugConfig, req: &DebugRequest) -> u64 {
    opts_u64(req, "duration_ms")
        .unwrap_or(2000)
        .clamp(1, cfg.timeout_ms.max(1))
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_usize(req: &DebugRequest, key: &str) -> Option<usize> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "***")
    }
}
