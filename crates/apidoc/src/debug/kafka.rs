//! P3 Kafka 执行器（rskafka，纯 Rust）。
//!
//! 一元语义：解析 brokers → 取分区客户端 → 从 `earliest`/`latest`/数字
//! offset 起拉取，收集 `messages` 条或到 `duration_ms`。只读，不写。
//! 凭据只从 `opts` 读取（SASL PLAIN），错误文本中的密码会被替换。
//! 本 feature 不含 TLS transport，`kafkas://` 明确报不支持而不是静默明文。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint};
use rskafka::client::{
    partition::{OffsetAt, UnknownTopicHandling},
    ClientBuilder, SaslConfig,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        .ok_or("kafka requires opts.topic")?
        .clone();
    let partition = match req.opts.get("partition") {
        None => 0,
        Some(raw) => raw
            .parse::<i32>()
            .map_err(|_| format!("invalid kafka partition `{raw}`; use a non-negative integer"))?,
    };
    if partition < 0 {
        return Err("kafka partition must be non-negative".into());
    }
    let start = match req.opts.get("offset").map(String::as_str) {
        None | Some("latest") => Start::Latest,
        Some("earliest") => Start::Earliest,
        Some(raw) => Start::At(raw.parse::<i64>().map_err(|_| {
            format!("invalid kafka offset `{raw}`; use earliest, latest or a number")
        })?),
    };
    let password = req.opts.get("password").cloned().unwrap_or_default();
    let sasl = match req.opts.get("username") {
        Some(username) => Some(SaslConfig::Plain {
            username: username.clone(),
            password: password.clone(),
        }),
        None if !password.is_empty() => {
            return Err("kafka opts.password requires opts.username".into())
        }
        None => None,
    };
    let brokers = resolve_brokers(cfg, &req.target).await?;
    // rskafka 没有 resolver 钩子：连接前先自己拉一次 Metadata，拒绝内网 broker。
    // 已知残差：rskafka 之后还会再拉一次 metadata，理论上存在窄 TOCTOU；至少阻断
    // 恶意 bootstrap 直接返回 127.0.0.1/169.254.169.254 的常见攻击。
    validate_metadata_brokers(cfg, &brokers[0], &topic).await?;

    let budget = cfg.max_bytes.clamp(1024, i32::MAX as usize) as i32;
    let mut builder = ClientBuilder::new(brokers).max_message_size(budget as usize);
    if let Some(sasl) = sasl {
        builder = builder.sasl_config(sasl);
    }
    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let client = tokio::time::timeout(connect_timeout, builder.build())
        .await
        .map_err(|_| "kafka connect timed out".to_string())?
        .map_err(|e| redact(&format!("kafka connect failed: {e}"), &password))?;
    let partition_client = tokio::time::timeout(
        connect_timeout,
        client.partition_client(topic.clone(), partition, UnknownTopicHandling::Error),
    )
    .await
    .map_err(|_| "kafka metadata lookup timed out".to_string())?
    .map_err(|e| {
        redact(
            &format!("kafka topic `{topic}` unavailable: {e}"),
            &password,
        )
    })?;

    let mut offset = match start {
        Start::At(offset) => offset,
        Start::Earliest => client_offset(&partition_client, OffsetAt::Earliest, &password).await?,
        Start::Latest => client_offset(&partition_client, OffsetAt::Latest, &password).await?,
    };

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
        let wait_ms = remaining.as_millis().clamp(1, 500) as i32;
        let (records, _high_watermark) = match tokio::time::timeout(
            remaining,
            partition_client.fetch_records(offset, 1..budget, wait_ms),
        )
        .await
        {
            Err(_) => break,
            Ok(Ok(fetched)) => fetched,
            Ok(Err(e)) => return Err(redact(&format!("kafka fetch failed: {e}"), &password)),
        };
        for record in records {
            offset = record.offset + 1;
            let value = record.record.value.unwrap_or_default();
            total += value.len();
            if total > cfg.max_bytes {
                return Err(format!(
                    "kafka messages exceed max_bytes ({})",
                    cfg.max_bytes
                ));
            }
            messages.push(json!({
                "partition": partition,
                "offset": record.offset,
                "payload": Payload::from_bytes(&value),
            }));
            if messages.len() >= want {
                break;
            }
        }
    }

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

enum Start {
    Earliest,
    Latest,
    At(i64),
}

async fn client_offset(
    partition_client: &rskafka::client::partition::PartitionClient,
    at: OffsetAt,
    password: &str,
) -> Result<i64, String> {
    partition_client
        .get_offset(at)
        .await
        .map_err(|e| redact(&format!("kafka offset lookup failed: {e}"), password))
}

/// `kafka://h1:9092,h2:9092` 或裸 `h1:9092,h2:9092`；逐个过 SSRF guard，
/// 返回已校验 IP（纯明文，无 SNI 需求）。
async fn resolve_brokers(cfg: &DebugConfig, target: &str) -> Result<Vec<String>, String> {
    let list = match target.strip_prefix("kafka://") {
        Some(rest) => rest,
        None if target.contains("://") => {
            return Err("unsupported kafka scheme; use kafka:// (plaintext only)".into())
        }
        None => target,
    };
    let mut brokers = Vec::new();
    for entry in list.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            return Err("kafka target contains an empty broker address".into());
        }
        let url = reqwest::Url::parse(&format!("kafka://{entry}"))
            .map_err(|e| format!("invalid kafka broker address: {e}"))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err("kafka credentials must be provided via opts, not the target URL".into());
        }
        if !matches!(url.path(), "" | "/") || url.query().is_some() {
            return Err("invalid kafka broker address".into());
        }
        let host = url
            .host_str()
            .ok_or_else(|| format!("kafka broker `{entry}` requires a host"))?;
        let port = url.port().unwrap_or(9092);
        let addrs = guard::check_host(cfg, host, port).await?;
        brokers.push(format!("{}:{}", addrs[0].ip(), addrs[0].port()));
    }
    Ok(brokers)
}

fn duration_ms(cfg: &DebugConfig, req: &DebugRequest) -> u64 {
    opts_u64(req, "duration_ms")
        .unwrap_or(2000)
        .clamp(1, cfg.timeout_ms.max(1))
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

async fn validate_metadata_brokers(
    cfg: &DebugConfig,
    bootstrap: &str,
    topic: &str,
) -> Result<(), String> {
    let timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let mut stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(bootstrap))
        .await
        .map_err(|_| "kafka metadata connect timed out".to_string())?
        .map_err(|e| format!("kafka metadata connect failed: {e}"))?;
    stream
        .write_all(&metadata_request(topic))
        .await
        .map_err(|e| format!("kafka metadata write failed: {e}"))?;
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("kafka metadata read failed: {e}"))?;
    let len = i32::from_be_bytes(len_buf);
    if len < 0 || len as usize > cfg.max_bytes {
        return Err(format!("kafka metadata response length {len} exceeds max_bytes"));
    }
    let mut response = vec![0u8; len as usize];
    stream
        .read_exact(&mut response)
        .await
        .map_err(|e| format!("kafka metadata body read failed: {e}"))?;
    for (host, port) in parse_metadata_brokers(&response)? {
        guard::check_host(cfg, &host, port)
            .await
            .map_err(|e| format!("kafka metadata broker `{host}:{port}` rejected: {e}"))?;
    }
    Ok(())
}

fn metadata_request(topic: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&3i16.to_be_bytes()); // Metadata
    body.extend_from_slice(&1i16.to_be_bytes()); // version 1
    body.extend_from_slice(&1i32.to_be_bytes()); // correlation id
    put_string(&mut body, "apidoc");
    body.extend_from_slice(&1i32.to_be_bytes()); // one topic
    put_string(&mut body, topic);
    let mut out = (body.len() as i32).to_be_bytes().to_vec();
    out.extend_from_slice(&body);
    out
}

fn parse_metadata_brokers(response: &[u8]) -> Result<Vec<(String, u16)>, String> {
    let mut pos = 0;
    let _correlation_id = read_i32(response, &mut pos)?;
    let count = read_i32(response, &mut pos)?;
    if count < 0 || count > 1000 {
        return Err(format!("invalid kafka broker count {count}"));
    }
    let mut brokers = Vec::new();
    for _ in 0..count {
        let _node_id = read_i32(response, &mut pos)?;
        let host = read_string(response, &mut pos)?;
        let port = read_i32(response, &mut pos)?;
        let _rack = read_nullable_string(response, &mut pos)?;
        if !(1..=65535).contains(&port) {
            return Err(format!("invalid kafka broker port {port}"));
        }
        brokers.push((host, port as u16));
    }
    if brokers.is_empty() {
        return Err("kafka metadata returned no brokers".into());
    }
    Ok(brokers)
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as i16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn read_i32(bytes: &[u8], pos: &mut usize) -> Result<i32, String> {
    let end = pos.checked_add(4).ok_or("kafka metadata length overflow")?;
    let slice = bytes.get(*pos..end).ok_or("truncated kafka metadata")?;
    *pos = end;
    Ok(i32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_string(bytes: &[u8], pos: &mut usize) -> Result<String, String> {
    let len = read_i16(bytes, pos)?;
    if len < 0 {
        return Err("kafka metadata string length is negative".into());
    }
    let end = pos.checked_add(len as usize).ok_or("kafka metadata length overflow")?;
    let slice = bytes.get(*pos..end).ok_or("truncated kafka metadata string")?;
    *pos = end;
    String::from_utf8(slice.to_vec()).map_err(|_| "kafka metadata string is not UTF-8".into())
}

fn read_nullable_string(bytes: &[u8], pos: &mut usize) -> Result<Option<String>, String> {
    let len = read_i16(bytes, pos)?;
    if len < 0 {
        return Ok(None);
    }
    let end = pos.checked_add(len as usize).ok_or("kafka metadata length overflow")?;
    let slice = bytes.get(*pos..end).ok_or("truncated kafka metadata string")?;
    *pos = end;
    String::from_utf8(slice.to_vec())
        .map(Some)
        .map_err(|_| "kafka metadata string is not UTF-8".into())
}

fn read_i16(bytes: &[u8], pos: &mut usize) -> Result<i16, String> {
    let end = pos.checked_add(2).ok_or("kafka metadata length overflow")?;
    let slice = bytes.get(*pos..end).ok_or("truncated kafka metadata")?;
    *pos = end;
    Ok(i16::from_be_bytes([slice[0], slice[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_response_and_request_roundtrip_shape() {
        let request = metadata_request("demo");
        // length + api_key 3 + version 1 + corr id 1 + client "apidoc" + topic "demo"
        assert_eq!(&request[4..6], &3i16.to_be_bytes());
        assert_eq!(&request[6..8], &1i16.to_be_bytes());
        assert!(request.len() > 20);

        let mut body = Vec::new();
        body.extend_from_slice(&1i32.to_be_bytes()); // correlation
        body.extend_from_slice(&1i32.to_be_bytes()); // one broker
        body.extend_from_slice(&1i32.to_be_bytes()); // node
        put_string(&mut body, "127.0.0.1");
        body.extend_from_slice(&9092i32.to_be_bytes());
        body.extend_from_slice(&(-1i16).to_be_bytes()); // null rack
        let brokers = parse_metadata_brokers(&body).unwrap();
        assert_eq!(brokers, vec![("127.0.0.1".to_string(), 9092)]);
    }
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
