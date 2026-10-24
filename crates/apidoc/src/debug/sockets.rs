//! P2 原始 socket 执行器：TCP / UDP / Raw / Unix Domain Socket。
//!
//! 一元语义：连接 → 写入 body → 在 read_ms 窗口内读取（或读到 expect_bytes）→ 关闭。
//! 所有目标先过 `guard::check_host`；Unix socket 需显式 allow_unix_sockets。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use base64::Engine;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let transport = match req.kind.as_str() {
        "tcp" => "tcp",
        "udp" => "udp",
        "raw" => req.opts.get("transport").map(String::as_str).unwrap_or("tcp"),
        "unix" => "unix",
        other => return Err(format!("unsupported socket kind `{other}`")),
    };

    if transport == "unix" {
        if !cfg.allow_unix_sockets {
            return Err(
                "unix sockets are disabled; set DebugConfig.allow_unix_sockets = true".into(),
            );
        }
        let path = unix_path(&req.target);
        let body = unix_exchange(cfg, req, &path).await?;
        return Ok(response(started, body, path, 1));
    }

    let (host, port) = parse_host_port(&req.target, transport)?;
    let addrs = guard::check_host(cfg, &host, port).await?;
    let addr = addrs[0];
    if transport == "udp" {
        let (body, count) = udp_exchange(cfg, req, addr).await?;
        Ok(response(started, body, addr.to_string(), count))
    } else {
        let body = tcp_exchange(cfg, req, addr).await?;
        Ok(response(started, body, addr.to_string(), 1))
    }
}

pub(crate) fn parse_host_port(target: &str, transport: &str) -> Result<(String, u16), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("{transport}://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme)
        .map_err(|e| format!("invalid target `{target}`: {e}"))?;
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    let port = url.port_or_known_default().ok_or("target requires a port")?;
    Ok((host, port))
}

fn unix_path(target: &str) -> String {
    target.strip_prefix("unix://").unwrap_or(target).to_string()
}

fn read_ms(cfg: &DebugConfig, req: &DebugRequest) -> u64 {
    opts_u64(req, "read_ms").unwrap_or(500).clamp(1, cfg.timeout_ms.max(1))
}

async fn tcp_exchange(
    cfg: &DebugConfig,
    req: &DebugRequest,
    addr: SocketAddr,
) -> Result<Vec<u8>, String> {
    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let mut stream = tokio::time::timeout(connect_timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| "tcp connect timed out".to_string())?
        .map_err(|e| format!("tcp connect failed: {e}"))?;
    write_body(cfg, req, &mut stream).await?;
    read_window(cfg, req, &mut stream).await
}

async fn unix_exchange(cfg: &DebugConfig, req: &DebugRequest, path: &str) -> Result<Vec<u8>, String> {
    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let mut stream = tokio::time::timeout(
        connect_timeout,
        tokio::net::UnixStream::connect(path),
    )
    .await
    .map_err(|_| "unix connect timed out".to_string())?
    .map_err(|e| format!("unix connect failed: {e}"))?;
    write_body(cfg, req, &mut stream).await?;
    read_window(cfg, req, &mut stream).await
}

async fn write_body<W: AsyncWriteExt + Unpin>(
    cfg: &DebugConfig,
    req: &DebugRequest,
    writer: &mut W,
) -> Result<(), String> {
    let Some(body) = req.body.as_ref() else {
        return Ok(());
    };
    let bytes = body.decode()?;
    if bytes.len() > cfg.max_bytes {
        return Err(format!("request body exceeds max_bytes ({})", cfg.max_bytes));
    }
    if !bytes.is_empty() {
        writer
            .write_all(&bytes)
            .await
            .map_err(|e| format!("socket write failed: {e}"))?;
    }
    Ok(())
}

async fn read_window<R: AsyncRead + Unpin>(
    cfg: &DebugConfig,
    req: &DebugRequest,
    reader: &mut R,
) -> Result<Vec<u8>, String> {
    let expect = opts_usize(req, "expect_bytes");
    let deadline = Instant::now() + Duration::from_millis(read_ms(cfg, req));
    let mut out = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut buf = [0u8; 4096];
        match tokio::time::timeout(remaining, reader.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > cfg.max_bytes {
                    return Err(format!("response exceeds max_bytes ({})", cfg.max_bytes));
                }
                if expect.is_some_and(|want| out.len() >= want) {
                    break;
                }
            }
            Ok(Err(e)) => return Err(format!("socket read failed: {e}")),
            Err(_) => break,
        }
    }
    Ok(out)
}

async fn udp_exchange(
    cfg: &DebugConfig,
    req: &DebugRequest,
    addr: SocketAddr,
) -> Result<(Vec<u8>, usize), String> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("udp bind failed: {e}"))?;
    socket
        .connect(addr)
        .await
        .map_err(|e| format!("udp connect failed: {e}"))?;
    let body = req
        .body
        .as_ref()
        .map(Payload::decode)
        .transpose()?
        .unwrap_or_default();
    if body.len() > cfg.max_bytes {
        return Err(format!("request body exceeds max_bytes ({})", cfg.max_bytes));
    }
    if !body.is_empty() {
        socket
            .send(&body)
            .await
            .map_err(|e| format!("udp send failed: {e}"))?;
    }

    let count = opts_usize(req, "datagrams").unwrap_or(1).clamp(1, 100);
    let deadline = Instant::now() + Duration::from_millis(read_ms(cfg, req));
    let mut datagrams: Vec<Vec<u8>> = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut buf = vec![0u8; 65_535];
        match tokio::time::timeout(remaining, socket.recv(&mut buf)).await {
            Ok(Ok(n)) => {
                total += n;
                if total > cfg.max_bytes {
                    return Err(format!("response exceeds max_bytes ({})", cfg.max_bytes));
                }
                datagrams.push(buf[..n].to_vec());
            }
            Ok(Err(e)) => return Err(format!("udp recv failed: {e}")),
            Err(_) => break,
        }
    }

    if datagrams.len() <= 1 {
        Ok((datagrams.into_iter().next().unwrap_or_default(), 1))
    } else {
        let arr: Vec<serde_json::Value> = datagrams
            .iter()
            .map(|d| {
                serde_json::json!({
                    "encoding": "base64",
                    "data": base64::engine::general_purpose::STANDARD.encode(d),
                })
            })
            .collect();
        let count = arr.len();
        Ok((
            serde_json::json!({ "datagrams": arr }).to_string().into_bytes(),
            count,
        ))
    }
}

fn response(
    started: Instant,
    body: Vec<u8>,
    peer: String,
    datagrams: usize,
) -> DebugResponse {
    let mut headers = BTreeMap::new();
    headers.insert("x-peer".into(), peer);
    headers.insert("x-datagrams".into(), datagrams.to_string());
    DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload::from_bytes(&body),
        frames: Vec::new(),
    }
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_usize(req: &DebugRequest, key: &str) -> Option<usize> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}
