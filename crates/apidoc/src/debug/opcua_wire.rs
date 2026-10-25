//! OPC UA 二进制消息组帧与请求编码（opcua 子模块）。
//!
//! 仅 SecurityPolicy None 的明文消息；所有整数小端。
//! HEL/ACK 无安全通道头；OPN 用非对称安全头（策略串 + 空证书），MSG 用令牌 id。

use super::codec::{status_label, Dec};
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

const RECEIVE_BUFFER: u32 = 65_535;

pub(crate) struct Message {
    pub(crate) kind: [u8; 3],
    pub(crate) channel: u32,
    pub(crate) policy: Option<String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) async fn read_message(
    stream: &mut TcpStream,
    deadline: Instant,
    max_bytes: usize,
) -> Result<Message, String> {
    let mut header = [0u8; 8];
    read_exact_deadline(stream, &mut header, deadline).await?;
    let kind = [header[0], header[1], header[2]];
    if header[3] == b'C' {
        return Err("opcua multi-chunk responses are not supported".into());
    }
    if header[3] != b'F' {
        return Err("opcua message was aborted by the server".into());
    }
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if !(8..=max_bytes).contains(&size) {
        return Err(format!(
            "opcua message size {size} exceeds max_bytes ({max_bytes})"
        ));
    }
    let mut rest = vec![0u8; size - 8];
    read_exact_deadline(stream, &mut rest, deadline).await?;
    if &kind == b"ERR" {
        let mut dec = Dec::new(&rest);
        let status = dec.u32().unwrap_or(0);
        let reason = dec.string().unwrap_or_default().unwrap_or_default();
        return Err(format!("opcua error {} {reason}", status_label(status)));
    }
    if &kind == b"ACK" {
        return Ok(Message {
            kind,
            channel: 0,
            policy: None,
            body: rest,
        });
    }
    let mut dec = Dec::new(&rest);
    let channel = dec.u32()?;
    let policy = if &kind == b"OPN" {
        let policy = dec.string()?;
        let _sender_certificate = dec.byte_string()?;
        let _receiver_thumbprint = dec.byte_string()?;
        Some(policy.unwrap_or_default())
    } else {
        let _token_id = dec.u32()?;
        None
    };
    let _sequence = dec.u32()?;
    let _request = dec.u32()?;
    Ok(Message {
        kind,
        channel,
        policy,
        body: dec.rest().to_vec(),
    })
}

pub(crate) fn encode_hel(endpoint: &str, max_message: u32) -> Vec<u8> {
    let mut out = vec![b'H', b'E', b'L', b'F'];
    let body_len = 5 * 4 + 4 + endpoint.len();
    put_u32(&mut out, (8 + body_len) as u32);
    put_u32(&mut out, 0); // protocolVersion
    put_u32(&mut out, RECEIVE_BUFFER);
    put_u32(&mut out, RECEIVE_BUFFER);
    put_u32(&mut out, max_message.max(8_192));
    put_u32(&mut out, 1); // maxChunkCount：不支持多分块
    put_string(&mut out, endpoint);
    out
}

pub(crate) fn encode_opn(
    channel: u32,
    policy: &str,
    seq: u32,
    request: u32,
    body: &[u8],
) -> Vec<u8> {
    let mut out = vec![b'O', b'P', b'N', b'F'];
    put_u32(&mut out, (32 + policy.len() + body.len()) as u32);
    put_u32(&mut out, channel);
    put_string(&mut out, policy);
    put_bytes(&mut out, None); // senderCertificate
    put_bytes(&mut out, None); // receiverCertificateThumbprint
    put_u32(&mut out, seq);
    put_u32(&mut out, request);
    out.extend_from_slice(body);
    out
}

pub(crate) fn encode_msg(
    kind: &[u8; 3],
    channel: u32,
    token: u32,
    seq: u32,
    request: u32,
    body: &[u8],
) -> Vec<u8> {
    let mut out = vec![kind[0], kind[1], kind[2], b'F'];
    put_u32(&mut out, (24 + body.len()) as u32);
    put_u32(&mut out, channel);
    put_u32(&mut out, token);
    put_u32(&mut out, seq);
    put_u32(&mut out, request);
    out.extend_from_slice(body);
    out
}

pub(crate) fn put_request_header(out: &mut Vec<u8>, auth: &[u8], handle: u32) {
    out.extend_from_slice(auth);
    put_i64(out, 0); // timestamp
    put_u32(out, handle);
    put_u32(out, 0); // returnDiagnostics
    put_bytes(out, None); // auditEntryId
    put_u32(out, 10_000); // timeoutHint
    put_extension_object(out, 0, |_| {}); // additionalHeader null
}

pub(crate) fn encode_variant(out: &mut Vec<u8>, value: &str) {
    if let Ok(boolean) = value.parse::<bool>() {
        out.push(1);
        out.push(u8::from(boolean));
    } else if let Ok(integer) = value.parse::<i64>() {
        if i32::try_from(integer).is_ok() {
            out.push(6);
            put_i32(out, integer as i32);
        } else {
            out.push(8);
            put_i64(out, integer);
        }
    } else if let Ok(float) = value.parse::<f64>() {
        out.push(11);
        put_f64(out, float);
    } else {
        out.push(12);
        put_string(out, value);
    }
}

pub(crate) fn put_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

pub(crate) fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn put_i32(out: &mut Vec<u8>, value: i32) {
    put_u32(out, value as u32);
}

pub(crate) fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn put_f64(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn put_string(out: &mut Vec<u8>, value: &str) {
    put_bytes(out, Some(value.as_bytes()));
}

pub(crate) fn put_bytes(out: &mut Vec<u8>, value: Option<&[u8]>) {
    match value {
        Some(bytes) => {
            put_i32(out, bytes.len() as i32);
            out.extend_from_slice(bytes);
        }
        None => put_i32(out, -1),
    }
}

pub(crate) fn put_node_id(out: &mut Vec<u8>, id: u32) {
    if id <= 0xFF {
        out.extend_from_slice(&[0x00, id as u8]);
    } else if id <= 0xFFFF {
        out.extend_from_slice(&[0x01, 0x00]);
        put_u16(out, id as u16);
    } else {
        out.push(0x02);
        put_u16(out, 0);
        put_u32(out, id);
    }
}

pub(crate) fn put_node_id_string(out: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let (namespace, rest) = match value.strip_prefix("ns=") {
        Some(rest) => {
            let (ns, rest) = rest
                .split_once(';')
                .ok_or("node_id `ns=` must be followed by `;`")?;
            (
                ns.parse::<u16>().map_err(|_| "invalid node_id namespace")?,
                rest,
            )
        }
        None => (0, value),
    };
    if let Some(id) = rest.strip_prefix("i=") {
        let id: u32 = id.parse().map_err(|_| "invalid numeric node_id")?;
        if namespace == 0 && id <= 0xFF {
            out.extend_from_slice(&[0x00, id as u8]);
        } else if namespace <= 0xFF && id <= 0xFFFF {
            out.extend_from_slice(&[0x01, namespace as u8]);
            put_u16(out, id as u16);
        } else {
            out.push(0x02);
            put_u16(out, namespace);
            put_u32(out, id);
        }
        Ok(())
    } else if let Some(text) = rest.strip_prefix("s=") {
        out.push(0x03);
        put_u16(out, namespace);
        put_string(out, text);
        Ok(())
    } else {
        Err("node_id must look like `i=85`, `ns=2;i=5` or `s=name`".into())
    }
}

pub(crate) fn put_localized_text(out: &mut Vec<u8>, text: &str) {
    out.push(0x02); // 只有 text 字段
    put_string(out, text);
}

pub(crate) fn put_extension_object(out: &mut Vec<u8>, type_id: u32, body: impl FnOnce(&mut Vec<u8>)) {
    put_node_id(out, type_id);
    if type_id == 0 {
        out.push(0x00); // null ExtensionObject
        return;
    }
    out.push(0x01); // Body
    let mut encoded = Vec::new();
    body(&mut encoded);
    put_u32(out, encoded.len() as u32);
    out.extend_from_slice(&encoded);
}

async fn read_exact_deadline(
    stream: &mut TcpStream,
    buf: &mut [u8],
    deadline: Instant,
) -> Result<(), String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("opcua read timed out".into());
    }
    tokio::time::timeout(remaining, stream.read_exact(buf))
        .await
        .map_err(|_| "opcua read timed out".to_string())?
        .map_err(|e| format!("opcua read failed: {e}"))
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hel_length_matches_bytes() {
        let hel = encode_hel("opc.tcp://h:4840", 65_535);
        assert_eq!(&hel[..4], b"HELF");
        assert_eq!(
            u32::from_le_bytes([hel[4], hel[5], hel[6], hel[7]]) as usize,
            hel.len()
        );
    }

    #[test]
    fn opn_length_matches_bytes() {
        let opn = encode_opn(0, "policy", 1, 1, &[0xaa, 0xbb]);
        assert_eq!(&opn[..4], b"OPNF");
        assert_eq!(
            u32::from_le_bytes([opn[4], opn[5], opn[6], opn[7]]) as usize,
            opn.len()
        );
    }

    #[test]
    fn numeric_node_ids_pick_compact_encoding() {
        let mut two = Vec::new();
        put_node_id_string(&mut two, "i=85").unwrap();
        assert_eq!(two, vec![0x00, 85]);

        let mut four = Vec::new();
        put_node_id_string(&mut four, "ns=2;i=1000").unwrap();
        assert_eq!(four, vec![0x01, 2, 0xE8, 0x03]);

        let mut string = Vec::new();
        put_node_id_string(&mut string, "ns=3;s=Demo").unwrap();
        assert_eq!(string[..3], [0x03, 3, 0]);
        assert_eq!(&string[3..], &[4, 0, 0, 0, b'D', b'e', b'm', b'o']);
        assert!(put_node_id_string(&mut Vec::new(), "x=1").is_err());
    }
}
