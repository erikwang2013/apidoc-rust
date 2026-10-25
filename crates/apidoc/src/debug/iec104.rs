//! IEC 60870-5-104 只读最小集：STARTDT + 总召唤 / 单点读。
//!
//! Spike 结论：`iec104` 0.6.0 使用 edition 2024（Rust ≥ 1.85），与项目 MSRV 1.82
//! 不兼容；因此手写 APDU（I/S/U 帧）与常见监视 ASDU 解析。
//! 不做：遥控/设定值下发、时钟同步、文件传输、冗余连接、TLS。
//! 所有等待受 `duration_ms`（封顶 `DebugConfig.timeout_ms`）与帧/字节上限约束。

use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const DEFAULT_PORT: u16 = 2404;
const MAX_FRAMES: usize = 1000;
const MAX_APDU: usize = 253;

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = parse_target(&req.target)?;
    let addr = *super::guard::check_host(cfg, &host, port)
        .await?
        .first()
        .ok_or("target resolved to no addresses")?;
    let common_address = opts_u16(req, "common_address").unwrap_or(1);
    let duration = opts_u64(req, "duration_ms")
        .unwrap_or(3_000)
        .clamp(1, cfg.timeout_ms.max(1));
    let operation = req.opts.get("operation").map(String::as_str).unwrap_or("interrogation");

    // 先构造/校验 ASDU，非法操作不触达设备。
    let asdu = match operation {
        "interrogation" => {
            let mut asdu = vec![100, 0x01, 0x06, 0x00];
            asdu.extend_from_slice(&common_address.to_le_bytes());
            asdu.extend_from_slice(&[0x00, 0x00, 0x00, 20]); // IOA 0, QOI 20 = station
            asdu
        }
        "read" => {
            let ioa = opts_u32(req, "ioa").ok_or("iec104 `ioa` is required for read")?;
            if ioa > 0xFF_FFFF {
                return Err("iec104 ioa must fit in 24 bits".into());
            }
            let mut asdu = vec![102, 0x01, 0x05, 0x00];
            asdu.extend_from_slice(&common_address.to_le_bytes());
            asdu.extend_from_slice(&ioa.to_le_bytes()[..3]);
            asdu
        }
        other => {
            return Err(format!(
                "unsupported iec104 operation `{other}`; use interrogation or read"
            ))
        }
    };

    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let mut stream = tokio::time::timeout(connect_timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| "iec104 connect timed out".to_string())?
        .map_err(|e| format!("iec104 connect failed: {e}"))?;
    let deadline = Instant::now() + Duration::from_millis(cfg.timeout_ms.max(1));

    // STARTDT act -> con。con 之前不接受数据帧。
    write_apdu(&mut stream, &[0x07, 0x00, 0x00, 0x00], &[]).await?;
    let mut started_dt = false;
    while !started_dt {
        let Some(body) = read_apdu(&mut stream, deadline).await? else {
            return Err("iec104 STARTDT was not confirmed".into());
        };
        if body.len() == 4 && body[0] == 0x0B {
            started_dt = true;
        }
    }

    write_apdu(&mut stream, &[0x00, 0x00, 0x00, 0x00], &asdu).await?;

    let collect_deadline = Instant::now() + Duration::from_millis(duration);
    let (rows, received_seq) = collect(cfg, &mut stream, collect_deadline).await?;
    // 只读会话收尾：S 帧确认已收到的 I 帧。
    let s_frame = [
        0x01,
        0x00,
        ((received_seq << 1) & 0xFF) as u8,
        ((received_seq >> 7) & 0xFF) as u8,
    ];
    let _ = write_apdu(&mut stream, &s_frame, &[]).await;

    if rows.is_empty() {
        return Err(format!("iec104 timed out after {duration}ms without an ASDU"));
    }
    let bytes = serde_json::to_vec(&json!({ "asdu": rows }))
        .map_err(|e| format!("encode response: {e}"))?;
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers: BTreeMap::from([("x-peer".into(), addr.to_string())]),
        body: Payload::from_bytes(&bytes),
        frames: Vec::new(),
    })
}

/// 发送一个 APDU：`68 len ctrl(4) asdu…`。U/S 帧 asdu 为空。
async fn write_apdu(stream: &mut TcpStream, ctrl: &[u8; 4], asdu: &[u8]) -> Result<(), String> {
    let length = 4 + asdu.len();
    if length > MAX_APDU {
        return Err(format!("iec104 ASDU exceeds {MAX_APDU} bytes"));
    }
    let mut apdu = Vec::with_capacity(2 + length);
    apdu.push(0x68);
    apdu.push(length as u8);
    apdu.extend_from_slice(ctrl);
    apdu.extend_from_slice(asdu);
    stream
        .write_all(&apdu)
        .await
        .map_err(|e| format!("iec104 write failed: {e}"))
}

/// 读取一个 APDU，返回控制域(4)+ASDU；超时/EOF 返回 None。
async fn read_apdu(stream: &mut TcpStream, deadline: Instant) -> Result<Option<Vec<u8>>, String> {
    let mut header = [0u8; 2];
    let Some(()) = read_exact_deadline(stream, &mut header, deadline).await? else { return Ok(None) };
    if header[0] != 0x68 {
        return Err("invalid iec104 start byte".into());
    }
    let length = header[1] as usize;
    if !(4..=MAX_APDU).contains(&length) {
        return Err(format!("invalid iec104 APDU length {length}"));
    }
    let mut body = vec![0u8; length];
    let Some(()) = read_exact_deadline(stream, &mut body, deadline).await? else { return Ok(None) };
    Ok(Some(body))
}

async fn read_exact_deadline(
    stream: &mut TcpStream,
    buf: &mut [u8],
    deadline: Instant,
) -> Result<Option<()>, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(None);
    }
    match tokio::time::timeout(remaining, stream.read_exact(buf)).await {
        Ok(Ok(_)) => Ok(Some(())),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Ok(Err(e)) => Err(format!("iec104 read failed: {e}")),
        Err(_) => Ok(None),
    }
}

/// 收集 I 帧并解析 ASDU；返回 (行, 已收到的发送序号) 用于 S 帧确认。
async fn collect(
    cfg: &DebugConfig,
    stream: &mut TcpStream,
    deadline: Instant,
) -> Result<(Vec<Value>, u16), String> {
    let mut rows = Vec::new();
    let mut total = 0usize;
    let mut received_seq = 0u16;
    let mut frames = 0usize;
    while frames < MAX_FRAMES {
        let Some(body) = read_apdu(stream, deadline).await? else { break };
        total += body.len() + 2;
        if total > cfg.max_bytes {
            return Err(format!("iec104 response exceeds max_bytes ({})", cfg.max_bytes));
        }
        frames += 1;
        if body[0] & 0x01 != 0 {
            continue; // S 帧或 U 帧
        }
        received_seq = u16::from_le_bytes([body[2], body[3]]) >> 1;
        rows.extend(parse_asdus(&body[4..])?);
    }
    Ok((rows, received_seq))
}

fn parse_asdus(mut data: &[u8]) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    while !data.is_empty() {
        if data.len() < 6 {
            return Err("truncated iec104 ASDU header".into());
        }
        let type_id = data[0];
        let vsq = data[1];
        let sequence = vsq & 0x80 != 0;
        let count = usize::from(vsq & 0x7F);
        let cause = data[2] & 0x3F;
        data = &data[6..];

        let mut base_ioa = 0u32;
        for index in 0..count {
            let ioa = if sequence && index > 0 {
                base_ioa + index as u32
            } else {
                let bytes = data.get(..3).ok_or("truncated iec104 IOA")?;
                data = &data[3..];
                base_ioa = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]);
                base_ioa
            };
            let (value, used) = value_of(type_id, data)?;
            data = data.get(used..).ok_or("truncated iec104 value")?;
            rows.push(json!({
                "type_id": type_id,
                "cause": cause,
                "ioa": ioa,
                "value": value,
            }));
        }
    }
    Ok(rows)
}

/// (value, 消耗字节数)；时间标签解析后丢弃，只保留值。
fn value_of(type_id: u8, data: &[u8]) -> Result<(Value, usize), String> {
    let byte = |n: usize| data.get(n).copied().ok_or("truncated iec104 value");
    let i16_at = |n: usize| -> Result<i16, String> {
        let b = data.get(n..n + 2).ok_or("truncated iec104 value")?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    };
    let f32_at = |n: usize| -> Result<Value, String> {
        let b = data.get(n..n + 4).ok_or("truncated iec104 value")?;
        let value = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        Ok(if value.is_finite() { json!(value) } else { json!(value.to_string()) })
    };
    let i32_at = |n: usize| -> Result<i32, String> {
        let b = data.get(n..n + 4).ok_or("truncated iec104 value")?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    Ok(match type_id {
        1 => (json!(byte(0)? & 0x01 != 0), 1),
        3 => (json!(byte(0)? & 0x03), 1),
        5 => (json!(byte(0)?), 2),
        9 | 11 => (json!(i16_at(0)?), 3),
        13 => (f32_at(0)?, 5),
        15 | 37 => (json!(i32_at(0)?), if type_id == 37 { 12 } else { 5 }),
        30 => (json!(byte(0)? & 0x01 != 0), 8),
        31 => (json!(byte(0)? & 0x03), 8),
        34 | 35 => (json!(i16_at(0)?), 10),
        36 => (f32_at(0)?, 12),
        45 | 46 | 47 => (json!(byte(0)? & 0x01 != 0), 1),
        48 | 49 => (json!(i16_at(0)?), 3),
        50 => (f32_at(0)?, 5),
        58..=63 => {
            // 带时标命令：值 1/3/5 字节 + CP56Time2a 7 字节；只读会话不应出现。
            let (value, size) = match type_id {
                58 | 59 | 60 => (json!(byte(0)? & 0x01 != 0), 1),
                61 => (json!(i16_at(0)?), 3),
                62 => (json!(f32_at(0)?), 5),
                _ => (Value::Null, 1),
            };
            (value, size + 7)
        }
        70 | 100 | 101 => (json!(byte(0)?), 1),
        102 => (Value::Null, 0),
        103 => (Value::Null, 7),
        other => {
            return Err(format!(
                "unsupported iec104 ASDU type {other}; read-only monitoring types are supported"
            ))
        }
    })
}

fn parse_target(target: &str) -> Result<(String, u16), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("iec104://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid target `{target}`: {e}"))?;
    match url.scheme() {
        "iec104" | "iec-60870-5-104" | "tcp" => {}
        other => return Err(format!("unsupported iec104 target scheme `{other}`")),
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    Ok((host, url.port().unwrap_or(DEFAULT_PORT)))
}

fn opts_u16(req: &DebugRequest, key: &str) -> Option<u16> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_u32(req: &DebugRequest, key: &str) -> Option<u32> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i_frame(asdu: &[u8]) -> Vec<u8> {
        let mut apdu = vec![0x68, (4 + asdu.len()) as u8, 0x00, 0x00, 0x02, 0x00];
        apdu.extend_from_slice(asdu);
        apdu
    }

    #[test]
    fn parses_short_float_and_termination() {
        let mut asdu = vec![13, 0x01, 0x14, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00];
        asdu.extend_from_slice(&10.0f32.to_le_bytes());
        asdu.push(0x00);
        asdu.extend_from_slice(&[100, 0x01, 0x0A, 0x00, 0x01, 0x00, 0, 0, 0, 20]);
        let apdu = i_frame(&asdu);
        let rows = parse_asdus(&apdu[6..]).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["type_id"], 13);
        assert_eq!(rows[0]["ioa"], 1);
        assert_eq!(rows[0]["value"], 10.0);
        assert_eq!(rows[1]["cause"], 10);
    }

    #[test]
    fn rejection_has_no_payload() {
        let (value, used) = value_of(102, &[1, 2, 3]).unwrap();
        assert!(value.is_null());
        assert_eq!(used, 0);
    }

    #[test]
    fn target_defaults_to_2404() {
        assert_eq!(parse_target("host").unwrap(), ("host".into(), 2404));
        assert_eq!(parse_target("tcp://host:2405").unwrap(), ("host".into(), 2405));
        assert!(parse_target("http://host").is_err());
    }
}
