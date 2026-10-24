//! SSRF 守卫：协议代理的安全边界。
//!
//! 规则（与 DebugConfig 的默认值一起构成 fail-closed）：
//! - 只允许 http/https/ws/wss（ws/wss 供 P2 会话桥复用同一套主机/IP 校验）；
//! - allow_hosts 非空时，host 必须精确命中或命中 `*.suffix`；
//! - 默认拒绝回环/私网/链路本地/ULA/CGNAT/保留段；
//! - 云元数据地址即使 allow_private_networks=true 也永久拒绝；
//! - 域名先解析，逐个 IP 校验，后续连接固定到已校验地址（防 DNS rebinding）。

use crate::DebugConfig;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub struct CheckedTarget {
    pub url: reqwest::Url,
    /// 原始 host（用于把连接钉到已校验 IP，不能是归一化后的版本）。
    pub host: String,
    pub addrs: Vec<SocketAddr>,
}

pub async fn check(cfg: &DebugConfig, target: &str) -> Result<CheckedTarget, String> {
    let url = reqwest::Url::parse(target).map_err(|e| format!("invalid target URL: {e}"))?;
    match url.scheme() {
        "http" | "https" | "ws" | "wss" => {}
        other => {
            return Err(format!(
                "unsupported target scheme `{other}`; only http/https/ws/wss"
            ))
        }
    }
    let host = url.host_str().ok_or("target URL requires a host")?.to_string();
    let port = url.port_or_known_default().ok_or("target URL requires a port")?;
    let addrs = check_host(cfg, &host, port).await?;
    Ok(CheckedTarget { url, host, addrs })
}

/// 供原始 socket（tcp/udp/raw）复用：allowlist + DNS + 逐 IP 校验。
pub(crate) async fn check_host(
    cfg: &DebugConfig,
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, String> {
    if !host_allowed(cfg, host) {
        return Err(format!("target host `{host}` is not in DebugConfig.allow_hosts"));
    }
    let addrs: Vec<SocketAddr> = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        let lookup = tokio::net::lookup_host((host, port));
        let timeout = std::time::Duration::from_millis(cfg.timeout_ms.max(1));
        tokio::time::timeout(timeout, lookup)
            .await
            .map_err(|_| format!("DNS lookup timed out for `{host}`"))?
            .map_err(|e| format!("DNS lookup failed for `{host}`: {e}"))?
            .collect()
    };
    if addrs.is_empty() {
        return Err(format!("target host `{host}` resolved to no addresses"));
    }
    for addr in &addrs {
        check_ip(cfg, addr.ip())?;
    }
    Ok(addrs)
}

fn host_allowed(cfg: &DebugConfig, host: &str) -> bool {
    if cfg.allow_hosts.is_empty() {
        return true;
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    cfg.allow_hosts.iter().any(|entry| {
        let entry = entry.trim().trim_end_matches('.').to_ascii_lowercase();
        match entry.strip_prefix("*.") {
            Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
            None => host == entry,
        }
    })
}

fn check_ip(cfg: &DebugConfig, ip: IpAddr) -> Result<(), String> {
    if is_always_blocked(ip) {
        return Err(format!("target address {ip} is blocked (cloud metadata)"));
    }
    if cfg.allow_private_networks {
        return Ok(());
    }
    if is_private(ip) {
        return Err(format!(
            "target address {ip} is private/loopback/link-local; \
             set DebugConfig.allow_private_networks = true to allow it"
        ));
    }
    Ok(())
}

fn is_always_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => matches!(
            v4.octets(),
            [169, 254, 169, 254] | [169, 254, 170, 2] | [100, 100, 100, 200]
        ),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            // AWS IMDS IPv6: fd00:ec2::254
            (s[0] == 0xfd00 && s[1] == 0x0ec2 && s[2..7].iter().all(|x| *x == 0) && s[7] == 0x0254)
                // to_ipv4 同时覆盖 IPv4-mapped 与已废弃的 IPv4-compatible 形式
                || v6.to_ipv4().is_some_and(|v4| is_always_blocked(IpAddr::V4(v4)))
        }
    }
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                return is_private_v4(v4);
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fc00::/7 unique local
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 link-local
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
        // 100.64.0.0/10 CGNAT
        || (o[0] == 100 && (64..=127).contains(&o[1]))
        // 192.0.0.0/24, 198.18.0.0/15, 240.0.0.0/4, 0.0.0.0/8
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
        || o[0] >= 240
        || o[0] == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(private: bool, hosts: &[&str]) -> DebugConfig {
        DebugConfig {
            allow_private_networks: private,
            allow_hosts: hosts.iter().map(|s| s.to_string()).collect(),
            ..DebugConfig::default()
        }
    }

    #[test]
    fn public_address_is_allowed_by_default() {
        assert!(check_ip(&cfg(false, &[]), "8.8.8.8".parse().unwrap()).is_ok());
        assert!(check_ip(&cfg(false, &[]), "2606:4700:4700::1111".parse().unwrap()).is_ok());
    }

    #[test]
    fn private_ranges_are_denied_by_default() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.1.1", "100.64.0.1", "::1", "fc00::1", "fe80::1"] {
            assert!(check_ip(&cfg(false, &[]), ip.parse().unwrap()).is_err(), "{ip} should be denied");
        }
    }

    #[test]
    fn private_allowed_when_explicit() {
        assert!(check_ip(&cfg(true, &[]), "10.1.2.3".parse().unwrap()).is_ok());
        assert!(check_ip(&cfg(true, &[]), "127.0.0.1".parse().unwrap()).is_ok());
    }

    #[test]
    fn cloud_metadata_is_always_blocked() {
        for ip in ["169.254.169.254", "169.254.170.2", "100.100.100.200", "fd00:ec2::254", "::ffff:169.254.169.254"] {
            assert!(check_ip(&cfg(true, &[]), ip.parse().unwrap()).is_err(), "{ip} must stay blocked");
        }
    }

    #[test]
    fn host_allowlist_supports_exact_and_wildcard() {
        assert!(host_allowed(&cfg(false, &["api.example.com"]), "api.example.com"));
        assert!(!host_allowed(&cfg(false, &["api.example.com"]), "evil.example.com"));
        assert!(host_allowed(&cfg(false, &["*.example.com"]), "api.example.com"));
        assert!(host_allowed(&cfg(false, &["*.example.com"]), "example.com"));
        assert!(!host_allowed(&cfg(false, &["*.example.com"]), "example.com.evil"));
    }

    #[test]
    fn target_scheme_must_be_http_or_https() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert!(rt.block_on(check(&cfg(false, &[]), "file:///etc/passwd")).is_err());
        assert!(rt.block_on(check(&cfg(false, &[]), "ftp://example.com/x")).is_err());
    }
}
