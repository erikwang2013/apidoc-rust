//! P5 HTTP/3 执行器（quinn + h3）。
//!
//! 只支持 `https://` 目标，TLS 1.3 + ALPN h3，生产路径只用系统根证书；
//! 不提供跳过证书校验的开关。目标先过 `guard::check`，连接固定到已校验 IP。

use super::guard;
use super::{DebugRequest, DebugResponse, Payload};
use crate::DebugConfig;
use bytes::{Buf, Bytes};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String> {
    execute_with_roots(cfg, req, None).await
}

/// 测试注入自签 CA 用；生产分发只走 `execute`，始终使用系统根证书。
#[doc(hidden)]
pub async fn execute_with_roots(
    cfg: &DebugConfig,
    req: &DebugRequest,
    roots: Option<&rustls::RootCertStore>,
) -> Result<DebugResponse, String> {
    let started = Instant::now();
    let checked = guard::check(cfg, &req.target).await?;
    if checked.url.scheme() != "https" {
        return Err(format!(
            "http3 target must use https:// (got `{}`)",
            checked.url.scheme()
        ));
    }
    let addr = *checked
        .addrs
        .first()
        .ok_or("target host resolved to no addresses")?;
    let window = Duration::from_millis(
        req.timeout_ms
            .unwrap_or(cfg.timeout_ms)
            .min(cfg.timeout_ms)
            .max(1),
    );
    let response = tokio::time::timeout(
        window,
        request(cfg, req, &checked.url, addr, &checked.host, roots),
    )
    .await
    .map_err(|_| "http3 request timed out".to_string())??;
    Ok(DebugResponse {
        status: response.status,
        ms: started.elapsed().as_millis() as u64,
        headers: response.headers,
        body: Payload::from_bytes(&response.body),
        frames: Vec::new(),
    })
}

struct HttpResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn root_store(injected: Option<&rustls::RootCertStore>) -> Result<rustls::RootCertStore, String> {
    if let Some(roots) = injected {
        return Ok(roots.clone());
    }
    let native = rustls_native_certs::load_native_certs();
    let mut roots = rustls::RootCertStore::empty();
    for cert in native.certs {
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        return Err(format!(
            "no system root certificates available for HTTP/3 ({} errors loading them)",
            native.errors.len()
        ));
    }
    Ok(roots)
}

async fn request(
    cfg: &DebugConfig,
    req: &DebugRequest,
    url: &reqwest::Url,
    addr: SocketAddr,
    host: &str,
    roots: Option<&rustls::RootCertStore>,
) -> Result<HttpResponse, String> {
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("http3 TLS config failed: {e}"))?
    .with_root_certificates(root_store(roots)?)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|e| format!("http3 TLS config failed: {e}"))?;
    let mut client = quinn::ClientConfig::new(Arc::new(crypto));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        Duration::from_millis(cfg.timeout_ms.max(1))
            .try_into()
            .map_err(|e| format!("invalid HTTP/3 idle timeout: {e}"))?,
    ));
    client.transport_config(Arc::new(transport));

    let bind = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let mut endpoint = quinn::Endpoint::client(
        bind.parse()
            .map_err(|e| format!("http3 bind failed: {e}"))?,
    )
    .map_err(|e| format!("http3 endpoint failed: {e}"))?;
    endpoint.set_default_client_config(client);
    let connection = endpoint
        .connect(addr, host)
        .map_err(|e| format!("http3 connect failed: {e}"))?
        .await
        .map_err(|e| format!("http3 connect failed (certificate/ALPN?): {e}"))?;

    let (mut h3_conn, mut send_request) = h3::client::new(h3_quinn::Connection::new(connection))
        .await
        .map_err(|e| format!("http3 handshake failed: {e}"))?;
    let driver = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| h3_conn.poll_close(cx)).await;
    });

    let method_text = req.method.as_deref().unwrap_or("GET");
    let method = http::Method::from_bytes(method_text.as_bytes())
        .map_err(|e| format!("invalid HTTP method `{method_text}`: {e}"))?;
    let mut builder = http::Request::builder().method(method).uri(url.as_str());
    for (name, value) in &req.headers {
        let name = http::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| format!("invalid HTTP header name `{name}`: {e}"))?;
        let value = http::header::HeaderValue::from_str(value)
            .map_err(|e| format!("invalid HTTP header value: {e}"))?;
        builder = builder.header(name, value);
    }
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
    let request = builder
        .body(())
        .map_err(|e| format!("failed to build HTTP/3 request: {e}"))?;

    let mut stream = send_request
        .send_request(request)
        .await
        .map_err(|e| format!("http3 request failed: {e}"))?;
    if !body.is_empty() {
        stream
            .send_data(Bytes::from(body))
            .await
            .map_err(|e| format!("http3 request body failed: {e}"))?;
    }
    stream
        .finish()
        .await
        .map_err(|e| format!("http3 request finish failed: {e}"))?;
    let response = stream
        .recv_response()
        .await
        .map_err(|e| format!("http3 response failed: {e}"))?;

    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    for (name, value) in response.headers() {
        headers
            .entry(name.as_str().to_string())
            .and_modify(|v: &mut String| {
                v.push_str(", ");
                v.push_str(value.to_str().unwrap_or("<non-utf8>"));
            })
            .or_insert_with(|| value.to_str().unwrap_or("<non-utf8>").to_string());
    }
    let mut out = Vec::new();
    while let Some(chunk) = stream
        .recv_data()
        .await
        .map_err(|e| format!("http3 response body failed: {e}"))?
    {
        let data = chunk.chunk();
        if out.len() + data.len() > cfg.max_bytes {
            return Err(format!("response exceeds max_bytes ({})", cfg.max_bytes));
        }
        out.extend_from_slice(data);
    }
    driver.abort();
    Ok(HttpResponse {
        status,
        headers,
        body: out,
    })
}
