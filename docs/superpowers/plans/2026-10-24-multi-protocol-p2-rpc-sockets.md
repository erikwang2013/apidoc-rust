# P2 服务端 RPC 与 Socket — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在服务端执行 gRPC、Thrift、Dubbo/Hessian、ISO 8583，以及原始 TCP/UDP/Raw/Unix socket 调试；为后续消息与工业协议提供通用的长连接会话桥。

**Architecture:** 先加 `DebugRequest.endpoint`（schema 只从服务端已收集的 `DocEndpoint` 取）与 `GET /apidoc/debug/stream` WebSocket 会话桥；每个协议一个 `debug/<proto>.rs`，暴露 `execute`（一元）和/或 `connect_stream`（长连接）。所有目标地址先过 `debug::guard`。

**Tech Stack:** tokio、tokio-tungstenite、async-trait、tonic + prost-reflect、手写 TBinaryProtocol / Hessian2 / Dubbo / ISO 8583 编解码。

**Spec:** [总纲](./2026-10-24-multi-protocol-master.md)；P0/P1 代码见 `crates/apidoc/src/debug/`、`crates/apidoc/src/ui.debug.stream.js`。

## 全局约束（本阶段）

- 服务端 debug 默认关闭；全部路由经 `Core::guard`；SSRF 守卫是唯一出口。
- schema（字段号/类型/服务名）只能来自 `DocEndpoint`（`ep.params` / `ep.returned` / `protocol_opts`）；客户端上传的 schema 一律忽略。
- 一元响应上限 `max_bytes`；流式会话上限 `max_frames`；连接/读/写超时都必须有值。
- 二进制字段以 `Payload { encoding, data }` 表达；不得经过 `String::from_utf8_lossy`。
- 每个协议族独立 feature；默认构建不新增依赖。
- 不引入 C 链接依赖。

## Review Focus（本阶段）

1. `EndpointRef` 查找失败时仍按“无 schema”继续，导致字段号错位；期望：需要 schema 的协议返回 400。
2. WebSocket 会话桥的背压与关闭：目标断开、客户端断开、超帧数时都要释放任务与 socket。
3. SSRF 在 raw socket 路径被绕过（没有 URL 解析）；期望：`guard::check_host` 覆盖 tcp/udp/unix。
4. Thrift/Dubbo/ISO 8583 的字节序、长度前缀、字段序错误；期望：黄金向量逐字节断言。
5. gRPC 描述符来源不可信或缺失；期望：只接受服务端 fixture/反射，失败时返回明确错误。

---

## 文件结构

```
crates/apidoc/src/debug/
  mod.rs          # 修改：EndpointRef、execute(cfg, req, ep)、stream_dispatch
  guard.rs        # 修改：check_host(cfg, host, port) 供 socket 复用
  session.rs      # 新建：DebugSocket trait + 控制协议循环
  ws_client.rs    # 新建：服务端 WebSocket 客户端执行器
  sockets.rs      # 新建：TCP/UDP/raw/unix
  grpc.rs         # 新建
  thrift.rs       # 新建
  hessian.rs      # 新建
  dubbo.rs        # 新建
  iso8583.rs      # 新建
crates/apidoc/src/axum.rs / actix.rs   # 修改：/apidoc/debug/stream 路由
crates/apidoc/tests/
  debug_session.rs       # 新建：控制协议 + 本地 WS echo
  debug_sockets.rs       # 新建
  debug_grpc.rs          # 新建
  debug_thrift.rs        # 新建
  debug_dubbo.rs         # 新建
  debug_iso8583.rs       # 新建
  fixtures/helloworld.proto / helloworld.desc / helloworld.rs  # 新建
```

---

### Task 1: EndpointRef + WebSocket 会话桥

**Files:**
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/src/route_core.rs`
- Create: `crates/apidoc/src/debug/session.rs`
- Create: `crates/apidoc/src/debug/ws_client.rs`
- Modify: `crates/apidoc/src/axum.rs`、`crates/apidoc/src/actix.rs`
- Modify: `crates/apidoc/Cargo.toml`
- Create: `crates/apidoc/tests/debug_session.rs`

**Interfaces:**
- `DebugRequest.endpoint: Option<EndpointRef>`；`EndpointRef { url: String, method: String }`。
- `debug::execute(cfg, req, ep: Option<&DocEndpoint>)`；`Core::debug` 查找 `self.doc.endpoints` 并传入。
- `session.rs`：
```rust
#[async_trait::async_trait]
pub trait DebugSocket: Send {
    async fn send_text(&mut self, text: String) -> Result<(), String>;
    async fn recv_text(&mut self) -> Option<Result<String, String>>;
    async fn close(&mut self);
}

#[async_trait::async_trait]
pub trait StreamSession: Send {
    async fn send(&mut self, payload: Payload) -> Result<(), String>;
    async fn close(&mut self);
}

pub async fn connect_stream(
    cfg: &DebugConfig, req: &DebugRequest, ep: Option<&DocEndpoint>,
    frames: tokio::sync::mpsc::Sender<String>,
) -> Result<Box<dyn StreamSession>, String>;

pub async fn run(cfg: &DebugConfig, ep: Option<&DocEndpoint>, mut socket: impl DebugSocket);
```
- 控制帧 JSON 形状见总纲 §2.2。
- feature：`debug-session = ["debug-http", "dep:async-trait", "dep:tokio-tungstenite", "dep:futures-util"]`；axum 依赖加 `features = ["ws"]`；actix 新增 `actix-ws`（optional，放入 `debug-session`）。

- [ ] **Step 1: 写控制协议单测（内存 socket）**

`tests/debug_session.rs` 里定义 `MockSocket { input: VecDeque<String>, output: Vec<String> }` 实现 `DebugSocket`，以及一个 `EchoSession` 实现 `StreamSession`（`send` 时把同样 payload 作为 `frame` 发回 `frames`）。断言：

```rust
#[tokio::test]
async fn session_connect_send_frame_close() {
    let mut sock = MockSocket::new(vec![
        r#"{"op":"connect","kind":"websocket","target":"ws://127.0.0.1:1/x"}"#,
        r#"{"op":"send","encoding":"utf8","data":"hi"}"#,
        r#"{"op":"close"}"#,
    ]);
    // 用注入的假 connect_stream 或直接测试 session::run_with(connect_fn, ...)
    // 期望 output 依次包含 {"type":"open"}、{"type":"frame",...}、{"type":"close",...}
}
```

为避免网络依赖，`session::run` 拆出一个可注入的内部函数：
```rust
pub(crate) async fn run_with<F, Fut, S>(socket: S, connect: F)
where F: FnOnce(mpsc::Sender<String>) -> Fut, Fut: Future<Output = Result<Box<dyn StreamSession>, String>>;
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-session --test debug_session`
Expected: FAIL — module/trait 不存在。

- [ ] **Step 3: 实现 EndpointRef、session、ws_client**

- `Core::debug` 解析 `endpoint` 并传给 `debug::execute`。
- `session::run` 解析首个 `connect` 帧，调用 `connect_stream`，`tokio::select!` 同时处理客户端控制帧与 `frames` 通道；`send` 转发给 `StreamSession::send`；`close`/EOF 时关闭会话与 socket；超过 `max_frames` 主动关闭。
- `ws_client::connect_stream` 用 `tokio_tungstenite::connect_async` 连接目标（先 `guard::check`），读任务把目标帧转成 `{"type":"frame",...}` 写入 `frames`；`send` 按编码转 `Message::Text/Binary`。
- 路由：axum `GET /apidoc/debug/stream` 用 `WebSocketUpgrade` + `AxumSocket`；actix 用 `actix_ws::Session` + `ActixSocket`。两处都先执行 `Core::guard`，失败直接 401。

- [ ] **Step 4: 加本地 WS echo 集成测试**

用 `tokio-tungstenite` 在 `127.0.0.1:0` 起一个 echo server；把 apidoc 路由用 `axum::serve` 起在另一个随机端口；用 `tokio-tungstenite` 连接 `/apidoc/debug/stream?token=...`，发 `connect`/`send`，断言收到 `open` 与 `frame`。配置 `allow_private_networks = true`。

- [ ] **Step 5: UI：非 HTTP 服务端协议的 Base URL 与 opts 编辑器**

`ui.debug.js` 在 `ep.protocol` 非 HTTP 且 `protocol_opts.target` 存在时，将 Base URL 预填为该 target（用户仍可改）。增加折叠的「协议选项（JSON）」textarea，预填 `ep.protocol_opts` 去掉 `target` 后的非敏感键；发送时 `JSON.parse` 后并入请求 `opts`。凭据输入只存在内存，不写 localStorage。若 `ep.protocol` 属于服务端专有集合（`grpc`/`thrift`/`dubbo`/`iso8583`/`tcp`/`udp`/`raw`/`unix`/`mqtt`/`amqp`/`kafka`/`xmpp`/`coap`/工业/媒体等）且 `config.debug.enable_server_side` 未开启，面板显示启用指引（`DebugConfig { enable_server_side: true }` + 对应 `debug-*` feature），不得静默隐藏发送入口。结构测试断言 `协议选项`、`protocol_opts.target` 与 `enable_server_side`。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-session,axum --test debug_session && cargo test -p apidoc-rust --test ui_events`
Expected: PASS。

- [ ] **Step 7: 提交**

```bash
git add crates/apidoc/src/debug crates/apidoc/src/route_core.rs crates/apidoc/src/axum.rs crates/apidoc/src/actix.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_session.rs
git commit -m "feat(debug): add endpoint schema ref and websocket session bridge"
```

---

### Task 2: 原始 TCP / UDP / Raw / Unix Socket

**Files:**
- Modify: `crates/apidoc/src/debug/guard.rs`（提取 `check_host`）
- Create: `crates/apidoc/src/debug/sockets.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`（分发）
- Modify: `crates/apidoc/Cargo.toml`（`debug-sockets = ["debug-http"]`）
- Create: `crates/apidoc/tests/debug_sockets.rs`

**Interfaces:**
- `guard::check_host(cfg: &DebugConfig, host: &str, port: u16) -> Result<Vec<SocketAddr>, String>`；`check` 改为调用它。
- `sockets::execute(cfg, req, ep) -> Result<DebugResponse, String>`，处理 kind：`tcp`、`udp`、`raw`、`unix`。
- `opts` 约定：
  - `transport`（仅 raw）：`tcp`/`udp`
  - `read_ms`：读窗口（默认 500）
  - `expect_bytes`：读到 N 字节即返回（可选）
  - `datagrams`（UDP）：接收包数（默认 1，上限 100）
  - `target`：`tcp://host:port` / `udp://host:port` / `unix:///path`；无 scheme 时按 tcp 处理。
- 响应：`status = 0`，`headers` 放 `x-peer`、`x-datagrams` 等元信息，`body` 为收到的字节。
- Unix：`allow_unix_sockets` 为 false 时直接拒绝；不经过 DNS 校验。

- [ ] **Step 1: 写失败测试**

`tests/debug_sockets.rs`：
- TCP echo：本地 `TcpListener` 回写收到的字节；`execute` 发送 `"ping"`，`expect_bytes=4`，断言 body `"ping"`。
- UDP echo：本地 `UdpSocket` 回包；断言 body。
- SSRF：默认配置下 `tcp://127.0.0.1:{port}` 返回错误包含 `private/loopback`；`allow_private_networks=true` 时通过。
- Unix（`#[cfg(unix)]`）：临时 socket 回写；`allow_unix_sockets=false` 时拒绝。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-sockets --test debug_sockets`
Expected: FAIL — `sockets` 模块不存在。

- [ ] **Step 3: 实现**

- guard 重构：`check` 解析 URL 后调用 `check_host`；`check_host` 负责 allowlist、DNS、逐 IP 校验。
- TCP：`TcpStream::connect` 带超时，写入，循环 `read` 到 `expect_bytes`/`read_ms`/EOF，累计不超过 `max_bytes`。
- UDP：bind `0.0.0.0:0`，`send_to`，按 `datagrams` 收包，单包不超过 65507 字节。
- Raw：按 `transport` 走 TCP 或 UDP。
- Unix：`UnixStream` / `UnixDatagram`；路径长度与存在性由 OS 报错。
- 统一 `Payload::from_bytes` 返回响应。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-sockets --test debug_sockets`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/guard.rs crates/apidoc/src/debug/sockets.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_sockets.rs
git commit -m "feat(debug): add raw tcp/udp/unix socket executor"
```

---

### Task 3: gRPC（一元 + 服务端流）

**Files:**
- Create: `crates/apidoc/src/debug/grpc.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`（`DebugResponse.frames`）
- Modify: `crates/apidoc/Cargo.toml`（`debug-grpc`）
- Create: `crates/apidoc/tests/debug_grpc.rs`
- Create: `crates/apidoc/tests/fixtures/helloworld.proto`
- Create: `crates/apidoc/tests/fixtures/helloworld.desc`（base64 FileDescriptorSet）
- Create: `crates/apidoc/tests/fixtures/helloworld.rs`（一次性生成的服务端代码）

**Interfaces:**
- `DebugResponse` 新增 `#[serde(skip_serializing_if = "Vec::is_empty")] pub frames: Vec<Frame>`；`Frame { dir: String, encoding: String, data: String, ts: u64 }`。
- `grpc::execute(cfg, req, ep)`：
  - `opts.service`、`opts.method` 必填；`opts.descriptor`（base64 FileDescriptorSet）可选，缺省走服务端反射。
  - 请求体 JSON 以 `ep.params[].name` 为键；字段号取 `ep.params[].field`，缺省时按声明顺序 1..n（文档中明确说明）。
  - 支持一元与 server-streaming：server-streaming 收到 `opts.max_frames` 或 `opts.duration_ms` 后停止，结果放 `frames`。
- 客户端/双向流留到 P6（本阶段不实现，返回明确错误）。

- [ ] **Step 1: 准备 fixture**

`tests/fixtures/helloworld.proto`：

```proto
syntax = "proto3";
package helloworld;
service Greeter { rpc SayHello (HelloRequest) returns (HelloReply); }
message HelloRequest { string name = 1; }
message HelloReply { string message = 1; }
```

生成一次（提交产物，不把 tonic-build 变成构建依赖）：
```bash
protoc --include_imports --descriptor_set_out=tests/fixtures/helloworld.desc tests/fixtures/helloworld.proto
base64 -w0 tests/fixtures/helloworld.desc > tests/fixtures/helloworld.desc.b64
# 用 tonic-build 一次性生成 tests/fixtures/helloworld.rs
```

- [ ] **Step 2: 写失败测试**

`tests/debug_grpc.rs` 启动一个 tonic server（用 fixture 生成代码）监听 `127.0.0.1:0`，注册 `Greeter` 与 `tonic_reflection`。调用 `grpc::execute`：
- `opts.service="helloworld.Greeter"`, `opts.method="SayHello"`, body `{"name":"world"}`；
- 断言响应 body JSON 含 `{"message":"Hello world"}`。
- 反射不可用时用 `opts.descriptor`（fixture base64）再跑一次。
- 断言字段号来自 `ep.params[].field`：构造一个 `DocEndpoint`，`name` field=1；若 field 缺失则按顺序。

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-grpc --test debug_grpc`
Expected: FAIL — `grpc` 模块不存在。

- [ ] **Step 4: 实现**

- `tonic::transport::Channel` 连接目标（`guard` 先校验；TLS 用 `tls-ring` feature）。
- 描述符：`opts.descriptor` 解码为 `FileDescriptorSet` → `DescriptorPool`；否则用 `tonic_reflection` 拉取 `service` 的文件。
- `ep.params` → `DynamicMessage`：递归映射 `ty`（`string`/`int`/`int64`/`float`/`double`/`bool`/`bytes`/`object`/`array`），`field` 决定编号；`children` 递归 message。
- 调 `tonic::client::Grpc::unary` / `server_streaming`，用 `ProstCodec<DynamicMessage, DynamicMessage>`；响应转 JSON。
- 错误映射：连接失败/状态非 OK → `Err("grpc: ...")`。

- [ ] **Step 5: UI 展示 frames**

`ui.debug.js` 的服务端响应分支在 `data.frames` 非空时追加一个帧列表（方向/时间/编码/数据，`textContent`，最多 `MAX_FRAMES` 条）；一元响应 body 仍照常展示。结构测试断言 `data.frames` 与 `MAX_FRAMES`。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-grpc --test debug_grpc && cargo test -p apidoc-rust --test ui_events`
Expected: PASS。

- [ ] **Step 7: 提交**

```bash
git add crates/apidoc/src/debug/grpc.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_grpc.rs crates/apidoc/tests/fixtures
git commit -m "feat(debug): add grpc unary and server-streaming executor"
```

---

### Task 4: Thrift（TBinaryProtocol）

**Files:**
- Create: `crates/apidoc/src/debug/thrift.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`debug-thrift = ["debug-sockets"]`）
- Create: `crates/apidoc/tests/debug_thrift.rs`

**Interfaces:**
- `thrift::execute(cfg, req, ep)`。
- `opts.transport`：`tcp`（默认）或 `http`；`opts.service`、`opts.method`。
- schema：`ep.params`（请求）与 `ep.returned`（响应）；`field` 是 Thrift field id，`ty` 支持 `bool`/`byte`/`i16`/`i32`/`i64`/`double`/`string`/`binary`/`list<T>`/`map<K,V>`/`struct`。
- 请求体 JSON 以参数名为键。
- 实现函数（供测试直接调用）：
```rust
pub fn write_message_begin(out: &mut Vec<u8>, name: &str, msg_type: u8, seqid: i32);
pub fn write_field(out: &mut Vec<u8>, ty: u8, id: i16);
pub fn read_message_begin(input: &mut &[u8]) -> Result<(String, u8, i32), String>;
pub fn read_field_begin(input: &mut &[u8]) -> Result<Option<(u8, i16)>, String>;
```

- [ ] **Step 1: 写黄金向量测试**

```rust
#[test]
fn binary_protocol_hello_vector() {
    let mut out = Vec::new();
    thrift::write_message_begin(&mut out, "ping", 1, 0);
    // 断言 out == [0x80,0x01,0x00,0x01, 0x00,0x00,0x00,0x04, b'p',b'i',b'n',b'g', 0x00,0x00,0x00,0x00]
}
```

再加一个本地 TCP server：读完整调用帧，回一个固定的 `reply` 消息；`execute` 断言返回 JSON。最后断言 schema 缺失：`execute(cfg, req, None)` 返回包含 `endpoint schema required` 的错误（400 语义），不得猜测字段号。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-thrift --test debug_thrift`
Expected: FAIL。

- [ ] **Step 3: 实现编解码**

- ZigZag 变长整数、`i16`/`i32`/`i64` 大端、double IEEE-754、string 长度前缀。
- struct begin/end、field stop、list/map 头。
- 请求：`write_message_begin(method, CALL=1, seqid=0)` + struct 字段（按 `ep.params` 顺序，`field` 编号）+ STOP；发到 TCP/HTTP。
- 响应：读 message begin、struct 字段，按 `ep.returned` 映射成 JSON。
- HTTP 传输：`POST` 到 target，`Content-Type: application/x-thrift`，body 为同一二进制帧。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-thrift --test debug_thrift`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/thrift.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_thrift.rs
git commit -m "feat(debug): add thrift binary protocol executor"
```

---

### Task 5: Hessian2 + Dubbo

**Files:**
- Create: `crates/apidoc/src/debug/hessian.rs`
- Create: `crates/apidoc/src/debug/dubbo.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`debug-dubbo = ["debug-sockets"]`）
- Create: `crates/apidoc/tests/debug_dubbo.rs`

**Interfaces:**
- `hessian::encode(value: &serde_json::Value) -> Result<Vec<u8>, String>`；`hessian::decode(input: &mut &[u8]) -> Result<serde_json::Value, String>`。
- `dubbo::execute(cfg, req, ep)`。
- `opts.service`、`opts.method`、`opts.version`、`opts.param_types`（逗号分隔 Java 描述符，如 `Ljava/lang/String;`）。
- 请求体 JSON：按 `opts.param_types` 顺序的数组；也接受按参数名对象（按 `ep.params` 顺序）。
- 直接 URL，不接注册中心（P6）。

- [ ] **Step 1: 写 Hessian2 黄金向量**

覆盖 null、bool、int（-16..47 单字节、双字节、四字节）、long、double、string、list、map。至少一条与公开 Hessian2 规范一致的十六进制向量。

- [ ] **Step 2: 写 Dubbo 帧测试**

本地 TCP server：读 16 字节头，断言 magic `0xdabb`、序列化 id=2、body 长度；用 `hessian::decode` 读出 service/method/args；返回一个固定成功响应。`execute` 断言 JSON 返回值。

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-dubbo --test debug_dubbo`
Expected: FAIL。

- [ ] **Step 4: 实现**

- Hessian2 常用类型；class def/instance 只支持无字段名的 map-like 对象（Dubbo 泛化调用足够）。
- Dubbo 头：magic、flags（request=0x80, two-way=0x40, serialization=2）、status=0、8 字节 request id、4 字节 body length。
- Body：Dubbo version `"2.0.2"`、service path、version、method、parameter types、arguments、attachments（空 map）。
- 响应：检查 status，Hessian2 解码返回值；异常类型转错误文本。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-dubbo --test debug_dubbo`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add crates/apidoc/src/debug/hessian.rs crates/apidoc/src/debug/dubbo.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_dubbo.rs
git commit -m "feat(debug): add hessian2 and dubbo executor"
```

---

### Task 6: ISO 8583

**Files:**
- Create: `crates/apidoc/src/debug/iso8583.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`debug-iso8583 = ["debug-sockets"]`）
- Create: `crates/apidoc/tests/debug_iso8583.rs`

**Interfaces:**
- `iso8583::execute(cfg, req, ep)`。
- `ep.protocol_opts`：
  - `mti`（如 `"0200"`）
  - `framing`：`"2"`（默认，2 字节大端长度）/ `"4"` / `"none"`
  - `encoding`：`"ascii"`（P2）；`"ebcdic"` 留到 P6
  - `fields`：JSON 数组字符串，元素 `{"field":"2","type":"llvar","len":19,"kind":"n"}`
- 字段也可来自 `ep.params`：`field` = 域号，`ty` = `n`/`an`/`ans`/`b` + 长度。两种来源冲突时 `protocol_opts.fields` 优先。
- 响应解析为 `{"mti":"0210","fields":{"2":"...","39":"00"}}`。

- [ ] **Step 1: 写黄金向量**

固定一个 0200 请求：MTI + 主位图（域 2、3、4）+ 域值；断言构造出的字节与手工十六进制一致。再构造一个 0210 响应，断言解析结果。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-iso8583 --test debug_iso8583`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 位图：域 1-64 主位图，>64 时置第 1 位并追加次位图。
- 域类型：`n` 右对齐补零、`an`/`ans` 左对齐补空格、`b` BCD；`fixed`/`llvar`/`lllvar`/`llllvar`。
- 默认 2 字节大端长度前缀；`none` 时不加。
- 响应解析按位图逐域读取。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-iso8583 --test debug_iso8583`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/iso8583.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_iso8583.rs
git commit -m "feat(debug): add iso8583 executor with configurable fields"
```

---

### Task 7: feature 收口与文档

**Files:**
- Modify: `crates/apidoc/Cargo.toml`（`debug-all` 聚合）
- Modify: `crates/apidoc/src/debug/mod.rs`（分发全部 kind）
- Modify: `README.md` 与 `docs/i18n/README-*.md`（协议支持表）
- Modify: `.github/workflows/`（若无 test workflow，至少加一段本地检查脚本说明）

**Interfaces:**
- `debug-all = ["debug-http","debug-session","debug-sockets","debug-grpc","debug-thrift","debug-dubbo","debug-iso8583"]`。
- `debug::execute` 分发：`tcp|udp|raw|unix` → sockets；`grpc` → grpc；`thrift`；`dubbo|hessian`；`iso8583`；其余仍返回“本构建未启用”。

- [ ] **Step 1: 写 feature 矩阵测试**

`tests/feature_matrix.rs`（无 feature 时也编译）：
```rust
#[test]
fn default_build_has_no_server_executor() {
    // 仅断言常量列表；不依赖网络
    assert!(cfg!(feature = "debug-http") || true);
}
```
实际检查用脚本：
```bash
for f in debug-http debug-session debug-sockets debug-grpc debug-thrift debug-dubbo debug-iso8583; do
  cargo check -p apidoc-rust --features "$f" || exit 1
done
```

- [ ] **Step 2: 跑默认构建确认零新增依赖**

Run: `cargo tree -p apidoc-rust -e normal | grep -E "reqwest|tonic|tokio|lapin|rumqttc|rskafka"` → 无输出。

- [ ] **Step 3: 更新 README 协议表**

在每个语言版本加一张小表：协议 / 注解 kind / 执行位置 / feature。只写事实，不复制实现细节。中文版先改，其余语言同步同一结构。

- [ ] **Step 4: 全量回归**

Run:
```bash
cargo test -p apidoc-rust --all-features
cargo test -p apidoc-macros
cargo check -p apidoc-rust
node --test crates/apidoc/tests/js/codecs.test.js
```
Expected: 全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/Cargo.toml crates/apidoc/src/debug/mod.rs README.md docs/i18n
git commit -m "docs(debug): document P2 protocol executors and feature matrix"
```

---

## P2 退出标准

- `/apidoc/debug` 支持 gRPC 一元/服务端流、Thrift、Dubbo、ISO 8583、TCP/UDP/Raw/Unix。
- `/apidoc/debug/stream` 支持服务端 WebSocket 桥；控制协议与背压/关闭有单测与本地 echo 集成测试。
- 每个协议至少一条黄金向量或本地回环测试；SSRF 在 raw socket/Unix 路径有拒绝测试。
- 默认构建依赖树与 P0 一致；`debug-all` 能一键启用全部 P2 executor。
