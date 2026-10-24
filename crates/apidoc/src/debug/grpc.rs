//! P2 gRPC 执行器：一元 + 服务端流。
//!
//! schema 来源：`opts.descriptor`（base64 FileDescriptorSet）或服务端反射。
//! 请求/响应走 prost-reflect 的 DynamicMessage；字段号/类型以描述符为准，
//! `ep.params[].field` 只用于文档展示，不参与 wire 编码。
//! client/bidi streaming 留到 P6。

use super::{DebugRequest, DebugResponse, Frame, Payload};
use crate::debug::guard;
use crate::{DebugConfig, DocEndpoint};
use base64::Engine;
use prost::Message as _;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor};
use serde::de::DeserializeSeed;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::codegen::http::uri::PathAndQuery;
use tonic::transport::{Channel, Endpoint};

pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let service_name = req
        .opts
        .get("service")
        .cloned()
        .or_else(|| ep.and_then(|e| e.protocol_opts.get("service").cloned()))
        .ok_or("grpc requires opts.service")?;
    let method_name = req
        .opts
        .get("method")
        .cloned()
        .or_else(|| ep.and_then(|e| e.protocol_opts.get("method").cloned()))
        .ok_or("grpc requires opts.method")?;
    let descriptor_b64 = req
        .opts
        .get("descriptor")
        .or_else(|| ep.and_then(|e| e.protocol_opts.get("descriptor")));

    let pool = load_pool(cfg, req, &service_name, descriptor_b64).await?;
    let service = pool
        .get_service_by_name(&service_name)
        .ok_or_else(|| format!("grpc service `{service_name}` not found in descriptor"))?;
    let method = service
        .methods()
        .find(|m| m.name() == method_name)
        .ok_or_else(|| format!("grpc method `{method_name}` not found on `{service_name}`"))?;
    if method.is_client_streaming() {
        return Err("grpc client/bidi streaming is not supported yet".into());
    }

    let channel = connect(cfg, &req.target).await?;
    let input = method.input();
    let output = method.output();
    let message = build_request(req, &input)?;
    let path = PathAndQuery::from_maybe_shared(format!("/{service_name}/{method_name}"))
        .map_err(|e| format!("invalid grpc path: {e}"))?;
    let mut client = tonic::client::Grpc::new(channel);
    client
        .ready()
        .await
        .map_err(|e| format!("grpc client not ready: {e}"))?;
    let codec = DynamicCodec { output };

    if method.is_server_streaming() {
        let response = client
            .server_streaming(tonic::Request::new(message), path, codec)
            .await
            .map_err(status_err)?;
        let mut stream = response.into_inner();
        let max_frames = opts_usize(req, "max_frames").unwrap_or(100).min(1000);
        let duration = Duration::from_millis(
            opts_u64(req, "duration_ms")
                .unwrap_or(cfg.timeout_ms)
                .min(cfg.timeout_ms.max(1)),
        );
        let deadline = Instant::now() + duration;
        let mut frames = Vec::new();
        while frames.len() < max_frames {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, stream.message()).await {
                Ok(Ok(Some(item))) => {
                    let data = serde_json::to_string(&item)
                        .map_err(|e| format!("grpc response encode failed: {e}"))?;
                    frames.push(Frame {
                        dir: "in".into(),
                        encoding: "utf8".into(),
                        data,
                        ts: now_ms(),
                    });
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(status_err(e)),
                Err(_) => break,
            }
        }
        let body = if frames.len() == 1 {
            frames[0].data.clone()
        } else {
            serde_json::json!({ "frames": frames.len() }).to_string()
        };
        Ok(DebugResponse {
            status: 0,
            ms: started.elapsed().as_millis() as u64,
            headers: grpc_headers(),
            body: Payload {
                encoding: "utf8".into(),
                data: body,
            },
            frames,
        })
    } else {
        let response = client
            .unary(tonic::Request::new(message), path, codec)
            .await
            .map_err(status_err)?;
        let message = response.into_inner();
        let data = serde_json::to_string(&message)
            .map_err(|e| format!("grpc response encode failed: {e}"))?;
        Ok(DebugResponse {
            status: 0,
            ms: started.elapsed().as_millis() as u64,
            headers: grpc_headers(),
            body: Payload {
                encoding: "utf8".into(),
                data,
            },
            frames: Vec::new(),
        })
    }
}

async fn connect(cfg: &DebugConfig, target: &str) -> Result<Channel, String> {
    // shortcut: 与 WS 相同，guard 校验后 tonic 会再解析一次 DNS（窄 TOCTOU）。
    guard::check(cfg, target).await?;
    Endpoint::from_shared(target.to_string())
        .map_err(|e| format!("invalid grpc target: {e}"))?
        .connect()
        .await
        .map_err(|e| format!("grpc connect failed: {e}"))
}

async fn load_pool(
    cfg: &DebugConfig,
    req: &DebugRequest,
    service: &str,
    descriptor_b64: Option<&String>,
) -> Result<DescriptorPool, String> {
    if let Some(encoded) = descriptor_b64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|e| format!("invalid grpc descriptor base64: {e}"))?;
        return DescriptorPool::decode(bytes.as_slice())
            .map_err(|e| format!("invalid grpc descriptor: {e}"));
    }

    let channel = connect(cfg, &req.target).await?;
    use tonic_reflection::pb::v1::{
        server_reflection_client::ServerReflectionClient,
        server_reflection_request::MessageRequest,
        server_reflection_response::MessageResponse, ServerReflectionRequest,
    };
    let mut client = ServerReflectionClient::new(channel);
    let request = ServerReflectionRequest {
        host: String::new(),
        message_request: Some(MessageRequest::FileContainingSymbol(service.to_string())),
    };
    let mut stream = client
        .server_reflection_info(tokio_stream::iter(vec![request]))
        .await
        .map_err(|e| format!("grpc reflection failed: {e}"))?
        .into_inner();
    let response = stream
        .message()
        .await
        .map_err(|e| format!("grpc reflection read failed: {e}"))?
        .ok_or("grpc reflection returned no response")?;
    let files = match response.message_response {
        Some(MessageResponse::FileDescriptorResponse(f)) => f.file_descriptor_proto,
        _ => return Err("grpc reflection returned an unexpected response".into()),
    };
    let mut set = prost_reflect::prost_types::FileDescriptorSet::default();
    for bytes in files {
        set.file.push(
            prost::Message::decode(bytes.as_slice())
                .map_err(|e| format!("grpc reflection descriptor decode failed: {e}"))?,
        );
    }
    DescriptorPool::from_file_descriptor_set(set)
        .map_err(|e| format!("grpc reflection descriptor invalid: {e}"))
}

fn build_request(req: &DebugRequest, input: &MessageDescriptor) -> Result<DynamicMessage, String> {
    let body = req
        .body
        .as_ref()
        .map(Payload::decode)
        .transpose()?
        .unwrap_or_default();
    if body.is_empty() {
        return Ok(DynamicMessage::new(input.clone()));
    }
    let text = String::from_utf8(body).map_err(|_| "grpc request body must be UTF-8 JSON")?;
    let mut de = serde_json::Deserializer::from_str(&text);
    let message = input
        .clone()
        .deserialize(&mut de)
        .map_err(|e| format!("invalid grpc request JSON: {e}"))?;
    de.end()
        .map_err(|e| format!("trailing JSON in grpc request: {e}"))?;
    Ok(message)
}

fn status_err(status: tonic::Status) -> String {
    format!("grpc status {:?}: {}", status.code(), status.message())
}

fn grpc_headers() -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    headers.insert("x-grpc-status".into(), "0".into());
    headers
}

fn opts_u64(req: &DebugRequest, key: &str) -> Option<u64> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn opts_usize(req: &DebugRequest, key: &str) -> Option<usize> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- tonic 自定义 codec：DynamicMessage 没有 Default，不能直接用 ProstCodec ----

struct DynamicEncoder;

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = tonic::Status;

    fn encode(&mut self, item: DynamicMessage, dst: &mut EncodeBuf<'_>) -> Result<(), tonic::Status> {
        item.encode(dst)
            .map_err(|e| tonic::Status::internal(format!("grpc encode failed: {e}")))
    }
}

struct DynamicDecoder {
    output: MessageDescriptor,
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = tonic::Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<DynamicMessage>, tonic::Status> {
        DynamicMessage::decode(self.output.clone(), src)
            .map(Some)
            .map_err(|e| tonic::Status::internal(format!("grpc decode failed: {e}")))
    }
}

struct DynamicCodec {
    output: MessageDescriptor,
}

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> DynamicEncoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> DynamicDecoder {
        DynamicDecoder {
            output: self.output.clone(),
        }
    }
}
