# 多协议文档与调试（P1–P6）实施计划 — 总纲

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 apidoc-rust 的注解、文档 UI、导出与在线调试覆盖 43 项非 HTTP 协议/传输，同时保持默认构建零重型依赖、老文档 api.json 字节级不变、服务端调试默认关闭。

**Architecture:** 统一文档模型（`protocol` / `protocol_opts` / `events` / `field`）+ 双执行路径（浏览器直连 / 服务端 executor）。服务端 executor 按协议族独立 feature，统一走 `POST /apidoc/debug`（一元）与 `GET /apidoc/debug/stream`（长连接 WebSocket 桥）；SSRF 守卫是所有服务端路径的唯一入口。

**Tech Stack:** Rust 1.82、axum 0.8、actix-web 4、tokio、reqwest（rustls）、可选 tonic / lapin / rumqttc / rskafka / coap-lite / tokio-modbus；UI 为无构建步骤的原生 JS（`textContent` 注入，禁用 `innerHTML`）。

**Spec:** P0 已实现并合入当前工作树：`crates/apidoc/src/lib.rs`、`crates/apidoc/src/protocol.rs`、`crates/apidoc/src/debug/`、`crates/apidoc-macros/src/protocols.rs`。本文件是 P1–P6 的总纲与共享接口定义；每个阶段一份独立计划，见文末索引。

---

## 0. 状态

- **P0 已完成**：模型、`protocol` / `event` / `field` 注解、45 个规范 kind 与别名、`DebugConfig`、`POST /apidoc/debug`、HTTP 执行器、SSRF 守卫、协议徽标/服务端发送开关/原始 Body、导出适配、测试。
- **P1–P6 未开始**。每个阶段独立交付、独立可测，不要求前序阶段全部完成才能开始（除下方依赖图明确要求者）。

## 1. 全局约束

- 默认构建（无 feature）不得新增任何重型依赖；每个协议族一个 feature，`debug-all` 仅作为聚合开关。
- 服务端调试默认关闭；`DebugConfig.enable_server_side = true` 必须同时满足全局鉴权（`AuthConfig { enable: true, password: 非空 }`），否则构建期 panic。
- 所有服务端目标必须先过 `debug::guard::check`：默认拒绝回环/私网/链路本地/ULA/CGNAT，永久拒绝云元数据地址；DNS 解析后逐 IP 校验并钉住连接；禁止重定向、系统代理与 CONNECT；Unix socket 默认关闭。
- 不使用 `innerHTML`；所有来自 api.json / 对端的字符串一律 `textContent` 注入。
- 未使用非 HTTP 注解时，api.json 与当前离线版本字节级一致；新增字段必须 `skip_serializing_if`。
- 单文件 ≤ 500 行；超限按职责拆分（见各阶段文件结构）。
- 端口/目标/帧数/字节数/超时全部有上限；默认：`timeout_ms = 10_000`、`max_bytes = 1_048_576`、`max_frames = 1_000`。
- 不引入 C 链接依赖（Kafka 用 `rskafka`，不用 `rdkafka`）。
- 凭据（密码、token、证书、私钥）永不进 api.json、日志或错误消息；UI 侧只存 localStorage，并在文档中标注风险。
- 所有新增 crate 必须先确认支持 Rust 1.82 且为纯 Rust；不满足则按计划内“手写编解码”回退。

## 2. 共享接口（P0 已存在，后续阶段依赖）

```rust
// crates/apidoc/src/lib.rs
pub struct DocEndpoint {
    pub title: String, pub desc: String, pub url: String, pub method: String,
    pub protocol: String,                    // 默认 "http"，序列化时省略
    pub protocol_opts: BTreeMap<String, String>,
    pub events: Vec<DocEvent>,
    // ... P0 之前的字段不变
}
pub struct DocEvent {
    pub direction: &'static str,             // "in" | "out"
    pub name: &'static str,
    pub ty: &'static str,                    // "json" | "text" | "binary" | ...
    pub desc: Option<&'static str>,
    pub mock: Option<&'static str>,
}
pub struct DocParam { /* ... */ pub field: Option<&'static str> }

pub struct DebugConfig {
    pub enable_server_side: bool,
    pub allow_private_networks: bool,        // #[serde(skip)]
    pub allow_hosts: Vec<String>,            // #[serde(skip)]
    pub allow_unix_sockets: bool,            // #[serde(skip)]
    pub timeout_ms: u64,
    pub max_bytes: usize,
}

// crates/apidoc/src/protocol.rs
pub enum Interaction { Unary, Stream, Datagram, Media, Peer }
pub enum Execution { Browser, Server, Both }
pub struct ProtocolSpec { pub kind: &'static str, pub interaction: Interaction,
                          pub execution: Execution, pub http_compatible: bool }
pub fn spec(kind: &str) -> Option<&'static ProtocolSpec>;
pub fn is_http_compatible(kind: &str) -> bool;

// crates/apidoc/src/debug/mod.rs（P0）
pub struct DebugRequest {
    pub kind: String, pub target: String,
    pub method: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Payload>,
    pub opts: BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
}
pub struct Payload { pub encoding: String, pub data: String } // utf8 | hex | base64
pub struct DebugResponse {
    pub status: u16, pub ms: u64,
    pub headers: BTreeMap<String, String>,
    pub body: Payload,
}
pub async fn execute(cfg: &DebugConfig, req: &DebugRequest) -> Result<DebugResponse, String>;
```

### 2.1 P2 扩展（后续阶段必须遵守）

```rust
// crates/apidoc/src/debug/mod.rs
#[derive(Deserialize)]
pub struct EndpointRef { pub url: String, pub method: String }

pub struct DebugRequest {
    // ... P0 字段
    #[serde(default)] pub endpoint: Option<EndpointRef>,
}

#[derive(Serialize, Deserialize)]
pub struct Frame {
    pub dir: String, pub encoding: String, pub data: String, pub ts: u64,
}
pub struct DebugResponse {
    // ... P0 字段
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<Frame>,       // gRPC server-streaming 等多帧一元响应
}

// 签名变更：schema 只能来自服务端已收集的 DocEndpoint，绝不信任客户端上传的 schema。
pub async fn execute(
    cfg: &DebugConfig,
    req: &DebugRequest,
    ep: Option<&DocEndpoint>,
) -> Result<DebugResponse, String>;
```

`Core::debug` 用 `req.endpoint` 在 `self.doc.endpoints` 中精确匹配 `(url, method)`；匹配不到时 `ep = None`，需要 schema 的协议返回 400。

### 2.2 长连接控制协议（P2）

客户端 → 服务端：

```json
{"op":"connect","kind":"websocket","target":"ws://host/path","headers":{},"opts":{}}
{"op":"send","encoding":"utf8","data":"..."}
{"op":"close"}
```

服务端 → 客户端：

```json
{"type":"open"}
{"type":"frame","dir":"in","encoding":"utf8","data":"...","ts":1720000000000}
{"type":"close","code":1000,"reason":"..."}
{"type":"error","error":"..."}
```

### 2.3 UI 面板契约（P1）

```js
// 纯函数编解码放在 ui.codecs.js，Node 可直接 require 做单元测试。
// 流式适配器统一形状：
// connect(target, opts, handlers) -> { send({encoding,data}), close() }
// handlers = { onOpen(), onFrame({dir,encoding,data}), onClose(code,reason), onError(msg) }
const STREAM_ADAPTERS = { websocket, sse, socketio, stomp, 'graphql-ws' };

// HTTP 族协议的表单预设；返回 null 表示沿用通用表单。
// 返回 { headers?, querys?, body?: {encoding,data}, extras?: Node[] }
const PROTOCOL_FORMS = { soap, graphql, jsonrpc, xmlrpc, odata, mcp, a2a, webhook };

// 非 HTTP 服务端协议（P2 起）：
// - Base URL 预填 ep.protocol_opts.target；
// - 显示「协议选项（JSON）」文本框，预填非敏感 opts（target/topic/service/method…）；
// - 密码/token/证书由用户在运行期输入，只存在内存，不写 localStorage、不进 api.json。
```

## 3. 阶段路线图

| 阶段 | 内容 | 新 feature | 依赖 | 交付标准 |
|---|---|---|---|---|
| P1 | 浏览器原生：SOAP/GraphQL/JSON-RPC/XML-RPC/OData/MCP/A2A/Webhooks、WS/WS binary、SSE、Socket.IO、STOMP、GraphQL 订阅、WebRTC/DataChannel、HLS | 无（纯 JS） | P0 | 所有面板在浏览器可用；纯编解码有 Node 单测 |
| P2 | 服务端基础：EndpointRef、WS 会话桥、TCP/UDP/Raw/Unix、gRPC、Thrift、Dubbo/Hessian、ISO 8583 | `debug-session`、`debug-sockets`、`debug-grpc`、`debug-thrift`、`debug-dubbo`、`debug-iso8583` | P0、P1 的 UI 面板 | 每协议有本地回环/黄金向量测试；SSRF 测试覆盖每种传输 |
| P3 | 消息中间件：MQTT、AMQP、Kafka、XMPP、STOMP over TCP | `debug-mqtt`、`debug-amqp`、`debug-kafka`、`debug-xmpp` | P2 会话桥 | 本地 broker 或 env-gated 集成测试；无 broker 时编解码单测 |
| P4 | IoT/工业：CoAP、LwM2M、Modbus TCP、DNP3、IEC 104、BACnet/IP、OPC UA | `debug-coap`、`debug-industrial` | P2 sockets | 本地测试服务器/黄金向量；超时与帧上限测试 |
| P5 | 媒体/信令：RTP、RTCP、RTSP、SIP、RTMP、HTTP/3 | `debug-media`、`debug-http3` | P2 sockets/session | 本地回环；媒体只做握手/包级调试，不做转码 |
| P6 | 增强：Webhook 入站捕获、IDL 导入、Dubbo 注册中心、非 HTTP mock、AsyncAPI 导出、文档/i18n | 独立 crate `apidoc-idl`；Webhook 无新 feature（复用核心依赖） | P1–P5 | 迁移/导入有快照测试；导出可解析 |

## 4. Review Focus（最容易被忽略、必须由各阶段测试钉住的前五类）

1. **服务端代理的 SSRF 旁路**：DNS rebinding、`HTTP_PROXY`、重定向、CONNECT、IPv4-mapped IPv6、Unix socket。期望：默认全部拒绝；P2 每个传输都有 guard 测试。
2. **二进制载荷被字符串化/UTF-8 化**：hex/base64 请求与响应、WS 二进制帧、STOMP 二进制 body、gRPC/Thrift bytes 字段。期望：字节级往返一致；P1/P2/P3 各有编解码测试。
3. **流式内存无界**：帧日志、Kafka fetch、MQTT 消息、TCP 读取。期望：`max_frames`/`max_bytes` 触发时关闭并提示；P1/P3/P4 各有上限测试。
4. **字段号/schema 不一致**：gRPC tag、Thrift field id、Dubbo 参数类型、ISO 8583 域定义。期望：服务端从 `DocEndpoint` 取 schema，客户端不能注入；黄金向量测试。
5. **凭据泄漏**：`DebugConfig` 白名单、协议 opts 里的密码/证书、Webhook 捕获内容。期望：api.json 无敏感项、错误消息不回显凭据；P0 已有序列化测试，P2/P3/P6 继续覆盖。

## 5. 执行顺序与提交策略

1. 按 P1 → P6 顺序执行；P1 不依赖服务端改动，可先独立合并。
2. 每个 Task 一个 commit，提交信息用 `feat(debug): ...` / `test(debug): ...` / `docs(debug): ...`。
3. 每个阶段结束跑：`cargo test -p apidoc-rust --all-features`、`cargo test -p apidoc-macros`、`cargo check -p apidoc-rust`（默认 feature，确认零新增依赖）、`node --test crates/apidoc/tests/js/`（P1 起）。
4. 发版前统一升 `VERSION` / workspace version / 三处 path 依赖版本，并同步 README 各语言版本；发版流程见项目记忆 `release-flow-apidoc-rust`。

## 6. 计划文件索引

- [P1 浏览器原生协议](./2026-10-24-multi-protocol-p1-browser.md)
- [P2 服务端 RPC 与 Socket](./2026-10-24-multi-protocol-p2-rpc-sockets.md)
- [P3 消息中间件](./2026-10-24-multi-protocol-p3-messaging.md)
- [P4 IoT / 工业协议](./2026-10-24-multi-protocol-p4-industrial.md)
- [P5 媒体 / 信令 / HTTP3](./2026-10-24-multi-protocol-p5-media.md)
- [P6 增强与收敛](./2026-10-24-multi-protocol-p6-enhancements.md)
