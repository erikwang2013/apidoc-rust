//! DNP3 链路层与对象解码（dnp3 子模块）：CRC-16/DNP、16 字节数据块、对象头/范围。
//!
//! 只解析响应；遇到未支持的限定符/变体显式报错，不猜测长度。

use serde_json::{json, Value};
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

/// DNP3 链路层长度字段最大值（控制 + 地址 + 用户数据）。
const MAX_LINK_DATA: usize = 250;

pub(crate) struct LinkFrame {
    pub(crate) ctrl: u8,
    pub(crate) dst: u16,
    pub(crate) src: u16,
    pub(crate) data: Vec<u8>,
}

pub(crate) async fn read_link_frame(
    stream: &mut TcpStream,
    deadline: Instant,
) -> Result<Option<LinkFrame>, String> {
    let mut header = [0u8; 10];
    let Some(()) = read_exact_deadline(stream, &mut header, deadline).await? else {
        return Ok(None);
    };
    if header[0..2] != [0x05, 0x64] {
        return Err("invalid dnp3 start bytes".into());
    }
    let length = header[2] as usize;
    if !(5..=MAX_LINK_DATA).contains(&length) {
        return Err(format!("invalid dnp3 link length {length}"));
    }
    if crc16(&header[2..8]) != u16::from_le_bytes([header[8], header[9]]) {
        return Err("dnp3 header CRC mismatch".into());
    }
    let data_len = length - 5;
    let blocks = data_len.div_ceil(16);
    let mut body = vec![0u8; data_len + blocks * 2];
    let Some(()) = read_exact_deadline(stream, &mut body, deadline).await? else {
        return Err("truncated dnp3 frame".into());
    };
    let mut data = Vec::with_capacity(data_len);
    for (index, chunk) in body.chunks(16 + 2).enumerate() {
        let block_len = if index + 1 == blocks {
            data_len - index * 16
        } else {
            16
        };
        let block = &chunk[..block_len];
        let expected = u16::from_le_bytes([chunk[block_len], chunk[block_len + 1]]);
        if crc16(block) != expected {
            return Err(format!("dnp3 data CRC mismatch in block {index}"));
        }
        data.extend_from_slice(block);
    }
    Ok(Some(LinkFrame {
        ctrl: header[3],
        dst: u16::from_le_bytes([header[4], header[5]]),
        src: u16::from_le_bytes([header[6], header[7]]),
        data,
    }))
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
        Ok(Err(e)) => Err(format!("dnp3 read failed: {e}")),
        Err(_) => Ok(None),
    }
}

/// 链路头（LEN/CTRL/DST/SRC + CRC）+ 每个 16 字节用户数据块 + CRC。
pub(crate) fn encode_link_frame(
    ctrl: u8,
    dst: u16,
    src: u16,
    user_data: &[u8],
) -> Result<Vec<u8>, String> {
    if user_data.len() + 5 > MAX_LINK_DATA {
        return Err(format!("dnp3 user data exceeds {MAX_LINK_DATA} bytes"));
    }
    let length = (user_data.len() + 5) as u8;
    let mut header = vec![0x05, 0x64, length, ctrl];
    header.extend_from_slice(&dst.to_le_bytes());
    header.extend_from_slice(&src.to_le_bytes());
    let mut out = header.clone();
    out.extend_from_slice(&crc16(&header[2..]).to_le_bytes());
    for chunk in user_data.chunks(16) {
        out.extend_from_slice(chunk);
        out.extend_from_slice(&crc16(chunk).to_le_bytes());
    }
    Ok(out)
}

/// CRC-16/DNP：poly 0x3D65（反射 0xA6BC），init 0，xorout 0xFFFF。
pub(crate) fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in data {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA6BC
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF
}

const MAX_POINTS: usize = 10000;

pub(crate) fn parse_objects(mut data: &[u8]) -> Result<Vec<Value>, String> {
    let mut points = Vec::new();
    while !data.is_empty() {
        if data.len() < 3 {
            return Err("truncated dnp3 object header".into());
        }
        let group = data[0];
        let variation = data[1];
        let qualifier = data[2];
        data = &data[3..];
        if qualifier >> 4 == 6 {
            return Err("dnp3 all-objects qualifier in a response is not supported".into());
        }
        let (start, count, after_range) = read_range(data, qualifier)?;
        data = after_range;
        let prefix = qualifier & 0x0F;
        if prefix > 3 {
            return Err(format!("dnp3 size-prefix qualifier {prefix} is not supported"));
        }
        if matches!((group, variation), (1, 1) | (3, 1)) {
            if prefix != 0 {
                return Err("dnp3 index-prefixed packed bit fields are not supported".into());
            }
            if points.len() + count as usize > MAX_POINTS {
                return Err(format!("dnp3 response exceeds {MAX_POINTS} points"));
            }
            let (used, packed) = read_packed(group, variation, data, start, count)?;
            points.extend(packed);
            data = &data[used..];
            continue;
        }
        if points.len() + count as usize > MAX_POINTS {
            return Err(format!("dnp3 response exceeds {MAX_POINTS} points"));
        }
        for offset in 0..count {
            let index = match prefix {
                1 => take_u8(&mut data)? as u32,
                2 => take_u16(&mut data)? as u32,
                3 => take_u32(&mut data)?,
                _ => start + offset,
            };
            let (value, flags) = read_point(group, variation, &mut data, offset)?;
            points.push(json!({
                "group": group,
                "variation": variation,
                "index": index,
                "value": value,
                "flags": flags,
            }));
        }
    }
    Ok(points)
}

/// 打包位字段（G1V1/G3V1）：整组一起消费，返回 (消耗字节数, 点列表)。
fn read_packed(
    group: u8,
    variation: u8,
    data: &[u8],
    start: u32,
    count: u32,
) -> Result<(usize, Vec<Value>), String> {
    let bits = if variation == 1 && group == 1 { 1 } else { 2 };
    let bytes = (count as usize * bits).div_ceil(8);
    let packed = data.get(..bytes).ok_or("truncated dnp3 packed bits")?;
    let points = (0..count)
        .map(|i| {
            let value = if bits == 1 {
                u8::from(packed[i as usize / 8] & (1 << (i % 8)) != 0)
            } else {
                (packed[i as usize / 4] >> ((i as usize % 4) * 2)) & 0x03
            };
            json!({
                "group": group, "variation": variation,
                "index": start + i, "value": value, "flags": Value::Null
            })
        })
        .collect();
    Ok((bytes, points))
}

fn read_range(data: &[u8], qualifier: u8) -> Result<(u32, u32, &[u8]), String> {
    match qualifier >> 4 {
        0 => {
            let (start, stop) = (take_u8_raw(data, 0)?, take_u8_raw(data, 1)?);
            if stop < start {
                return Err("dnp3 range stop < start".into());
            }
            Ok((start as u32, (stop - start + 1) as u32, &data[2..]))
        }
        1 => {
            let start = u16::from_le_bytes([take_u8_raw(data, 0)?, take_u8_raw(data, 1)?]);
            let stop = u16::from_le_bytes([take_u8_raw(data, 2)?, take_u8_raw(data, 3)?]);
            if stop < start {
                return Err("dnp3 range stop < start".into());
            }
            Ok((start as u32, (u32::from(stop) - u32::from(start) + 1), &data[4..]))
        }
        2 => {
            let start = take_u32_raw(data, 0)?;
            let stop = take_u32_raw(data, 4)?;
            if stop < start {
                return Err("dnp3 range stop < start".into());
            }
            Ok((start, (stop - start).saturating_add(1), &data[8..]))
        }
        7 => Ok((0, take_u8_raw(data, 0)? as u32, &data[1..])),
        8 => Ok((0, take_u16_raw(data, 0)? as u32, &data[2..])),
        9 => Ok((0, take_u32_raw(data, 0)?, &data[4..])),
        other => Err(format!("unsupported dnp3 range qualifier {other}")),
    }
}

/// 消费一个点值，返回 (value, flags)。flags 对无标志变体为 null。
fn read_point(
    group: u8,
    variation: u8,
    data: &mut &[u8],
    offset: u32,
) -> Result<(Value, Value), String> {
    let numeric = |value: Value, flags: Option<u8>| (value, flags.map_or(Value::Null, |f| json!(f)));
    match (group, variation) {
        (1, 2) | (10, 2) => {
            let flags = take_u8(data)?;
            Ok(numeric(json!(flags & 0x80 != 0), Some(flags)))
        }
        (3, 2) => {
            let flags = take_u8(data)?;
            Ok(numeric(json!((flags >> 6) & 0x03), Some(flags)))
        }
        (20, 1) | (21, 1) => {
            let flags = take_u8(data)?;
            Ok(numeric(json!(take_u32(data)?), Some(flags)))
        }
        (20, 2) | (21, 2) => {
            let flags = take_u8(data)?;
            Ok(numeric(json!(take_u16(data)?), Some(flags)))
        }
        (20, 5) | (21, 5) => Ok(numeric(json!(take_u32(data)?), None)),
        (20, 6) | (21, 6) => Ok(numeric(json!(take_u16(data)?), None)),
        (30, 1) | (40, 1) => {
            let flags = take_u8(data)?;
            Ok(numeric(json!(take_i32(data)?), Some(flags)))
        }
        (30, 2) | (40, 2) => {
            let flags = take_u8(data)?;
            Ok(numeric(json!(take_i16(data)?), Some(flags)))
        }
        (30, 3) | (40, 3) => Ok(numeric(json!(take_i32(data)?), None)),
        (30, 4) | (40, 4) => Ok(numeric(json!(take_i16(data)?), None)),
        (30, 5) => {
            let flags = take_u8(data)?;
            Ok(numeric(float_value(f64::from(take_f32(data)?)), Some(flags)))
        }
        (30, 6) => {
            let flags = take_u8(data)?;
            Ok(numeric(float_value(take_f64(data)?), Some(flags)))
        }
        (30, 7) => Ok(numeric(float_value(f64::from(take_f32(data)?)), None)),
        (30, 8) => Ok(numeric(float_value(take_f64(data)?), None)),
        (50, 1..=3) => Ok(numeric(
            json!({ "encoding": "hex", "data": hex(take_bytes(data, 6)?) }),
            None,
        )),
        (50, 4) => Ok(numeric(
            json!({ "encoding": "hex", "data": hex(take_bytes(data, 10)?) }),
            None,
        )),
        _ => Err(format!(
            "unsupported dnp3 object group {group} variation {variation} at point offset {offset}"
        )),
    }
}

fn float_value(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        json!(value.to_string())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn take_bytes<'a>(data: &mut &'a [u8], len: usize) -> Result<&'a [u8], String> {
    if data.len() < len {
        return Err("truncated dnp3 object value".into());
    }
    let (head, tail) = data.split_at(len);
    *data = tail;
    Ok(head)
}

fn take_u8(data: &mut &[u8]) -> Result<u8, String> {
    Ok(take_bytes(data, 1)?[0])
}

fn take_u16(data: &mut &[u8]) -> Result<u16, String> {
    let bytes = take_bytes(data, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn take_u32(data: &mut &[u8]) -> Result<u32, String> {
    let bytes = take_bytes(data, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn take_i16(data: &mut &[u8]) -> Result<i16, String> {
    Ok(take_u16(data)? as i16)
}

fn take_i32(data: &mut &[u8]) -> Result<i32, String> {
    Ok(take_u32(data)? as i32)
}

fn take_f32(data: &mut &[u8]) -> Result<f32, String> {
    Ok(f32::from_bits(take_u32(data)?))
}

fn take_f64(data: &mut &[u8]) -> Result<f64, String> {
    let bytes = take_bytes(data, 8)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(bytes);
    Ok(f64::from_bits(u64::from_le_bytes(buf)))
}

fn take_u8_raw(data: &[u8], offset: usize) -> Result<u8, String> {
    data.get(offset).copied().ok_or_else(|| "truncated dnp3 range".into())
}

fn take_u16_raw(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data.get(offset..offset + 2).ok_or("truncated dnp3 range")?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn take_u32_raw(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data.get(offset..offset + 4).ok_or("truncated dnp3 range")?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_reference_vector() {
        // 由独立实现计算：链路头 LEN=0x17 CTRL=0x44 DST=0 SRC=1 的 CRC 为 0x4EEC。
        assert_eq!(crc16(&[0x17, 0x44, 0x00, 0x00, 0x01, 0x00]), 0x4EEC);
    }

    #[test]
    fn link_frame_matches_golden_vector() {
        let user_data = [0xC0, 0xC0, 0x01, 0x3C, 0x01, 0x06];
        let frame = encode_link_frame(0xC4, 1, 0, &user_data).unwrap();
        assert_eq!(hex(&frame), "05640bc4010000004de9c0c0013c0106ff50");
    }

    #[test]
    fn caps_response_points() {
        let data = [0x01, 0x01, 0x10, 0x00, 0x00, 0xff, 0xff];
        let err = parse_objects(&data).unwrap_err();
        assert!(err.contains("exceeds 10000 points"), "{err}");
    }

    #[test]
    fn golden_response_parses_points() {
        // group 30 var 1 (32-bit analog input with flags), 1-byte start/stop, 2 points.
        let data = hex_decode("1e01000001012a00000001f9ffffff");
        let points = parse_objects(&data).unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0]["value"], 42);
        assert_eq!(points[0]["flags"], 1);
        assert_eq!(points[1]["value"], -7);
    }

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
