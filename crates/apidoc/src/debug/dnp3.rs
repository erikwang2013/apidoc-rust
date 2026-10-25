//! DNP3（IEEE 1815）只读最小集：链路层 + 传输层 + Class 0..3 / 范围读。
//!
//! Spike 结论：`dnp3` 1.7.0 虽满足 MSRV 1.75，但 `license-file` 是非商业/非生产
//! 的专有协议，不能作为本 crate 的依赖；因此手写客户端子集。
//! 覆盖：CRC-16/DNP、链路头、未确认用户数据、Integrity Poll（Class 0）与
//! Class 1/2/3、按 group/variation/start/stop 的读；常见静态点映射为 JSON。
//! 不做：事件确认、选择/操作、时间同步、文件传输、安全认证、多帧重组上限之外的分片。
//! 所有等待受 `duration_ms`（封顶 `DebugConfig.timeout_ms`）与帧/字节上限约束。

use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[path = "dnp3_codec.rs"]
mod codec;

use codec::{encode_link_frame, parse_objects, read_link_frame};

const DEFAULT_PORT: u16 = 20_000;
const MAX_FRAMES: usize = 1000;

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port) = parse_target(&req.target)?;
    let addr = *super::guard::check_host(cfg, &host, port)
        .await?
        .first()
        .ok_or("target resolved to no addresses")?;
    let link_address = opts_u16(req, "link_address").unwrap_or(1);
    let master_address = opts_u16(req, "source_address").unwrap_or(0);
    if link_address == master_address {
        return Err("dnp3 link_address and source_address must differ".into());
    }
    let duration = opts_u64(req, "duration_ms")
        .unwrap_or(3_000)
        .clamp(1, cfg.timeout_ms.max(1));
    let operation = req.opts.get("operation").map(String::as_str).unwrap_or("integrity");

    let app_payload = read_request(operation, req)?;
    let mut app = vec![0xC0, 0x01]; // FIR+FIN，Function Code = Read
    app.extend_from_slice(&app_payload);
    let mut user_data = vec![0xC0]; // 传输层 FIR+FIN，SEQ=0
    user_data.extend_from_slice(&app);
    let request = encode_link_frame(0xC4, link_address, master_address, &user_data)?;

    let connect_timeout = Duration::from_millis(cfg.timeout_ms.max(1));
    let mut stream = tokio::time::timeout(connect_timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| "dnp3 connect timed out".to_string())?
        .map_err(|e| format!("dnp3 connect failed: {e}"))?;
    stream
        .write_all(&request)
        .await
        .map_err(|e| format!("dnp3 write failed: {e}"))?;

    let deadline = Instant::now() + Duration::from_millis(duration);
    let points = collect_points(cfg, &mut stream, deadline, link_address, master_address).await?;
    if points.is_empty() {
        return Err(format!("dnp3 timed out after {duration}ms without a data response"));
    }
    let bytes = serde_json::to_vec(&json!({ "points": points }))
        .map_err(|e| format!("encode response: {e}"))?;
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers: BTreeMap::from([("x-peer".into(), addr.to_string())]),
        body: Payload::from_bytes(&bytes),
        frames: Vec::new(),
    })
}

fn read_request(operation: &str, req: &DebugRequest) -> Result<Vec<u8>, String> {
    match operation {
        "integrity" | "class0" => Ok(vec![0x3C, 0x01, 0x06]),
        "class1" => Ok(vec![0x3C, 0x02, 0x06]),
        "class2" => Ok(vec![0x3C, 0x03, 0x06]),
        "class3" => Ok(vec![0x3C, 0x04, 0x06]),
        "read" => {
            let group = opts_u8(req, "group").unwrap_or(30);
            let variation = opts_u8(req, "variation").unwrap_or(1);
            let start = opts_u32(req, "start").unwrap_or(0);
            let stop = opts_u32(req, "stop").unwrap_or(start);
            if start > stop {
                return Err("dnp3 start must be <= stop".into());
            }
            if stop > 0xFFFF {
                // 只支持 8/16 位索引范围；更大范围对调试没有实际意义。
                return Err("dnp3 stop must be <= 65535".into());
            }
            let mut out = vec![group, variation];
            if stop <= 0xFF {
                out.extend_from_slice(&[0x00, start as u8, stop as u8]);
            } else {
                out.push(0x01);
                out.extend_from_slice(&(start as u16).to_le_bytes());
                out.extend_from_slice(&(stop as u16).to_le_bytes());
            }
            Ok(out)
        }
        other => Err(format!(
            "unsupported dnp3 operation `{other}`; use integrity, class0, class1, class2, class3 or read"
        )),
    }
}

async fn collect_points(
    cfg: &DebugConfig,
    stream: &mut TcpStream,
    deadline: Instant,
    link_address: u16,
    master_address: u16,
) -> Result<Vec<Value>, String> {
    let mut points = Vec::new();
    let mut app = Vec::new();
    let mut total_bytes = 0usize;
    let mut frames = 0usize;
    while frames < MAX_FRAMES {
        let Some(frame) = read_link_frame(stream, deadline).await? else { break };
        total_bytes += frame.data.len() + 10;
        if total_bytes > cfg.max_bytes {
            return Err(format!("dnp3 response exceeds max_bytes ({})", cfg.max_bytes));
        }
        // 只接受目标为 master、来源为 outstation 的用户数据帧。
        if frame.ctrl & 0xC0 != 0x40 || frame.dst != master_address || frame.src != link_address {
            continue;
        }
        let Some((&transport, payload)) = frame.data.split_first() else { continue };
        if transport & 0x80 != 0 {
            app.clear();
        }
        app.extend_from_slice(payload);
        if transport & 0x40 == 0 {
            continue; // 分片未结束
        }
        if app.len() < 2 {
            app.clear();
            continue;
        }
        let function = app[1];
        if function != 0x81 {
            app.clear(); // Confirm/Unsolicited 等非读响应，跳过。
            continue;
        }
        points.extend(parse_objects(&app[2..])?);
        app.clear();
        frames += 1;
    }
    Ok(points)
}

fn parse_target(target: &str) -> Result<(String, u16), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("dnp3://{target}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid target `{target}`: {e}"))?;
    match url.scheme() {
        "dnp3" | "tcp" => {}
        other => return Err(format!("unsupported dnp3 target scheme `{other}`")),
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    Ok((host, url.port().unwrap_or(DEFAULT_PORT)))
}

fn opts_u8(req: &DebugRequest, key: &str) -> Option<u8> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_u16(req: &DebugRequest, key: &str) -> Option<u16> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_u32(req: &DebugRequest, key: &str) -> Option<u32> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity_payload_is_class0_read() {
        let req = DebugRequest {
            kind: "dnp3".into(),
            target: "127.0.0.1:1".into(),
            method: None,
            headers: BTreeMap::new(),
            body: None,
            opts: BTreeMap::new(),
            endpoint: None,
            timeout_ms: None,
        };
        assert_eq!(read_request("integrity", &req).unwrap(), vec![0x3C, 0x01, 0x06]);
        assert_eq!(read_request("class2", &req).unwrap(), vec![0x3C, 0x03, 0x06]);
    }
}
