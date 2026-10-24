//! P0 服务端调试执行器（feature `debug-http`）。
//!
//! 浏览器直连不了的协议由适配器代发。这里只放与框架无关的线格式和分发；
//! 每种协议族一个子模块，按 feature gate 编译。安全边界在 `guard`：默认拒绝
//! 内网/回环/链路本地，DNS 解析后逐 IP 校验并固定连接地址。

mod guard;
mod http;

use crate::DebugConfig;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize)]
pub struct DebugRequest {
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<Payload>,
    #[serde(default)]
    pub opts: BTreeMap<String, String>,
    /// 单次请求超时；超过 DebugConfig.timeout_ms 时按配置封顶。
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// 二进制安全的载荷：encoding = utf8 / hex / base64。
#[derive(Deserialize, Serialize)]
pub struct Payload {
    pub encoding: String,
    pub data: String,
}

impl Payload {
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        match self.encoding.as_str() {
            "utf8" | "text" => Ok(self.data.as_bytes().to_vec()),
            "hex" => decode_hex(&self.data),
            "base64" => base64::engine::general_purpose::STANDARD
                .decode(self.data.trim())
                .map_err(|e| format!("invalid base64 body: {e}")),
            other => Err(format!("unsupported body encoding `{other}`; use utf8, hex or base64")),
        }
    }

    /// UTF-8 合法时原样返回，否则 base64（响应体可能是任意字节）。
    pub fn from_bytes(bytes: &[u8]) -> Payload {
        match std::str::from_utf8(bytes) {
            Ok(s) => Payload { encoding: "utf8".into(), data: s.to_string() },
            Err(_) => Payload {
                encoding: "base64".into(),
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            },
        }
    }
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() % 2 != 0 {
        return Err("invalid hex body: odd number of digits".into());
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|_| "invalid hex body".to_string()))
        .collect()
}

#[derive(Serialize)]
pub struct DebugResponse {
    pub status: u16,
    pub ms: u64,
    pub headers: BTreeMap<String, String>,
    pub body: Payload,
}

/// 分发到具体协议执行器。P0 只有 HTTP 族；其余协议在后续阶段按 feature 接入。
pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    match req.kind.as_str() {
        "http" | "http2" | "http-binary" | "soap" | "graphql" | "jsonrpc" | "xmlrpc"
        | "odata" | "mcp" | "a2a" | "webhook" => http::execute(cfg, req).await,
        // HTTP/3 需要 QUIC 执行器，P0 只保留文档/UI 标识；服务端代发在后续阶段接入。
        "http3" => Err("HTTP/3 server executor is not available in this build yet".into()),
        other => Err(format!("protocol `{other}` has no server executor in this build")),
    }
}
