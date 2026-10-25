//! BACnet/IP 最小集：Who-Is / I-Am 与 ReadProperty。
//!
//! 手写 BVLC + NPDU + APDU（BACnet 无纯 Rust 常用客户端 crate）。
//! 只解析直连 BACnet/IP 报文（NPDU 无路由信息）；常见应用标签 unsigned/signed/
//! real/double/boolean/character-string/enumerated/octet-string。
//! 广播规则：`broadcast=true` 仅用于 who_is，必须 `allow_private_networks=true`，
//! 且永久拒绝 255.255.255.255 与组播目标。

use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const DEFAULT_PORT: u16 = 47_808;
const MAX_FRAMES: usize = 1000;
const PRESENT_VALUE: u8 = 85;

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = parse_target(&req.target)?;
    let addrs = super::guard::check_host(cfg, &host, port).await?;
    let broadcast = opts_bool(req, "broadcast");
    validate_destination(cfg, broadcast, &addrs)?;
    let addr = addrs[0];
    let operation = req.opts.get("operation").map(String::as_str).unwrap_or("who_is");
    let duration = opts_u64(req, "duration_ms")
        .unwrap_or(3_000)
        .clamp(1, cfg.timeout_ms.max(1));

    let apdu: Vec<u8> = match (operation, broadcast) {
        ("who_is", _) => vec![0x10, 0x08],
        ("read_property", false) => {
            let object_type = opts_u16(req, "object_type").unwrap_or(8); // 8 = device
            let object_instance = required_u32(req, "object_instance")?;
            let property = opts_u16(req, "property").unwrap_or(u16::from(PRESENT_VALUE));
            if object_instance > 0x3F_FFFF {
                return Err("bacnet object_instance must fit in 22 bits".into());
            }
            if object_type > 0x3FF || property > 0xFF {
                return Err("bacnet object_type <= 1023 and property <= 255 are required".into());
            }
            let object_id = (u32::from(object_type) << 22) | object_instance;
            let mut apdu = vec![0x00, 0x00, 0x0C]; // Confirmed-Request, invoke 0, ReadProperty
            apdu.push(0xC4);
            apdu.extend_from_slice(&object_id.to_be_bytes());
            apdu.extend_from_slice(&[0x19, property as u8]);
            apdu
        }
        ("read_property", true) => {
            return Err("bacnet broadcast is only valid for who_is".into())
        }
        (other, _) => {
            return Err(format!(
                "unsupported bacnet operation `{other}`; use who_is or read_property"
            ))
        }
    };
    let packet = encode_bvlc(broadcast, &apdu)?;

    let socket = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("bacnet bind failed: {e}"))?;
    if broadcast {
        socket
            .set_broadcast(true)
            .map_err(|e| format!("bacnet broadcast socket failed: {e}"))?;
    }
    // 不 connect：广播 Who-Is 的 I-Am 来自各设备单播地址。
    socket
        .send_to(&packet, addr)
        .await
        .map_err(|e| format!("bacnet send failed: {e}"))?;

    let deadline = Instant::now() + Duration::from_millis(duration);
    let mut total = 0usize;
    let mut frames = 0usize;
    let mut devices = Vec::new();
    let mut property_value = None;
    while frames < MAX_FRAMES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut buf = vec![0u8; 1500];
        let (n, peer) = match tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await {
            Ok(Ok(received)) => received,
            Ok(Err(e)) => return Err(format!("bacnet recv failed: {e}")),
            Err(_) => break,
        };
        total += n;
        if total > cfg.max_bytes {
            return Err(format!("bacnet response exceeds max_bytes ({})", cfg.max_bytes));
        }
        frames += 1;
        let peer = peer.to_string();
        let Some(apdu) = decode_bvlc(&buf[..n])? else { continue };
        match operation {
            "who_is" => {
                if let Some(instance) = parse_i_am(apdu) {
                    devices.push(json!({ "instance": instance, "address": peer }));
                }
            }
            _ => {
                // 忽略非响应 PDU（例如同端口上的其他广播流量）。
                if matches!(apdu.first(), Some(0x30 | 0x50 | 0x60 | 0x70)) {
                    property_value = Some(parse_read_property_ack(apdu)?);
                }
            }
        }
        if property_value.is_some() {
            break;
        }
    }

    let body = if operation == "who_is" {
        if devices.is_empty() {
            return Err(format!("bacnet who_is timed out after {duration}ms without I-Am"));
        }
        json!({ "devices": devices })
    } else {
        json!({ "value": property_value.ok_or_else(|| format!(
            "bacnet read_property timed out after {duration}ms"
        ))? })
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

fn validate_destination(
    cfg: &DebugConfig,
    broadcast: bool,
    addrs: &[SocketAddr],
) -> Result<(), String> {
    if broadcast && !cfg.allow_private_networks {
        return Err("bacnet broadcast requires DebugConfig.allow_private_networks = true".into());
    }
    for addr in addrs {
        match addr.ip() {
            IpAddr::V4(ip) if ip == Ipv4Addr::BROADCAST => {
                return Err("255.255.255.255 is never a valid BACnet target".into())
            }
            ip if ip.is_multicast() => {
                return Err("BACnet/IP multicast targets are not supported".into())
            }
            _ => {}
        }
    }
    Ok(())
}

/// BVLC：`81 0A/0B len(BE) NPDU APDU`；length 含 4 字节 BVLC 头。
fn encode_bvlc(broadcast: bool, apdu: &[u8]) -> Result<Vec<u8>, String> {
    let mut npdu = vec![0x01, 0x00]; // version 1, control 0（直连，无路由信息）
    npdu.extend_from_slice(apdu);
    let length = 4 + npdu.len();
    if length > u16::MAX as usize {
        return Err("bacnet packet too large".into());
    }
    let mut out = vec![0x81, if broadcast { 0x0B } else { 0x0A }];
    out.extend_from_slice(&(length as u16).to_be_bytes());
    out.extend_from_slice(&npdu);
    Ok(out)
}

/// 校验 BVLC 并返回 APDU；非数据链路帧或带路由信息的 NPDU 返回 None。
fn decode_bvlc(data: &[u8]) -> Result<Option<&[u8]>, String> {
    if data.len() < 6 {
        return Ok(None);
    }
    if data[0] != 0x81 {
        return Ok(None);
    }
    match data[1] {
        0x04 | 0x0A | 0x0B => {}
        _ => return Ok(None),
    }
    let length = u16::from_be_bytes([data[2], data[3]]) as usize;
    if length < 6 || length > data.len() {
        return Err("invalid bacnet BVLC length".into());
    }
    if data[4] != 0x01 {
        return Err("unsupported bacnet NPDU version".into());
    }
    let control = data[5];
    if control & 0x30 != 0 {
        // DNET/SNET 路由信息：本执行器只处理直连 BACnet/IP。
        return Ok(None);
    }
    Ok(Some(&data[6..length]))
}

fn parse_i_am(apdu: &[u8]) -> Option<u32> {
    if apdu.len() < 7 || apdu[0] != 0x10 || apdu[1] != 0x00 {
        return None;
    }
    // 应用标签 context 0，长度 4，后面 4 字节 object identifier。
    if apdu[2] != 0xC4 {
        return None;
    }
    let object_id = u32::from_be_bytes([apdu[3], apdu[4], apdu[5], apdu[6]]);
    let family = object_id >> 22;
    (family == 8).then_some(object_id & 0x3F_FFFF) // 8 = device
}

fn parse_read_property_ack(apdu: &[u8]) -> Result<Value, String> {
    match apdu.first() {
        Some(0x30) => {}
        Some(0x50) => return Err(format!("bacnet error PDU: {}", hex(apdu))),
        Some(0x60) => return Err(format!("bacnet reject PDU: {}", hex(apdu))),
        Some(0x70) => return Err(format!("bacnet abort PDU: {}", hex(apdu))),
        _ => return Err("not a bacnet ComplexACK".into()),
    }
    // ComplexACK: 30 invoke 0C | C4 + object id | 19 property [| 29 array index] | 3E value 3F
    let mut pos = 3;
    if apdu.get(pos) != Some(&0xC4) {
        return Err("unsupported bacnet ComplexACK shape".into());
    }
    pos += 5;
    if apdu.get(pos) != Some(&0x19) {
        return Err("bacnet ComplexACK is missing property id".into());
    }
    pos += 2;
    if apdu.get(pos) == Some(&0x29) {
        pos += 2; // array index
    }
    if apdu.get(pos) == Some(&0x3E) {
        pos += 1; // opening tag
    }
    let (value, _) = parse_app_value(&apdu[pos..])?;
    Ok(value)
}

/// 解析一个应用标签值，返回 (JSON, 消耗字节数)。
fn parse_app_value(data: &[u8]) -> Result<(Value, usize), String> {
    let Some(&tag) = data.first() else { return Err("empty bacnet value".into()) };
    let tag_number = tag >> 4;
    if tag_number == 0x0F {
        return Err("bacnet extended tag numbers are not supported".into());
    }
    let mut header = 1;
    let mut length = usize::from(tag & 0x0F);
    if length == 5 {
        length = usize::from(*data.get(1).ok_or("truncated bacnet extended length")?);
        header = 2;
    } else if length > 5 {
        return Err("invalid bacnet tag length code".into());
    }
    // Boolean 的值在长度位里，没有数据字节。
    if tag_number == 1 {
        return Ok((json!(length == 1), header));
    }
    let bytes = data
        .get(header..header + length)
        .ok_or("truncated bacnet value payload")?;
    let consumed = header + length;
    let unsigned = |bytes: &[u8]| -> Result<u32, String> {
        if bytes.len() > 4 {
            return Err("bacnet unsigned value longer than 4 bytes".into());
        }
        Ok(bytes.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)))
    };
    let value = match tag_number {
        0 => Value::Null,
        2 | 9 => json!(unsigned(bytes)?),
        3 => {
            if bytes.is_empty() || bytes.len() > 4 {
                return Err("bacnet signed value must be 1..=4 bytes".into());
            }
            let raw = unsigned(bytes)? as i64;
            let shift = 64 - 8 * bytes.len() as u32;
            json!(((raw << shift) as i64) >> shift)
        }
        4 => {
            if bytes.len() != 4 {
                return Err("bacnet REAL must be 4 bytes".into());
            }
            let value = f32::from_bits(unsigned(bytes)?);
            if value.is_finite() { json!(value) } else { json!(value.to_string()) }
        }
        5 => {
            if bytes.len() != 8 {
                return Err("bacnet DOUBLE must be 8 bytes".into());
            }
            let mut raw = [0u8; 8];
            raw.copy_from_slice(bytes);
            let value = f64::from_bits(u64::from_be_bytes(raw));
            if value.is_finite() { json!(value) } else { json!(value.to_string()) }
        }
        6 | 8 => json!({ "encoding": "hex", "data": hex(bytes) }),
        7 => {
            let charset = bytes.first().copied().unwrap_or(0);
            if charset != 0 {
                return Err(format!("unsupported bacnet string charset {charset}"));
            }
            json!(String::from_utf8_lossy(bytes.get(1..).unwrap_or_default()))
        }
        _ => json!({ "encoding": "hex", "data": hex(bytes) }),
    };
    Ok((value, consumed))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_target(target: &str) -> Result<(String, u16), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("bacnet://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid target `{target}`: {e}"))?;
    match url.scheme() {
        "bacnet" | "bacnet-ip" | "bacnet/ip" | "udp" => {}
        other => return Err(format!("unsupported bacnet target scheme `{other}`")),
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    Ok((host, url.port().unwrap_or(DEFAULT_PORT)))
}

fn required_u32(req: &DebugRequest, key: &str) -> Result<u32, String> {
    req.opts
        .get(key)
        .ok_or_else(|| format!("bacnet {key} is required"))?
        .parse()
        .map_err(|_| format!("invalid bacnet {key}"))
}

fn opts_bool(req: &DebugRequest, key: &str) -> bool {
    req.opts
        .get(key)
        .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
}

fn opts_u16(req: &DebugRequest, key: &str) -> Option<u16> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_is_bvlc_is_well_formed() {
        let packet = encode_bvlc(false, &[0x10, 0x08]).unwrap();
        assert_eq!(packet, vec![0x81, 0x0A, 0x00, 0x08, 0x01, 0x00, 0x10, 0x08]);
    }

    #[test]
    fn parses_i_am_device_instance() {
        let object_id: u32 = (8 << 22) | 26_000;
        let mut apdu = vec![0x10, 0x00, 0xC4];
        apdu.extend_from_slice(&object_id.to_be_bytes());
        apdu.extend_from_slice(&[0x22, 0x01, 0x40, 0x91, 0x03, 0x21, 0x00]);
        assert_eq!(parse_i_am(&apdu), Some(26_000));
    }

    #[test]
    fn real_and_boolean_values() {
        let mut real = vec![0x44];
        real.extend_from_slice(&21.5f32.to_be_bytes());
        assert_eq!(parse_app_value(&real).unwrap().0, 21.5);
        assert_eq!(parse_app_value(&[0x11]).unwrap().0, true);
        assert_eq!(parse_app_value(&[0x10]).unwrap().0, false);
    }

    #[test]
    fn complex_ack_yields_present_value() {
        let object_id: u32 = (8 << 22) | 1;
        let mut apdu = vec![0x30, 0x00, 0x0C, 0xC4];
        apdu.extend_from_slice(&object_id.to_be_bytes());
        apdu.extend_from_slice(&[0x19, 0x55, 0x3E, 0x44]);
        apdu.extend_from_slice(&7.25f32.to_be_bytes());
        apdu.push(0x3F);
        assert_eq!(parse_read_property_ack(&apdu).unwrap(), 7.25);
    }

    #[test]
    fn broadcast_rules_fail_closed() {
        let cfg = DebugConfig::default();
        let global = [SocketAddr::from(([255, 255, 255, 255], 47808))];
        assert!(validate_destination(&cfg, true, &global).is_err());
        let directed = [SocketAddr::from(([192, 168, 1, 255], 47808))];
        assert!(validate_destination(&cfg, true, &directed).is_err()); // 需 allow_private
        let private_cfg = DebugConfig { allow_private_networks: true, ..DebugConfig::default() };
        assert!(validate_destination(&private_cfg, true, &directed).is_ok());
        let multicast = [SocketAddr::from(([239, 1, 1, 1], 47808))];
        assert!(validate_destination(&private_cfg, true, &multicast).is_err());
    }
}
