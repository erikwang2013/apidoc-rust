//! LwM2M 1.1 客户端最小集：Register / Update / Deregister + 单资源 Read / Write。
//!
//! 复用 `coap::roundtrip`；TLV 子集只覆盖单资源（整数 0/1/2/4/8 字节、布尔、
//! 字符串），不做对象实例、多资源和 Observe。Bootstrap 与固件升级不在范围内。

use super::coap;
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use coap_lite::Packet;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;

const OP_REGISTER: &str = "register";
const OP_UPDATE: &str = "update";
const OP_DEREGISTER: &str = "deregister";
const OP_READ: &str = "read";
const OP_WRITE: &str = "write";

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = coap::parse_target(&req.target)?;
    let addr = *super::guard::check_host(cfg, &host, port)
        .await?
        .first()
        .ok_or("target resolved to no addresses")?;
    let duration = coap::duration_ms(cfg, req);
    let operation = req.opts.get("operation").map(String::as_str).unwrap_or(OP_REGISTER);
    let token = vec![0xA5, 0x5A];

    let (request, expect) = match operation {
        OP_REGISTER => {
            let endpoint = required(req, "endpoint")?;
            let lifetime = req.opts.get("lifetime").map(String::as_str).unwrap_or("300");
            let lifetime = lifetime
                .parse::<u32>()
                .map_err(|_| format!("invalid lifetime `{lifetime}`"))?
                .to_string();
            let payload = req.body.as_ref().map(Payload::decode).transpose()?.unwrap_or_default();
            if payload.len() > cfg.max_bytes {
                return Err(format!("register payload exceeds max_bytes ({})", cfg.max_bytes));
            }
            let request = coap::build_request(
                "POST",
                "/rd",
                &[
                    ("ep", endpoint),
                    ("lt", lifetime.as_str()),
                    ("lwm2m", "1.1"),
                    ("b", "U"),
                ],
                &payload,
                coap::CONTENT_FORMAT_TLV,
                false,
                1,
                &token,
            )?;
            (request, "2.01")
        }
        OP_UPDATE => {
            let location = required(req, "location")?;
            let payload = req.body.as_ref().map(Payload::decode).transpose()?.unwrap_or_default();
            (
                coap::build_request("POST", location, &[], &payload, coap::CONTENT_FORMAT_TLV, false, 2, &token)?,
                "2.04",
            )
        }
        OP_DEREGISTER => {
            let location = required(req, "location")?;
            (coap::build_request("DELETE", location, &[], &[], 0, false, 3, &token)?, "2.02")
        }
        OP_READ => {
            let path = resource_path(req)?;
            (
                coap::build_request("GET", &path, &[], &[], 0, false, 4, &token)?,
                "2.05",
            )
        }
        OP_WRITE => {
            let path = resource_path(req)?;
            let value = required(req, "value")?;
            let encoded = tlv_encode_resource(resource_id(req)?, &TlvValue::parse(value));
            (
                coap::build_request(
                    "PUT",
                    &path,
                    &[],
                    &encoded,
                    coap::CONTENT_FORMAT_TLV,
                    false,
                    5,
                    &token,
                )?,
                "2.04",
            )
        }
        other => {
            return Err(format!(
                "unsupported lwm2m operation `{other}`; use register, update, deregister, read or write"
            ))
        }
    };

    let responses = coap::roundtrip(cfg, addr, &request, duration, 1).await?;
    let response = &responses[0];
    let code = coap::code_string(response);
    if code != expect {
        return Err(format!(
            "lwm2m {operation} failed: expected {expect}, got {code}{}",
            payload_suffix(response)
        ));
    }

    let body = match operation {
        OP_REGISTER => {
            let location = coap::option_strings(response, 8)
                .iter()
                .map(|s| format!("/{s}"))
                .collect::<String>();
            json!({ "code": code, "location": location })
        }
        OP_READ => {
            let cf = coap::option_uint(response, 12).unwrap_or(0) as u32;
            json!({ "code": code, "value": decode_value(response, cf) })
        }
        _ => json!({ "code": code }),
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

fn required<'a>(req: &'a DebugRequest, key: &str) -> Result<&'a str, String> {
    req.opts
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("lwm2m {key} is required"))
}

fn resource_id(req: &DebugRequest) -> Result<u16, String> {
    req.opts
        .get("resource_id")
        .ok_or("lwm2m resource_id is required")?
        .parse()
        .map_err(|_| "invalid resource_id".to_string())
}

fn resource_path(req: &DebugRequest) -> Result<String, String> {
    let id = |key: &str| -> Result<&str, String> {
        req.opts
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("lwm2m {key} is required"))
    };
    Ok(format!("/{}/{}/{}", id("object_id")?, id("instance_id")?, id("resource_id")?))
}

fn payload_suffix(packet: &Packet) -> String {
    if packet.payload.is_empty() {
        String::new()
    } else {
        format!(": {}", String::from_utf8_lossy(&packet.payload))
    }
}

/// 单资源值。TLV 里没有类型信息：按长度猜整数（0/1/2/4/8 字节），否则 UTF-8，
/// 再否则 base64。shortcut: 无法区分 1 字节 bool 与 0/1 整数，调用方按数值处理。
fn decode_value(packet: &Packet, content_format: u32) -> Value {
    if content_format != coap::CONTENT_FORMAT_TLV {
        return match std::str::from_utf8(&packet.payload) {
            Ok(s) => json!(s),
            Err(_) => json!({ "encoding": "base64", "data": base64_encode(&packet.payload) }),
        };
    }
    match tlv_decode_first(&packet.payload) {
        Ok(Some((_, value))) => match value.len() {
            0 => json!(0),
            1 | 2 | 4 | 8 => json!(signed_be(&value)),
            _ => match std::str::from_utf8(&value) {
                Ok(s) => json!(s),
                Err(_) => json!({ "encoding": "base64", "data": base64_encode(&value) }),
            },
        },
        Ok(None) => json!(null),
        Err(e) => json!({ "error": e, "encoding": "base64", "data": base64_encode(&packet.payload) }),
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn signed_be(bytes: &[u8]) -> i64 {
    if bytes.is_empty() {
        return 0;
    }
    if bytes.len() >= 8 {
        return i64::from_be_bytes(bytes[..8].try_into().expect("8-byte slice"));
    }
    let mut buf = [0u8; 8];
    let start = 8 - bytes.len();
    buf[start..].copy_from_slice(bytes);
    let unsigned = u64::from_be_bytes(buf);
    // 符号扩展到 64 位；len 1..=7 时 shift 为 8..=56，不会溢出。
    let shift = 64 - 8 * bytes.len() as u32;
    ((unsigned << shift) as i64) >> shift
}

#[derive(Debug, PartialEq)]
enum TlvValue {
    Int(i64),
    Bool(bool),
    Str(String),
}

impl TlvValue {
    fn parse(value: &str) -> TlvValue {
        if let Ok(n) = value.parse::<i64>() {
            TlvValue::Int(n)
        } else if value.eq_ignore_ascii_case("true") {
            TlvValue::Bool(true)
        } else if value.eq_ignore_ascii_case("false") {
            TlvValue::Bool(false)
        } else {
            TlvValue::Str(value.to_string())
        }
    }

    fn encode(&self) -> Vec<u8> {
        match self {
            TlvValue::Int(0) => Vec::new(),
            TlvValue::Int(n) if (-128..=127).contains(n) => vec![*n as u8],
            TlvValue::Int(n) if (-32_768..=32_767).contains(n) => (*n as i16).to_be_bytes().to_vec(),
            TlvValue::Int(n) if (i32::MIN as i64..=i32::MAX as i64).contains(n) => {
                (*n as i32).to_be_bytes().to_vec()
            }
            TlvValue::Int(n) => n.to_be_bytes().to_vec(),
            TlvValue::Bool(b) => vec![u8::from(*b)],
            TlvValue::Str(s) => s.as_bytes().to_vec(),
        }
    }
}

fn tlv_encode_resource(id: u16, value: &TlvValue) -> Vec<u8> {
    let bytes = value.encode();
    let id16 = id > 0xFF;
    let mut out = Vec::with_capacity(4 + bytes.len());
    let (length_type, inline_length) = match bytes.len() {
        0..=7 => (0u8, bytes.len() as u8),
        8..=0xFF => (1, 0),
        0x100..=0xFFFF => (2, 0),
        _ => (3, 0),
    };
    out.push(0xC0 | (u8::from(id16) << 5) | (length_type << 3) | inline_length);
    if id16 {
        out.extend_from_slice(&id.to_be_bytes());
    } else {
        out.push(id as u8);
    }
    match length_type {
        1 => out.push(bytes.len() as u8),
        2 => out.extend_from_slice(&(bytes.len() as u16).to_be_bytes()),
        3 => out.extend_from_slice(&(bytes.len() as u32).to_be_bytes()[1..]),
        _ => {}
    }
    out.extend_from_slice(&bytes);
    out
}

/// 返回 (resource id, raw value)；空 TLV 或无法识别时 None。
fn tlv_decode_first(data: &[u8]) -> Result<Option<(u16, Vec<u8>)>, String> {
    let Some(&header) = data.first() else { return Ok(None) };
    let kind = header >> 6;
    if kind != 0b11 {
        return Err(format!("unsupported tlv entry type {kind:#b}"));
    }
    let id16 = header & 0x20 != 0;
    let length_type = (header >> 3) & 0x03;
    let inline_length = (header & 0x07) as usize;
    let mut pos = 1;
    let id = if id16 {
        let bytes = data.get(pos..pos + 2).ok_or("truncated tlv id")?;
        pos += 2;
        u16::from_be_bytes([bytes[0], bytes[1]])
    } else {
        let byte = *data.get(pos).ok_or("truncated tlv id")?;
        pos += 1;
        u16::from(byte)
    };
    let length = match length_type {
        0 => inline_length,
        1 => {
            let byte = *data.get(pos).ok_or("truncated tlv length")?;
            pos += 1;
            byte as usize
        }
        2 => {
            let bytes = data.get(pos..pos + 2).ok_or("truncated tlv length")?;
            pos += 2;
            u16::from_be_bytes([bytes[0], bytes[1]]) as usize
        }
        _ => {
            let bytes = data.get(pos..pos + 3).ok_or("truncated tlv length")?;
            pos += 3;
            u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]) as usize
        }
    };
    let value = data.get(pos..pos + length).ok_or("truncated tlv value")?;
    Ok(Some((id, value.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_encodes_zero_one_and_two_bytes() {
        // value 0 -> 空值；5 -> 1 字节；300 -> 2 字节。
        assert_eq!(tlv_encode_resource(0, &TlvValue::Int(0)), vec![0xC0, 0x00]);
        assert_eq!(tlv_encode_resource(0, &TlvValue::Int(5)), vec![0xC1, 0x00, 0x05]);
        assert_eq!(
            tlv_encode_resource(0, &TlvValue::Int(300)),
            vec![0xC2, 0x00, 0x01, 0x2C]
        );
        assert_eq!(tlv_encode_resource(300, &TlvValue::Bool(true)), vec![0xE1, 0x01, 0x2C, 0x01]);
    }

    #[test]
    fn tlv_roundtrip_values() {
        for value in [TlvValue::Int(-70000), TlvValue::Bool(false), TlvValue::Str("hi".into())] {
            let encoded = tlv_encode_resource(7, &value);
            let (id, raw) = tlv_decode_first(&encoded).unwrap().unwrap();
            assert_eq!(id, 7);
            assert_eq!(raw, value.encode());
        }
        assert_eq!(tlv_decode_first(&[]).unwrap(), None);
        // inline length 3，但只剩 1 字节值。
        assert!(tlv_decode_first(&[0xC3, 0x00, 0x01]).is_err());
    }

    #[test]
    fn signed_decoding_sign_extends() {
        assert_eq!(signed_be(&[]), 0);
        assert_eq!(signed_be(&[0xFF]), -1);
        assert_eq!(signed_be(&[0x01, 0x2C]), 300);
        assert_eq!(signed_be(&[0xFF, 0xFF, 0xFF, 0xFF]), -1);
    }
}
