//! P5 RTP 包级调试（RFC 3550）。
//!
//! 只收发 RTP 包并解析头部；不重建媒体流、不转码。目标先过 `guard::check_host`，
//! 发送/接收包数、单包大小与接收总量都有上限。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::json;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

/// 单次调用的收发包数上限（计划中的 max_frames）。
pub(crate) const MAX_DATAGRAMS: usize = 1000;
/// UDP 载荷上限：65507 是 IPv4 下 UDP 的最大载荷。
pub(crate) const MAX_DATAGRAM_BYTES: usize = 65_507;

#[derive(Debug, PartialEq, Eq)]
pub struct RtpPacket {
    pub version: u8,
    pub padding: bool,
    pub extension: bool,
    pub marker: bool,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub csrc: Vec<u32>,
    pub payload: Vec<u8>,
}

/// 固定 12 字节头 + 载荷；无 CSRC、扩展与 padding（调试发送用最简单形态）。
pub fn encode_packet(
    payload_type: u8,
    sequence: u16,
    timestamp: u32,
    ssrc: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + payload.len());
    out.push(0x80); // V=2, P=0, X=0, CC=0
    out.push(payload_type & 0x7f); // M=0
    out.extend_from_slice(&sequence.to_be_bytes());
    out.extend_from_slice(&timestamp.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn decode_packet(bytes: &[u8]) -> Result<RtpPacket, String> {
    if bytes.len() < 12 {
        return Err(format!("rtp packet too short: {} bytes", bytes.len()));
    }
    let cc = (bytes[0] & 0x0f) as usize;
    let extension = bytes[0] & 0x10 != 0;
    let padding = bytes[0] & 0x20 != 0;
    let mut offset = 12 + cc * 4;
    if offset > bytes.len() {
        return Err("rtp CSRC list exceeds packet length".into());
    }
    let csrc = (0..cc)
        .map(|i| {
            let at = 12 + i * 4;
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        })
        .collect();
    if extension {
        if offset + 4 > bytes.len() {
            return Err("rtp extension header exceeds packet length".into());
        }
        // 扩展长度字段以 32-bit 字为单位（不含这 4 字节头本身）。
        let words = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        offset += 4 + words * 4;
        if offset > bytes.len() {
            return Err("rtp extension data exceeds packet length".into());
        }
    }
    let mut end = bytes.len();
    if padding {
        let pad = *bytes.last().unwrap() as usize;
        // padding 计数包含最后一个字节本身，且不能吃掉整个头。
        if pad == 0 || pad > end - offset {
            return Err("rtp padding is invalid".into());
        }
        end -= pad;
    }
    Ok(RtpPacket {
        version: bytes[0] >> 6,
        padding,
        extension,
        marker: bytes[1] & 0x80 != 0,
        payload_type: bytes[1] & 0x7f,
        sequence: u16::from_be_bytes([bytes[2], bytes[3]]),
        timestamp: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        ssrc: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        csrc,
        payload: bytes[offset..end].to_vec(),
    })
}

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = parse_host_port(&req.target, 5004)?;
    let addr = first_addr(cfg, &host, port).await?;

    let payload_type = opts_u64(req, "payload_type").unwrap_or(96);
    if payload_type > 127 {
        return Err("payload_type must be 0-127".into());
    }
    let sequence = opts_u64(req, "sequence").unwrap_or(0) as u16;
    let timestamp = opts_u64(req, "timestamp").unwrap_or(0) as u32;
    let ssrc = opts_u64(req, "ssrc").unwrap_or(0) as u32;
    let payload = req
        .body
        .as_ref()
        .map(Payload::decode)
        .transpose()?
        .unwrap_or_default();
    if payload.len() + 12 > MAX_DATAGRAM_BYTES {
        return Err(format!("rtp payload exceeds {MAX_DATAGRAM_BYTES} bytes"));
    }

    let outcome = udp_exchange(cfg, req, addr, |i| {
        encode_packet(
            payload_type as u8,
            sequence.wrapping_add(i as u16),
            timestamp.wrapping_add(i as u32 * 160),
            ssrc,
            &payload,
        )
    })
    .await?;

    let packets: Vec<serde_json::Value> = outcome
        .packets
        .iter()
        .filter_map(|b| decode_packet(b).ok())
        .map(|p| {
            json!({
                "version": p.version,
                "padding": p.padding,
                "extension": p.extension,
                "marker": p.marker,
                "payload_type": p.payload_type,
                "sequence": p.sequence,
                "timestamp": p.timestamp,
                "ssrc": p.ssrc,
                "csrc": p.csrc,
                "payload": Payload::from_bytes(&p.payload),
            })
        })
        .collect();
    Ok(json_response(
        started,
        outcome.sent,
        outcome.packets.len(),
        json!({ "packets": packets }),
    ))
}

pub(crate) struct UdpOutcome {
    pub sent: usize,
    pub packets: Vec<Vec<u8>>,
}

/// 发送 `count` 个由 `build` 生成的包；`opts.recv=true` 时再在窗口内接收。
pub(crate) async fn udp_exchange(
    cfg: &DebugConfig,
    req: &DebugRequest,
    addr: SocketAddr,
    mut build: impl FnMut(usize) -> Vec<u8>,
) -> Result<UdpOutcome, String> {
    let count = opts_u64(req, "count")
        .unwrap_or(1)
        .clamp(1, MAX_DATAGRAMS as u64) as usize;
    let bind = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|e| format!("udp bind failed: {e}"))?;
    socket
        .connect(addr)
        .await
        .map_err(|e| format!("udp connect failed: {e}"))?;
    let mut sent_bytes = 0usize;
    for i in 0..count {
        let packet = build(i);
        if packet.len() > MAX_DATAGRAM_BYTES {
            return Err(format!("udp packet exceeds {MAX_DATAGRAM_BYTES} bytes"));
        }
        sent_bytes += packet.len();
        if sent_bytes > cfg.max_bytes {
            return Err(format!("udp send exceeds max_bytes ({})", cfg.max_bytes));
        }
        socket
            .send(&packet)
            .await
            .map_err(|e| format!("udp send failed: {e}"))?;
    }

    let mut packets = Vec::new();
    if req.opts.get("recv").is_some_and(|v| v == "true") {
        let window = opts_u64(req, "duration_ms")
            .unwrap_or(500)
            .min(timeout_ms(cfg, req));
        let deadline = Instant::now() + Duration::from_millis(window.max(1));
        let mut total = 0usize;
        for _ in 0..count {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let mut buf = vec![0u8; MAX_DATAGRAM_BYTES];
            match tokio::time::timeout(remaining, socket.recv(&mut buf)).await {
                Ok(Ok(n)) => {
                    total += n;
                    if total > cfg.max_bytes {
                        return Err(format!("response exceeds max_bytes ({})", cfg.max_bytes));
                    }
                    packets.push(buf[..n].to_vec());
                }
                Ok(Err(e)) => return Err(format!("udp recv failed: {e}")),
                Err(_) => break,
            }
        }
    }
    Ok(UdpOutcome {
        sent: count,
        packets,
    })
}

pub(crate) async fn first_addr(
    cfg: &DebugConfig,
    host: &str,
    port: u16,
) -> Result<SocketAddr, String> {
    guard::check_host(cfg, host, port)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| format!("target host `{host}` resolved to no addresses"))
}

/// `rtp://host:port`、`host:port` 或带路径的 RTSP/RTMP 目标都归一到 (host, port)。
pub(crate) fn parse_host_port(target: &str, default_port: u16) -> Result<(String, u16), String> {
    let rest = target
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(target);
    let url = reqwest::Url::parse(&format!("udp://{rest}"))
        .map_err(|e| format!("invalid target `{target}`: {e}"))?;
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    Ok((host, url.port().unwrap_or(default_port)))
}

pub(crate) fn timeout_ms(cfg: &DebugConfig, req: &DebugRequest) -> u64 {
    req.timeout_ms
        .unwrap_or(cfg.timeout_ms)
        .min(cfg.timeout_ms)
        .max(1)
}

pub(crate) fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

pub(crate) fn json_response(
    started: Instant,
    sent: usize,
    received: usize,
    body: serde_json::Value,
) -> DebugResponse {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("x-sent".into(), sent.to_string());
    headers.insert("x-received".into(), received.to_string());
    DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload::from_bytes(body.to_string().as_bytes()),
        frames: Vec::new(),
    }
}
