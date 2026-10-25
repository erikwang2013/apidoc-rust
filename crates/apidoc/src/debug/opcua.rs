//! OPC UA Binary 最小客户端：SecurityPolicy None（浏览/读/写）。
//!
//! Spike 结论：`opcua` 0.12.0 强制依赖 openssl/openssl-sys（C 链接），且未声明
//! MSRV，违反项目“纯 Rust、Rust 1.82”约束；因此手写二进制协议子集。
//! 覆盖：HEL/ACK、OpenSecureChannel(None)、CreateSession、ActivateSession
//! （匿名或用户名令牌，明文密码仅用于 None 策略）、Browse、Read、Write。
//! 不做：加密/签名通道、订阅、历史读、多分块响应（收到 'C' 块直接报错）、CloseSession。
//! 凭据只进请求，不出现在错误信息或响应里。

use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;

#[path = "opcua_codec.rs"]
mod codec;
#[path = "opcua_wire.rs"]
mod wire;

use codec::{
    check_response_header, decode_data_value, decode_localized_text, decode_node_id, skip_node_id,
    status_label, Dec,
};
use wire::{
    encode_hel, encode_msg, encode_opn, encode_variant, put_bytes, put_extension_object, put_f64,
    put_i32, put_i64, put_localized_text, put_node_id, put_node_id_string, put_request_header,
    put_string, put_u16, put_u32, put_u8, read_message, Message,
};

const DEFAULT_PORT: u16 = 4840;
const NONE_POLICY: &str = "http://opcfoundation.org/UA/SecurityPolicy#None";
const ANONYMOUS_TOKEN: u32 = 321;
const USERNAME_TOKEN: u32 = 324;
const OPEN_CHANNEL: u32 = 446;
const CREATE_SESSION: u32 = 461;
const ACTIVATE_SESSION: u32 = 467;
const BROWSE: u32 = 527;
const READ: u32 = 631;
const WRITE: u32 = 673;

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let (host, port, endpoint) = parse_target(&req.target)?;
    let addr = *super::guard::check_host(cfg, &host, port)
        .await?
        .first()
        .ok_or("target resolved to no addresses")?;
    let operation = req.opts.get("operation").map(String::as_str).unwrap_or("read");
    if !matches!(operation, "read" | "browse" | "write") {
        return Err(format!(
            "unsupported opcua operation `{operation}`; use browse, read or write"
        ));
    }
    if operation == "write" && !req.opts.contains_key("value") {
        return Err("opcua `value` is required for write".into());
    }
    let node_id = req.opts.get("node_id").map(String::as_str).unwrap_or("i=85");
    let attribute = opts_u32(req, "attribute").unwrap_or(13); // 13 = Value

    let mut client = Client::connect(addr, endpoint, cfg).await?;
    client
        .create_session(
            req.opts.get("username").map(String::as_str),
            req.opts.get("password").map(String::as_str),
            req.opts.get("user_token_policy").map(String::as_str).unwrap_or("username"),
        )
        .await?;

    let body = match operation {
        "read" => json!({ "value": client.read(node_id, attribute).await?, "status": "Good" }),
        "browse" => json!({ "references": client.browse(node_id).await?, "status": "Good" }),
        "write" => {
            let value = req
                .opts
                .get("value")
                .ok_or("opcua `value` is required for write")?;
            client.write(node_id, attribute, value).await?;
            json!({ "status": "Good" })
        }
        other => {
            return Err(format!(
                "unsupported opcua operation `{other}`; use browse, read or write"
            ))
        }
    };
    let bytes = serde_json::to_vec(&body).map_err(|e| format!("encode response: {e}"))?;
    Ok(DebugResponse {
        status: 0,
        ms: started.elapsed().as_millis() as u64,
        headers: BTreeMap::from([
            ("x-peer".into(), addr.to_string()),
            ("x-opcua-status".into(), "Good".into()),
        ]),
        body: Payload::from_bytes(&bytes),
        frames: Vec::new(),
    })
}

struct Client {
    stream: TcpStream,
    channel: u32,
    token: u32,
    seq: u32,
    request: u32,
    auth: Vec<u8>,
    endpoint_url: String,
    deadline: Instant,
    max_bytes: usize,
}

impl Client {
    async fn connect(
        addr: SocketAddr,
        endpoint_url: String,
        cfg: &DebugConfig,
    ) -> Result<Client, String> {
        let deadline = Instant::now() + Duration::from_millis(cfg.timeout_ms.max(1));
        let stream = tokio::time::timeout(
            Duration::from_millis(cfg.timeout_ms.max(1)),
            TcpStream::connect(addr),
        )
        .await
        .map_err(|_| "opcua connect timed out".to_string())?
        .map_err(|e| format!("opcua connect failed: {e}"))?;
        let mut client = Client {
            stream,
            channel: 0,
            token: 0,
            seq: 1,
            request: 1,
            auth: vec![0x00, 0x00],
            endpoint_url,
            deadline,
            max_bytes: cfg.max_bytes,
        };
        let hel = encode_hel(
            &client.endpoint_url,
            cfg.max_bytes.min(u32::MAX as usize) as u32,
        );
        client.send_bytes(&hel).await?;
        let ack = client.read_message().await?;
        if &ack.kind != b"ACK" {
            return Err(format!(
                "opcua expected ACK, got {}",
                String::from_utf8_lossy(&ack.kind)
            ));
        }
        client.open_secure_channel().await?;
        Ok(client)
    }

    async fn open_secure_channel(&mut self) -> Result<(), String> {
        let mut body = Vec::new();
        put_node_id(&mut body, OPEN_CHANNEL);
        put_request_header(&mut body, &[0x00, 0x00], 1);
        put_u32(&mut body, 0); // clientProtocolVersion
        put_u32(&mut body, 0); // requestType = Issue
        put_u32(&mut body, 1); // securityMode = None
        put_bytes(&mut body, None); // clientNonce
        put_u32(&mut body, 3_600_000); // requestedLifetime
        let seq = self.next_seq();
        let request = self.next_request();
        let message = encode_opn(self.channel, NONE_POLICY, seq, request, &body);
        self.send_bytes(&message).await?;
        let response = self.read_message().await?;
        if &response.kind != b"OPN" {
            return Err(format!(
                "opcua expected OPN, got {}",
                String::from_utf8_lossy(&response.kind)
            ));
        }
        if let Some(policy) = response.policy.as_deref() {
            if policy != NONE_POLICY {
                return Err(format!(
                    "opcua server selected security policy `{policy}`; only None is supported"
                ));
            }
        }
        self.channel = response.channel;
        let mut dec = Dec::new(&response.body);
        skip_node_id(&mut dec)?; // OpenSecureChannelResponse
        check_response_header(&mut dec)?;
        let _server_version = dec.u32()?;
        let _security_token_channel = dec.u32()?;
        self.token = dec.u32()?;
        Ok(())
    }

    async fn create_session(
        &mut self,
        username: Option<&str>,
        password: Option<&str>,
        token_policy: &str,
    ) -> Result<(), String> {
        let mut body = Vec::new();
        put_node_id(&mut body, CREATE_SESSION);
        put_request_header(&mut body, &self.auth, 2);
        put_string(&mut body, "urn:apidoc:debug");
        put_string(&mut body, "urn:apidoc");
        put_localized_text(&mut body, "apidoc-debug");
        put_u32(&mut body, 0); // ApplicationType = Client
        put_bytes(&mut body, None); // gatewayServerUri
        put_bytes(&mut body, None); // discoveryProfileUri
        put_i32(&mut body, -1); // discoveryUrls
        put_bytes(&mut body, None); // serverUri
        put_string(&mut body, &self.endpoint_url);
        put_string(&mut body, "apidoc-debug");
        put_bytes(&mut body, None); // clientNonce
        put_bytes(&mut body, None); // clientCertificate
        put_f64(&mut body, 600_000.0);
        put_u32(&mut body, 0); // maxResponseMessageSize
        let response = self.roundtrip(b"MSG", &body).await?;
        let mut dec = Dec::new(&response);
        skip_node_id(&mut dec)?;
        check_response_header(&mut dec)?;
        skip_node_id(&mut dec)?; // sessionId
        let start = dec.pos;
        skip_node_id(&mut dec)?; // authenticationToken
        self.auth = dec.data[start..dec.pos].to_vec();
        self.activate_session(username, password, token_policy).await
    }

    async fn activate_session(
        &mut self,
        username: Option<&str>,
        password: Option<&str>,
        token_policy: &str,
    ) -> Result<(), String> {
        let mut body = Vec::new();
        put_node_id(&mut body, ACTIVATE_SESSION);
        put_request_header(&mut body, &self.auth, 3);
        put_bytes(&mut body, None); // clientSignature.algorithm
        put_bytes(&mut body, None); // clientSignature.signature
        put_i32(&mut body, -1); // clientSoftwareCertificates
        put_i32(&mut body, -1); // localeIds
        match username {
            Some(user) => put_extension_object(&mut body, USERNAME_TOKEN, |out| {
                put_string(out, token_policy);
                put_string(out, user);
                put_bytes(out, Some(password.unwrap_or("").as_bytes()));
                put_bytes(out, None);
            }),
            None => {
                put_extension_object(&mut body, ANONYMOUS_TOKEN, |out| put_string(out, "anonymous"));
            }
        }
        put_bytes(&mut body, None); // userTokenSignature.algorithm
        put_bytes(&mut body, None); // userTokenSignature.signature
        let response = self.roundtrip(b"MSG", &body).await?;
        let mut dec = Dec::new(&response);
        skip_node_id(&mut dec)?; // ActivateSessionResponse
        check_response_header(&mut dec)?;
        Ok(())
    }

    async fn read(&mut self, node_id: &str, attribute: u32) -> Result<Value, String> {
        let mut body = Vec::new();
        put_node_id(&mut body, READ);
        put_request_header(&mut body, &self.auth, 4);
        put_f64(&mut body, 0.0); // maxAge
        put_u32(&mut body, 0); // timestampsToReturn = Source
        put_i32(&mut body, 1);
        put_node_id_string(&mut body, node_id)?;
        put_u32(&mut body, attribute);
        put_bytes(&mut body, None); // indexRange
        put_u16(&mut body, 0);
        put_bytes(&mut body, None); // dataEncoding.name
        let response = self.roundtrip(b"MSG", &body).await?;
        let mut dec = Dec::new(&response);
        skip_node_id(&mut dec)?; // ReadResponse
        check_response_header(&mut dec)?;
        let count = dec.i32()?;
        if count != 1 {
            return Err(format!("opcua read returned {count} results (expected 1)"));
        }
        decode_data_value(&mut dec)
    }

    async fn browse(&mut self, node_id: &str) -> Result<Value, String> {
        let mut body = Vec::new();
        put_node_id(&mut body, BROWSE);
        put_request_header(&mut body, &self.auth, 5);
        put_node_id(&mut body, 0); // viewId null
        put_i64(&mut body, 0); // timestamp
        put_u32(&mut body, 0); // viewVersion
        put_u32(&mut body, 0); // requestedMaxReferencesPerNode
        put_i32(&mut body, 1);
        put_node_id_string(&mut body, node_id)?;
        put_u32(&mut body, 0); // browseDirection = Forward
        put_node_id(&mut body, 0); // referenceTypeId = null (all)
        put_u8(&mut body, 1); // includeSubtypes
        put_u32(&mut body, 0); // nodeClassMask
        put_u32(&mut body, 0x3F); // resultMask
        let response = self.roundtrip(b"MSG", &body).await?;
        let mut dec = Dec::new(&response);
        skip_node_id(&mut dec)?; // BrowseResponse
        check_response_header(&mut dec)?;
        let count = dec.i32()?;
        if count != 1 {
            return Err(format!("opcua browse returned {count} results (expected 1)"));
        }
        let status = dec.u32()?;
        if status != 0 {
            return Err(format!("opcua browse status {}", status_label(status)));
        }
        let _continuation = dec.byte_string()?;
        let references = dec.i32()?;
        if references < 0 {
            return Ok(json!([]));
        }
        let mut out = Vec::new();
        for _ in 0..references.min(10_000) {
            skip_node_id(&mut dec)?; // referenceTypeId
            let _is_forward = dec.u8()?;
            let target = decode_node_id(&mut dec)?;
            let namespace = dec.u16()?;
            let name = dec.string()?.unwrap_or_default();
            let display = decode_localized_text(&mut dec)?;
            let node_class = dec.u32()?;
            skip_node_id(&mut dec)?; // typeDefinition
            out.push(json!({
                "node_id": target,
                "browse_name": format!("ns={namespace};{name}"),
                "display_name": display,
                "node_class": node_class,
            }));
        }
        Ok(json!(out))
    }

    async fn write(&mut self, node_id: &str, attribute: u32, value: &str) -> Result<(), String> {
        let mut body = Vec::new();
        put_node_id(&mut body, WRITE);
        put_request_header(&mut body, &self.auth, 6);
        put_i32(&mut body, 1);
        put_node_id_string(&mut body, node_id)?;
        put_u32(&mut body, attribute);
        put_bytes(&mut body, None); // indexRange
        body.push(0x01); // DataValue encoding mask: value only
        encode_variant(&mut body, value);
        let response = self.roundtrip(b"MSG", &body).await?;
        let mut dec = Dec::new(&response);
        skip_node_id(&mut dec)?; // WriteResponse
        check_response_header(&mut dec)?;
        let count = dec.i32()?;
        if count != 1 {
            return Err(format!("opcua write returned {count} results (expected 1)"));
        }
        let status = dec.u32()?;
        if status != 0 {
            return Err(format!("opcua write status {}", status_label(status)));
        }
        Ok(())
    }

    fn next_seq(&mut self) -> u32 {
        let value = self.seq;
        self.seq += 1;
        value
    }

    fn next_request(&mut self) -> u32 {
        let value = self.request;
        self.request += 1;
        value
    }

    async fn roundtrip(&mut self, kind: &[u8; 3], body: &[u8]) -> Result<Vec<u8>, String> {
        let seq = self.next_seq();
        let request = self.next_request();
        let message = encode_msg(kind, self.channel, self.token, seq, request, body);
        self.send_bytes(&message).await?;
        let response = self.read_message().await?;
        if &response.kind != kind {
            return Err(format!(
                "opcua expected {}, got {}",
                String::from_utf8_lossy(kind),
                String::from_utf8_lossy(&response.kind)
            ));
        }
        Ok(response.body)
    }

    async fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("opcua request timed out".into());
        }
        use tokio::io::AsyncWriteExt;
        tokio::time::timeout(remaining, self.stream.write_all(bytes))
            .await
            .map_err(|_| "opcua write timed out".to_string())?
            .map_err(|e| format!("opcua write failed: {e}"))
    }

    async fn read_message(&mut self) -> Result<Message, String> {
        read_message(&mut self.stream, self.deadline, self.max_bytes).await
    }
}

fn parse_target(target: &str) -> Result<(String, u16, String), String> {
    let with_scheme = if target.contains("://") {
        target.to_string()
    } else {
        format!("opc.tcp://{target}")
    };
    let url =
        reqwest::Url::parse(&with_scheme).map_err(|e| format!("invalid target `{target}`: {e}"))?;
    if url.scheme() != "opc.tcp" {
        return Err(format!("unsupported opcua target scheme `{}`", url.scheme()));
    }
    let host = url.host_str().ok_or("target requires a host")?.to_string();
    let port = url.port().unwrap_or(DEFAULT_PORT);
    Ok((host.clone(), port, format!("opc.tcp://{host}:{port}")))
}

fn opts_u32(req: &DebugRequest, key: &str) -> Option<u32> {
    req.opts.get(key).and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_defaults_to_4840() {
        assert_eq!(
            parse_target("host").unwrap(),
            ("host".into(), 4840, "opc.tcp://host:4840".into())
        );
        assert_eq!(
            parse_target("opc.tcp://host:4841").unwrap(),
            ("host".into(), 4841, "opc.tcp://host:4841".into())
        );
        assert!(parse_target("https://host").is_err());
    }
}
