//! P3 STOMP 1.2 over TCP 执行器。
//!
//! 一元语义：CONNECT → CONNECTED → SUBSCRIBE →（有 body 时）SEND →
//! 收集 `messages` 条或到 `duration_ms` → DISCONNECT → 关闭。
//! 帧语义与 P1 `ui.codecs.js` 一致：NUL 分帧、`content-length` 决定二进制
//! body、头转义 `\\` / `\r` / `\n` / `\c`。WS 版由浏览器适配器负责。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::{DebugConfig, DocEndpoint};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_FRAMES: usize = 1000;

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    _ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    if let Some(transport) = req.opts.get("transport") {
        if transport != "tcp" {
            return Err(format!(
                "stomp transport `{transport}` is not supported by the server executor; use tcp"
            ));
        }
    }
    let destination = req
        .opts
        .get("destination")
        .filter(|s| !s.is_empty())
        .ok_or("stomp requires opts.destination")?
        .clone();
    let passcode = req.opts.get("passcode").cloned().unwrap_or_default();
    let (host, port) = parse_target(&req.target)?;
    let addrs = guard::check_host(cfg, &host, port).await?;

    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let stream = tokio::time::timeout(connect_timeout, TcpStream::connect(addrs[0]))
        .await
        .map_err(|_| "stomp connect timed out".to_string())?
        .map_err(|e| format!("stomp connect failed: {e}"))?;
    let _ = stream.set_nodelay(true);
    let (reader, mut writer) = stream.into_split();
    let mut reader = FrameReader::new(reader);

    let mut connect_headers = vec![
        ("accept-version".to_string(), "1.2".to_string()),
        ("host".to_string(), host.clone()),
        ("heart-beat".to_string(), "0,0".to_string()),
    ];
    if let Some(login) = req.opts.get("login") {
        connect_headers.push(("login".to_string(), login.clone()));
    }
    if req.opts.contains_key("passcode") {
        connect_headers.push(("passcode".to_string(), passcode.clone()));
    }
    write_frame(&mut writer, "CONNECT", &connect_headers, None).await?;

    let handshake_deadline = Instant::now() + connect_timeout;
    loop {
        let frame = read_frame_before(&mut reader, cfg, handshake_deadline).await?;
        match frame.command.as_str() {
            "CONNECTED" => break,
            "ERROR" => {
                return Err(redact(
                    &format!("stomp server error: {}", error_text(&frame)),
                    &passcode,
                ))
            }
            _ => {}
        }
    }

    let mut headers = vec![
        ("id".to_string(), "0".to_string()),
        ("destination".to_string(), destination.clone()),
        ("ack".to_string(), "auto".to_string()),
    ];
    write_frame(&mut writer, "SUBSCRIBE", &headers, None).await?;

    if let Some(body) = req.body.as_ref() {
        let bytes = body.decode()?;
        if bytes.len() > cfg.max_bytes {
            return Err(format!(
                "request body exceeds max_bytes ({})",
                cfg.max_bytes
            ));
        }
        // request headers 作为消息头；content-length 由编码器计算。
        for (key, value) in &req.headers {
            if key.eq_ignore_ascii_case("content-length") {
                continue;
            }
            headers.push((key.clone(), value.clone()));
        }
        write_frame(&mut writer, "SEND", &headers, Some(&bytes)).await?;
    }

    let want = opts_usize(req, "messages")
        .unwrap_or(1)
        .clamp(1, MAX_FRAMES);
    let deadline = Instant::now() + Duration::from_millis(duration_ms(cfg, req));
    let mut messages = Vec::new();
    let mut total = 0usize;
    while messages.len() < want {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let frame = match tokio::time::timeout(remaining, read_frame(&mut reader, cfg)).await {
            Err(_) => break,
            Ok(Ok(None)) => break,
            Ok(Ok(Some(frame))) => frame,
            Ok(Err(e)) => return Err(redact(&e, &passcode)),
        };
        match frame.command.as_str() {
            "MESSAGE" => {
                total += frame.body.len();
                if total > cfg.max_bytes {
                    return Err(format!(
                        "stomp messages exceed max_bytes ({})",
                        cfg.max_bytes
                    ));
                }
                let headers: Map<String, Value> = frame
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect();
                messages.push(json!({
                    "destination": frame.headers.get("destination").cloned().unwrap_or_default(),
                    "headers": headers,
                    "payload": Payload::from_bytes(&frame.body),
                }));
            }
            "ERROR" => {
                return Err(redact(
                    &format!("stomp server error: {}", error_text(&frame)),
                    &passcode,
                ))
            }
            // 心跳/未知命令忽略并继续。
            _ => {}
        }
    }

    let _ = tokio::time::timeout(
        Duration::from_millis(100),
        write_frame(&mut writer, "DISCONNECT", &[], None),
    )
    .await;
    // 关闭写端后对端会看到 EOF；读端随函数返回释放。
    let _ = writer.shutdown().await;

    let body = json!({ "messages": messages });
    let mut resp_headers = BTreeMap::new();
    resp_headers.insert("x-messages".to_string(), messages.len().to_string());
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers: resp_headers,
        body: Payload {
            encoding: "utf8".into(),
            data: body.to_string(),
        },
        frames: Vec::new(),
    })
}

fn parse_target(target: &str) -> Result<(String, u16), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("tcp://{target}")
    };
    let url =
        reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid stomp target: {e}"))?;
    match url.scheme() {
        "tcp" | "stomp" => {}
        other => {
            return Err(format!(
                "stomp transport `{other}` is not supported; use tcp://"
            ))
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("stomp credentials must be provided via opts, not the target URL".into());
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    let port = url.port_or_known_default().unwrap_or(61613);
    Ok((host, port))
}

fn duration_ms(cfg: &DebugConfig, req: &DebugRequest) -> u64 {
    opts_u64(req, "duration_ms")
        .unwrap_or(2000)
        .clamp(1, cfg.timeout_ms.max(1))
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_usize(req: &DebugRequest, key: &str) -> Option<usize> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "***")
    }
}

fn error_text(frame: &Frame) -> String {
    let message = frame.headers.get("message").cloned().unwrap_or_default();
    let body = String::from_utf8_lossy(&frame.body);
    match (message.is_empty(), body.trim().is_empty()) {
        (true, true) => "unknown error".into(),
        (true, false) => body.into_owned(),
        (false, true) => message,
        (false, false) => format!("{message}: {body}"),
    }
}

async fn read_frame_before<R: AsyncRead + Unpin>(
    reader: &mut FrameReader<R>,
    cfg: &DebugConfig,
    deadline: Instant,
) -> Result<Frame, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("stomp timed out waiting for CONNECTED".into());
    }
    match tokio::time::timeout(remaining, read_frame(reader, cfg)).await {
        Err(_) => Err("stomp timed out waiting for CONNECTED".into()),
        Ok(Ok(Some(frame))) => Ok(frame),
        Ok(Ok(None)) => Err("stomp connection closed before CONNECTED".into()),
        Ok(Err(e)) => Err(e),
    }
}

async fn write_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    command: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<(), String> {
    let mut out = Vec::new();
    out.extend_from_slice(command.as_bytes());
    out.push(b'\n');
    for (key, value) in headers {
        out.extend_from_slice(escape(key).as_bytes());
        out.push(b':');
        out.extend_from_slice(escape(value).as_bytes());
        out.push(b'\n');
    }
    if let Some(body) = body {
        out.extend_from_slice(format!("content-length:{}\n", body.len()).as_bytes());
        out.push(b'\n');
        out.extend_from_slice(body);
    } else {
        out.push(b'\n');
    }
    out.push(0);
    writer
        .write_all(&out)
        .await
        .map_err(|e| format!("stomp write failed: {e}"))
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            ':' => out.push_str("\\c"),
            other => out.push(other),
        }
    }
    out
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('c') => out.push(':'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

struct Frame {
    command: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// `\0` 分帧的增量读取器：header 行长与单帧 body 都受 max_bytes 约束。
struct FrameReader<R> {
    inner: R,
    buf: Vec<u8>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    fn new(inner: R) -> Self {
        FrameReader {
            inner,
            buf: Vec::new(),
        }
    }

    async fn read_frame(&mut self, cfg: &DebugConfig) -> Result<Option<Frame>, String> {
        let command = loop {
            let Some(line) = self.read_line(cfg.max_bytes).await? else {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                return Err("stomp connection closed mid-frame".into());
            };
            let line = trim_eol(&line);
            if !line.is_empty() {
                break String::from_utf8_lossy(line).into_owned();
            }
            // 帧前心跳/空行。
        };

        let mut headers = BTreeMap::new();
        loop {
            let Some(line) = self.read_line(cfg.max_bytes).await? else {
                return Err("stomp connection closed in headers".into());
            };
            let line = trim_eol(&line);
            if line.is_empty() {
                break;
            }
            let Some(at) = line.iter().position(|b| *b == b':') else {
                continue;
            };
            let key = String::from_utf8_lossy(&line[..at]);
            let value = String::from_utf8_lossy(&line[at + 1..]);
            headers.insert(unescape(&key), unescape(&value));
        }

        let body = match headers.get("content-length") {
            Some(raw) => {
                let len = raw
                    .parse::<usize>()
                    .map_err(|_| format!("invalid stomp content-length `{raw}`"))?;
                if len > cfg.max_bytes {
                    return Err(format!(
                        "stomp frame body exceeds max_bytes ({})",
                        cfg.max_bytes
                    ));
                }
                let body = self.read_exact_n(len).await?;
                let nul = self.read_exact_n(1).await?;
                if nul != [0] {
                    return Err("stomp frame missing NUL terminator".into());
                }
                body
            }
            None => self.read_until_nul(cfg.max_bytes).await?,
        };
        Ok(Some(Frame {
            command,
            headers,
            body,
        }))
    }

    async fn read_line(&mut self, max: usize) -> Result<Option<Vec<u8>>, String> {
        loop {
            if let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
                return Ok(Some(self.buf.drain(..=pos).collect()));
            }
            if self.buf.len() > max {
                return Err(format!("stomp frame header exceeds max_bytes ({max})"));
            }
            if !self.fill().await? {
                return Ok(None);
            }
        }
    }

    async fn read_exact_n(&mut self, len: usize) -> Result<Vec<u8>, String> {
        while self.buf.len() < len {
            if !self.fill().await? {
                return Err("stomp connection closed mid-frame".into());
            }
        }
        Ok(self.buf.drain(..len).collect())
    }

    async fn read_until_nul(&mut self, max: usize) -> Result<Vec<u8>, String> {
        loop {
            if let Some(pos) = self.buf.iter().position(|b| *b == 0) {
                let body: Vec<u8> = self.buf.drain(..pos).collect();
                self.buf.drain(..1);
                return Ok(body);
            }
            if self.buf.len() > max {
                return Err(format!("stomp frame body exceeds max_bytes ({max})"));
            }
            if !self.fill().await? {
                return Err("stomp connection closed mid-frame".into());
            }
        }
    }

    async fn fill(&mut self) -> Result<bool, String> {
        let mut chunk = [0u8; 4096];
        let n = self
            .inner
            .read(&mut chunk)
            .await
            .map_err(|e| format!("stomp read failed: {e}"))?;
        if n == 0 {
            return Ok(false);
        }
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(true)
    }
}

async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut FrameReader<R>,
    cfg: &DebugConfig,
) -> Result<Option<Frame>, String> {
    reader.read_frame(cfg).await
}

fn trim_eol(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}
