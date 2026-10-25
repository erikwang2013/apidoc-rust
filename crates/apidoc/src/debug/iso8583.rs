//! P2 ISO 8583 执行器（ASCII 报文 + 二进制位图）。
//!
//! 字段定义优先级：`req.opts.fields` > `ep.protocol_opts.fields` > `ep.params`。
//! 位图按 ISO 8583：域 1 = 主位图最高位，域 1 置位表示后随 8 字节次位图。
//! framing：`2`（默认，2 字节大端长度）/ `4` / `none`；ebcdic 留到 P6。
//!
//! shortcut: an/ans 解码去掉尾部空格填充；固定长度域不做截断，超长报错。
//! shortcut: 一条连接只处理一条报文，收到尾随字节即报错。

use super::sockets::parse_host_port;
use super::{guard, DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint, DocParam};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_FIELD: u8 = 128;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Numeric,
    Alpha,
    AlphaNum,
    Binary,
}

#[derive(Clone, Copy)]
enum Framing {
    Fixed(usize),
    Llvar(usize),
    Lllvar(usize),
    Llllvar(usize),
}

#[derive(Clone)]
struct FieldSpec {
    field: u8,
    kind: Kind,
    framing: Framing,
}

pub fn encode_message(mti: &str, fields_json: &str, body: &Value) -> Result<Vec<u8>, String> {
    let specs = parse_fields(fields_json)?;
    encode_with_specs(mti, &specs, body)
}

pub fn decode_message(bytes: &[u8], fields_json: &str) -> Result<Value, String> {
    let specs = parse_fields(fields_json)?;
    decode_with_specs(bytes, &specs)
}

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let encoding = opt(req, ep, "encoding").unwrap_or_else(|| "ascii".into());
    if encoding != "ascii" {
        return Err(format!(
            "iso8583 encoding `{encoding}` is not supported in this build; only ascii (ebcdic is P6)"
        ));
    }
    let specs = match opt(req, ep, "fields") {
        Some(fields) => parse_fields(&fields)?,
        None => match ep {
            Some(ep) if !ep.params.is_empty() => fields_from_params(&ep.params)?,
            _ => {
                return Err("iso8583 requires opts.fields JSON or endpoint params with field/ty".into())
            }
        },
    };

    let body: Value = match req.body.as_ref() {
        Some(payload) => {
            let bytes = payload.decode()?;
            if bytes.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(&bytes)
                    .map_err(|e| format!("invalid iso8583 request JSON: {e}"))?
            }
        }
        None => json!({}),
    };
    let mti = opt(req, ep, "mti")
        .or_else(|| body.get("mti").and_then(Value::as_str).map(str::to_string))
        .ok_or("iso8583 requires opts.mti (e.g. \"0200\")")?;
    let message = encode_with_specs(&mti, &specs, &body)?;
    if message.len() > cfg.max_bytes {
        return Err(format!("iso8583 request exceeds max_bytes ({})", cfg.max_bytes));
    }

    let framing = opt(req, ep, "framing").unwrap_or_else(|| "2".into());
    let width = match framing.as_str() {
        "2" => 2,
        "4" => 4,
        "none" => 0,
        other => {
            return Err(format!(
                "unsupported iso8583 framing `{other}`; use 2, 4 or none"
            ))
        }
    };
    if width == 2 && message.len() > u16::MAX as usize {
        return Err("iso8583 request exceeds 2-byte framing length".into());
    }

    let (host, port) = parse_host_port(&req.target, "tcp")?;
    let addrs = guard::check_host(cfg, &host, port).await?;
    let peer = addrs[0];
    let budget = Duration::from_millis(cfg.timeout_ms.max(1));
    let exchange = async {
        let mut stream = TcpStream::connect(peer)
            .await
            .map_err(|e| format!("iso8583 connect failed: {e}"))?;
        let frame = match width {
            2 => {
                let mut frame = Vec::with_capacity(2 + message.len());
                frame.extend_from_slice(&(message.len() as u16).to_be_bytes());
                frame.extend_from_slice(&message);
                frame
            }
            4 => {
                let mut frame = Vec::with_capacity(4 + message.len());
                frame.extend_from_slice(&(message.len() as u32).to_be_bytes());
                frame.extend_from_slice(&message);
                frame
            }
            _ => message.clone(),
        };
        stream
            .write_all(&frame)
            .await
            .map_err(|e| format!("iso8583 write failed: {e}"))?;
        if width == 0 {
            read_until_parsed(&mut stream, cfg, req, &specs).await
        } else {
            read_framed(&mut stream, width, cfg.max_bytes).await
        }
    };
    let response = tokio::time::timeout(budget, exchange)
        .await
        .map_err(|_| "iso8583 request timed out".to_string())??;
    let value = decode_with_specs(&response, &specs)?;

    let mut headers = BTreeMap::new();
    headers.insert("x-peer".into(), peer.to_string());
    headers.insert(
        "x-mti".into(),
        value["mti"].as_str().unwrap_or_default().to_string(),
    );
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload {
            encoding: "utf8".into(),
            data: serde_json::to_string(&value)
                .map_err(|e| format!("iso8583 response encode failed: {e}"))?,
        },
        frames: Vec::new(),
    })
}

fn opt(req: &DebugRequest, ep: Option<&DocEndpoint>, key: &str) -> Option<String> {
    req.opts
        .get(key)
        .cloned()
        .or_else(|| ep.and_then(|e| e.protocol_opts.get(key).cloned()))
}

fn parse_fields(fields_json: &str) -> Result<Vec<FieldSpec>, String> {
    let items: Vec<Value> = serde_json::from_str(fields_json)
        .map_err(|e| format!("invalid iso8583 fields JSON: {e}"))?;
    let mut specs = Vec::with_capacity(items.len());
    for item in items {
        let field = item
            .get("field")
            .and_then(scalar_string)
            .ok_or("iso8583 field entry requires `field`")?
            .parse::<u8>()
            .map_err(|_| "invalid iso8583 field number".to_string())?;
        validate_field(field)?;
        let framing = parse_framing(
            item.get("type").and_then(Value::as_str).unwrap_or("fixed"),
            parse_len(item.get("len"))?,
        )?;
        let kind = parse_kind(item.get("kind").and_then(Value::as_str).unwrap_or("ans"))?;
        specs.push(FieldSpec { field, kind, framing });
    }
    finish_specs(specs)
}

fn fields_from_params(params: &[DocParam]) -> Result<Vec<FieldSpec>, String> {
    let mut specs = Vec::with_capacity(params.len());
    for param in params {
        let field = param
            .field
            .ok_or_else(|| format!("iso8583 param `{}` requires field", param.name))?
            .parse::<u8>()
            .map_err(|_| format!("invalid iso8583 field number for `{}`", param.name))?;
        validate_field(field)?;
        let ty = param.ty.trim();
        let (framing_name, kind_len) = match ty.split_once(':').or_else(|| ty.split_once('.')) {
            Some((framing, rest)) => (framing.trim(), rest.trim()),
            None => ("fixed", ty),
        };
        let (kind, len) = split_kind_len(kind_len)
            .map_err(|e| format!("iso8583 param `{}`: {e}", param.name))?;
        let framing = parse_framing(framing_name, len)?;
        specs.push(FieldSpec { field, kind, framing });
    }
    finish_specs(specs)
}

fn split_kind_len(text: &str) -> Result<(Kind, usize), String> {
    let digits_at = text
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(text.len());
    let (kind, len) = text.split_at(digits_at);
    let len: usize = len
        .parse()
        .map_err(|_| "ty requires a length, e.g. n6 or llvar:n19".to_string())?;
    if len == 0 || len > 9999 {
        return Err(format!("len {len} is out of range 1..=9999"));
    }
    Ok((parse_kind(kind)?, len))
}

fn validate_field(field: u8) -> Result<(), String> {
    if (2..=MAX_FIELD).contains(&field) {
        Ok(())
    } else {
        Err(format!("iso8583 field {field} is out of range 2..={MAX_FIELD}"))
    }
}

fn scalar_string(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_u64().map(|n| n.to_string()))
}

fn parse_len(value: Option<&Value>) -> Result<usize, String> {
    let len = value
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .ok_or("iso8583 field entry requires numeric `len`")?;
    if len == 0 || len > 9999 {
        return Err(format!("iso8583 field len {len} is out of range 1..=9999"));
    }
    Ok(len as usize)
}

fn parse_framing(ty: &str, len: usize) -> Result<Framing, String> {
    match ty.trim().to_ascii_lowercase().as_str() {
        "fixed" => Ok(Framing::Fixed(len)),
        "llvar" => require_max(len, 99).map(|_| Framing::Llvar(len)),
        "lllvar" => require_max(len, 999).map(|_| Framing::Lllvar(len)),
        "llllvar" => require_max(len, 9999).map(|_| Framing::Llllvar(len)),
        other => Err(format!(
            "unsupported iso8583 field type `{other}`; use fixed/llvar/lllvar/llllvar"
        )),
    }
}

fn require_max(len: usize, max: usize) -> Result<(), String> {
    if len > max {
        Err(format!(
            "iso8583 variable field len {len} exceeds {max} for its framing"
        ))
    } else {
        Ok(())
    }
}

fn parse_kind(raw: &str) -> Result<Kind, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "n" => Ok(Kind::Numeric),
        "an" => Ok(Kind::Alpha),
        "ans" => Ok(Kind::AlphaNum),
        "b" => Ok(Kind::Binary),
        other => Err(format!(
            "unsupported iso8583 field kind `{other}`; use n/an/ans/b"
        )),
    }
}

fn finish_specs(mut specs: Vec<FieldSpec>) -> Result<Vec<FieldSpec>, String> {
    specs.sort_by_key(|s| s.field);
    for pair in specs.windows(2) {
        if pair[0].field == pair[1].field {
            return Err(format!("duplicate iso8583 field {}", pair[0].field));
        }
    }
    for spec in &specs {
        if let (Kind::Binary, Framing::Fixed(len)) = (spec.kind, spec.framing) {
            if len % 2 != 0 {
                return Err(format!(
                    "iso8583 binary field {} fixed len must be even",
                    spec.field
                ));
            }
        }
    }
    Ok(specs)
}

fn encode_with_specs(mti: &str, specs: &[FieldSpec], body: &Value) -> Result<Vec<u8>, String> {
    let values = body_fields(body)?;
    let mti = validate_mti(mti)?;

    let mut primary = [0u8; 8];
    let mut secondary = [0u8; 8];
    let mut has_secondary = false;
    for key in values.keys() {
        let field: u8 = key
            .parse()
            .map_err(|_| format!("invalid iso8583 field number `{key}`"))?;
        if !specs.iter().any(|s| s.field == field) {
            return Err(format!("iso8583 field {field} is set but not configured"));
        }
        if field > 64 {
            has_secondary = true;
        }
    }
    if has_secondary {
        set_bit(&mut primary, 1);
    }
    for spec in specs {
        if !values.contains_key(&spec.field.to_string()) {
            continue;
        }
        if spec.field <= 64 {
            set_bit(&mut primary, spec.field);
        } else {
            set_bit(&mut secondary, spec.field - 64);
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(mti.as_bytes());
    out.extend_from_slice(&primary);
    if has_secondary {
        out.extend_from_slice(&secondary);
    }
    for spec in specs {
        if let Some(value) = values.get(&spec.field.to_string()) {
            encode_field(&mut out, spec, value)?;
        }
    }
    Ok(out)
}

fn decode_with_specs(bytes: &[u8], specs: &[FieldSpec]) -> Result<Value, String> {
    let mut input = bytes;
    let mti = read_ascii(&mut input, 4)?;
    if !mti.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid iso8583 MTI `{mti}`"));
    }
    let mut primary = [0u8; 8];
    primary.copy_from_slice(take(&mut input, 8)?);
    let mut secondary = [0u8; 8];
    let has_secondary = primary[0] & 0x80 != 0;
    if has_secondary {
        secondary.copy_from_slice(take(&mut input, 8)?);
    }

    let mut set = Vec::new();
    for field in 2..=64u8 {
        if bit(&primary, field) {
            set.push(field);
        }
    }
    if has_secondary {
        for field in 65..=MAX_FIELD {
            if bit(&secondary, field - 64) {
                set.push(field);
            }
        }
    }
    for field in &set {
        if !specs.iter().any(|s| s.field == *field) {
            return Err(format!("iso8583 field {field} is set but not configured"));
        }
    }

    let mut fields = Map::new();
    for spec in specs {
        if !set.contains(&spec.field) {
            continue;
        }
        fields.insert(spec.field.to_string(), decode_field(&mut input, spec)?);
    }
    if !input.is_empty() {
        return Err(format!(
            "iso8583: {} trailing byte(s) after last field",
            input.len()
        ));
    }
    Ok(json!({"mti": mti, "fields": Value::Object(fields)}))
}

fn encode_field(out: &mut Vec<u8>, spec: &FieldSpec, value: &Value) -> Result<(), String> {
    let text = scalar_string(value)
        .ok_or_else(|| format!("iso8583 field {} must be a string or number", spec.field))?;
    if spec.kind == Kind::Binary {
        let hex = normalize_hex(&text)?;
        return encode_binary(out, spec, &hex);
    }
    validate_text(spec, &text)?;
    if text.len() > framing_max(spec.framing) {
        return Err(format!(
            "iso8583 field {} is {} chars, max is {}",
            spec.field,
            text.len(),
            framing_max(spec.framing)
        ));
    }
    match spec.framing {
        Framing::Fixed(len) => {
            if spec.kind == Kind::Numeric {
                out.extend(std::iter::repeat(b'0').take(len - text.len()));
                out.extend_from_slice(text.as_bytes());
            } else {
                out.extend_from_slice(text.as_bytes());
                out.extend(std::iter::repeat(b' ').take(len - text.len()));
            }
        }
        framing => {
            let width = prefix_width(framing);
            out.extend_from_slice(format!("{:0width$}", text.len(), width = width).as_bytes());
            out.extend_from_slice(text.as_bytes());
        }
    }
    Ok(())
}

fn decode_field(input: &mut &[u8], spec: &FieldSpec) -> Result<Value, String> {
    let len = match spec.framing {
        Framing::Fixed(len) => len,
        framing => {
            let prefix = take(input, prefix_width(framing))?;
            let text = std::str::from_utf8(prefix)
                .map_err(|_| "iso8583: non-ASCII length prefix")?;
            if !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err("iso8583: invalid length prefix".into());
            }
            text.parse::<usize>()
                .map_err(|_| "iso8583: invalid length prefix")?
        }
    };
    if len > framing_max(spec.framing) {
        return Err(format!(
            "iso8583 field {} length {len} exceeds configured max",
            spec.field
        ));
    }
    match spec.kind {
        Kind::Binary => {
            if len % 2 != 0 {
                return Err(format!(
                    "iso8583 binary field {} has odd digit length {len}",
                    spec.field
                ));
            }
            Ok(json!(hex_upper(take(input, len / 2)?)))
        }
        Kind::Numeric | Kind::Alpha | Kind::AlphaNum => {
            let bytes = take(input, len)?;
            let text = std::str::from_utf8(bytes)
                .map_err(|_| format!("iso8583 field {} is not ASCII", spec.field))?;
            let text = if spec.kind == Kind::Numeric {
                text.to_string()
            } else {
                text.trim_end().to_string()
            };
            Ok(json!(text))
        }
    }
}

fn body_fields(body: &Value) -> Result<&Map<String, Value>, String> {
    match body {
        Value::Object(map) => match map.get("fields") {
            Some(Value::Object(fields)) => Ok(fields),
            Some(_) => Err("iso8583 body.fields must be an object".into()),
            None => Ok(map),
        },
        _ => Err("iso8583 request body must be a JSON object".into()),
    }
}

fn validate_mti(mti: &str) -> Result<&str, String> {
    if mti.len() == 4 && mti.bytes().all(|b| b.is_ascii_digit()) {
        Ok(mti)
    } else {
        Err(format!(
            "invalid iso8583 MTI `{mti}`; expected 4 ASCII digits"
        ))
    }
}

fn validate_text(spec: &FieldSpec, text: &str) -> Result<(), String> {
    match spec.kind {
        Kind::Numeric => {
            if !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("iso8583 field {} requires digits", spec.field));
            }
        }
        Kind::Alpha | Kind::AlphaNum => {
            if !text.is_ascii() {
                return Err(format!("iso8583 field {} requires ASCII text", spec.field));
            }
        }
        Kind::Binary => {}
    }
    Ok(())
}

fn encode_binary(out: &mut Vec<u8>, spec: &FieldSpec, hex: &str) -> Result<(), String> {
    let max = framing_max(spec.framing);
    if hex.len() > max {
        return Err(format!(
            "iso8583 field {} is {} hex digits, max is {max}",
            spec.field,
            hex.len()
        ));
    }
    match spec.framing {
        Framing::Fixed(len) => {
            let padded = format!("{:0>width$}", hex, width = len);
            push_hex_bytes(out, &padded)
        }
        framing => {
            if hex.len() % 2 != 0 {
                return Err(format!(
                    "iso8583 binary field {} requires an even number of hex digits",
                    spec.field
                ));
            }
            let width = prefix_width(framing);
            out.extend_from_slice(format!("{:0width$}", hex.len(), width = width).as_bytes());
            push_hex_bytes(out, hex)
        }
    }
}

fn normalize_hex(text: &str) -> Result<String, String> {
    let hex: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("iso8583 binary field requires hex digits".into());
    }
    Ok(hex.to_ascii_uppercase())
}

fn push_hex_bytes(out: &mut Vec<u8>, hex: &str) -> Result<(), String> {
    for pair in hex.as_bytes().chunks(2) {
        let text = std::str::from_utf8(pair).map_err(|_| "invalid iso8583 hex")?;
        out.push(u8::from_str_radix(text, 16).map_err(|_| "invalid iso8583 hex")?);
    }
    Ok(())
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn framing_max(framing: Framing) -> usize {
    match framing {
        Framing::Fixed(len)
        | Framing::Llvar(len)
        | Framing::Lllvar(len)
        | Framing::Llllvar(len) => len,
    }
}

fn prefix_width(framing: Framing) -> usize {
    match framing {
        Framing::Llvar(_) => 2,
        Framing::Lllvar(_) => 3,
        Framing::Llllvar(_) => 4,
        Framing::Fixed(_) => 0,
    }
}

fn set_bit(bitmap: &mut [u8; 8], field: u8) {
    let idx = (field - 1) as usize;
    bitmap[idx / 8] |= 0x80 >> (idx % 8);
}

fn bit(bitmap: &[u8; 8], field: u8) -> bool {
    let idx = (field - 1) as usize;
    bitmap[idx / 8] & (0x80 >> (idx % 8)) != 0
}

fn take<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8], String> {
    if input.len() < len {
        return Err("incomplete".into());
    }
    let (head, tail) = input.split_at(len);
    *input = tail;
    Ok(head)
}

fn read_ascii(input: &mut &[u8], len: usize) -> Result<String, String> {
    let bytes = take(input, len)?;
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|_| "iso8583: non-ASCII message header".into())
}

async fn read_framed(
    stream: &mut TcpStream,
    width: usize,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut head = vec![0u8; width];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|e| format!("iso8583 read length failed: {e}"))?;
    let len = match width {
        2 => u16::from_be_bytes([head[0], head[1]]) as usize,
        _ => u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize,
    };
    if len > max_bytes {
        return Err(format!(
            "iso8583 response exceeds max_bytes ({max_bytes})"
        ));
    }
    let mut body = vec![0u8; len];
    stream
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("iso8583 read body failed: {e}"))?;
    Ok(body)
}

async fn read_until_parsed(
    stream: &mut TcpStream,
    cfg: &DebugConfig,
    req: &DebugRequest,
    specs: &[FieldSpec],
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + Duration::from_millis(read_ms(cfg, req));
    let mut buf = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut chunk = [0u8; 4096];
        match tokio::time::timeout(remaining, stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > cfg.max_bytes {
                    return Err(format!(
                        "iso8583 response exceeds max_bytes ({})",
                        cfg.max_bytes
                    ));
                }
                match decode_with_specs(&buf, specs) {
                    Ok(_) => return Ok(buf),
                    Err(e) if e == "incomplete" => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(Err(e)) => return Err(format!("iso8583 read failed: {e}")),
            Err(_) => break,
        }
    }
    if buf.is_empty() {
        Err("iso8583 response timed out".into())
    } else {
        Err("incomplete iso8583 response".into())
    }
}

fn read_ms(cfg: &DebugConfig, req: &DebugRequest) -> u64 {
    req.opts
        .get("read_ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(500)
        .clamp(1, cfg.timeout_ms.max(1))
}
