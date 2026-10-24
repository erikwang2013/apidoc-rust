//! HTTP/HTTPS 服务端执行器（含 HTTP/2 prior-knowledge）。
//! P0 覆盖 REST/SOAP/GraphQL/JSON-RPC/XML-RPC/OData/MCP/A2A/Webhooks 的服务端兜底；
//! 浏览器直连仍是首选，这里解决 CORS / 内网 / 强制 HTTP/2 三种场景。

use super::{DebugRequest, DebugResponse, Payload};
use crate::debug::guard;
use crate::DebugConfig;
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    let checked = guard::check(cfg, &req.target).await?;
    let timeout_ms = req.timeout_ms.unwrap_or(cfg.timeout_ms).min(cfg.timeout_ms).max(1);
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        // 不跟随重定向：每一跳都可能是新的未校验地址
        .redirect(reqwest::redirect::Policy::none())
        // 不走 HTTP_PROXY/HTTPS_PROXY：否则连接会绕过上面的 IP 校验
        .no_proxy();
    // 把连接钉到已校验的 IP（域名目标）；IP 字面量本身就是校验过的地址。
    if checked.host.parse::<IpAddr>().is_err() {
        if let Some(addr) = checked.addrs.first() {
            builder = builder.resolve(&checked.host, *addr);
        }
    }
    if req.kind == "http2" || req.opts.get("version").is_some_and(|v| v == "2") {
        builder = builder.http2_prior_knowledge();
    }
    let client = builder.build().map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let method_text = req.method.as_deref().unwrap_or("GET");
    let method = reqwest::Method::from_bytes(method_text.as_bytes())
        .map_err(|e| format!("invalid HTTP method `{method_text}`: {e}"))?;
    // CONNECT 会把已校验目标变成隧道，绕过 allow_hosts。
    if method == reqwest::Method::CONNECT {
        return Err("HTTP CONNECT is not allowed in server-side debug".into());
    }
    let mut rb = client.request(method, checked.url);
    for (name, value) in &req.headers {
        rb = rb.header(name.as_str(), value.as_str());
    }
    if let Some(body) = &req.body {
        let bytes = body.decode()?;
        if bytes.len() > cfg.max_bytes {
            return Err(format!("request body exceeds max_bytes ({})", cfg.max_bytes));
        }
        rb = rb.body(bytes);
    }

    let started = Instant::now();
    let mut res = rb.send().await.map_err(|e| format!("request failed: {e}"))?;
    let status = res.status().as_u16();
    let mut headers = BTreeMap::new();
    for (name, value) in res.headers() {
        let name = name.as_str().to_string();
        let value = value.to_str().unwrap_or("<non-utf8>").to_string();
        headers
            .entry(name)
            .and_modify(|v: &mut String| {
                v.push_str(", ");
                v.push_str(&value);
            })
            .or_insert(value);
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| format!("failed to read response: {e}"))? {
        if bytes.len() + chunk.len() > cfg.max_bytes {
            return Err(format!("response exceeds max_bytes ({})", cfg.max_bytes));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(DebugResponse {
        status,
        ms: started.elapsed().as_millis() as u64,
        headers,
        body: Payload::from_bytes(&bytes),
        frames: Vec::new(),
    })
}
