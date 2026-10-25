//! P5 RTMP 信令调试：简单握手 + AMF0 子集 + chunk 基础（fmt 0/1/2/3）。
//!
//! 只建立连接并发送 connect/createStream/publish/play 命令，不转码、不推流。
//! 支持常见 AMF0 类型（number/boolean/string/object/null/ecma array/date），
//! 其余类型按错误返回。

use super::rtp::{first_addr, parse_host_port, timeout_ms};
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const RTMP_VERSION: u8 = 3;
const HANDSHAKE_SIZE: usize = 1536;
const DEFAULT_CHUNK_SIZE: usize = 128;

mod amf;

pub use amf::{decode_all, encode_number, encode_object, encode_string, Amf0};

/// 写一个 fmt 0 消息，载荷按 chunk_size 分块（后续块用 fmt 3 续块头）。
pub fn write_message(
    out: &mut Vec<u8>,
    csid: u8,
    type_id: u8,
    stream_id: u32,
    payload: &[u8],
    chunk_size: usize,
) -> Result<(), String> {
    if !(2..64).contains(&csid) {
        return Err("rtmp chunk stream id must be 2-63".into());
    }
    out.push(csid);
    out.extend_from_slice(&[0, 0, 0]); // timestamp
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]); // 24-bit length
    out.push(type_id);
    out.extend_from_slice(&stream_id.to_le_bytes());
    for (i, chunk) in payload.chunks(chunk_size.max(1)).enumerate() {
        if i > 0 {
            out.push(0xc0 | csid); // fmt 3 continuation
        }
        out.extend_from_slice(chunk);
    }
    Ok(())
}

struct ChunkState {
    length: usize,
    type_id: u8,
    stream_id: u32,
    payload: Vec<u8>,
}

/// 增量 chunk 解析器；处理 fmt 0/1/2/3、扩展 csid 与 Set Chunk Size。
pub struct ChunkReader {
    chunk_size: usize,
    max_bytes: usize,
    states: HashMap<u32, ChunkState>,
}

impl ChunkReader {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            max_bytes,
            states: HashMap::new(),
        }
    }

    /// 返回下一条消息 (type_id, message_stream_id, payload)；协议控制消息被内部消费。
    pub async fn read_message<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
    ) -> Result<(u8, u32, Vec<u8>), String> {
        loop {
            let (type_id, stream_id, payload) = self.read_chunk_message(reader).await?;
            match type_id {
                1 if payload.len() >= 4 => {
                    let size = u32::from_be_bytes(payload[..4].try_into().unwrap());
                    self.chunk_size = (size & 0x7fff_ffff).max(1) as usize;
                }
                // Window Ack Size / Set Peer Bandwidth / User Control：调试客户端无需回应
                4..=6 => {}
                _ => return Ok((type_id, stream_id, payload)),
            }
        }
    }

    async fn read_chunk_message<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
    ) -> Result<(u8, u32, Vec<u8>), String> {
        loop {
            let first = read_u8(reader).await?;
            let fmt = first >> 6;
            let mut csid = (first & 0x3f) as u32;
            if csid == 0 {
                csid = 64 + read_u8(reader).await? as u32;
            } else if csid == 1 {
                let b = read_u8(reader).await? as u32;
                let b2 = read_u8(reader).await? as u32;
                csid = 64 + b + (b2 << 8);
            }

            let mut state = self.states.remove(&csid);
            match fmt {
                0 => {
                    let _timestamp = read_u24(reader).await?;
                    let length = read_u24(reader).await? as usize;
                    let type_id = read_u8(reader).await?;
                    let stream_id = read_u32_le(reader).await?;
                    if state.as_ref().is_some_and(|s| !s.payload.is_empty()) {
                        return Err("rtmp new chunk header while message incomplete".into());
                    }
                    state = Some(ChunkState {
                        length,
                        type_id,
                        stream_id,
                        payload: Vec::new(),
                    });
                }
                1 => {
                    let _delta = read_u24(reader).await?;
                    let length = read_u24(reader).await? as usize;
                    let type_id = read_u8(reader).await?;
                    let prev = state
                        .as_mut()
                        .ok_or("rtmp fmt 1 chunk without previous state")?;
                    if !prev.payload.is_empty() {
                        return Err("rtmp new chunk header while message incomplete".into());
                    }
                    prev.length = length;
                    prev.type_id = type_id;
                }
                2 => {
                    let _delta = read_u24(reader).await?;
                    let prev = state
                        .as_mut()
                        .ok_or("rtmp fmt 2 chunk without previous state")?;
                    if !prev.payload.is_empty() {
                        return Err("rtmp new chunk header while message incomplete".into());
                    }
                }
                3 => {
                    if state.is_none() {
                        return Err("rtmp fmt 3 chunk without previous state".into());
                    }
                }
                _ => unreachable!(),
            }
            let (want, buffered) = {
                let current = state.as_ref().ok_or("rtmp chunk without state")?;
                if current.length > self.max_bytes {
                    return Err(format!(
                        "rtmp message exceeds max_bytes ({})",
                        self.max_bytes
                    ));
                }
                let want = (current.length - current.payload.len()).min(self.chunk_size);
                let buffered = self
                    .states
                    .values()
                    .map(|s| s.payload.len())
                    .sum::<usize>()
                    + current.payload.len();
                (want, buffered)
            };
            if buffered + want > self.max_bytes {
                return Err(format!(
                    "rtmp cross-csid buffered data exceeds max_bytes ({})",
                    self.max_bytes
                ));
            }
            let current = state.as_mut().unwrap();
            let mut data = vec![0u8; want];
            reader
                .read_exact(&mut data)
                .await
                .map_err(|e| format!("rtmp chunk read failed: {e}"))?;
            current.payload.extend_from_slice(&data);
            if current.payload.len() == current.length {
                let type_id = current.type_id;
                let stream_id = current.stream_id;
                let payload = std::mem::take(&mut current.payload);
                return Ok((type_id, stream_id, payload));
            }
            if self.states.len() >= 64 {
                return Err("too many concurrent rtmp chunk streams".into());
            }
            self.states.insert(csid, state.unwrap());
        }
    }
}

async fn read_u8<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u8, String> {
    let mut buf = [0u8; 1];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("rtmp read failed: {e}"))?;
    Ok(buf[0])
}

async fn read_u24<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u32, String> {
    let mut buf = [0u8; 3];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("rtmp read failed: {e}"))?;
    Ok(u32::from_be_bytes([0, buf[0], buf[1], buf[2]]))
}

async fn read_u32_le<R: AsyncRead + Unpin>(reader: &mut R) -> Result<u32, String> {
    let mut buf = [0u8; 4];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("rtmp read failed: {e}"))?;
    Ok(u32::from_le_bytes(buf))
}

/// 简单握手：C0+C1 → S0+S1+S2 → C2=S1。
async fn handshake(stream: &mut TcpStream) -> Result<(), String> {
    let mut c1 = vec![0u8; HANDSHAKE_SIZE];
    if getrandom::getrandom(&mut c1[8..]).is_err() {
        for (i, byte) in c1[8..].iter_mut().enumerate() {
            *byte = i as u8;
        }
    }
    let mut out = Vec::with_capacity(1 + HANDSHAKE_SIZE);
    out.push(RTMP_VERSION);
    out.extend_from_slice(&c1);
    stream
        .write_all(&out)
        .await
        .map_err(|e| format!("rtmp handshake write failed: {e}"))?;

    let mut s0 = [0u8; 1];
    stream
        .read_exact(&mut s0)
        .await
        .map_err(|e| format!("rtmp handshake read failed: {e}"))?;
    if s0[0] != RTMP_VERSION {
        return Err(format!("unsupported RTMP version {} (expected 3)", s0[0]));
    }
    let mut s1 = vec![0u8; HANDSHAKE_SIZE];
    stream
        .read_exact(&mut s1)
        .await
        .map_err(|e| format!("rtmp handshake read failed: {e}"))?;
    let mut s2 = vec![0u8; HANDSHAKE_SIZE];
    stream
        .read_exact(&mut s2)
        .await
        .map_err(|e| format!("rtmp handshake read failed: {e}"))?;
    // 简单握手：C2 回显 S1 即可（不支持 digest 握手）。
    stream
        .write_all(&s1)
        .await
        .map_err(|e| format!("rtmp handshake write failed: {e}"))?;
    Ok(())
}

async fn read_command(
    stream: &mut TcpStream,
    reader: &mut ChunkReader,
) -> Result<Vec<Amf0>, String> {
    loop {
        let (type_id, _stream_id, payload) = reader.read_message(stream).await?;
        if type_id == 20 {
            return decode_all(&payload);
        }
    }
}

fn command_name(values: &[Amf0]) -> Option<&str> {
    match values.first() {
        Some(Amf0::String(name)) => Some(name),
        _ => None,
    }
}

fn transaction_id(values: &[Amf0]) -> Option<f64> {
    match values.get(1) {
        Some(Amf0::Number(n)) => Some(*n),
        _ => None,
    }
}

fn amf_error(values: &[Amf0]) -> String {
    let mut code = "unknown".to_string();
    let mut description = String::new();
    for value in values {
        if let Amf0::Object(fields) = value {
            if let Some(Amf0::String(c)) = fields.get("code") {
                code = c.clone();
            }
            if let Some(Amf0::String(d)) = fields.get("description") {
                description = d.clone();
            }
        }
    }
    format!("rtmp server error {code}: {description}")
}

async fn expect_result(
    stream: &mut TcpStream,
    reader: &mut ChunkReader,
    transaction: f64,
) -> Result<Vec<Amf0>, String> {
    loop {
        let values = read_command(stream, reader).await?;
        match command_name(&values) {
            Some("_error") => return Err(amf_error(&values)),
            Some("_result") if transaction_id(&values) == Some(transaction) => return Ok(values),
            _ => continue,
        }
    }
}

async fn send_command(
    stream: &mut TcpStream,
    csid: u8,
    stream_id: u32,
    values: &[Amf0],
) -> Result<(), String> {
    let mut payload = Vec::new();
    for value in values {
        value.write(&mut payload);
    }
    let mut out = Vec::new();
    write_message(&mut out, csid, 20, stream_id, &payload, DEFAULT_CHUNK_SIZE)?;
    stream
        .write_all(&out)
        .await
        .map_err(|e| format!("rtmp write failed: {e}"))
}

async fn run(
    cfg: &DebugConfig,
    req: &DebugRequest,
    addr: std::net::SocketAddr,
    host: &str,
    operation: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("rtmp connect failed: {e}"))?;
    handshake(&mut stream).await?;
    let mut reader = ChunkReader::new(cfg.max_bytes);

    let app = req
        .opts
        .get("app")
        .cloned()
        .unwrap_or_else(|| "live".into());
    let mut props = BTreeMap::new();
    props.insert("app".to_string(), Amf0::String(app.clone()));
    props.insert("type".to_string(), Amf0::String("nonprivate".into()));
    props.insert(
        "flashVer".to_string(),
        Amf0::String("FMLE/3.0 (compatible; apidoc)".into()),
    );
    props.insert(
        "tcUrl".to_string(),
        Amf0::String(format!("rtmp://{host}/{app}")),
    );
    send_command(
        &mut stream,
        3,
        0,
        &[
            Amf0::String("connect".into()),
            Amf0::Number(1.0),
            Amf0::Object(props),
        ],
    )
    .await?;
    expect_result(&mut stream, &mut reader, 1.0).await?;

    if operation == "connect" {
        return Ok((None, None));
    }

    let stream_name = req
        .opts
        .get("stream")
        .cloned()
        .ok_or("rtmp publish/play requires opts.stream")?;
    send_command(
        &mut stream,
        3,
        0,
        &[
            Amf0::String("createStream".into()),
            Amf0::Number(2.0),
            Amf0::Null,
        ],
    )
    .await?;
    let result = expect_result(&mut stream, &mut reader, 2.0).await?;
    let stream_id = match result.get(3) {
        Some(Amf0::Number(id)) => *id as u32,
        _ => 1,
    };

    let mut command = vec![
        Amf0::String(operation.into()),
        Amf0::Number(0.0),
        Amf0::Null,
        Amf0::String(stream_name.clone()),
    ];
    if operation == "publish" {
        command.push(Amf0::String("live".into()));
    }
    send_command(&mut stream, 8, stream_id, &command).await?;

    // publish/play 的 onStatus 不是所有服务器都回；在 duration_ms 内尽力读取。
    let wait = req
        .opts
        .get("duration_ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1000)
        .min(cfg.timeout_ms.max(1));
    let deadline = Instant::now() + Duration::from_millis(wait);
    let mut status = None;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match tokio::time::timeout(left, read_command(&mut stream, &mut reader)).await {
            Ok(Ok(values)) => {
                if command_name(&values) == Some("onStatus") {
                    let code = values.iter().find_map(|v| match v {
                        Amf0::Object(fields) => match fields.get("code") {
                            Some(Amf0::String(code)) => Some(code.clone()),
                            _ => None,
                        },
                        _ => None,
                    });
                    if let Some(level) = values.iter().find_map(|v| match v {
                        Amf0::Object(fields) => match fields.get("level") {
                            Some(Amf0::String(level)) => Some(level.clone()),
                            _ => None,
                        },
                        _ => None,
                    }) {
                        if level == "error" {
                            return Err(amf_error(&values));
                        }
                    }
                    status = code;
                    break;
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(_) => break,
        }
    }
    Ok((Some(stream_name), status))
}

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = parse_host_port(&req.target, 1935)?;
    let addr = first_addr(cfg, &host, port).await?;
    let operation = req
        .opts
        .get("operation")
        .map(|o| o.to_ascii_lowercase())
        .unwrap_or_else(|| "connect".into());
    if !["connect", "publish", "play"].contains(&operation.as_str()) {
        return Err(format!(
            "unsupported rtmp operation `{operation}`; use connect/publish/play"
        ));
    }
    let window = Duration::from_millis(timeout_ms(cfg, req));
    let (stream_name, status) =
        tokio::time::timeout(window, run(cfg, req, addr, &host, &operation))
            .await
            .map_err(|_| "rtmp session timed out".to_string())??;
    let body = json!({
        "connected": true,
        "operation": operation,
        "stream": stream_name,
        "status": status,
    });
    let mut headers = BTreeMap::new();
    headers.insert("x-peer".to_string(), addr.to_string());
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload::from_bytes(body.to_string().as_bytes()),
        frames: Vec::new(),
    })
}
