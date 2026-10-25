//! P2 Hessian 2.0 编解码（Dubbo 默认序列化）。
//!
//! 编码只发非紧凑形式（N/T/F/I/L/D/S/W/H），解码兼容 Java 端常见的紧凑
//! int/long/string/list。class def / object instance 不支持：Dubbo 泛化调用
//! 只需要 map/list/标量，遇到时返回明确错误。
//!
//! shortcut: 紧凑 double（0x60-0x7f 区间）与对象引用未实现；真实对端返回
//! 这些形式时报错，需要时按 Hessian 2.0 规范补全。

use serde_json::{json, Map, Value};

pub fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    write_value(&mut out, value)?;
    Ok(out)
}

pub fn decode(input: &mut &[u8]) -> Result<Value, String> {
    let tag = read_u8(input)?;
    decode_tag(input, tag)
}

fn write_value(out: &mut Vec<u8>, value: &Value) -> Result<(), String> {
    match value {
        Value::Null => out.push(b'N'),
        Value::Bool(true) => out.push(b'T'),
        Value::Bool(false) => out.push(b'F'),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if let Ok(v) = i32::try_from(i) {
                    out.push(b'I');
                    out.extend_from_slice(&v.to_be_bytes());
                } else {
                    out.push(b'L');
                    out.extend_from_slice(&i.to_be_bytes());
                }
            } else if let Some(f) = n.as_f64() {
                out.push(b'D');
                out.extend_from_slice(&f.to_bits().to_be_bytes());
            } else {
                return Err("hessian: unsupported number".into());
            }
        }
        Value::String(s) => write_string(out, s)?,
        Value::Array(items) => {
            out.push(0x57); // untyped variable-length list
            for item in items {
                write_value(out, item)?;
            }
            out.push(b'Z');
        }
        Value::Object(map) => {
            out.push(b'H'); // untyped map
            for (key, val) in map {
                write_string(out, key)?;
                write_value(out, val)?;
            }
            out.push(b'Z');
        }
    }
    Ok(())
}

fn write_string(out: &mut Vec<u8>, s: &str) -> Result<(), String> {
    let units = u16::try_from(s.encode_utf16().count())
        .map_err(|_| "hessian: string exceeds 65535 UTF-16 units")?;
    out.push(b'S');
    out.extend_from_slice(&units.to_be_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

fn decode_tag(input: &mut &[u8], tag: u8) -> Result<Value, String> {
    match tag {
        b'N' => Ok(Value::Null),
        b'T' => Ok(json!(true)),
        b'F' => Ok(json!(false)),
        b'I' => Ok(json!(read_i32(input)?)),
        b'L' => Ok(json!(read_i64(input)?)),
        b'D' => Ok(json!(read_f64(input)?)),
        b'S' => {
            let units = read_u16(input)? as usize;
            Ok(json!(read_string(input, units)?))
        }
        0x00..=0x1f => Ok(json!(read_string(input, tag as usize)?)),
        0x30..=0x33 => {
            let units = (((tag - 0x30) as usize) << 8) | read_u8(input)? as usize;
            Ok(json!(read_string(input, units)?))
        }
        // 紧凑 int
        0x80..=0xbf => Ok(json!(tag as i8 as i64 + 112)),
        0xc0..=0xcf => Ok(json!(((tag as i16 - 0xc8) << 8) + read_u8(input)? as i16)),
        0xd0..=0xd7 => {
            let high = (tag as i32 - 0xd4) << 16;
            let mid = (read_u8(input)? as i32) << 8;
            Ok(json!(high + mid + read_u8(input)? as i32))
        }
        // 紧凑 long
        0xd8..=0xef => Ok(json!(tag as i8 as i64 + 32)),
        0xf0..=0xff => Ok(json!(((tag as i16 - 0xf8) << 8) + read_u8(input)? as i16)),
        0x38..=0x3f => {
            let high = (tag as i32 - 0x3c) << 16;
            let mid = (read_u8(input)? as i32) << 8;
            Ok(json!(high + mid + read_u8(input)? as i32))
        }
        0x59 => Ok(json!(read_i32(input)?)),
        0x5b => Ok(json!(0.0)),
        0x5c => Ok(json!(1.0)),
        0x5d => Ok(json!(read_i8(input)? as f64)),
        0x5e => Ok(json!(read_i16(input)? as f64)),
        0x5f => Ok(json!(read_f32(input)? as f64)),
        // list
        0x57 => read_list_until_end(input),
        0x58 => {
            let len = read_i32(input)?;
            read_list_items(input, len)
        }
        0x55 => {
            skip_type(input)?;
            read_list_until_end(input)
        }
        0x56 => {
            skip_type(input)?;
            let len = read_i32(input)?;
            read_list_items(input, len)
        }
        0x70..=0x77 => {
            skip_type(input)?;
            read_list_items(input, (tag - 0x70) as i32)
        }
        0x78..=0x7f => read_list_items(input, (tag - 0x78) as i32),
        // map
        b'H' => read_map(input),
        b'M' => {
            skip_type(input)?;
            read_map(input)
        }
        b'C' | b'O' => Err(
            "hessian: class definitions/object instances are not supported; send maps/lists/scalars"
                .into(),
        ),
        other => Err(format!("hessian: unsupported tag 0x{other:02x}")),
    }
}

fn read_list_until_end(input: &mut &[u8]) -> Result<Value, String> {
    let mut items = Vec::new();
    loop {
        let tag = read_u8(input)?;
        if tag == b'Z' {
            break;
        }
        items.push(decode_tag(input, tag)?);
    }
    Ok(Value::Array(items))
}

fn read_list_items(input: &mut &[u8], len: i32) -> Result<Value, String> {
    if len < 0 {
        return Err(format!("hessian: invalid list length {len}"));
    }
    let mut items = Vec::with_capacity((len as usize).min(1024));
    for _ in 0..len {
        items.push(decode(input)?);
    }
    Ok(Value::Array(items))
}

fn read_map(input: &mut &[u8]) -> Result<Value, String> {
    let mut map = Map::new();
    loop {
        let tag = read_u8(input)?;
        if tag == b'Z' {
            break;
        }
        let key = match decode_tag(input, tag)? {
            Value::String(s) => s,
            other => other.to_string(),
        };
        map.insert(key, decode(input)?);
    }
    Ok(Value::Object(map))
}

fn skip_type(input: &mut &[u8]) -> Result<(), String> {
    decode(input).map(|_| ())
}

fn read_string(input: &mut &[u8], units: usize) -> Result<String, String> {
    let mut consumed = 0usize;
    let mut seen = 0usize;
    while seen < units {
        let first = *input.get(consumed).ok_or("incomplete")?;
        let len = match first {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return Err("hessian: invalid UTF-8 in string".into()),
        };
        let bytes = input.get(consumed..consumed + len).ok_or("incomplete")?;
        let text = std::str::from_utf8(bytes).map_err(|_| "hessian: invalid UTF-8 in string")?;
        let ch = text.chars().next().ok_or("hessian: invalid UTF-8 in string")?;
        seen += ch.len_utf16();
        consumed += len;
    }
    if seen != units {
        return Err("hessian: string length splits a surrogate pair".into());
    }
    let text = std::str::from_utf8(&input[..consumed])
        .map_err(|_| "hessian: invalid UTF-8 in string")?;
    let s = text.to_string();
    *input = &input[consumed..];
    Ok(s)
}

fn read_u8(input: &mut &[u8]) -> Result<u8, String> {
    let byte = *input.first().ok_or("incomplete")?;
    *input = &input[1..];
    Ok(byte)
}

fn read_i8(input: &mut &[u8]) -> Result<i8, String> {
    Ok(read_u8(input)? as i8)
}

fn read_u16(input: &mut &[u8]) -> Result<u16, String> {
    let b = take(input, 2)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn read_i16(input: &mut &[u8]) -> Result<i16, String> {
    Ok(read_u16(input)? as i16)
}

fn read_f32(input: &mut &[u8]) -> Result<f32, String> {
    let b = take(input, 4)?;
    Ok(f32::from_bits(u32::from_be_bytes([b[0], b[1], b[2], b[3]])))
}

fn read_i32(input: &mut &[u8]) -> Result<i32, String> {
    let b = take(input, 4)?;
    Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_i64(input: &mut &[u8]) -> Result<i64, String> {
    let b = take(input, 8)?;
    Ok(i64::from_be_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

fn read_f64(input: &mut &[u8]) -> Result<f64, String> {
    Ok(f64::from_bits(read_i64(input)? as u64))
}

fn take<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8], String> {
    if input.len() < len {
        return Err("incomplete".into());
    }
    let (head, tail) = input.split_at(len);
    *input = tail;
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_length_counts_surrogate_pairs() {
        let encoded = encode(&serde_json::json!("😀")).unwrap();
        assert_eq!(encoded, vec![b'S', 0, 2, 0xf0, 0x9f, 0x98, 0x80]);
        let mut input = &encoded[..];
        assert_eq!(decode(&mut input).unwrap(), serde_json::json!("😀"));
    }
}
