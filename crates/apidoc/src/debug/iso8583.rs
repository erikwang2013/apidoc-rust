//! P2 ISO 8583 执行器（ASCII 报文 + 二进制位图）。
//!
//! 字段定义优先级：`req.opts.fields` > `ep.protocol_opts.fields` > `ep.params`。
//! 位图与域编解码在 `codec` 子模块；本文件只做目标校验、连接、framing 与读超时。
//! framing：`2`（默认，2 字节大端长度）/ `4` / `none`；ebcdic 留到 P6。
//!
//! shortcut: 一条连接只处理一条报文，收到尾随字节即报错。

mod codec;

pub use codec::{decode_message, encode_message};

use super::sockets::parse_host_port;
use super::{guard, DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint};
use codec::{decode_with_specs, encode_with_specs, fields_from_params, parse_fields, FieldSpec};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let encoding = opt(req, ep, "encoding").unwrap_or_else(|| "ascii".into());
    if encoding != "ascii" {
        return Err(format!(
            "iso8583 encoding `{encoding}` is not supported in this build; only ascii (ebcdic is P6)"
        ));
    }
    let specs = match opt(req, ep, "fields") {
        Some(fields) => parse_fields(&fields)?,
        None => match ep {
            Some(ep) if !ep.params.is_empty() => fields_from_params(&ep.params)?,
            _ => {
                return Err("iso8583 requires opts.fields JSON or endpoint params with field/ty".into())
            }
        },
    };

    let body: Value = match req.body.as_ref() {
        Some(payload) => {
            let bytes = payload.decode()?;
            if bytes.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(&bytes)
                    .map_err(|e| format!("invalid iso8583 request JSON: {e}"))?
            }
        }
        None => json!({}),
    };
    let mti = opt(req, ep, "mti")
        .or_else(|| body.get("mti").and_then(Value::as_str).map(str::to_string))
        .ok_or("iso8583 requires opts.mti (e.g. \"0200\")")?;
    let message = encode_with_specs(&mti, &specs, &body)?;
    if message.len() > cfg.max_bytes {
        return Err(format!("iso8583 request exceeds max_bytes ({})", cfg.max_bytes));
    }

    let framing = opt(req, ep, "framing").unwrap_or_else(|| "2".into());
    let width = match framing.as_str() {
        "2" => 2,
        "4" => 4,
        "none" => 0,
        other => {
            return Err(format!(
                "unsupported iso8583 framing `{other}`; use 2, 4 or none"
            ))
        }
    };
    if width == 2 && message.len() > u16::MAX as usize {
        return Err("iso8583 request exceeds 2-byte framing length".into());
    }

    let (host, port) = parse_host_port(&req.target, "tcp")?;
    let addrs = guard::check_host(cfg, &host, port).await?;
    let peer = addrs[0];
    let budget = Duration::from_millis(cfg.timeout_ms.max(1));
    let exchange = async {
        let mut stream = TcpStream::connect(peer)
            .await
            .map_err(|e| format!("iso8583 connect failed: {e}"))?;
        let frame = match width {
            2 => {
                let mut frame = Vec::with_capacity(2 + message.len());
                frame.extend_from_slice(&(message.len() as u16).to_be_bytes());
                frame.extend_from_slice(&message);
                frame
            }
            4 => {
                let mut frame = Vec::with_capacity(4 + message.len());
                frame.extend_from_slice(&(message.len() as u32).to_be_bytes());
                frame.extend_from_slice(&message);
                frame
            }
            _ => message.clone(),
        };
        stream
            .write_all(&frame)
            .await
            .map_err(|e| format!("iso8583 write failed: {e}"))?;
        if width == 0 {
            read_until_parsed(&mut stream, cfg, req, &specs).await
        } else {
            read_framed(&mut stream, width, cfg.max_bytes).await
        }
    };
    let response = tokio::time::timeout(budget, exchange)
        .await
        .map_err(|_| "iso8583 request timed out".to_string())??;
    let value = decode_with_specs(&response, &specs)?;

    let mut headers = BTreeMap::new();
    headers.insert("x-peer".into(), peer.to_string());
    headers.insert(
        "x-mti".into(),
        value["mti"].as_str().unwrap_or_default().to_string(),
    );
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload {
            encoding: "utf8".into(),
            data: serde_json::to_string(&value)
                .map_err(|e| format!("iso8583 response encode failed: {e}"))?,
        },
        frames: Vec::new(),
    })
}

fn opt(req: &DebugRequest, ep: Option<&DocEndpoint>, key: &str) -> Option<String> {
    req.opts
        .get(key)
        .cloned()
        .or_else(|| ep.and_then(|e| e.protocol_opts.get(key).cloned()))
}

async fn read_framed(
    stream: &mut TcpStream,
    width: usize,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut head = vec![0u8; width];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|e| format!("iso8583 read length failed: {e}"))?;
    let len = match width {
        2 => u16::from_be_bytes([head[0], head[1]]) as usize,
        _ => u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize,
    };
    if len > max_bytes {
        return Err(format!(
            "iso8583 response exceeds max_bytes ({max_bytes})"
        ));
    }
    let mut body = vec![0u8; len];
    stream
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("iso8583 read body failed: {e}"))?;
    Ok(body)
}

async fn read_until_parsed(
    stream: &mut TcpStream,
    cfg: &DebugConfig,
    req: &DebugRequest,
    specs: &[FieldSpec],
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + Duration::from_millis(read_ms(cfg, req));
    let mut buf = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut chunk = [0u8; 4096];
        match tokio::time::timeout(remaining, stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > cfg.max_bytes {
                    return Err(format!(
                        "iso8583 response exceeds max_bytes ({})",
                        cfg.max_bytes
                    ));
                }
                match decode_with_specs(&buf, specs) {
                    Ok(_) => return Ok(buf),
                    Err(e) if e == "incomplete" => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(Err(e)) => return Err(format!("iso8583 read failed: {e}")),
            Err(_) => break,
        }
    }
    if buf.is_empty() {
        Err("iso8583 response timed out".into())
    } else {
        Err("incomplete iso8583 response".into())
    }
}

fn read_ms(cfg: &DebugConfig, req: &DebugRequest) -> u64 {
    req.opts
        .get("read_ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
        .clamp(1, cfg.timeout_ms.max(1))
}
