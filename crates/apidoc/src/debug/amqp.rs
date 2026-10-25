//! P3 AMQP 0-9-1 执行器（lapin）。
//!
//! `consume=true` 时声明/消费队列（`declare=true` 时先声明），否则发布一条。
//! 凭据只从 `opts` 读取；连接 URI 由字段直接构造，避免密码出现在可打印的
//! URL 里。错误文本中的密码会被替换。TLS 走 rustls，不提供跳过校验开关。
//!
//! 消费不 ack：debug 读取不应吞掉业务消息，取消消费者并断开后 broker 会
//! 重新投递。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint};
use futures_util::StreamExt;
use lapin::{
    options::{BasicCancelOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions},
    publisher_confirm::Confirmation,
    types::FieldTable,
    uri::{AMQPAuthority, AMQPQueryString, AMQPScheme, AMQPUri, AMQPUserInfo},
    BasicProperties, Connection, ConnectionProperties,
};
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
    let consume = match req.opts.get("consume").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(other) => {
            return Err(format!(
                "invalid amqp consume `{other}`; use \"true\" or \"false\""
            ))
        }
    };
    let queue = req.opts.get("queue").filter(|s| !s.is_empty()).cloned();
    if consume && queue.is_none() {
        return Err("amqp consume requires opts.queue".into());
    }
    let routing_key = req
        .opts
        .get("routing_key")
        .filter(|s| !s.is_empty())
        .cloned();
    if !consume && routing_key.is_none() {
        return Err("amqp publish requires opts.routing_key".into());
    }
    let password = req
        .opts
        .get("password")
        .cloned()
        .unwrap_or_else(|| "guest".into());

    let (host, port, tls) = parse_target(&req.target)?;
    let addrs = guard::check_host(cfg, &host, port).await?;
    if tls {
        return Err(
            "amqp over TLS requires a pinned transport; use amqp:// (plaintext) in this build"
                .into(),
        );
    }
    let connect_host = if tls { host } else { addrs[0].ip().to_string() };
    let uri = AMQPUri {
        scheme: if tls {
            AMQPScheme::AMQPS
        } else {
            AMQPScheme::AMQP
        },
        authority: AMQPAuthority {
            userinfo: AMQPUserInfo {
                username: req
                    .opts
                    .get("username")
                    .cloned()
                    .unwrap_or_else(|| "guest".into()),
                password: password.clone(),
            },
            host: connect_host,
            port: addrs[0].port(),
        },
        vhost: req.opts.get("vhost").cloned().unwrap_or_else(|| "/".into()),
        query: AMQPQueryString::default(),
    };

    let timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let connection = tokio::time::timeout(
        timeout,
        Connection::connect_uri(uri, ConnectionProperties::default()),
    )
    .await
    .map_err(|_| "amqp connect timed out".to_string())?
    .map_err(|e| redact(&format!("amqp connect failed: {e}"), &password))?;
    let channel = connection
        .create_channel()
        .await
        .map_err(|e| redact(&format!("amqp channel failed: {e}"), &password))?;

    let (body, messages) = if consume {
        let queue = queue.unwrap_or_default();
        if req.opts.get("declare").map(String::as_str) == Some("true") {
            channel
                .queue_declare(
                    &queue,
                    QueueDeclareOptions::default(),
                    FieldTable::default(),
                )
                .await
                .map_err(|e| redact(&format!("amqp queue declare failed: {e}"), &password))?;
        }
        let tag = format!(
            "apidoc-{}-{}",
            std::process::id(),
            started.elapsed().as_nanos()
        );
        let mut consumer = channel
            .basic_consume(
                &queue,
                &tag,
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|e| redact(&format!("amqp consume failed: {e}"), &password))?;

        let want = opts_usize(req, "messages")
            .unwrap_or(1)
            .clamp(1, MAX_FRAMES);
        let deadline = Instant::now() + Duration::from_millis(duration_ms(cfg, req));
        let mut deliveries = Vec::new();
        let mut total = 0usize;
        while deliveries.len() < want {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let delivery = match tokio::time::timeout(remaining, consumer.next()).await {
                Err(_) => break,
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    return Err(redact(&format!("amqp consume failed: {e}"), &password))
                }
                Ok(Some(Ok(delivery))) => delivery,
            };
            total += delivery.data.len();
            if total > cfg.max_bytes {
                return Err(format!(
                    "amqp messages exceed max_bytes ({})",
                    cfg.max_bytes
                ));
            }
            deliveries.push(json!({
                "exchange": delivery.exchange.to_string(),
                "routing_key": delivery.routing_key.to_string(),
                "payload": Payload::from_bytes(&delivery.data),
            }));
        }
        let _ = channel
            .basic_cancel(&tag, BasicCancelOptions::default())
            .await;
        let count = deliveries.len();
        let body = if count == 1 {
            json!({ "delivery": deliveries.remove(0) })
        } else {
            json!({ "deliveries": deliveries })
        };
        (body, count)
    } else {
        let body = match req.body.as_ref() {
            Some(payload) => payload.decode()?,
            None => Vec::new(),
        };
        if body.len() > cfg.max_bytes {
            return Err(format!(
                "request body exceeds max_bytes ({})",
                cfg.max_bytes
            ));
        }
        let confirmation = channel
            .basic_publish(
                req.opts.get("exchange").map(String::as_str).unwrap_or(""),
                routing_key.as_deref().unwrap_or(""),
                BasicPublishOptions::default(),
                &body,
                BasicProperties::default(),
            )
            .await
            .map_err(|e| redact(&format!("amqp publish failed: {e}"), &password))?;
        match tokio::time::timeout(timeout, confirmation).await {
            Err(_) => return Err("amqp publish confirmation timed out".into()),
            Ok(Err(e)) => {
                return Err(redact(
                    &format!("amqp publish confirmation failed: {e}"),
                    &password,
                ))
            }
            Ok(Ok(Confirmation::Nack(_))) => {
                return Err("amqp publish was rejected by the broker (nack)".into())
            }
            Ok(Ok(_)) => {}
        }
        (json!({ "published": true }), 0)
    };

    let _ = channel.close(200, "apidoc done").await;
    let _ = connection.close(200, "apidoc done").await;

    let mut headers = BTreeMap::new();
    headers.insert("x-messages".to_string(), messages.to_string());
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
        format!("amqp://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid amqp target: {e}"))?;
    let tls = match url.scheme() {
        "amqp" => false,
        "amqps" => true,
        other => {
            return Err(format!(
                "unsupported amqp scheme `{other}`; use amqp:// or amqps://"
            ))
        }
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err("amqp credentials must be provided via opts, not the target URL".into());
    }
    if !matches!(url.path(), "" | "/") {
        return Err("amqp vhost must be provided via opts.vhost, not the target URL".into());
    }
    if url.query().is_some() {
        return Err(
            "amqp connection parameters must be provided via opts, not the target URL".into(),
        );
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    let port = url.port().unwrap_or(if tls { 5671 } else { 5672 });
    Ok((host, port, tls))
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
