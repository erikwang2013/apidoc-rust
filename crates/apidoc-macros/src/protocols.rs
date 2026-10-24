//! 编译期协议 kind 白名单与别名归一。
//!
//! 规范列表必须与 `apidoc/src/protocol.rs` 的 `SPECS` 保持一致；两处分开是因为
//! proc-macro crate 不能把普通常量导出给运行期 crate。collect 时遇到核心侧不认识的
//! kind 会 eprintln 警告，漂移能被测试/使用发现。

/// 规范 kind。
pub const CANONICAL: &[&str] = &[
    "http", "http2", "http3", "soap", "graphql", "websocket", "sse", "socketio", "webrtc",
    "grpc", "dubbo", "thrift", "jsonrpc", "xmlrpc", "hessian", "amqp", "mqtt", "stomp",
    "kafka", "webhook", "tcp", "udp", "coap", "lwm2m", "xmpp", "sip", "rtsp", "rtmp", "hls",
    "mcp", "a2a", "iso8583", "odata", "raw", "unix", "http-binary", "ws-binary", "stomp-binary",
    "opcua", "modbus", "dnp3", "iec104", "bacnet", "rtp", "rtcp",
];

/// 别名 → 规范 kind。比较时输入先转小写。
pub const ALIASES: &[(&str, &str)] = &[
    ("rest", "http"),
    ("https", "http"),
    ("http/1.1", "http"),
    ("http/2", "http2"),
    ("http/3", "http3"),
    ("ws", "websocket"),
    ("socket.io", "socketio"),
    ("socket-io", "socketio"),
    ("json-rpc", "jsonrpc"),
    ("xml-rpc", "xmlrpc"),
    ("iso-8583", "iso8583"),
    ("raw-socket", "raw"),
    ("unix-socket", "unix"),
    ("unix-domain-socket", "unix"),
    ("http-binary-body", "http-binary"),
    ("websocket-binary", "ws-binary"),
    ("ws-binary-frame", "ws-binary"),
    ("stomp-binary-body", "stomp-binary"),
    ("opc-ua", "opcua"),
    ("opc-ua-binary", "opcua"),
    ("modbus-tcp", "modbus"),
    ("iec-60870-5-104", "iec104"),
    ("bacnet/ip", "bacnet"),
    ("bacnet-ip", "bacnet"),
    ("webrtc-datachannel", "webrtc"),
];

/// 归一 kind：大小写不敏感，命中别名取规范名。未知返回 None。
pub fn normalize(kind: &str) -> Option<&'static str> {
    let lower = kind.trim().to_ascii_lowercase();
    if let Some(k) = CANONICAL.iter().find(|k| **k == lower) {
        return Some(k);
    }
    ALIASES.iter().find(|(a, _)| *a == lower).map(|(_, k)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_and_aliases_resolve() {
        assert_eq!(normalize("REST"), Some("http"));
        assert_eq!(normalize("https"), Some("http"));
        assert_eq!(normalize("HTTP/2"), Some("http2"));
        assert_eq!(normalize("ws"), Some("websocket"));
        assert_eq!(normalize("socket.io"), Some("socketio"));
        assert_eq!(normalize("modbus-tcp"), Some("modbus"));
        assert_eq!(normalize("webrtc-datachannel"), Some("webrtc"));
        assert_eq!(normalize("grpc"), Some("grpc"));
    }

    #[test]
    fn unknown_kind_is_rejected() {
        assert_eq!(normalize("carrier-pigeon"), None);
        assert_eq!(normalize(""), None);
    }

    #[test]
    fn no_alias_shadows_a_canonical_kind() {
        for (alias, target) in ALIASES {
            assert!(CANONICAL.contains(target), "alias `{alias}` → unknown `{target}`");
            assert!(!CANONICAL.contains(alias), "alias `{alias}` is also canonical");
        }
    }
}
