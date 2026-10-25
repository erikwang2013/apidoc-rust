//! OPC UA 二进制响应解码（opcua 子模块）：小端原语、NodeId、DataValue、Variant。
//!
//! 只覆盖浏览/读/写需要的类型；遇到未实现的复杂类型显式报错，不猜测长度。

use base64::Engine;
use serde_json::{json, Value};

pub(crate) struct Dec<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Dec<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Dec<'a> {
        Dec { data, pos: 0 }
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(len).ok_or("opcua length overflow")?;
        let bytes = self.data.get(self.pos..end).ok_or("truncated opcua message")?;
        self.pos = end;
        Ok(bytes)
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    pub(crate) fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }

    pub(crate) fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }

    pub(crate) fn u64(&mut self) -> Result<u64, String> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(raw))
    }

    pub(crate) fn i64(&mut self) -> Result<i64, String> {
        Ok(self.u64()? as i64)
    }

    pub(crate) fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub(crate) fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.u64()?))
    }

    pub(crate) fn string(&mut self) -> Result<Option<String>, String> {
        Ok(self
            .byte_string()?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
    }

    pub(crate) fn byte_string(&mut self) -> Result<Option<Vec<u8>>, String> {
        let len = self.i32()?;
        if len < 0 {
            return Ok(None);
        }
        Ok(Some(self.take(len as usize)?.to_vec()))
    }
}

pub(crate) fn check_response_header(dec: &mut Dec<'_>) -> Result<(), String> {
    dec.i64()?; // timestamp
    dec.u32()?; // requestHandle
    let status = dec.u32()?;
    skip_diagnostic_info(dec)?;
    let strings = dec.i32()?;
    for _ in 0..strings.max(0) {
        let _ = dec.string()?;
    }
    skip_extension_object(dec)?;
    if status != 0 {
        return Err(format!("opcua service status {}", status_label(status)));
    }
    Ok(())
}

pub(crate) fn decode_data_value(dec: &mut Dec<'_>) -> Result<Value, String> {
    let mask = dec.u8()?;
    let value = if mask & 0x01 != 0 {
        Some(decode_variant(dec)?)
    } else {
        None
    };
    let status = if mask & 0x02 != 0 { dec.u32()? } else { 0 };
    if mask & 0x04 != 0 {
        dec.i64()?;
    }
    if mask & 0x08 != 0 {
        dec.u32()?;
    }
    if mask & 0x10 != 0 {
        dec.i64()?;
    }
    if mask & 0x20 != 0 {
        dec.u32()?;
    }
    match value {
        Some(value) if status == 0 => Ok(value),
        _ => Err(format!("opcua read status {}", status_label(status))),
    }
}

fn decode_variant(dec: &mut Dec<'_>) -> Result<Value, String> {
    let mask = dec.u8()?;
    let is_array = mask & 0x80 != 0;
    let has_dimensions = mask & 0x40 != 0;
    let type_id = mask & 0x3F;
    let value = if is_array {
        let count = dec.i32()?;
        if count < 0 {
            json!([])
        } else {
            let mut items = Vec::new();
            for _ in 0..count.min(10_000) {
                items.push(decode_scalar(dec, type_id)?);
            }
            json!(items)
        }
    } else {
        decode_scalar(dec, type_id)?
    };
    if has_dimensions {
        let count = dec.i32()?;
        for _ in 0..count.max(0) {
            dec.u32()?;
        }
    }
    Ok(value)
}

fn decode_scalar(dec: &mut Dec<'_>, type_id: u8) -> Result<Value, String> {
    Ok(match type_id {
        0 => Value::Null,
        1 => json!(dec.u8()? != 0),
        2 => json!(dec.u8()? as i8),
        3 => json!(dec.u8()?),
        4 => json!(dec.i16()?),
        5 => json!(dec.u16()?),
        6 => json!(dec.i32()?),
        7 => json!(dec.u32()?),
        8 => json!(dec.i64()?),
        9 => json!(dec.u64()?),
        10 => json!(dec.f32()?),
        11 => json!(dec.f64()?),
        12 => match dec.string()? {
            Some(s) => json!(s),
            None => Value::Null,
        },
        13 => json!(dec.i64()?),
        15 => match dec.byte_string()? {
            Some(bytes) => json!({
                "encoding": "base64",
                "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            }),
            None => Value::Null,
        },
        17 => json!(decode_node_id(dec)?),
        19 => json!(status_label(dec.u32()?)),
        20 => {
            let namespace = dec.u16()?;
            let name = dec.string()?.unwrap_or_default();
            json!({ "namespace_index": namespace, "name": name })
        }
        21 => json!(decode_localized_text(dec)?),
        other => return Err(format!("unsupported opcua variant type {other}")),
    })
}

pub(crate) fn decode_node_id(dec: &mut Dec<'_>) -> Result<String, String> {
    let encoding = dec.u8()?;
    let id = match encoding & 0x0F {
        0 => format!("i={}", dec.u8()?),
        1 => {
            let namespace = dec.u8()?;
            let id = dec.u16()?;
            if namespace == 0 {
                format!("i={id}")
            } else {
                format!("ns={namespace};i={id}")
            }
        }
        2 => {
            let namespace = dec.u16()?;
            let id = dec.u32()?;
            if namespace == 0 {
                format!("i={id}")
            } else {
                format!("ns={namespace};i={id}")
            }
        }
        3 => {
            let namespace = dec.u16()?;
            let name = dec.string()?.unwrap_or_default();
            format!("ns={namespace};s={name}")
        }
        4 => {
            let guid = dec.take(16)?;
            format!(
                "g={}",
                guid.iter().map(|b| format!("{b:02x}")).collect::<String>()
            )
        }
        5 => {
            let bytes = dec.byte_string()?.unwrap_or_default();
            format!(
                "b={}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        }
        other => return Err(format!("unsupported opcua node id encoding {other}")),
    };
    if encoding & 0x40 != 0 {
        let _ = dec.string()?; // namespace URI
    }
    if encoding & 0x80 != 0 {
        dec.u32()?; // server index
    }
    Ok(id)
}

pub(crate) fn skip_node_id(dec: &mut Dec<'_>) -> Result<(), String> {
    decode_node_id(dec).map(|_| ())
}

fn skip_extension_object(dec: &mut Dec<'_>) -> Result<(), String> {
    skip_node_id(dec)?;
    match dec.u8()? {
        0 => Ok(()),
        1 | 2 => {
            let len = dec.i32()?;
            dec.take(len.max(0) as usize)?;
            Ok(())
        }
        other => Err(format!("unsupported opcua extension object encoding {other}")),
    }
}

fn skip_diagnostic_info(dec: &mut Dec<'_>) -> Result<(), String> {
    let mask = dec.u8()?;
    if mask & 0x01 != 0 {
        let _ = dec.string()?;
    }
    if mask & 0x02 != 0 {
        decode_localized_text(dec)?;
    }
    if mask & 0x04 != 0 {
        let _ = dec.string()?;
    }
    if mask & 0x08 != 0 {
        let _ = dec.string()?;
    }
    if mask & 0x10 != 0 {
        dec.u32()?;
    }
    if mask & 0x20 != 0 {
        skip_diagnostic_info(dec)?;
    }
    Ok(())
}

pub(crate) fn decode_localized_text(dec: &mut Dec<'_>) -> Result<String, String> {
    let mask = dec.u8()?;
    let _locale = if mask & 0x01 != 0 {
        dec.string()?
    } else {
        None
    };
    let text = if mask & 0x02 != 0 {
        dec.string()?
    } else {
        None
    };
    Ok(text.unwrap_or_default())
}

pub(crate) fn status_label(status: u32) -> String {
    match status {
        0 => "Good".into(),
        0x8000_0000 => "BadUnexpectedError".into(),
        0x801F_0000 => "BadUserAccessDenied".into(),
        0x8020_0000 => "BadIdentityTokenInvalid".into(),
        0x8021_0000 => "BadIdentityTokenRejected".into(),
        0x8035_0000 => "BadNotConnected".into(),
        other => format!("Bad (0x{other:08X})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_id(id: u32) -> Vec<u8> {
        if id <= 0xFF {
            vec![0x00, id as u8]
        } else {
            let mut out = vec![0x01, 0x00];
            out.extend_from_slice(&(id as u16).to_le_bytes());
            out
        }
    }

    #[test]
    fn decodes_data_value_scalars_and_arrays() {
        // Double 7.25
        let mut double = vec![0x01, 11];
        double.extend_from_slice(&7.25f64.to_le_bytes());
        assert_eq!(decode_data_value(&mut Dec::new(&double)).unwrap(), 7.25);

        // Int32 数组 [1, 2]
        let mut array = vec![0x01, 0x80 | 6, 2, 0, 0, 0];
        array.extend_from_slice(&1i32.to_le_bytes());
        array.extend_from_slice(&2i32.to_le_bytes());
        assert_eq!(decode_data_value(&mut Dec::new(&array)).unwrap(), json!([1, 2]));
    }

    #[test]
    fn decodes_response_header_and_flags_bad_status() {
        let mut ok = Vec::new();
        ok.extend_from_slice(&0i64.to_le_bytes());
        ok.extend_from_slice(&1u32.to_le_bytes());
        ok.extend_from_slice(&0u32.to_le_bytes());
        ok.extend_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00]);
        assert!(check_response_header(&mut Dec::new(&ok)).is_ok());

        let mut bad = Vec::new();
        bad.extend_from_slice(&0i64.to_le_bytes());
        bad.extend_from_slice(&1u32.to_le_bytes());
        bad.extend_from_slice(&0x801F_0000u32.to_le_bytes());
        bad.extend_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00]);
        let err = check_response_header(&mut Dec::new(&bad)).unwrap_err();
        assert!(err.contains("BadUserAccessDenied"), "{err}");
    }

    #[test]
    fn decodes_numeric_node_ids() {
        assert_eq!(decode_node_id(&mut Dec::new(&node_id(85))).unwrap(), "i=85");
        assert_eq!(
            decode_node_id(&mut Dec::new(&[0x01, 2, 0xE8, 0x03])).unwrap(),
            "ns=2;i=1000"
        );
    }
}
