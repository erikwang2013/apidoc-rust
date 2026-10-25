//! P3 MQTT 执行器（rumqttc）。
//!
//! 一元语义：CONNECT → SUBSCRIBE →（有 body 时）PUBLISH → 收集 `messages`
//! 条或到 `duration_ms` → DISCONNECT。凭据只从 `opts` 读取，错误文本里
//! 的密码会被替换。TLS 走 rustls（系统根证书），不提供跳过校验开关。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const MAX_FRAMES: usize = 1000;

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    _ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let topic = req
        .opts
        .get("topic")
        .filter(|s| !s.is_empty())
        .ok_or("mqtt requires opts.topic")?
        .clone();
    let qos = match req.opts.get("qos").map(String::as_str) {
        None | Some("0") => QoS::AtMostOnce,
        Some("1") => QoS::AtLeastOnce,
        Some("2") => QoS::ExactlyOnce,
        Some(other) => return Err(format!("invalid mqtt qos `{other}`; use 0, 1 or 2")),
    };
    let password = req.opts.get("password").cloned().unwrap_or_default();
    let (host, port, tls) = parse_target(&req.target)?;
    let addrs = guard::check_host(cfg, &host, port).await?;
    if tls {
        return Err(
            "mqtt over TLS requires a pinned transport; use mqtt:// (plaintext) in this build"
                .into(),
        );
    }

    // 明文连接钉到已校验 IP；TLS 需要原始 host 做 SNI。
    let (broker_host, broker_port) = if tls {
        (host, port)
    } else {
        (addrs[0].ip().to_string(), addrs[0].port())
    };
    let client_id = req
        .opts
        .get("client_id")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| {
            format!(
                "apidoc-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            )
        });

    let mut opts = MqttOptions::new(client_id, broker_host, broker_port);
    opts.set_keep_alive(Duration::from_secs(5));
    opts.set_clean_session(true);
    if let Some(username) = req.opts.get("username") {
        opts.set_credentials(username.clone(), password.clone());
    }
    if tls {
        opts.set_transport(Transport::tls_with_config(TlsConfiguration::default()));
    }

    let (client, mut eventloop) = AsyncClient::new(opts, 16);
    client
        .subscribe(topic.clone(), qos)
        .await
        .map_err(|e| redact(&format!("mqtt subscribe failed: {e}"), &password))?;

    if let Some(body) = req.body.as_ref() {
        let bytes = body.decode()?;
        if bytes.len() > cfg.max_bytes {
            return Err(format!(
                "request body exceeds max_bytes ({})",
                cfg.max_bytes
            ));
        }
        if !bytes.is_empty() {
            client
                .publish(topic.clone(), qos, false, bytes)
                .await
                .map_err(|e| redact(&format!("mqtt publish failed: {e}"), &password))?;
        }
    }

    let want = opts_usize(req, "messages")
        .unwrap_or(1)
        .clamp(1, MAX_FRAMES);
    let deadline = Instant::now() + Duration::from_millis(duration_ms(cfg, req));
    let mut messages = Vec::new();
    let mut total = 0usize;
    let mut connected = false;
    while messages.len() < want {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let event = match tokio::time::timeout(remaining, eventloop.poll()).await {
            Err(_) => break,
            Ok(Ok(event)) => event,
            Ok(Err(e)) => return Err(redact(&format!("mqtt connection failed: {e}"), &password)),
        };
        match event {
            Event::Incoming(Packet::ConnAck(_)) => connected = true,
            Event::Incoming(Packet::Publish(publish)) => {
                total += publish.payload.len();
                if total > cfg.max_bytes {
                    return Err(format!(
                        "mqtt messages exceed max_bytes ({})",
                        cfg.max_bytes
                    ));
                }
                messages.push(json!({
                    "topic": publish.topic,
                    "payload": Payload::from_bytes(&publish.payload),
                    "qos": qos_number(publish.qos),
                }));
            }
            _ => {}
        }
    }
    if !connected {
        return Err("mqtt did not receive CONNACK in time".into());
    }

    let _ = client.disconnect().await;
    let _ = tokio::time::timeout(Duration::from_millis(100), eventloop.poll()).await;
    drop(eventloop);

    let mut headers = BTreeMap::new();
    headers.insert("x-topic".to_string(), topic);
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

fn parse_target(target: &str) -> Result<(String, u16, bool), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("mqtt://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid mqtt target: {e}"))?;
    let tls = match url.scheme() {
        "mqtt" => false,
        "mqtts" | "ssl" => true,
        other => {
            return Err(format!(
                "unsupported mqtt scheme `{other}`; use mqtt:// or mqtts://"
            ))
        }
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err("mqtt credentials must be provided via opts, not the target URL".into());
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    let port = url.port().unwrap_or(if tls { 8883 } else { 1883 });
    Ok((host, port, tls))
}

fn qos_number(qos: QoS) -> u8 {
    match qos {
        QoS::AtMostOnce => 0,
        QoS::AtLeastOnce => 1,
        QoS::ExactlyOnce => 2,
    }
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
