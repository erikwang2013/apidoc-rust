//! AMF0 子集编解码：number/boolean/string/object/null/undefined/ECMA array/date。
//!
//! 其余 marker 直接报错，避免把未知结构当成功解析。

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Amf0 {
    Number(f64),
    Boolean(bool),
    String(String),
    Null,
    Object(BTreeMap<String, Amf0>),
    EcmaArray(BTreeMap<String, Amf0>),
}

impl Amf0 {
    pub fn write(&self, out: &mut Vec<u8>) {
        match self {
            Amf0::Number(n) => {
                out.push(0x00);
                out.extend_from_slice(&n.to_be_bytes());
            }
            Amf0::Boolean(b) => {
                out.push(0x01);
                out.push(*b as u8);
            }
            Amf0::String(s) => write_utf8(out, s),
            Amf0::Null => out.push(0x05),
            Amf0::Object(fields) => {
                out.push(0x03);
                write_properties(out, fields);
            }
            Amf0::EcmaArray(fields) => {
                out.push(0x08);
                out.extend_from_slice(&(fields.len() as u32).to_be_bytes());
                write_properties(out, fields);
            }
        }
    }
}

pub fn encode_string(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    write_utf8(&mut out, s);
    out
}

pub fn encode_number(n: f64) -> Vec<u8> {
    let mut out = vec![0x00];
    out.extend_from_slice(&n.to_be_bytes());
    out
}

pub fn encode_object(fields: &[(&str, Amf0)]) -> Vec<u8> {
    let map = fields
        .iter()
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect();
    let mut out = Vec::new();
    Amf0::Object(map).write(&mut out);
    out
}

fn write_utf8(out: &mut Vec<u8>, s: &str) {
    out.push(0x02);
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn write_properties(out: &mut Vec<u8>, fields: &BTreeMap<String, Amf0>) {
    for (name, value) in fields {
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        value.write(out);
    }
    out.extend_from_slice(&[0x00, 0x00, 0x09]); // object end
}

const MAX_DEPTH: usize = 64;

pub fn decode_all(bytes: &[u8]) -> Result<Vec<Amf0>, String> {
    let mut position = 0;
    let mut values = Vec::new();
    while position < bytes.len() {
        values.push(decode_value(bytes, &mut position, 0)?);
    }
    Ok(values)
}

fn decode_value(bytes: &[u8], position: &mut usize, depth: usize) -> Result<Amf0, String> {
    if depth >= MAX_DEPTH {
        return Err(format!("AMF0 nesting exceeds {MAX_DEPTH} levels"));
    }
    let marker = take(bytes, position, 1)?[0];
    match marker {
        0x00 => Ok(Amf0::Number(f64::from_be_bytes(
            take(bytes, position, 8)?.try_into().unwrap(),
        ))),
        0x01 => Ok(Amf0::Boolean(take(bytes, position, 1)?[0] != 0)),
        0x02 => {
            let len = u16::from_be_bytes(take(bytes, position, 2)?.try_into().unwrap()) as usize;
            Ok(Amf0::String(read_text(bytes, position, len)?))
        }
        0x03 => Ok(Amf0::Object(read_properties(bytes, position, depth + 1)?)),
        0x05 | 0x06 => Ok(Amf0::Null),
        0x08 => {
            take(bytes, position, 4)?; // count 不可信，按 object end 结束
            Ok(Amf0::EcmaArray(read_properties(bytes, position, depth + 1)?))
        }
        0x0b => {
            // Date: 毫秒数 + 时区，时区语义对调试无用
            let ms = f64::from_be_bytes(take(bytes, position, 8)?.try_into().unwrap());
            take(bytes, position, 2)?;
            Ok(Amf0::Number(ms))
        }
        0x0c => {
            let len = u32::from_be_bytes(take(bytes, position, 4)?.try_into().unwrap()) as usize;
            Ok(Amf0::String(read_text(bytes, position, len)?))
        }
        other => Err(format!("unsupported AMF0 type 0x{other:02x}")),
    }
}

fn read_properties(
    bytes: &[u8],
    position: &mut usize,
    depth: usize,
) -> Result<BTreeMap<String, Amf0>, String> {
    let mut fields = BTreeMap::new();
    loop {
        let len = u16::from_be_bytes(take(bytes, position, 2)?.try_into().unwrap()) as usize;
        if len == 0 {
            let end = take(bytes, position, 1)?[0];
            if end != 0x09 {
                return Err(format!("invalid AMF0 object end 0x{end:02x}"));
            }
            return Ok(fields);
        }
        let name = read_text(bytes, position, len)?;
        fields.insert(name, decode_value(bytes, position, depth)?);
    }
}

fn take<'a>(bytes: &'a [u8], position: &mut usize, len: usize) -> Result<&'a [u8], String> {
    let end = position.checked_add(len).ok_or("AMF0 length overflow")?;
    let slice = bytes.get(*position..end).ok_or("truncated AMF0 value")?;
    *position = end;
    Ok(slice)
}

fn read_text(bytes: &[u8], position: &mut usize, len: usize) -> Result<String, String> {
    let slice = take(bytes, position, len)?;
    String::from_utf8(slice.to_vec()).map_err(|_| "AMF0 string is not UTF-8".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_object(depth: usize) -> Vec<u8> {
        if depth == 0 {
            return vec![0x05]; // null
        }
        let mut out = vec![0x03, 0x00, 0x01, b'x'];
        out.extend(nested_object(depth - 1));
        out.extend_from_slice(&[0x00, 0x00, 0x09]); // object end
        out
    }

    #[test]
    fn rejects_deep_nesting() {
        assert!(decode_all(&nested_object(MAX_DEPTH + 1)).is_err());
        assert!(decode_all(&nested_object(8)).is_ok());
    }
}
