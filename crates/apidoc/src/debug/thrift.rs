//! P2 Thrift TBinaryProtocol 执行器（一元）。
//!
//! schema 来自 `ep.params` / `ep.returned`：`field` 是 Thrift field id，`ty` 支持
//! bool/byte/i16/i32/i64/double/string/binary/list<T>/map<K,V>/struct。
//! 传输支持 tcp（默认）与 http。

use super::sockets::parse_host_port;
use super::{guard, DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint, DocParam};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const T_STOP: u8 = 0;
const T_BOOL: u8 = 2;
const T_BYTE: u8 = 3;
const T_DOUBLE: u8 = 4;
const T_I16: u8 = 6;
const T_I32: u8 = 8;
const T_I64: u8 = 10;
const T_STRING: u8 = 11;
const T_STRUCT: u8 = 12;
const T_MAP: u8 = 13;
const T_SET: u8 = 14;
const T_LIST: u8 = 15;

#[derive(Clone)]
enum Spec {
    Bool,
    Byte,
    I16,
    I32,
    I64,
    Double,
    String,
    List(Box<Spec>),
    Map(Box<Spec>, Box<Spec>),
    Struct(Vec<FieldSpec>),
}

#[derive(Clone)]
struct FieldSpec {
    name: String,
    id: i16,
    spec: Spec,
}

impl Spec {
    fn wire(&self) -> u8 {
        match self {
            Spec::Bool => T_BOOL,
            Spec::Byte => T_BYTE,
            Spec::I16 => T_I16,
            Spec::I32 => T_I32,
            Spec::I64 => T_I64,
            Spec::Double => T_DOUBLE,
            Spec::String => T_STRING,
            Spec::List(_) => T_LIST,
            Spec::Map(_, _) => T_MAP,
            Spec::Struct(_) => T_STRUCT,
        }
    }

    fn parse(ty: &str, children: &[DocParam]) -> Result<Spec, String> {
        let ty = ty.trim();
        if let Some(inner) = ty.strip_prefix("list<").and_then(|s| s.strip_suffix('>')) {
            return Ok(Spec::List(Box::new(Spec::parse(inner, &[])?)));
        }
        if let Some(inner) = ty.strip_prefix("map<").and_then(|s| s.strip_suffix('>')) {
            let (key, value) = inner
                .split_once(',')
                .ok_or_else(|| format!("invalid thrift map type `{ty}`"))?;
            return Ok(Spec::Map(
                Box::new(Spec::parse(key, &[])?),
                Box::new(Spec::parse(value, &[])?),
            ));
        }
        match ty {
            "bool" => Ok(Spec::Bool),
            "byte" | "i8" => Ok(Spec::Byte),
            "i16" => Ok(Spec::I16),
            "i32" | "int" => Ok(Spec::I32),
            "i64" | "long" => Ok(Spec::I64),
            "double" | "float" => Ok(Spec::Double),
            "string" | "binary" | "bytes" => Ok(Spec::String),
            "enum" => Ok(Spec::I32),
            "struct" | "object" => {
                let mut fields = Vec::new();
                for child in children {
                    let id = child
                        .field
                        .ok_or_else(|| format!("thrift struct field `{}` requires field", child.name))?
                        .parse::<i16>()
                        .map_err(|_| format!("invalid thrift field id for `{}`", child.name))?;
                    fields.push(FieldSpec {
                        name: child.name.to_string(),
                        id,
                        spec: Spec::parse(child.ty, child.children)?,
                    });
                }
                Ok(Spec::Struct(fields))
            }
            other => Err(format!("unsupported thrift type `{other}`")),
        }
    }
}

pub fn write_message_begin(out: &mut Vec<u8>, name: &str, msg_type: u8, seqid: i32) {
    out.extend_from_slice(&(0x8001_0000u32 | msg_type as u32).to_be_bytes());
    out.extend_from_slice(&(name.len() as i32).to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&seqid.to_be_bytes());
}

pub fn write_field(out: &mut Vec<u8>, ty: u8, id: i16) {
    out.push(ty);
    out.extend_from_slice(&id.to_be_bytes());
}

pub fn read_message_begin(input: &mut &[u8]) -> Result<(String, u8, i32), String> {
    let first = read_i32(input)?;
    let (name, msg_type) = if first < 0 {
        if (first as u32 & 0xffff_0000) != 0x8001_0000 {
            return Err(format!("unsupported thrift version 0x{:08x}", first as u32));
        }
        let len = read_i32(input)?;
        (read_string(input, len)?, (first & 0xff) as u8)
    } else {
        let name = read_string(input, first)?;
        let ty = read_u8(input)?;
        (name, ty)
    };
    let seqid = read_i32(input)?;
    Ok((name, msg_type, seqid))
}

pub fn read_field_begin(input: &mut &[u8]) -> Result<Option<(u8, i16)>, String> {
    let ty = read_u8(input)?;
    if ty == T_STOP {
        return Ok(None);
    }
    let id = read_i16(input)?;
    Ok(Some((ty, id)))
}

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let method = req
        .opts
        .get("method")
        .cloned()
        .or_else(|| ep.and_then(|e| e.protocol_opts.get("method").cloned()))
        .ok_or("thrift requires opts.method")?;
    let frame = build_request(req, ep, &method)?;
    let transport = req.opts.get("transport").map(String::as_str).unwrap_or("tcp");
    let response_bytes = match transport {
        "http" => http_roundtrip(cfg, req, &frame).await?,
        "" | "tcp" => tcp_roundtrip(cfg, req, ep, &frame).await?,
        other => return Err(format!("unsupported thrift transport `{other}`")),
    };
    let value = parse_response(&response_bytes, ep)
        .map_err(|e| format!("thrift response parse failed: {e}"))?
        .ok_or("incomplete thrift response")?;
    let mut headers = BTreeMap::new();
    headers.insert("x-thrift-method".into(), method);
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload {
            encoding: "utf8".into(),
            data: serde_json::to_string(&value)
                .map_err(|e| format!("thrift response encode failed: {e}"))?,
        },
        frames: Vec::new(),
    })
}

fn build_request(
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
    method: &str,
) -> Result<Vec<u8>, String> {
    let body: Value = match req.body.as_ref() {
        Some(payload) => {
            let bytes = payload.decode()?;
            if bytes.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(&bytes)
                    .map_err(|e| format!("invalid thrift request JSON: {e}"))?
            }
        }
        None => json!({}),
    };
    let mut out = Vec::new();
    write_message_begin(&mut out, method, 1, 0);
    if let Some(ep) = ep {
        for param in &ep.params {
            let spec = Spec::parse(param.ty, param.children)?;
            let id = param
                .field
                .ok_or_else(|| format!("thrift param `{}` requires field", param.name))?
                .parse::<i16>()
                .map_err(|_| format!("invalid thrift field id for `{}`", param.name))?;
            if let Some(value) = body.get(param.name) {
                write_field(&mut out, spec.wire(), id);
                write_value(&mut out, &spec, value)?;
            } else if param.required {
                return Err(format!("thrift missing required param `{}`", param.name));
            }
        }
    }
    out.push(T_STOP);
    Ok(out)
}

fn parse_response(bytes: &[u8], ep: Option<&DocEndpoint>) -> Result<Option<Value>, String> {
    let mut input = bytes;
    let (_name, msg_type, _seq) = match read_message_begin(&mut input) {
        Ok(v) => v,
        Err(e) if e == "incomplete" => return Ok(None),
        Err(e) => return Err(e),
    };
    if msg_type == 3 {
        let ex = read_struct_value(&mut input, None)?;
        let msg = ex
            .get("field_1")
            .and_then(Value::as_str)
            .unwrap_or("thrift exception");
        return Err(format!("thrift exception: {msg}"));
    }
    let spec = ep
        .map(|endpoint| {
            let fields: Result<Vec<_>, String> = endpoint
                .returned
                .iter()
                .map(|p| {
                    let id = p
                        .field
                        .ok_or_else(|| format!("thrift returned `{}` requires field", p.name))?
                        .parse::<i16>()
                        .map_err(|_| format!("invalid thrift field id for `{}`", p.name))?;
                    Ok(FieldSpec {
                        name: p.name.to_string(),
                        id,
                        spec: Spec::parse(p.ty, p.children)?,
                    })
                })
                .collect();
            fields.map(Spec::Struct)
        })
        .transpose()?;
    match read_struct_value(&mut input, spec.as_ref()) {
        Ok(value) => Ok(Some(value)),
        Err(e) if e == "incomplete" => Ok(None),
        Err(e) => Err(e),
    }
}

fn write_value(out: &mut Vec<u8>, spec: &Spec, value: &Value) -> Result<(), String> {
    match spec {
        Spec::Bool => out.push(if value.as_bool().ok_or("expected bool")? { 1 } else { 0 }),
        Spec::Byte => out.push(value.as_i64().ok_or("expected i8")? as i8 as u8),
        Spec::I16 => out.extend_from_slice(&(value.as_i64().ok_or("expected i16")? as i16).to_be_bytes()),
        Spec::I32 => out.extend_from_slice(&(value.as_i64().ok_or("expected i32")? as i32).to_be_bytes()),
        Spec::I64 => out.extend_from_slice(&value.as_i64().ok_or("expected i64")?.to_be_bytes()),
        Spec::Double => {
            out.extend_from_slice(&value.as_f64().ok_or("expected double")?.to_bits().to_be_bytes())
        }
        Spec::String => {
            let text = value.as_str().ok_or("expected string")?;
            out.extend_from_slice(&(text.len() as i32).to_be_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        Spec::List(inner) => {
            let items = value.as_array().ok_or("expected array")?;
            out.push(inner.wire());
            out.extend_from_slice(&(items.len() as i32).to_be_bytes());
            for item in items {
                write_value(out, inner, item)?;
            }
        }
        Spec::Map(key, val) => {
            let items = value.as_object().ok_or("expected object")?;
            out.push(key.wire());
            out.push(val.wire());
            out.extend_from_slice(&(items.len() as i32).to_be_bytes());
            for (k, v) in items {
                write_map_key(out, key, k)?;
                write_value(out, val, v)?;
            }
        }
        Spec::Struct(fields) => {
            for field in fields {
                if let Some(v) = value.get(&field.name) {
                    write_field(out, field.spec.wire(), field.id);
                    write_value(out, &field.spec, v)?;
                }
            }
            out.push(T_STOP);
        }
    }
    Ok(())
}

fn write_map_key(out: &mut Vec<u8>, spec: &Spec, key: &str) -> Result<(), String> {
    match spec {
        Spec::String => {
            out.extend_from_slice(&(key.len() as i32).to_be_bytes());
            out.extend_from_slice(key.as_bytes());
        }
        Spec::Bool => out.push(if key == "true" { 1 } else { 0 }),
        Spec::Byte | Spec::I16 | Spec::I32 | Spec::I64 => {
            let n: i64 = key.parse().map_err(|_| format!("invalid numeric map key `{key}`"))?;
            match spec {
                Spec::Byte => out.push(n as i8 as u8),
                Spec::I16 => out.extend_from_slice(&(n as i16).to_be_bytes()),
                Spec::I32 => out.extend_from_slice(&(n as i32).to_be_bytes()),
                _ => out.extend_from_slice(&n.to_be_bytes()),
            }
        }
        _ => return Err("unsupported thrift map key type".into()),
    }
    Ok(())
}

fn read_value(input: &mut &[u8], wire: u8, spec: Option<&Spec>) -> Result<Value, String> {
    match wire {
        T_BOOL => Ok(json!(read_u8(input)? != 0)),
        T_BYTE => Ok(json!(read_u8(input)? as i8)),
        T_I16 => Ok(json!(read_i16(input)?)),
        T_I32 => Ok(json!(read_i32(input)?)),
        T_I64 => Ok(json!(read_i64(input)?)),
        T_DOUBLE => Ok(json!(read_f64(input)?)),
        T_STRING => {
            let len = read_i32(input)?;
            let bytes = take(input, len)?;
            Ok(json!(String::from_utf8_lossy(bytes)))
        }
        T_STRUCT => read_struct_value(input, spec),
        T_LIST | T_SET => read_list_value(input, spec),
        T_MAP => read_map_value(input, spec),
        other => Err(format!("unsupported thrift wire type {other}")),
    }
}

fn read_struct_value(input: &mut &[u8], spec: Option<&Spec>) -> Result<Value, String> {
    let fields: &[FieldSpec] = match spec {
        Some(Spec::Struct(fields)) => fields,
        _ => &[],
    };
    let mut obj = Map::new();
    loop {
        let Some((wire, id)) = read_field_begin(input)? else {
            break;
        };
        let known = fields.iter().find(|f| f.id == id);
        let value = read_value(input, wire, known.map(|f| &f.spec))?;
        let name = known
            .map(|f| f.name.clone())
            .unwrap_or_else(|| format!("field_{id}"));
        obj.insert(name, value);
    }
    Ok(Value::Object(obj))
}

fn read_list_value(input: &mut &[u8], spec: Option<&Spec>) -> Result<Value, String> {
    let elem_wire = read_u8(input)?;
    let len = read_i32(input)?;
    let elem_spec = match spec {
        Some(Spec::List(inner)) => Some(inner.as_ref()),
        _ => None,
    };
    let mut items = Vec::new();
    for _ in 0..len {
        items.push(read_value(input, elem_wire, elem_spec)?);
    }
    Ok(Value::Array(items))
}

fn read_map_value(input: &mut &[u8], spec: Option<&Spec>) -> Result<Value, String> {
    let key_wire = read_u8(input)?;
    let value_wire = read_u8(input)?;
    let len = read_i32(input)?;
    let (key_spec, value_spec) = match spec {
        Some(Spec::Map(k, v)) => (Some(k.as_ref()), Some(v.as_ref())),
        _ => (None, None),
    };
    let mut obj = Map::new();
    for _ in 0..len {
        let key = read_value(input, key_wire, key_spec)?;
        let key = match key {
            Value::String(s) => s,
            other => other.to_string(),
        };
        obj.insert(key, read_value(input, value_wire, value_spec)?);
    }
    Ok(Value::Object(obj))
}

async fn tcp_roundtrip(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
    frame: &[u8],
) -> Result<Vec<u8>, String> {
    let (host, port) = parse_host_port(&req.target, "tcp")?;
    let addrs = guard::check_host(cfg, &host, port).await?;
    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let mut stream = tokio::time::timeout(connect_timeout, TcpStream::connect(addrs[0]))
        .await
        .map_err(|_| "thrift tcp connect timed out".to_string())?
        .map_err(|e| format!("thrift tcp connect failed: {e}"))?;
    stream
        .write_all(frame)
        .await
        .map_err(|e| format!("thrift tcp write failed: {e}"))?;

    let read_ms = opts_u64(req, "read_ms").unwrap_or(500).clamp(1, cfg.timeout_ms.max(1));
    let deadline = Instant::now() + Duration::from_millis(read_ms);
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
                    return Err(format!("response exceeds max_bytes ({})", cfg.max_bytes));
                }
                match parse_response(&buf, ep) {
                    Ok(Some(_)) => return Ok(buf),
                    Ok(None) => continue,
                    Err(e) => return Err(e),
                }
            }
            Ok(Err(e)) => return Err(format!("thrift tcp read failed: {e}")),
            Err(_) => break,
        }
    }
    Ok(buf)
}

async fn http_roundtrip(
    cfg: &DebugConfig,
    req: &DebugRequest,
    frame: &[u8],
) -> Result<Vec<u8>, String> {
    guard::check(cfg, &req.target).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(cfg.timeout_ms.max(1)))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|e| format!("thrift http client failed: {e}"))?;
    let response = client
        .post(&req.target)
        .header("Content-Type", "application/x-thrift")
        .body(frame.to_vec())
        .send()
        .await
        .map_err(|e| format!("thrift http request failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("thrift http status {}", response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("thrift http read failed: {e}"))?;
    if bytes.len() > cfg.max_bytes {
        return Err(format!("response exceeds max_bytes ({})", cfg.max_bytes));
    }
    Ok(bytes.to_vec())
}

fn read_u8(input: &mut &[u8]) -> Result<u8, String> {
    let byte = *input.first().ok_or("incomplete")?;
    *input = &input[1..];
    Ok(byte)
}

fn take<'a>(input: &mut &'a [u8], len: i32) -> Result<&'a [u8], String> {
    if len < 0 {
        return Err(format!("invalid thrift length {len}"));
    }
    let len = len as usize;
    if input.len() < len {
        return Err("incomplete".into());
    }
    let (head, tail) = input.split_at(len);
    *input = tail;
    Ok(head)
}

fn read_i16(input: &mut &[u8]) -> Result<i16, String> {
    let bytes = take(input, 2)?;
    Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_i32(input: &mut &[u8]) -> Result<i32, String> {
    let bytes = take(input, 4)?;
    Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i64(input: &mut &[u8]) -> Result<i64, String> {
    let bytes = take(input, 8)?;
    Ok(i64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_f64(input: &mut &[u8]) -> Result<f64, String> {
    Ok(f64::from_bits(read_i64(input)? as u64))
}

fn read_string(input: &mut &[u8], len: i32) -> Result<String, String> {
    let bytes = take(input, len)?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}
