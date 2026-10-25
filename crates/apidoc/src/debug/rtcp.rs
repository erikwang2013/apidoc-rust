//! P5 RTCP 包级调试（RFC 3550 子集）。
//!
//! 只构造/解析 SR(200)、RR(201)、SDES(202)、BYE(203) 的头部与 sender/SSRC 字段；
//! 不参与会话带宽控制，报告块按不透明载荷处理。

use super::rtp::{first_addr, json_response, opts_u64, parse_host_port, udp_exchange};
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::json;
use std::time::Instant;

#[derive(Debug, PartialEq, Eq)]
pub struct RtcpPacket {
    pub version: u8,
    pub padding: bool,
    pub count: u8,
    pub packet_type: u8,
    /// 头部 length 字段 + 1 后的字节数。
    pub length: usize,
    pub ssrc: Option<u32>,
    /// SDES 的 CNAME 或 BYE 的 reason 文本（UTF-8 损失处理）。
    pub text: Option<String>,
}

/// 构造单个 RTCP 包。SDES/BYE 的 count 字段固定为 1（一个 SSRC）。
pub fn encode_packet(packet_type: u8, ssrc: u32, payload: &[u8]) -> Vec<u8> {
    let count = if packet_type == 202 || packet_type == 203 {
        1
    } else {
        0
    };
    let body: Vec<u8> = match packet_type {
        // SDES: 单个 CNAME item (type 1, len, text)；长度字段只有 8 bit。
        202 => {
            let text = &payload[..payload.len().min(255)];
            [vec![1, text.len() as u8], text.to_vec()].concat()
        }
        // BYE: reason 前有 8 bit 长度字节。
        203 if !payload.is_empty() => {
            let text = &payload[..payload.len().min(255)];
            [vec![text.len() as u8], text.to_vec()].concat()
        }
        // SR/RR 等按不透明载荷原样发送（发送方负责不超过 UDP 上限）。
        _ => payload.to_vec(),
    };
    let payload: &[u8] = &body;
    let padded = (payload.len() + 3) & !3;
    // header 的 length 是“整包 32bit 字数 - 1”，至少包含 SSRC 那一个字。
    let length = 1 + padded / 4;
    let mut out = Vec::with_capacity(8 + padded);
    out.push(0x80 | count); // V=2, P=0
    out.push(packet_type);
    out.extend_from_slice(&(length as u16).to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(payload);
    out.resize(8 + padded, 0);
    out
}

pub fn decode_packet(bytes: &[u8]) -> Result<RtcpPacket, String> {
    if bytes.len() < 4 {
        return Err(format!("rtcp packet too short: {} bytes", bytes.len()));
    }
    let length = (u16::from_be_bytes([bytes[2], bytes[3]]) as usize + 1) * 4;
    if length > bytes.len() {
        return Err(format!(
            "rtcp length field says {length} bytes but packet has {}",
            bytes.len()
        ));
    }
    let count = bytes[0] & 0x1f;
    let packet_type = bytes[1];
    let ssrc = (length >= 8).then(|| u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]));
    let text = match packet_type {
        // SDES：SSRC 后是 item(type,len,text)；取 CNAME(1) 或任意首项的文本。
        202 if length >= 10 => {
            let len = bytes[9] as usize;
            (10 + len <= length).then(|| String::from_utf8_lossy(&bytes[10..10 + len]).into())
        }
        // BYE：count 个 SSRC 之后是 reason（首字节长度）。
        203 if ssrc.is_some() => {
            let at = 4 + count as usize * 4;
            let len = bytes.get(at).copied().unwrap_or(0) as usize;
            (at + 1 + len <= length)
                .then(|| String::from_utf8_lossy(&bytes[at + 1..at + 1 + len]).into())
        }
        _ => None,
    };
    Ok(RtcpPacket {
        version: bytes[0] >> 6,
        padding: bytes[0] & 0x20 != 0,
        count,
        packet_type,
        length,
        ssrc,
        text,
    })
}

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = parse_host_port(&req.target, 5005)?;
    let addr = first_addr(cfg, &host, port).await?;

    let packet_type = match req.opts.get("packet_type").map(String::as_str) {
        None | Some("rr") => 201,
        Some("sr") => 200,
        Some("sdes") => 202,
        Some("bye") => 203,
        Some(other) => other
            .parse::<u8>()
            .map_err(|_| format!("unknown rtcp packet_type `{other}`; use sr/rr/sdes/bye"))?,
    };
    let ssrc = opts_u64(req, "ssrc").unwrap_or(0) as u32;
    let payload = req
        .body
        .as_ref()
        .map(Payload::decode)
        .transpose()?
        .unwrap_or_default();

    let outcome = udp_exchange(cfg, req, addr, |_| {
        encode_packet(packet_type, ssrc, &payload)
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
                "count": p.count,
                "packet_type": p.packet_type,
                "length": p.length,
                "ssrc": p.ssrc,
                "text": p.text,
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
