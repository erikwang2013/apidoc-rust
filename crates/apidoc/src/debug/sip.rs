//! P5 SIP 单事务执行器：OPTIONS / REGISTER / INVITE，UDP（默认）或 TCP。
//!
//! 只完成一次请求-响应事务，不做注册续期、媒体协商或重传状态机。响应必须匹配
//! 请求的 Call-ID 与 CSeq；401/407 明确报“不支持认证”，不静默重试。消息文本由
//! 本模块构造，解析交给 `rsip`（严格 RFC 3261 语法）。

use super::rtp::{first_addr, opts_u64, parse_host_port, timeout_ms};
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

const METHODS: [&str; 3] = ["OPTIONS", "REGISTER", "INVITE"];

/// 用 rsip 解析一条 SIP 请求，返回 (method, uri)。集成测试的黄金向量入口。
pub fn parse_request(raw: &str) -> Result<(String, String), String> {
    let request = rsip::Request::try_from(raw).map_err(|e| format!("invalid SIP request: {e}"))?;
    Ok((request.method.to_string(), request.uri.to_string()))
}

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let method = req
        .opts
        .get("method")
        .map(|m| m.to_ascii_uppercase())
        .unwrap_or_else(|| "OPTIONS".into());
    if !METHODS.contains(&method.as_str()) {
        return Err(format!(
            "unsupported SIP method `{method}`; use {} (dialogs/media are out of scope)",
            METHODS.join("/")
        ));
    }
    let transport = req
        .opts
        .get("transport")
        .map(|t| t.to_ascii_lowercase())
        .unwrap_or_else(|| "udp".into());
    if transport != "udp" && transport != "tcp" {
        return Err(format!(
            "unsupported SIP transport `{transport}`; use udp or tcp"
        ));
    }
    let (host, port) = parse_host_port(&req.target, 5060)?;
    let addr = first_addr(cfg, &host, port).await?;
    let window = Duration::from_millis(timeout_ms(cfg, req));

    let response =
        tokio::time::timeout(window, exchange(cfg, req, &method, &host, &transport, addr))
            .await
            .map_err(|_| "SIP transaction timed out".to_string())??;

    if response.status == 401 || response.status == 407 {
        let label = if response.status == 401 {
            "401 Unauthorized"
        } else {
            "407 Proxy Authentication Required"
        };
        return Err(format!(
            "SIP {label}: Digest authentication is required but credentials-based auth \
             is not supported by the debug executor"
        ));
    }
    let body = json!({
        "status": response.status,
        "reason": response.reason,
        "headers": response.headers,
        "body": Payload::from_bytes(&response.body),
    });
    Ok(DebugResponse {
        status: response.status,
        ms: started.elapsed().as_millis() as u64,
        headers: response.headers,
        body: Payload::from_bytes(body.to_string().as_bytes()),
        frames: Vec::new(),
    })
}

struct SipResponse {
    status: u16,
    reason: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn parse_response(raw: &str) -> Result<SipResponse, String> {
    let message = rsip::SipMessage::try_from(raw).map_err(|e| format!("SIP parse failed: {e}"))?;
    let response = match message {
        rsip::SipMessage::Response(response) => response,
        rsip::SipMessage::Request(_) => return Err("expected a SIP response, got a request".into()),
    };
    let status = response.status_code.code();
    let display = response.status_code.to_string();
    let reason = display
        .split_once(' ')
        .map(|(_, r)| r.to_string())
        .unwrap_or_default();
    let mut headers = BTreeMap::new();
    for header in response.headers.iter() {
        let text = header.to_string();
        let Some((name, value)) = text.split_once(':') else {
            continue;
        };
        headers
            .entry(name.trim().to_string())
            .and_modify(|v: &mut String| {
                v.push_str(", ");
                v.push_str(value.trim());
            })
            .or_insert_with(|| value.trim().to_string());
    }
    Ok(SipResponse {
        status,
        reason,
        headers,
        body: response.body,
    })
}

async fn exchange(
    cfg: &DebugConfig,
    req: &DebugRequest,
    method: &str,
    host: &str,
    transport: &str,
    addr: SocketAddr,
) -> Result<SipResponse, String> {
    if transport == "tcp" {
        let mut stream = TcpStream::connect(addr)
            .await
            .map_err(|e| format!("SIP TCP connect failed: {e}"))?;
        let local = stream
            .local_addr()
            .map_err(|e| format!("SIP local_addr failed: {e}"))?;
        let request = build_request(req, method, host, local, transport)?;
        stream
            .write_all(request.text.as_bytes())
            .await
            .map_err(|e| format!("SIP TCP write failed: {e}"))?;
        let mut buffer = Vec::new();
        loop {
            let message = read_tcp_message(&mut stream, &mut buffer, cfg.max_bytes).await?;
            let response = parse_response(&message)?;
            if is_final(&response, &request.call_id, request.cseq, method) {
                return Ok(response);
            }
        }
    }

    let bind = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|e| format!("SIP udp bind failed: {e}"))?;
    socket
        .connect(addr)
        .await
        .map_err(|e| format!("SIP udp connect failed: {e}"))?;
    let local = socket
        .local_addr()
        .map_err(|e| format!("SIP local_addr failed: {e}"))?;
    let request = build_request(req, method, host, local, transport)?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms(cfg, req));
    let mut buf = vec![0u8; 65_535];
    // UDP 无连接：500ms/1s/2s 后退重传，最多 4 次，之后等到总超时。
    for attempt in 0..4u32 {
        if let Err(e) = socket.send(request.text.as_bytes()).await {
            // 对端已关闭端口时内核用 ICMP 回 ConnectionRefused；视为无响应等超时。
            if e.kind() == ErrorKind::ConnectionRefused {
                break;
            }
            return Err(format!("SIP udp send failed: {e}"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = if attempt == 3 {
            remaining
        } else {
            remaining.min(Duration::from_millis(500 << attempt))
        };
        let wait_until = Instant::now() + wait;
        loop {
            let left = wait_until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match tokio::time::timeout(left, socket.recv(&mut buf)).await {
                Ok(Ok(n)) => {
                    let raw = String::from_utf8_lossy(&buf[..n]);
                    let Ok(response) = parse_response(&raw) else {
                        continue;
                    };
                    if is_final(&response, &request.call_id, request.cseq, method) {
                        return Ok(response);
                    }
                }
                Ok(Err(e)) if e.kind() == ErrorKind::ConnectionRefused => break,
                Ok(Err(e)) => return Err(format!("SIP udp recv failed: {e}")),
                Err(_) => break,
            }
        }
    }
    Err(format!(
        "SIP transaction timed out waiting for a final response matching Call-ID {} CSeq {}",
        request.call_id, request.cseq
    ))
}

/// 只有 2xx-6xx 且 Call-ID/CSeq 与请求一致的响应才算事务完成；1xx 继续等。
fn is_final(response: &SipResponse, call_id: &str, cseq: u64, method: &str) -> bool {
    if response.status < 200 {
        return false;
    }
    let call_id_matches =
        header(&response.headers, "call-id").is_some_and(|value| value.trim() == call_id);
    let cseq_matches = header(&response.headers, "cseq").is_some_and(|value| {
        let mut parts = value.split_whitespace();
        let number = parts.next().and_then(|n| n.parse::<u64>().ok());
        number == Some(cseq)
            && parts
                .next()
                .is_none_or(|echoed| echoed.eq_ignore_ascii_case(method))
    });
    call_id_matches && cseq_matches
}

struct RequestText {
    text: String,
    call_id: String,
    cseq: u64,
}

fn build_request(
    req: &DebugRequest,
    method: &str,
    host: &str,
    local: SocketAddr,
    transport: &str,
) -> Result<RequestText, String> {
    let uri = req
        .opts
        .get("uri")
        .cloned()
        .unwrap_or_else(|| format!("sip:{host}"));
    let from = req
        .opts
        .get("from")
        .cloned()
        .unwrap_or_else(|| format!("sip:apidoc@{host}"));
    let to = req.opts.get("to").cloned().unwrap_or_else(|| uri.clone());
    let branch = req
        .opts
        .get("branch")
        .cloned()
        .unwrap_or_else(|| format!("z9hG4bK{}", random_hex(16)));
    let call_id = req
        .opts
        .get("call_id")
        .cloned()
        .unwrap_or_else(|| format!("{}@apidoc", random_hex(16)));
    let cseq = opts_u64(req, "cseq").unwrap_or(1);
    let tag = format!("apidoc{}", random_hex(8));
    let transport_upper = transport.to_ascii_uppercase();
    let body = req
        .body
        .as_ref()
        .map(Payload::decode)
        .transpose()?
        .unwrap_or_default();

    for (name, value) in [
        ("uri", &uri),
        ("from", &from),
        ("to", &to),
        ("branch", &branch),
        ("call_id", &call_id),
    ] {
        if value.contains(['\r', '\n']) {
            return Err(format!("SIP `{name}` must not contain CR/LF"));
        }
    }

    let mut headers: Vec<(String, String)> = vec![
        (
            "Via".into(),
            format!(
                "SIP/2.0/{transport_upper} {}:{};branch={branch};rport",
                local.ip(),
                local.port()
            ),
        ),
        ("Max-Forwards".into(), "70".into()),
        ("From".into(), format!("<{from}>;tag={tag}")),
        ("To".into(), format!("<{to}>")),
        ("Call-ID".into(), call_id.clone()),
        ("CSeq".into(), format!("{cseq} {method}")),
        (
            "Contact".into(),
            format!(
                "<sip:apidoc@{}:{};transport={transport}>",
                local.ip(),
                local.port()
            ),
        ),
    ];
    const MANAGED: [&str; 8] = [
        "via",
        "max-forwards",
        "from",
        "to",
        "call-id",
        "cseq",
        "contact",
        "content-length",
    ];
    let mut has_content_type = false;
    for (name, value) in &req.headers {
        if MANAGED.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        has_content_type |= name.eq_ignore_ascii_case("content-type");
        headers.push((name.clone(), value.clone()));
    }
    if !body.is_empty() && !has_content_type {
        headers.push(("Content-Type".into(), "application/sdp".into()));
    }
    headers.push(("Content-Length".into(), body.len().to_string()));

    let mut text = format!("{method} {uri} SIP/2.0\r\n");
    for (name, value) in &headers {
        if name.is_empty() || name.contains([':', '\r', '\n']) {
            return Err(format!("invalid SIP header name `{name}`"));
        }
        if value.contains(['\r', '\n']) {
            return Err(format!("SIP header `{name}` must not contain CR/LF"));
        }
        text.push_str(name);
        text.push_str(": ");
        text.push_str(value);
        text.push_str("\r\n");
    }
    text.push_str("\r\n");
    if body.len() > MAX_SIP_BODY {
        return Err(format!("SIP body exceeds {MAX_SIP_BODY} bytes"));
    }
    // 二进制 body 用 latin-1 会破坏字节；本执行器仅发送 UTF-8/hex 解码后的原始字节。
    if !body.is_empty() {
        let text_body = String::from_utf8(body.clone())
            .map_err(|_| "SIP body must be valid UTF-8 (SDP/plain text)".to_string())?;
        text.push_str(&text_body);
    }
    Ok(RequestText {
        text,
        call_id,
        cseq,
    })
}

const MAX_SIP_BODY: usize = 65_535;

async fn read_tcp_message(
    stream: &mut TcpStream,
    buffer: &mut Vec<u8>,
    max_bytes: usize,
) -> Result<String, String> {
    let header_end = loop {
        if let Some(pos) = find(buffer, b"\r\n\r\n") {
            break pos;
        }
        if buffer.len() > max_bytes {
            return Err(format!("SIP response exceeds max_bytes ({max_bytes})"));
        }
        let mut chunk = [0u8; 4096];
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("SIP TCP read failed: {e}"))?;
        if n == 0 {
            return Err("SIP TCP connection closed before a complete message".into());
        }
        buffer.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let length = head
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let total = header_end + 4 + length;
    if total > max_bytes {
        return Err(format!("SIP response exceeds max_bytes ({max_bytes})"));
    }
    while buffer.len() < total {
        let mut chunk = [0u8; 4096];
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("SIP TCP read failed: {e}"))?;
        if n == 0 {
            return Err("SIP TCP connection closed before Content-Length was read".into());
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    let rest = buffer.split_off(total);
    let message = String::from_utf8_lossy(buffer).to_string();
    *buffer = rest;
    Ok(message)
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if getrandom::getrandom(&mut buf).is_err() {
        return "0".repeat(bytes * 2);
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
