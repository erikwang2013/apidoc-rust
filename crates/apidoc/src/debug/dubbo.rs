//! P2 Dubbo 2.0.x 执行器（hessian2 序列化，一元调用）。
//!
//! 请求体（JSON）：数组按 opts.param_types 顺序，或对象按 ep.params 顺序取参数。
//! 直连 `opts.target` / req.target 优先；否则用 `opts.registry` 走 Nacos HTTP
//! Open API 选第一个健康 provider。注册中心和 provider 都先过 `guard::check_host`。
//!
//! 帧：16 字节头（magic 0xdabb | request+two-way+hessian2 | status | request id |
//! body 长度），body 依次为 dubbo 版本、service、version、method、参数类型描述、
//! 各参数、attachments map。

use super::hessian;
use super::sockets::parse_host_port;
use super::{guard, DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAGIC: [u8; 2] = [0xda, 0xbb];
const FLAG_REQUEST: u8 = 0x80 | 0x40 | 0x02; // request | two-way | hessian2
const SERIALIZATION_HESSIAN2: u8 = 2;
const STATUS_OK: u8 = 20;
const NACOS_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let service = opt(req, ep, "service").ok_or("dubbo requires opts.service")?;
    let method = opt(req, ep, "method").ok_or("dubbo requires opts.method")?;
    let version = opt(req, ep, "version").unwrap_or_default();
    let types_desc = param_types_desc(opt(req, ep, "param_types").as_deref().unwrap_or(""));
    let args = arguments(req, ep)?;
    if types_desc.is_empty() && !args.is_empty() {
        return Err(format!(
            "dubbo requires opts.param_types ({} argument(s) in body)",
            args.len()
        ));
    }
    let declared = count_descriptors(&types_desc)?;
    if declared != args.len() {
        return Err(format!(
            "dubbo opts.param_types declares {declared} parameter(s) but body has {}",
            args.len()
        ));
    }

    let body = encode_request(&service, &version, &method, &types_desc, &args)?;
    if body.len() > cfg.max_bytes {
        return Err(format!("dubbo request exceeds max_bytes ({})", cfg.max_bytes));
    }
    // 直连优先（opts.target 也覆盖 req.target）；都没有才查注册中心。
    let direct = opt(req, ep, "target")
        .filter(|t| !t.trim().is_empty())
        .or_else(|| Some(req.target.clone()).filter(|t| !t.trim().is_empty()));
    let (target, provider) = match direct {
        Some(target) => (target, None),
        None => {
            let registry =
                opt(req, ep, "registry").ok_or("dubbo requires a target or opts.registry")?;
            let provider = resolve_provider(cfg, &registry).await?;
            (provider.clone(), Some(provider))
        }
    };
    let (host, port) = parse_host_port(&target, "tcp")?;
    let addrs = guard::check_host(cfg, &host, port).await?;
    let peer = addrs[0];
    let budget = Duration::from_millis(cfg.timeout_ms.max(1));
    let response_body = tokio::time::timeout(budget, roundtrip(peer, &body, cfg.max_bytes))
        .await
        .map_err(|_| "dubbo request timed out".to_string())??;
    let value = parse_response(&response_body)?;

    let mut headers = BTreeMap::new();
    headers.insert("x-peer".into(), peer.to_string());
    headers.insert("x-dubbo-service".into(), service);
    headers.insert("x-dubbo-method".into(), method);
    headers.insert("x-dubbo-status".into(), STATUS_OK.to_string());
    if let Some(provider) = provider {
        headers.insert("x-dubbo-provider".into(), provider);
    }
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload {
            encoding: "utf8".into(),
            data: serde_json::to_string(&value)
                .map_err(|e| format!("dubbo response encode failed: {e}"))?,
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

/// `opts.param_types` 是逗号分隔的 Java 描述符，Dubbo body 里存拼接后的单个字符串。
fn param_types_desc(raw: &str) -> String {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

fn arguments(req: &DebugRequest, ep: Option<&DocEndpoint>) -> Result<Vec<Value>, String> {
    let body = match req.body.as_ref() {
        Some(payload) => {
            let bytes = payload.decode()?;
            if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes)
                    .map_err(|e| format!("invalid dubbo request JSON: {e}"))?
            }
        }
        None => Value::Null,
    };
    match body {
        Value::Array(items) => Ok(items),
        Value::Object(map) if map.is_empty() => Ok(Vec::new()),
        Value::Object(map) => object_args(&map, ep),
        Value::Null => Ok(Vec::new()),
        _ => Err("dubbo request body must be a JSON object or array".into()),
    }
}

fn object_args(map: &Map<String, Value>, ep: Option<&DocEndpoint>) -> Result<Vec<Value>, String> {
    let ep = ep.ok_or(
        "dubbo object body requires an endpoint schema (ep.params); send a JSON array instead",
    )?;
    for key in map.keys() {
        if !ep.params.iter().any(|p| p.name == key) {
            return Err(format!("dubbo request body has unknown param `{key}`"));
        }
    }
    // 位置参数：缺失项补 null，保证与 param_types 下标对齐。
    Ok(ep
        .params
        .iter()
        .map(|p| map.get(p.name).cloned().unwrap_or(Value::Null))
        .collect())
}

/// 统计 Java 类型描述符里的参数个数（如 `Ljava/lang/String;I[B` → 3）。
fn count_descriptors(desc: &str) -> Result<usize, String> {
    let bytes = desc.as_bytes();
    let mut i = 0usize;
    let mut count = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] == b'[' {
            i += 1;
        }
        let Some(&b) = bytes.get(i) else {
            return Err(format!("invalid Java type descriptor `{desc}`"));
        };
        match b {
            b'L' => {
                let end = desc[i..]
                    .find(';')
                    .ok_or_else(|| format!("invalid Java type descriptor `{desc}`"))?;
                i += end + 1;
            }
            b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D' => i += 1,
            _ => return Err(format!("invalid Java type descriptor `{desc}` at byte {i}")),
        }
        count += 1;
    }
    Ok(count)
}

fn encode_request(
    service: &str,
    version: &str,
    method: &str,
    types_desc: &str,
    args: &[Value],
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for value in [
        json!("2.0.2"),
        json!(service),
        json!(version),
        json!(method),
        json!(types_desc),
    ] {
        out.extend_from_slice(&hessian::encode(&value)?);
    }
    for arg in args {
        out.extend_from_slice(&hessian::encode(arg)?);
    }
    out.extend_from_slice(&hessian::encode(&json!({}))?); // attachments
    Ok(out)
}

async fn roundtrip(peer: SocketAddr, body: &[u8], max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut frame = Vec::with_capacity(16 + body.len());
    frame.extend_from_slice(&MAGIC);
    frame.push(FLAG_REQUEST);
    frame.push(0);
    frame.extend_from_slice(&0i64.to_be_bytes()); // request id
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(body);

    let mut stream = TcpStream::connect(peer)
        .await
        .map_err(|e| format!("dubbo connect failed: {e}"))?;
    stream
        .write_all(&frame)
        .await
        .map_err(|e| format!("dubbo write failed: {e}"))?;

    let mut head = [0u8; 16];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|e| format!("dubbo read header failed: {e}"))?;
    if head[0..2] != MAGIC {
        return Err("dubbo: invalid response magic".into());
    }
    if head[2] & 0x80 != 0 {
        return Err("dubbo: expected a response frame".into());
    }
    if head[2] & 0x1f != SERIALIZATION_HESSIAN2 {
        return Err(format!(
            "dubbo: unsupported response serialization id {}",
            head[2] & 0x1f
        ));
    }
    let len = u32::from_be_bytes([head[12], head[13], head[14], head[15]]) as usize;
    if len > max_bytes {
        return Err(format!("dubbo response exceeds max_bytes ({max_bytes})"));
    }
    let mut response = vec![0u8; len];
    stream
        .read_exact(&mut response)
        .await
        .map_err(|e| format!("dubbo read body failed: {e}"))?;
    if head[3] != STATUS_OK {
        return Err(format!("dubbo status {}", head[3]));
    }
    Ok(response)
}

fn parse_response(body: &[u8]) -> Result<Value, String> {
    let (&flag, rest) = body.split_first().ok_or("dubbo: empty response body")?;
    let mut input = rest;
    match flag {
        1 | 4 => {
            let value = hessian::decode(&mut input)?;
            if flag == 4 {
                let _attachments = hessian::decode(&mut input)?;
            }
            Ok(value)
        }
        2 | 5 => Ok(Value::Null),
        0 | 3 => {
            let value = hessian::decode(&mut input)?;
            Err(format!("dubbo exception: {}", exception_message(&value)))
        }
        other => Err(format!("dubbo: unknown response flag {other}")),
    }
}

fn exception_message(value: &Value) -> String {
    if let Value::Object(map) = value {
        if let Some(message) = map
            .get("detailMessage")
            .or_else(|| map.get("message"))
            .and_then(Value::as_str)
        {
            return message.to_string();
        }
    }
    value.to_string()
}

/// Nacos 实例列表 → 第一个 `healthy=true` 的 `ip:port`。
/// 注册中心 host 与选出的 provider 后面都会再走一次 `guard::check_host`。
async fn resolve_provider(cfg: &DebugConfig, registry: &str) -> Result<String, String> {
    // shortcut: Nacos 认证（userinfo/accessToken）未支持，接入受保护注册中心时再加。
    let url = reqwest::Url::parse(registry)
        .map_err(|e| format!("invalid dubbo registry URL: {e}"))?;
    if url.scheme() != "nacos" {
        return Err(format!(
            "unsupported dubbo registry scheme `{}` (only nacos:// is supported)",
            url.scheme()
        ));
    }
    let host = url
        .host_str()
        .ok_or("nacos registry URL requires a host")?
        .to_string();
    let port = url.port().ok_or("nacos registry URL requires a port")?;
    let mut service = None;
    let mut group = None;
    let mut namespace = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "service" => service = Some(value.into_owned()),
            "group" => group = Some(value.into_owned()),
            "namespace" => namespace = Some(value.into_owned()),
            _ => {}
        }
    }
    let service = service
        .filter(|s| !s.is_empty())
        .ok_or("nacos registry URL requires service=<name>")?;

    let addrs = guard::check_host(cfg, &host, port).await?;
    let mut api = format!(
        "http://{host}:{port}/nacos/v1/ns/instance/list?serviceName={}",
        encode_query(&service)
    );
    for (key, value) in [("groupName", group), ("namespaceId", namespace)] {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            api.push('&');
            api.push_str(key);
            api.push('=');
            api.push_str(&encode_query(&value));
        }
    }
    let api =
        reqwest::Url::parse(&api).map_err(|e| format!("invalid nacos instance-list URL: {e}"))?;

    let mut builder = reqwest::Client::builder()
        .timeout(NACOS_TIMEOUT)
        // 每一跳都可能是新的未校验地址。
        .redirect(reqwest::redirect::Policy::none())
        // 走系统代理会绕过上面的 IP 校验。
        .no_proxy();
    if host.parse::<IpAddr>().is_err() {
        if let Some(addr) = addrs.first() {
            builder = builder.resolve(&host, *addr);
        }
    }
    let client = builder
        .build()
        .map_err(|e| format!("failed to build nacos client: {e}"))?;
    let mut res = client
        .get(api)
        .send()
        .await
        // without_url：错误里不带注册中心查询参数（service/group/namespace）。
        .map_err(|e| format!("nacos request failed: {}", e.without_url()))?;
    if !res.status().is_success() {
        return Err(format!(
            "nacos registry returned HTTP {}",
            res.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| format!("nacos response read failed: {}", e.without_url()))?
    {
        if bytes.len() + chunk.len() > cfg.max_bytes {
            return Err(format!("nacos response exceeds max_bytes ({})", cfg.max_bytes));
        }
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("invalid nacos response: {e}"))?;
    let hosts = data
        .get("hosts")
        .and_then(Value::as_array)
        .ok_or("nacos response has no hosts array")?;
    for entry in hosts {
        if entry.get("healthy").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let ip = entry
            .get("ip")
            .and_then(Value::as_str)
            .ok_or("nacos host entry is missing ip")?;
        let port = entry
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .ok_or("nacos host entry has an invalid port")?;
        let host = if ip.contains(':') {
            format!("[{ip}]") // IPv6 provider
        } else {
            ip.to_string()
        };
        return Ok(format!("{host}:{port}"));
    }
    Err("nacos registry has no healthy providers".into())
}

/// 查询参数值 percent-encode（保留 RFC 3986 unreserved）；服务名里的 `:` 会变成 `%3A`。
fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
