//! P0 协议注册表：kind → 交互模型 / 执行位置 / 能否映射进 OpenAPI。
//!
//! 宏侧的 kind 白名单在 `apidoc-macros/src/protocols.rs`，这里负责运行期语义。
//! 两处列表由 tests/protocol_registry.rs 锁定一致（宏是 proc-macro crate，
//! 不能把普通常量导出给核心 crate 用）。

/// 调试面板的交互模型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// 一问一答（HTTP 族、gRPC/Thrift/Dubbo 一元调用、SIP…）。
    Unary,
    /// 长连接、双向或持续接收（WS/SSE/MQTT/Kafka/TCP…）。
    Stream,
    /// 无连接数据报（UDP/CoAP/RTP…）。
    Datagram,
    /// 会话/媒体（RTSP/RTMP/HLS/OPC UA…），需要连接状态与可选预览。
    Media,
    /// 对等连接（WebRTC / DataChannel），需要信令与 ICE。
    Peer,
}

/// 调试请求的执行位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Execution {
    /// 浏览器原生可直连。
    Browser,
    /// 必须由 apidoc 适配器在服务端代发（浏览器开不了该 socket 或需要特殊编解码）。
    Server,
    /// 两者都行：浏览器优先，受 CORS/网络限制时可切服务端。
    Both,
}

pub struct ProtocolSpec {
    pub kind: &'static str,
    pub interaction: Interaction,
    pub execution: Execution,
    /// 有 HTTP method + path 语义，可进 OpenAPI（其余协议导出时跳过）。
    pub http_compatible: bool,
}

const fn s(
    kind: &'static str,
    interaction: Interaction,
    execution: Execution,
    http_compatible: bool,
) -> ProtocolSpec {
    ProtocolSpec { kind, interaction, execution, http_compatible }
}

/// 全部规范 kind（别名由宏在编译期归一，见 apidoc-macros/src/protocols.rs）。
pub const SPECS: &[ProtocolSpec] = &[
    s("http", Interaction::Unary, Execution::Both, true),
    s("http2", Interaction::Unary, Execution::Both, true),
    s("http3", Interaction::Unary, Execution::Server, true),
    s("soap", Interaction::Unary, Execution::Both, true),
    s("graphql", Interaction::Unary, Execution::Both, true),
    s("websocket", Interaction::Stream, Execution::Both, false),
    s("sse", Interaction::Stream, Execution::Both, false),
    s("socketio", Interaction::Stream, Execution::Both, false),
    s("webrtc", Interaction::Peer, Execution::Browser, false),
    s("grpc", Interaction::Unary, Execution::Server, false),
    s("dubbo", Interaction::Unary, Execution::Server, false),
    s("thrift", Interaction::Unary, Execution::Server, false),
    s("jsonrpc", Interaction::Unary, Execution::Both, true),
    s("xmlrpc", Interaction::Unary, Execution::Both, true),
    s("hessian", Interaction::Unary, Execution::Server, false),
    s("amqp", Interaction::Stream, Execution::Server, false),
    s("mqtt", Interaction::Stream, Execution::Server, false),
    s("stomp", Interaction::Stream, Execution::Both, false),
    s("kafka", Interaction::Stream, Execution::Server, false),
    s("webhook", Interaction::Unary, Execution::Both, true),
    s("tcp", Interaction::Stream, Execution::Server, false),
    s("udp", Interaction::Datagram, Execution::Server, false),
    s("coap", Interaction::Datagram, Execution::Server, false),
    s("lwm2m", Interaction::Datagram, Execution::Server, false),
    s("xmpp", Interaction::Stream, Execution::Server, false),
    s("sip", Interaction::Unary, Execution::Server, false),
    s("rtsp", Interaction::Media, Execution::Server, false),
    s("rtmp", Interaction::Media, Execution::Server, false),
    s("hls", Interaction::Media, Execution::Browser, false),
    s("mcp", Interaction::Unary, Execution::Both, true),
    s("a2a", Interaction::Unary, Execution::Both, true),
    s("iso8583", Interaction::Unary, Execution::Server, false),
    s("odata", Interaction::Unary, Execution::Both, true),
    s("raw", Interaction::Stream, Execution::Server, false),
    s("unix", Interaction::Stream, Execution::Server, false),
    s("http-binary", Interaction::Unary, Execution::Both, true),
    s("ws-binary", Interaction::Stream, Execution::Both, false),
    s("stomp-binary", Interaction::Stream, Execution::Both, false),
    s("opcua", Interaction::Media, Execution::Server, false),
    s("modbus", Interaction::Unary, Execution::Server, false),
    s("dnp3", Interaction::Stream, Execution::Server, false),
    s("iec104", Interaction::Stream, Execution::Server, false),
    s("bacnet", Interaction::Datagram, Execution::Server, false),
    s("rtp", Interaction::Datagram, Execution::Server, false),
    s("rtcp", Interaction::Datagram, Execution::Server, false),
];

pub fn spec(kind: &str) -> Option<&'static ProtocolSpec> {
    SPECS.iter().find(|p| p.kind == kind)
}

pub fn is_http_compatible(kind: &str) -> bool {
    spec(kind).is_some_and(|p| p.http_compatible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn kinds_are_unique() {
        let mut seen = HashSet::new();
        for p in SPECS {
            assert!(seen.insert(p.kind), "duplicate protocol kind `{}`", p.kind);
        }
    }

    #[test]
    fn unknown_kind_has_no_spec() {
        assert!(spec("nope").is_none());
        assert!(spec("http").is_some());
    }

    #[test]
    fn openapi_only_for_http_semantics() {
        assert!(is_http_compatible("soap"));
        assert!(is_http_compatible("graphql"));
        assert!(!is_http_compatible("websocket"));
        assert!(!is_http_compatible("grpc"));
    }
}
