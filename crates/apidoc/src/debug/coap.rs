//! CoAP over UDP（RFC 7252）：单次请求/响应 + Observe 通知窗口。
//!
//! 只做客户端最小集：CON 请求、piggyback 响应、Observe 通知收集（N 条或超时）。
//! 不做块传输（Block1/2）、重传退避与 coaps（DTLS）；遇到需要它们的响应按原始
//! 选项暴露。所有等待都受 `duration_ms`（封顶 `DebugConfig.timeout_ms`）约束。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use base64::Engine;
use coap_lite::{CoapOption, MessageClass, MessageType, Packet, RequestType};
use serde_json::{json, Value};
use std::collections::{BTreeMap, LinkedList};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

pub(crate) const DEFAULT_PORT: u16 = 5683;
/// LwM2M 的 application/vnd.oma.lwm2m+tlv。
pub(crate) const CONTENT_FORMAT_TLV: u32 = 11_542;
/// 一次调试最多收这么多响应/通知，防止对端持续推送拖垮会话。
const MAX_MESSAGES: usize = 1000;

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = parse_target(&req.target)?;
    let addr = *guard::check_host(cfg, &host, port)
        .await?
        .first()
        .ok_or("target resolved to no addresses")?;

    let method = req
        .opts
        .get("method")
        .cloned()
        .or_else(|| req.method.clone())
        .unwrap_or_else(|| "GET".into());
    let path = req.opts.get("path").map(String::as_str).unwrap_or("/");
    let content_format = opts_u64(req, "content_format").unwrap_or(0) as u32;
    let observe = opts_bool(req, "observe");
    let duration = duration_ms(cfg, req);
    let messages = if observe {
        opts_usize(req, "messages").unwrap_or(1).clamp(1, MAX_MESSAGES)
    } else {
        1
    };
    let payload = req
        .body
        .as_ref()
        .map(Payload::decode)
        .transpose()?
        .unwrap_or_default();
    if payload.len() > cfg.max_bytes {
        return Err(format!("request payload exceeds max_bytes ({})", cfg.max_bytes));
    }

    let request = build_request(
        &method,
        path,
        &[],
        &payload,
        content_format,
        observe,
        message_id(),
        &token(),
    )?;
    let responses = roundtrip(cfg, addr, &request, duration, messages).await?;

    let items: Vec<Value> = responses.iter().map(format_packet).collect();
    let body = if observe && items.len() > 1 {
        json!({ "notifications": items })
    } else {
        items.into_iter().next().expect("roundtrip returned at least one packet")
    };
    let bytes = serde_json::to_vec(&body).map_err(|e| format!("encode response: {e}"))?;
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers: BTreeMap::from([("x-peer".into(), addr.to_string())]),
        body: Payload::from_bytes(&bytes),
        frames: Vec::new(),
    })
}

/// `coap://host:port`、`host:port`；`coaps` 明确拒绝（未实现 DTLS）。
pub(crate) fn parse_target(target: &str) -> Result<(String, u16), String> {
    if target.starts_with("coaps://") {
        return Err("coaps (CoAP over DTLS) is not supported in this build".into());
    }
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("coap://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid target `{target}`: {e}"))?;
    if url.scheme() != "coap" {
        return Err(format!("unsupported coap target scheme `{}`", url.scheme()));
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    let port = url.port().unwrap_or(DEFAULT_PORT);
    Ok((host, port))
}

/// 构造 CoAP 请求包；LwM2M 复用。
pub(crate) fn build_request(
    method: &str,
    path: &str,
    query: &[(&str, &str)],
    payload: &[u8],
    content_format: u32,
    observe: bool,
    message_id: u16,
    token: &[u8],
) -> Result<Packet, String> {
    let mut packet = Packet::new();
    packet.header.set_version(1);
    packet.header.set_type(MessageType::Confirmable);
    packet.header.message_id = message_id;
    packet.header.code = MessageClass::Request(match method.to_ascii_uppercase().as_str() {
        "GET" => RequestType::Get,
        "POST" => RequestType::Post,
        "PUT" => RequestType::Put,
        "DELETE" => RequestType::Delete,
        other => return Err(format!("unsupported coap method `{other}`")),
    });
    packet.set_token(token.to_vec());
    for segment in path.trim_start_matches('/').split('/').filter(|s| !s.is_empty()) {
        packet.add_option(CoapOption::UriPath, segment.as_bytes().to_vec());
    }
    for (key, value) in query {
        packet.add_option(
            CoapOption::UriQuery,
            format!("{key}={}", percent_encode(value)).into_bytes(),
        );
    }
    if observe {
        packet.set_observe_value(0);
    }
    packet.add_option(CoapOption::ContentFormat, encode_uint(content_format as u64));
    packet.payload = payload.to_vec();
    Ok(packet)
}

/// 发送请求，按 token 匹配响应，收集 `messages` 条或到 `duration_ms` 截止。
pub(crate) async fn roundtrip(
    cfg: &DebugConfig,
    addr: SocketAddr,
    request: &Packet,
    duration_ms: u64,
    messages: usize,
) -> Result<Vec<Packet>, String> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("coap bind failed: {e}"))?;
    socket
        .connect(addr)
        .await
        .map_err(|e| format!("coap connect failed: {e}"))?;
    let bytes = request.to_bytes().map_err(|e| format!("encode coap request: {e}"))?;
    if bytes.len() > cfg.max_bytes {
        return Err(format!("coap request exceeds max_bytes ({})", cfg.max_bytes));
    }
    socket
        .send(&bytes)
        .await
        .map_err(|e| format!("coap send failed: {e}"))?;

    let deadline = Instant::now() + Duration::from_millis(duration_ms.max(1));
    let mut out = Vec::new();
    let mut total = 0usize;
    while out.len() < messages {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut buf = vec![0u8; 65_535];
        let n = match tokio::time::timeout(remaining, socket.recv(&mut buf)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(format!("coap recv failed: {e}")),
            Err(_) => break,
        };
        total += n;
        if total > cfg.max_bytes {
            return Err(format!("coap response exceeds max_bytes ({})", cfg.max_bytes));
        }
        let packet = match Packet::from_bytes(&buf[..n]) {
            Ok(packet) => packet,
            Err(_) => continue, // 不是 CoAP 的数据报（UDP 端口混用），忽略。
        };
        if packet.get_token() != request.get_token() {
            continue;
        }
        if packet.header.get_type() == MessageType::Reset {
            return Err("coap peer rejected the request (RST)".into());
        }
        out.push(packet);
    }
    if out.is_empty() {
        return Err(format!("coap request timed out after {duration_ms}ms (no response)"));
    }
    if request.get_observe_value().is_some() {
        // 从同一源端口发 RST 取消 Observe；丢失由服务端超时兜底。
        let mut reset = Packet::new();
        reset.header.set_version(1);
        reset.header.set_type(MessageType::Reset);
        reset.header.message_id = out.last().map(|p| p.header.message_id).unwrap_or_default();
        reset.set_token(request.get_token().to_vec());
        if let Ok(bytes) = reset.to_bytes() {
            let _ = socket.send(&bytes).await;
        }
    }
    Ok(out)
}

/// 响应码显示为 `2.05`；非响应码退化为 `0.xx`。
pub(crate) fn code_string(packet: &Packet) -> String {
    let raw: u8 = packet.header.code.into();
    if matches!(packet.header.code, MessageClass::Response(_)) {
        format!("{}.{:02}", raw >> 5, raw & 0x1F)
    } else {
        format!("0.{raw:02}")
    }
}

/// uint 选项（Content-Format、Observe、Max-Age…）解码。
pub(crate) fn option_uint(packet: &Packet, number: u16) -> Option<u64> {
    let option = match number {
        6 => CoapOption::Observe,
        7 => CoapOption::UriPort,
        12 => CoapOption::ContentFormat,
        14 => CoapOption::MaxAge,
        17 => CoapOption::Accept,
        _ => CoapOption::Unknown(number),
    };
    packet.get_first_option(option).map(|v| decode_uint(v))
}

fn format_packet(packet: &Packet) -> Value {
    let code = code_string(packet);
    let payload = Payload::from_bytes(&packet.payload);
    let mut options = BTreeMap::new();
    for (number, values) in packet.options() {
        options.insert(
            option_name(*number).to_string(),
            values
                .iter()
                .map(|v| format_option(*number, v))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    json!({
        "code": code,
        "payload": { "encoding": payload.encoding, "data": payload.data },
        "options": options,
    })
}

fn option_name(number: u16) -> &'static str {
    match number {
        4 => "etag",
        6 => "observe",
        7 => "uri-port",
        8 => "location-path",
        12 => "content-format",
        14 => "max-age",
        15 => "uri-query",
        17 => "accept",
        20 => "location-query",
        _ => "option",
    }
}

fn format_option(number: u16, value: &[u8]) -> String {
    match number {
        4 => base64::engine::general_purpose::STANDARD.encode(value),
        6 | 7 | 12 | 14 | 17 => decode_uint(value).to_string(),
        _ => match std::str::from_utf8(value) {
            Ok(s) => s.to_string(),
            Err(_) => base64::engine::general_purpose::STANDARD.encode(value),
        },
    }
}

/// CoAP uint 选项：大端最短编码，0 为空。
fn encode_uint(value: u64) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|b| *b != 0).unwrap_or(7);
    bytes[first..].to_vec()
}

fn decode_uint(value: &[u8]) -> u64 {
    value.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn message_id() -> u16 {
    let mut bytes = [0u8; 2];
    let _ = getrandom::getrandom(&mut bytes);
    u16::from_be_bytes(bytes)
}

fn token() -> Vec<u8> {
    let mut bytes = [0u8; 8];
    let _ = getrandom::getrandom(&mut bytes);
    bytes.to_vec()
}

pub(crate) fn duration_ms(cfg: &DebugConfig, req: &DebugRequest) -> u64 {
    opts_u64(req, "duration_ms")
        .unwrap_or(3_000)
        .clamp(1, cfg.timeout_ms.max(1))
}

pub(crate) fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

pub(crate) fn opts_usize(req: &DebugRequest, key: &str) -> Option<usize> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

pub(crate) fn opts_bool(req: &DebugRequest, key: &str) -> bool {
    req.opts
        .get(key)
        .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
}

/// 供 LwM2M 把 Location-Path 等选项拼回路径/查询串。
pub(crate) fn option_strings(packet: &Packet, number: u16) -> Vec<String> {
    packet
        .get_option(CoapOption::Unknown(number))
        .or_else(|| match number {
            8 => packet.get_option(CoapOption::LocationPath),
            15 => packet.get_option(CoapOption::UriQuery),
            20 => packet.get_option(CoapOption::LocationQuery),
            _ => None,
        })
        .map(|values: &LinkedList<Vec<u8>>| {
            values
                .iter()
                .map(|v| String::from_utf8_lossy(v).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uint_options_roundtrip() {
        assert_eq!(encode_uint(0), Vec::<u8>::new());
        assert_eq!(encode_uint(11_542), vec![0x2D, 0x16]);
        assert_eq!(decode_uint(&[0x2D, 0x16]), 11_542);
        assert_eq!(decode_uint(&[]), 0);
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(percent_encode("urn:imei:1"), "urn%3Aimei%3A1");
        assert_eq!(percent_encode("a-b_c.d~"), "a-b_c.d~");
    }

    #[test]
    fn target_defaults_and_coaps_rejection() {
        assert_eq!(parse_target("coap://h").unwrap(), ("h".into(), 5683));
        assert_eq!(parse_target("h:9999").unwrap(), ("h".into(), 9999));
        assert!(parse_target("coaps://h").unwrap_err().contains("coaps"));
        assert!(parse_target("http://h").is_err());
    }

    #[test]
    fn request_encodes_path_query_and_content_format() {
        let packet = build_request(
            "POST",
            "/rd",
            &[("ep", "node-1")],
            b"x",
            CONTENT_FORMAT_TLV,
            true,
            7,
            &[1, 2],
        )
        .unwrap();
        let request = coap_lite::CoapRequest::<()>::from_packet(packet, ());
        assert_eq!(request.get_path(), "rd");
        assert!(request.message.get_option(CoapOption::UriQuery).is_some());
        assert_eq!(request.message.get_observe_value().unwrap().unwrap(), 0);
        assert_eq!(request.message.header.message_id, 7);
    }
}
