# P3 消息中间件 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 服务端调试 MQTT、AMQP 0-9-1、Kafka、XMPP，以及 STOMP over TCP；复用 P2 的 `DebugSocket` 会话桥展示持续消息。

**Architecture:** 每个 broker 一个 `debug/<proto>.rs`，暴露 `execute`（一次性 connect→操作→收集 N 条/到时→断开）和可选的 `connect_stream`（长连接，经 `/apidoc/debug/stream`）。连接参数只放在 `DebugRequest.opts`，不落盘、不进 api.json、不写日志。

**Tech Stack:** `rumqttc`（MQTT）、`lapin`（AMQP）、`rskafka`（Kafka，纯 Rust）、`tokio-xmpp`（XMPP）、手写 STOMP 1.2 帧编解码。

**Spec:** [总纲](./2026-10-24-multi-protocol-master.md)；P2 会话桥见 `crates/apidoc/src/debug/session.rs`。

## 全局约束（本阶段）

- 凭据（username/password/token）只从请求 `opts` 读取；错误消息不得回显；不写日志。
- 每条连接都有 `connect_timeout`、操作超时、`max_frames`/`max_bytes`；到量即断开。
- TLS 一律 rustls；不提供“跳过证书校验”开关。
- 默认构建零新增依赖；broker 集成测试用 env var 或本地 dev 依赖，不能拖慢默认测试。
- 所有目标先过 `guard::check_host`；`allow_private_networks` 控制内网 broker。

## Review Focus（本阶段）

1. 凭据泄漏到错误/日志/api.json。
2. 消费无界（Kafka 分区无限流、MQTT 持续推送）导致内存增长；期望 `max_frames` 与 `duration_ms` 双重上限。
3. 断开/重连竞态导致任务泄漏（尤其是 MQTT event loop 与 AMQP consumer）。
4. 消息体二进制被 UTF-8 化；期望 `Payload` 往返。
5. 本地 broker 测试在无服务环境下被误判为失败；期望 env-gated 或明确 skip。

---

## 文件结构

```
crates/apidoc/src/debug/
  mqtt.rs        # 新建
  amqp.rs        # 新建
  kafka.rs       # 新建
  xmpp.rs        # 新建
  stomp.rs       # 新建（Rust 版，TCP；WS 版在 P1 的 JS）
  mod.rs         # 修改：分发
crates/apidoc/tests/
  debug_mqtt.rs
  debug_amqp.rs
  debug_kafka.rs
  debug_xmpp.rs
  debug_stomp.rs
```

---

### Task 1: MQTT

**Files:**
- Create: `crates/apidoc/src/debug/mqtt.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`
- Create: `crates/apidoc/tests/debug_mqtt.rs`

**Interfaces:**
- `mqtt::execute(cfg, req, ep) -> Result<DebugResponse, String>`。
- `opts`：`topic`（必填）、`qos`（0/1/2，默认 0）、`client_id`、`username`、`password`、`messages`（默认 1）、`duration_ms`（默认 2000）。
- 流程：连接 → 订阅 topic →（若 body 非空）publish 到 topic → 收集 `messages` 条或到 `duration_ms` → 断开。
- 响应 `body` 为 `{"messages":[{"topic","payload":{encoding,data},"qos"}...]}`。
- feature：`debug-mqtt = ["debug-session", "dep:rumqttc"]`；`rumqttc` 开 `use-rustls`。
- dev-dependency：`rumqttd`（仅测试）。

- [ ] **Step 1: 写失败测试**

`tests/debug_mqtt.rs` 用 `rumqttd` 在 `127.0.0.1:0` 起 broker；调用 `execute` 订阅 `test/topic`，同时用 `rumqttc` 另一个客户端 publish `"hello"`；断言响应 JSON 里有一条 `payload.data == "hello"`。再测超时：无消息时 `duration_ms=100` 返回空数组而不是报错。最后断言认证失败时错误文本不含 `opts.password` 原文。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-mqtt --test debug_mqtt`
Expected: FAIL — module 不存在。

- [ ] **Step 3: 实现**

- `MqttOptions::new(client_id, host, port)`；设置 keep_alive、credentials；`AsyncClient` + `EventLoop`。
- 先 `subscribe`，再 `publish`；event loop 里匹配 `Event::Incoming(Packet::Publish)`，按上限收集。
- 到量/超时后 `disconnect`，abort event loop task。
- 错误映射：连接拒绝、认证失败、topic 非法。
- 凭据只来自 `opts`；`DebugResponse` 不回显。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-mqtt --test debug_mqtt`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/mqtt.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_mqtt.rs
git commit -m "feat(debug): add mqtt executor"
```

---

### Task 2: AMQP 0-9-1

**Files:**
- Create: `crates/apidoc/src/debug/amqp.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`
- Create: `crates/apidoc/tests/debug_amqp.rs`

**Interfaces:**
- `amqp::execute(cfg, req, ep)`。
- `opts`：`exchange`（默认 `""`）、`routing_key`、`queue`（消费时必填）、`consume`（`"true"`/`"false"`）、`username`、`password`、`vhost`（默认 `/`）、`messages`（默认 1）、`duration_ms`（默认 2000）。
- 响应 `body`：`{"delivery":{"exchange","routing_key","payload":{encoding,data}}}` 或 `{"published":true}`。
- feature：`debug-amqp = ["debug-http", "dep:lapin"]`；`lapin` 开 rustls。

- [ ] **Step 1: 写失败测试（env-gated）**

```rust
#[tokio::test]
async fn amqp_publish_and_consume() {
    let Ok(url) = std::env::var("APIDOC_RABBITMQ_URL") else {
        eprintln!("skipping: APIDOC_RABBITMQ_URL not set");
        return;
    };
    // 连接、声明临时队列、publish 一条、consume 一条，断言 payload
}
```
另加一个不需要 broker 的配置解析单测：`opts` 缺 `queue` 且 `consume=true` 时返回明确错误。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-amqp --test debug_amqp`
Expected: FAIL（编译或配置错误断言不满足）。

- [ ] **Step 3: 实现**

- `lapin::Connection::connect(url.into(), ConnectionProperties::default())`；URL 由 target + `vhost` 组装。
- publish：`basic_publish(exchange, routing_key, options, payload)`。
- consume：声明队列（若 `opts.declare=true`）、`basic_consume`、等待 `messages` 条或超时。
- 取消 consumer、关闭 channel/connection；错误映射不包含 URL 中的凭据。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p apidoc-rust --features debug-amqp --test debug_amqp`
Expected: 无 env 时 skip；有 broker 时 PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/amqp.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_amqp.rs
git commit -m "feat(debug): add amqp executor"
```

---

### Task 3: Kafka

**Files:**
- Create: `crates/apidoc/src/debug/kafka.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`
- Create: `crates/apidoc/tests/debug_kafka.rs`

**Interfaces:**
- `kafka::execute(cfg, req, ep)`。
- `opts`：`topic`（必填）、`partition`（默认 0）、`offset`（`"earliest"`/`"latest"`/数字，默认 latest）、`messages`（默认 1）、`duration_ms`（默认 2000）。
- 响应 `body`：`{"messages":[{"partition","offset","payload":{encoding,data}}...]}`。
- feature：`debug-kafka = ["debug-http", "dep:rskafka"]`（禁止 `rdkafka`，避免 C 依赖）。

- [ ] **Step 1: 写失败测试**

- 配置解析单测：缺 `topic` 返回 400 类错误。
- env-gated 集成测试：`APIDOC_KAFKA_BROKERS=127.0.0.1:9092`；produce 一条到随机 topic，再 fetch 回来，断言 payload。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-kafka --test debug_kafka`
Expected: FAIL。

- [ ] **Step 3: 实现**

- `rskafka::ClientBuilder::new(brokers).build()`；取 partition client。
- Produce：`record_batch` 构造一条记录；Fetch：从 offset 起拉取，解码 record batch，按上限收集。
- 错误映射：topic 不存在、认证失败、broker 不可达。
- 注意 offset 类型（i64）、空 batch。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p apidoc-rust --features debug-kafka --test debug_kafka`
Expected: 无 env 时 skip；有 broker 时 PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/kafka.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_kafka.rs
git commit -m "feat(debug): add kafka executor via rskafka"
```

---

### Task 4: XMPP

**Files:**
- Create: `crates/apidoc/src/debug/xmpp.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`
- Create: `crates/apidoc/tests/debug_xmpp.rs`

**Interfaces:**
- `xmpp::execute(cfg, req, ep)`。
- `opts`：`jid`、`password`、`to`、`type`（`chat`/`groupchat`，默认 `chat`）、`messages`（默认 1）、`duration_ms`。
- 响应：`{"messages":[{"from","body"}...]}`。
- feature：`debug-xmpp = ["debug-session", "dep:tokio-xmpp"]`。
- **先 spike**：确认 `tokio-xmpp` 支持 Rust 1.82 且能完成 SASL PLAIN/TLS；不满足则本任务降级为“返回明确未支持错误”，并从 P3 退出标准中移除，记入 P6。

- [ ] **Step 1: 写编译级 spike 测试**

`tests/debug_xmpp.rs`：不连网，断言 `xmpp::execute` 缺 `jid`/`password` 时返回配置错误；再尝试构造 client（不 connect）确认依赖 API 可用。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-xmpp --test debug_xmpp`
Expected: FAIL。

- [ ] **Step 3: 实现（或降级）**

- `tokio_xmpp::Client::new(jid, password)` → `connect`（DNS SRV 或 target host:port）→ `send` stanza → 收集 `messages` 条或超时 → 关闭。
- 组包用 `xmpp-parser` 的 `Element`。
- 若 spike 失败：`execute` 返回 `Err("xmpp executor unavailable: tokio-xmpp incompatible with MSRV")`，测试断言该错误，P6 再评估。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-xmpp --test debug_xmpp`
Expected: PASS（配置错误/降级错误）。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/xmpp.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_xmpp.rs
git commit -m "feat(debug): add xmpp executor with msrv spike"
```

---

### Task 5: STOMP over TCP（Rust）

**Files:**
- Create: `crates/apidoc/src/debug/stomp.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Create: `crates/apidoc/tests/debug_stomp.rs`

**Interfaces:**
- `stomp::execute(cfg, req, ep)`，kind `stomp` 且 `opts.transport = "tcp"`（默认）；WS 版由 P1 浏览器适配器负责。
- `opts`：`destination`、`login`、`passcode`、`messages`、`duration_ms`。
- 流程：CONNECT → 等 CONNECTED → SUBSCRIBE →（若 body）SEND → 收集 MESSAGE → DISCONNECT。
- 复用 P1 的帧语义：`\0` 分帧、`content-length` 决定二进制 body、头转义 `\n`/`\c`/`\\`。

- [ ] **Step 1: 写失败测试**

本地 `TcpListener` 实现最小 STOMP server：读 CONNECT 回 CONNECTED；读 SUBSCRIBE 后推送一条 MESSAGE（含 `content-length` 与 UTF-8 body）；断言 `execute` 返回该 body。再加一条二进制 body（非 UTF-8 字节）断言 base64 编码。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-sockets --test debug_stomp`
Expected: FAIL。

- [ ] **Step 3: 实现**

- Rust 版 `stomp_frame` 编解码（与 `ui.codecs.js` 语义一致；两边各有测试）。
- 读循环按 `\0` 切帧；`content-length` 存在时按字节读取。
- 头转义解码；未知命令忽略并记录。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-sockets --test debug_stomp`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/stomp.rs crates/apidoc/src/debug/mod.rs crates/apidoc/tests/debug_stomp.rs
git commit -m "feat(debug): add stomp tcp executor"
```

---

### Task 6: feature 收口与集成测试约定

**Files:**
- Modify: `crates/apidoc/Cargo.toml`（`debug-all` 加入 P3 features）
- Modify: `README.md`（broker 调试说明与 env var 列表）
- Modify: `crates/apidoc/tests/common/`（如有共享 broker 启动辅助）

**Interfaces:**
- `debug-all += ["debug-mqtt","debug-amqp","debug-kafka","debug-xmpp"]`。
- 集成测试约定：
  - `APIDOC_RABBITMQ_URL`、`APIDOC_KAFKA_BROKERS`、`APIDOC_XMPP_TARGET`、`APIDOC_XMPP_JID`、`APIDOC_XMPP_PASSWORD`。
  - 未设置时打印 `skipping: <env> not set` 并返回，不 panic。
  - CI 若没有 broker，只跑编译与编解码单测。

- [ ] **Step 1: 验证 feature 组合**

```bash
cargo check -p apidoc-rust --features debug-mqtt
cargo check -p apidoc-rust --features debug-amqp
cargo check -p apidoc-rust --features debug-kafka
cargo check -p apidoc-rust --features debug-xmpp
cargo check -p apidoc-rust --features debug-all
```

- [ ] **Step 2: 跑默认构建确认零新增依赖**

Run: `cargo tree -p apidoc-rust -e normal | grep -E "rumqttc|lapin|rskafka|tokio-xmpp"` → 无输出。

- [ ] **Step 3: 全量回归**

Run: `cargo test -p apidoc-rust --all-features && cargo test -p apidoc-macros`
Expected: PASS（无 broker 时相关测试 skip）。

- [ ] **Step 4: 更新 README 并提交**

```bash
git add crates/apidoc/Cargo.toml README.md crates/apidoc/tests
git commit -m "docs(debug): document messaging executors and broker test env vars"
```

---

## P3 退出标准

- MQTT 有本地 broker 测试；AMQP/Kafka 有 env-gated 集成测试；XMPP 有 spike 结论（实现或明确降级）。
- 所有消费路径受 `messages`/`duration_ms`/`max_bytes` 限制。
- 凭据不出现在错误文本、日志或 api.json。
- `debug-all` 可一键启用 P2+P3 executor；默认构建依赖树不变。
