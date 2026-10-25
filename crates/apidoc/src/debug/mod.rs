//! P0 服务端调试执行器（feature `debug-http`）。
//!
//! 浏览器直连不了的协议由适配器代发。这里只放与框架无关的线格式和分发；
//! 每种协议族一个子模块，按 feature gate 编译。安全边界在 `guard`：默认拒绝
//! 内网/回环/链路本地，DNS 解析后逐 IP 校验并固定连接地址。

mod guard;
mod http;
#[cfg(feature = "debug-session")]
pub(crate) mod session;
#[cfg(feature = "debug-session")]
mod ws_client;
#[cfg(feature = "debug-sockets")]
mod sockets;
#[cfg(feature = "debug-grpc")]
mod grpc;
#[cfg(feature = "debug-thrift")]
pub mod thrift;
#[cfg(feature = "debug-dubbo")]
pub mod hessian;
#[cfg(feature = "debug-dubbo")]
mod dubbo;
#[cfg(feature = "debug-iso8583")]
pub mod iso8583;
#[cfg(feature = "debug-mqtt")]
mod mqtt;
#[cfg(feature = "debug-amqp")]
mod amqp;
#[cfg(feature = "debug-kafka")]
mod kafka;
#[cfg(feature = "debug-xmpp")]
mod xmpp;
#[cfg(feature = "debug-sockets")]
mod stomp;
#[cfg(feature = "debug-coap")]
mod coap;
#[cfg(feature = "debug-coap")]
mod lwm2m;
#[cfg(feature = "debug-industrial")]
mod modbus;
#[cfg(feature = "debug-industrial")]
mod dnp3;
#[cfg(feature = "debug-industrial")]
mod iec104;
#[cfg(feature = "debug-industrial")]
mod bacnet;
#[cfg(feature = "debug-industrial")]
mod opcua;
#[cfg(feature = "debug-media")]
pub mod rtp;
#[cfg(feature = "debug-media")]
pub mod rtcp;
#[cfg(feature = "debug-media")]
pub mod rtsp;
#[cfg(feature = "debug-media")]
pub mod sip;
#[cfg(feature = "debug-media")]
pub mod rtmp;
#[cfg(feature = "debug-http3")]
pub mod http3;

use crate::{DebugConfig, DocEndpoint};
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
    /// 服务端已收集的端点引用；schema（字段号/类型）只能从它取，不信任客户端上传。
    #[serde(default)]
    pub endpoint: Option<EndpointRef>,
    /// 单次请求超时；超过 DebugConfig.timeout_ms 时按配置封顶。
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
pub struct EndpointRef {
    pub url: String,
    pub method: String,
    /// 协议 kind；非 HTTP 端点通常共享空 url/GET，必须靠它区分。
    #[serde(default)]
    pub protocol: String,
}

/// 二进制安全的载荷：encoding = utf8 / hex / base64。
#[derive(Debug, Deserialize, Serialize)]
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
    fn digit(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut high: Option<u8> = None;
    for byte in s.bytes().filter(|b| !b.is_ascii_whitespace()) {
        let nibble = digit(byte).ok_or("invalid hex body: non-hex character")?;
        match high.take() {
            Some(h) => out.push((h << 4) | nibble),
            None => high = Some(nibble),
        }
    }
    if high.is_some() {
        return Err("invalid hex body: odd number of digits".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_decode_rejects_non_ascii_without_panic() {
        let payload = Payload {
            encoding: "hex".into(),
            data: "€€".into(),
        };
        assert!(payload.decode().is_err());
        let ok = Payload {
            encoding: "hex".into(),
            data: "00 ff".into(),
        };
        assert_eq!(ok.decode().unwrap(), vec![0, 255]);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Frame {
    pub dir: String,
    pub encoding: String,
    pub data: String,
    pub ts: u64,
}

#[derive(Debug, Serialize)]
pub struct DebugResponse {
    pub status: u16,
    pub ms: u64,
    pub headers: BTreeMap<String, String>,
    pub body: Payload,
    /// gRPC server-streaming 等多帧一元响应；空时省略。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<Frame>,
}

/// 分发到具体协议执行器。P0 只有 HTTP 族；其余协议在后续阶段按 feature 接入。
/// `ep` 是服务端从 `req.endpoint` 解析出的已收集端点，schema 只能从它取。
pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let _ = ep;
    match req.kind.as_str() {
        "http" | "http2" | "http-binary" | "soap" | "graphql" | "jsonrpc" | "xmlrpc"
        | "odata" | "mcp" | "a2a" | "webhook" => http::execute(cfg, req).await,
        #[cfg(feature = "debug-sockets")]
        "tcp" | "udp" | "raw" | "unix" => sockets::execute(cfg, req).await,
        #[cfg(feature = "debug-grpc")]
        "grpc" => grpc::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-thrift")]
        "thrift" => thrift::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-dubbo")]
        "dubbo" | "hessian" => dubbo::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-iso8583")]
        "iso8583" => iso8583::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-mqtt")]
        "mqtt" => mqtt::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-amqp")]
        "amqp" => amqp::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-kafka")]
        "kafka" => kafka::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-xmpp")]
        "xmpp" => xmpp::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-sockets")]
        "stomp" => stomp::execute(cfg, req, ep).await,
        #[cfg(feature = "debug-coap")]
        "coap" => coap::execute(cfg, req).await,
        #[cfg(feature = "debug-coap")]
        "lwm2m" => lwm2m::execute(cfg, req).await,
        #[cfg(feature = "debug-industrial")]
        "modbus" => modbus::execute(cfg, req).await,
        #[cfg(feature = "debug-industrial")]
        "dnp3" => dnp3::execute(cfg, req).await,
        #[cfg(feature = "debug-industrial")]
        "iec104" => iec104::execute(cfg, req).await,
        #[cfg(feature = "debug-industrial")]
        "bacnet" => bacnet::execute(cfg, req).await,
        #[cfg(feature = "debug-industrial")]
        "opcua" => opcua::execute(cfg, req).await,
        #[cfg(feature = "debug-media")]
        "rtp" => rtp::execute(cfg, req).await,
        #[cfg(feature = "debug-media")]
        "rtcp" => rtcp::execute(cfg, req).await,
        #[cfg(feature = "debug-media")]
        "rtsp" => rtsp::execute(cfg, req).await,
        #[cfg(feature = "debug-media")]
        "sip" => sip::execute(cfg, req).await,
        #[cfg(feature = "debug-media")]
        "rtmp" => rtmp::execute(cfg, req).await,
        #[cfg(feature = "debug-http3")]
        "http3" => http3::execute(cfg, req).await,
        other => Err(format!("protocol `{other}` has no server executor in this build")),
    }
}

/// 长连接协议分发：由 `session::run` 的 connect 帧调用。
#[cfg(feature = "debug-session")]
pub(crate) async fn stream_dispatch(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
    frames: tokio::sync::mpsc::Sender<String>,
) -> Result<Box<dyn session::StreamSession>, String> {
    let _ = ep;
    match req.kind.as_str() {
        "websocket" | "ws-binary" => ws_client::connect_stream(cfg, req, frames).await,
        other => Err(format!("protocol `{other}` has no streaming executor in this build")),
    }
}
