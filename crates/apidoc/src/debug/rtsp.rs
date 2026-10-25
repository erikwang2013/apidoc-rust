//! P5 RTSP 信令客户端：OPTIONS / DESCRIBE / SETUP / PLAY / PAUSE / TEARDOWN。
//!
//! 只发控制信令并读取响应，不拉流、不转码。只支持无认证的 RTSP；401/407 直接
//! 返回明确错误，不静默重试。SETUP 默认使用 TCP interleaved，避免额外开辟 UDP 端口。

use super::rtp::{first_addr, opts_u64, parse_host_port, timeout_ms};
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const METHODS: [&str; 6] = ["OPTIONS", "DESCRIBE", "SETUP", "PLAY", "PAUSE", "TEARDOWN"];

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let url = request_url(req);
    if !url.starts_with("rtsp://") {
        return Err("rtsp target must use the rtsp:// scheme".into());
    }
    if url.contains(['\r', '\n', ' ']) {
        return Err("rtsp URL must not contain whitespace or CR/LF".into());
    }
    let (host, port) = parse_host_port(&url, 554)?;
    let addr = first_addr(cfg, &host, port).await?;

    let method = req
        .opts
        .get("method")
        .map(|m| m.to_ascii_uppercase())
        .unwrap_or_else(|| "OPTIONS".into());
    if !METHODS.contains(&method.as_str()) {
        return Err(format!(
            "unsupported rtsp method `{method}`; use {}",
            METHODS.join("/")
        ));
    }
    let cseq = opts_u64(req, "cseq").unwrap_or(1);
    let body = req
        .body
        .as_ref()
        .map(Payload::decode)
        .transpose()?
        .unwrap_or_default();
    if body.len() > cfg.max_bytes {
        return Err(format!(
            "request body exceeds max_bytes ({})",
            cfg.max_bytes
        ));
    }
    let request = build_request(req, &method, &url, cseq, &body)?;

    let window = Duration::from_millis(timeout_ms(cfg, req));
    let (status, reason, headers, body) =
        tokio::time::timeout(window, exchange(&addr, &request, cfg.max_bytes))
            .await
            .map_err(|_| "rtsp request timed out".to_string())??;

    if status == 401 {
        return Err(auth_error("401 Unauthorized", &headers, "WWW-Authenticate"));
    }
    if status == 407 {
        return Err(auth_error(
            "407 Proxy Authentication Required",
            &headers,
            "Proxy-Authenticate",
        ));
    }
    let response_cseq = header(&headers, "cseq").unwrap_or_default();
    if response_cseq != cseq.to_string() {
        return Err(format!(
            "rtsp response CSeq `{response_cseq}` does not match request CSeq {cseq}"
        ));
    }
    Ok(DebugResponse {
        status,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload::from_bytes(&body),
        frames: Vec::new(),
    })
    .map(|mut res| {
        res.headers.insert("x-reason".into(), reason);
        res
    })
}

fn request_url(req: &DebugRequest) -> String {
    match req.opts.get("url") {
        Some(url) => url.clone(),
        None if req.target.contains("://") => req.target.clone(),
        None => format!("rtsp://{}", req.target),
    }
}

fn build_request(
    req: &DebugRequest,
    method: &str,
    url: &str,
    cseq: u64,
    body: &[u8],
) -> Result<Vec<u8>, String> {
    let mut headers: Vec<(String, String)> = vec![
        ("CSeq".into(), cseq.to_string()),
        ("User-Agent".into(), "apidoc-rust".into()),
    ];
    if let Some(session) = req.opts.get("session") {
        headers.push(("Session".into(), session.clone()));
    }
    if method == "SETUP" {
        let transport = req
            .opts
            .get("transport")
            .cloned()
            .unwrap_or_else(|| "RTP/AVP/TCP;interleaved=0-1".into());
        headers.push(("Transport".into(), transport));
    }
    for (name, value) in &req.headers {
        // CSeq/Content-Length 由执行器管理，避免重复或注入不一致的值。
        if name.eq_ignore_ascii_case("cseq")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("user-agent")
        {
            continue;
        }
        headers.push((name.clone(), value.clone()));
    }
    if !body.is_empty()
        && !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("Content-Type".into(), "application/sdp".into()));
    }
    headers.push(("Content-Length".into(), body.len().to_string()));

    let mut out = format!("{method} {url} RTSP/1.0\r\n");
    for (name, value) in &headers {
        if name.is_empty() || name.contains([':', '\r', '\n']) {
            return Err(format!("invalid rtsp header name `{name}`"));
        }
        if value.contains(['\r', '\n']) {
            return Err(format!("rtsp header `{name}` must not contain CR/LF"));
        }
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    Ok(bytes)
}

async fn exchange(
    addr: &std::net::SocketAddr,
    request: &[u8],
    max_bytes: usize,
) -> Result<(u16, String, BTreeMap<String, String>, Vec<u8>), String> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("rtsp connect failed: {e}"))?;
    stream
        .write_all(request)
        .await
        .map_err(|e| format!("rtsp write failed: {e}"))?;

    let mut buf = Vec::new();
    let header_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > max_bytes {
            return Err(format!(
                "rtsp response headers exceed max_bytes ({max_bytes})"
            ));
        }
        let mut chunk = [0u8; 4096];
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("rtsp read failed: {e}"))?;
        if n == 0 {
            return Err("rtsp connection closed before response headers".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("empty rtsp response")?;
    let mut parts = status_line.splitn(3, ' ');
    if !parts.next().unwrap_or_default().starts_with("RTSP/") {
        return Err(format!("invalid rtsp status line `{status_line}`"));
    }
    let status: u16 = parts
        .next()
        .unwrap_or_default()
        .parse()
        .map_err(|_| format!("invalid rtsp status line `{status_line}`"))?;
    let reason = parts.next().unwrap_or_default().to_string();
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        headers
            .entry(name.to_string())
            .and_modify(|v: &mut String| {
                v.push_str(", ");
                v.push_str(value);
            })
            .or_insert_with(|| value.to_string());
    }

    let length = header(&headers, "content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if length > max_bytes {
        return Err(format!(
            "rtsp response body exceeds max_bytes ({max_bytes})"
        ));
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < length {
        let mut chunk = [0u8; 4096];
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("rtsp read failed: {e}"))?;
        if n == 0 {
            return Err(format!(
                "rtsp connection closed before Content-Length ({length}) was read"
            ));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok((status, reason, headers, body))
}

fn auth_error(status: &str, headers: &BTreeMap<String, String>, header_name: &str) -> String {
    let scheme = header(headers, header_name)
        .and_then(|v| v.split_whitespace().next())
        .unwrap_or("Digest/Basic");
    format!(
        "RTSP {status}: {scheme} authentication is required but credentials-based auth \
         is not supported by the debug executor"
    )
}

pub fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
