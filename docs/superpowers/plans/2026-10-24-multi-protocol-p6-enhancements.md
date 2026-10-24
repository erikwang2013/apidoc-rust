# P6 增强与收敛 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 补齐 Webhook 入站捕获、IDL 导入（.proto / .thrift）、Dubbo Nacos 注册中心、非 HTTP mock、AsyncAPI 导出与全量文档，完成 2.1.0 发版准备。

**Architecture:** 新增独立 crate `apidoc-idl`（CLI，不进入默认依赖）生成注解代码；Webhook 捕获作为 `Core` 内的有界环形缓冲；AsyncAPI 导出与 OpenAPI 并列；协议 mock 优先在 UI 侧从注解 `mock` 预填，不新增服务端协议 mock 路由。

**Tech Stack:** `protox`（纯 Rust proto 解析）、手写 Thrift IDL 解析器、Nacos HTTP Open API；不引入 ZooKeeper 客户端。

**Spec:** [总纲](./2026-10-24-multi-protocol-master.md)；P0–P5 计划见同目录。

## 全局约束（本阶段）

- 默认构建零新增依赖；`apidoc-idl` 是独立 crate，仅作为 CLI/可选依赖。
- Webhook 捕获路由是**公开写入**端点：必须有大小上限、环形缓冲上限、可选 HMAC、不可猜测 id；不得记录 PII 到日志。
- IDL 导入不得访问网络、不展开远程 import、限制文件大小与递归深度。
- 导出产物必须可被标准解析器解析；快照测试锁定。
- 发版前版本号统一（`VERSION`、workspace version、三处 path 依赖），README 各语言同步。

## Review Focus（本阶段）

1. Webhook 捕获被当作开放存储滥用（刷爆内存/磁盘）；期望有界 + 可选 HMAC + 限流。
2. IDL 解析器路径穿越/无限递归/超大文件；期望只读显式路径、深度与大小上限。
3. 生成代码快照漂移（protox 版本变化）；期望固定版本并快照。
4. AsyncAPI 导出把 HTTP 端点重复写入或字段非法；期望只导出非 HTTP 协议并通过 JSON Schema 校验。
5. 版本号不同步导致 crates.io 发布失败；期望发版前检查脚本。

---

## 文件结构

```
crates/apidoc-idl/              # 新 crate（CLI + lib）
  src/lib.rs                    # proto/thrift → 注解代码
  src/proto.rs
  src/thrift_idl.rs
  src/main.rs
  tests/snapshots/
crates/apidoc/src/
  webhook.rs                    # 新建：捕获缓冲 + 路由处理
  export/asyncapi.rs            # 新建
  export/mod.rs                 # 修改
  route_core.rs                 # 修改：webhook/asyncapi 路由
  axum.rs / actix.rs            # 修改
  ui.protocols.js               # 修改：mock 预填/示例
crates/apidoc/tests/
  webhook_capture.rs
  asyncapi_export.rs
  idl_import.rs
```

---

### Task 1: Webhook 入站捕获

**Files:**
- Create: `crates/apidoc/src/webhook.rs`
- Modify: `crates/apidoc/src/lib.rs`（`WebhookConfig`）
- Modify: `crates/apidoc/src/route_core.rs`
- Modify: `crates/apidoc/src/axum.rs`、`actix.rs`
- Create: `crates/apidoc/tests/webhook_capture.rs`

**Interfaces:**
- `ApidocConfig.webhooks: Vec<WebhookConfig>`，`#[serde(skip)]`。
```rust
pub struct WebhookConfig {
    pub id: String,            // 不可猜测，建议 32 hex
    pub title: String,
    pub secret: Option<String>,// HMAC-SHA256 校验 X-Apidoc-Signature
    pub max_body: usize,       // 默认 64 KiB
    pub capacity: usize,       // 默认 50 条
}
```
- `POST /apidoc/webhook/{id}`：公开；校验 id 存在、body ≤ `max_body`、可选 HMAC（hex HMAC-SHA256(body) 与 `X-Apidoc-Signature` 恒定时间比较）；存入环形缓冲；返回 202。
- `GET /apidoc/webhook/{id}?since=<seq>`：走 `Core::guard`；返回 `{"events":[{"seq","ts","headers","body":{encoding,data}}...]}`。
- `GET /apidoc/webhook`：走 guard；列出配置的 id/title（不含 secret）。

- [ ] **Step 1: 写失败测试**

`tests/webhook_capture.rs`（axum）：
- 配置一个 id/secret；POST 一条 body，断言 202；GET 列表（带 token）读到该事件。
- 错误签名返回 401；超大 body 返回 413；未知 id 返回 404。
- `since` 过滤：先两条，再 `since=1` 只返回第二条。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features axum --test webhook_capture`
Expected: FAIL。

- [ ] **Step 3: 实现**

- `Core` 新增 `webhooks: Vec<WebhookRuntime>`，每个含 `Mutex<VecDeque<StoredEvent>>` + 原子 seq。
- 路由在 axum/actix 各加两条（POST 公开、GET guard）。
- HMAC 复用 `auth` 的 `hmac`/`sha2` 依赖；恒定时间比较。
- 响应与错误不记录 body；日志只记 id/seq/大小。

- [ ] **Step 4: UI 面板与手工验证**

`webhook` 协议的调试面板增加「入站捕获」区域：展示 `GET /apidoc/webhook/{id}` 的事件（id 来自 `ep.protocol_opts.webhook_id`），支持手动刷新。结构测试断言 `webhookPanel`。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features axum --test webhook_capture && cargo test -p apidoc-rust --test ui_events`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add crates/apidoc/src/webhook.rs crates/apidoc/src/lib.rs crates/apidoc/src/route_core.rs crates/apidoc/src/axum.rs crates/apidoc/src/actix.rs crates/apidoc/src/ui.protocols.js crates/apidoc/tests/webhook_capture.rs crates/apidoc/tests/ui_events.rs
git commit -m "feat(debug): add bounded webhook capture"
```

---

### Task 2: 非 HTTP 协议 mock / 示例预填

**Files:**
- Modify: `crates/apidoc/src/ui.protocols.js`
- Modify: `crates/apidoc/src/ui.debug.stream.js`
- Modify: `crates/apidoc/tests/ui_events.rs`

**Interfaces:**
- `mockFromEvents(events, direction): Array<{name, payload}>`：从 `ep.events[].mock` 读取示例；缺失时按 `ty` 生成（json→`{}`，text→`"string"`，binary→`"00"`）。
- 流式面板的发送区增加「示例」下拉，选中后填入 textarea（编码按事件类型）。
- 一元协议沿用 `ep.params[].mock`（P0 已有）。

- [ ] **Step 1: 写失败测试**

Node 测试：
```js
test('mockFromEvents filters direction and falls back by type', () => {
  const evs = [{direction:'out',name:'join',type:'json',mock:'{"room":1}'},{direction:'in',name:'msg',type:'text'}];
  assert.deepEqual(c.mockFromEvents(evs, 'out'), [{name:'join', payload:{encoding:'utf8', data:'{"room":1}'}}]);
  assert.deepEqual(c.mockFromEvents(evs, 'in'), [{name:'msg', payload:{encoding:'utf8', data:'string'}}]);
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `node --test crates/apidoc/tests/js/codecs.test.js`
Expected: FAIL。

- [ ] **Step 3: 实现并接入面板**

`mockFromEvents` 放 `ui.codecs.js`（纯函数）；`ui.debug.stream.js` 增加下拉与填充。二进制示例按 hex 编码。

- [ ] **Step 4: 跑测试确认通过**

Run: `node --test crates/apidoc/tests/js/codecs.test.js && cargo test -p apidoc-rust --test ui_events`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/ui.codecs.js crates/apidoc/src/ui.protocols.js crates/apidoc/src/ui.debug.stream.js crates/apidoc/tests/js/codecs.test.js crates/apidoc/tests/ui_events.rs
git commit -m "feat(debug): prefill stream examples from annotations"
```

---

### Task 3: `.proto` IDL 导入

**Files:**
- Create: `crates/apidoc-idl/Cargo.toml`
- Create: `crates/apidoc-idl/src/lib.rs`
- Create: `crates/apidoc-idl/src/proto.rs`
- Create: `crates/apidoc-idl/src/main.rs`
- Create: `crates/apidoc-idl/tests/idl_import.rs`
- Modify: workspace `Cargo.toml`（members，**不**加入 apidoc 依赖）

**Interfaces:**
- CLI：`apidoc-idl proto --in service.proto --out generated.rs [--package helloworld]`
- `proto::generate(source: &str, package: Option<&str>) -> Result<String, String>`：
  - 用 `protox` 解析；不展开网络 import；单文件 ≤ 1 MiB。
  - 每个 `service` 生成一个函数：
```rust
#[allow(dead_code)]
#[apidoc::title("SayHello")]
#[apidoc::protocol("grpc", target = "http://127.0.0.1:50051", service = "helloworld.Greeter", method = "SayHello")]
#[apidoc::param(name = "name", ty = "string", field = "1", required)]
#[apidoc::returned(name = "message", ty = "string", field = "1")]
fn say_hello() {}
```
  - message → param/returned；嵌套 message 用 `children`；repeated 用 `ty="array"`；enum 用 `ty="enum"`；bytes 用 `ty="bytes"`。
- 生成文件不包含 `main`，由用户 `include!` 或复制。

- [ ] **Step 1: 写快照测试**

`tests/idl_import.rs` 读取 `fixtures/helloworld.proto`，断言输出包含精确的 service/method/param 行；未知类型（map/oneof）输出注释 `// TODO: map/oneof unsupported` 而不是 panic。再加一条安全测试：含 `import "http://evil/x.proto";` 或超过 1 MiB 的文件必须返回错误，不得联网或读入。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-idl`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 新增 workspace member（workspace members 不参与 `apidoc` 默认依赖）。
- `protox::compile` 或 `protox::parse` 取 `FileDescriptorProto`；递归 message 生成字段。
- 字段号直接用 descriptor 的 number；类型映射表固定。
- 输出格式稳定（排序：文件内 service 声明序、method 声明序）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-idl`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml crates/apidoc-idl
git commit -m "feat(idl): generate apidoc annotations from proto files"
```

---

### Task 4: `.thrift` IDL 导入

**Files:**
- Create: `crates/apidoc-idl/src/thrift_idl.rs`
- Create: `crates/apidoc-idl/tests/fixtures/service.thrift`
- Modify: `crates/apidoc-idl/tests/idl_import.rs`

**Interfaces:**
- CLI：`apidoc-idl thrift --in service.thrift --out generated.rs`
- `thrift_idl::generate(source: &str) -> Result<String, String>`：
  - 支持 `namespace`、`struct`、`enum`、`service`、`typedef`、`list<T>`、`map<K,V>`、可选字段 `optional`/`required`。
  - 生成 `#[apidoc::protocol("thrift", target = "tcp://127.0.0.1:9090", service = "...", method = "...")]` + `param(field="N")`。
  - 不解析 `include`/`cpp_include`；遇到时输出注释并继续。

- [ ] **Step 1: 写快照测试**

fixture 含一个 service、两个 struct、一个 enum、一个 list 字段；断言生成的字段号、类型、required 与 snapshot 一致。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-idl`
Expected: FAIL。

- [ ] **Step 3: 实现解析器**

手写 tokenizer + 递归下降；限制嵌套深度 64、文件 1 MiB；错误带行号。类型映射：`i32`→`i32`、`string`→`string`、`list<T>`→`array`、`map`→`object`、struct→`object` + children。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-idl`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc-idl/src/thrift_idl.rs crates/apidoc-idl/tests
git commit -m "feat(idl): generate apidoc annotations from thrift idl"
```

---

### Task 5: Dubbo Nacos 注册中心（仅 HTTP Open API）

**Files:**
- Modify: `crates/apidoc/src/debug/dubbo.rs`
- Create: `crates/apidoc/tests/debug_dubbo_registry.rs`

**Interfaces:**
- `opts.registry`：`nacos://host:port?namespace=...&group=...&service=...`。
- 解析后调用 Nacos Open API `GET /nacos/v1/ns/instance/list?serviceName=...&groupName=...&namespaceId=...`，取第一个 `healthy=true` 的 `ip:port` 作为 provider。
- 不支持 ZooKeeper；遇到 `zookeeper://` 返回明确错误。
- 保留 `opts.target` 直连优先。

- [ ] **Step 1: 写失败测试**

本地 HTTP mock 返回 Nacos JSON；`dubbo::resolve_provider` 断言得到 `ip:port`。再测 `healthy=false` 跳过、无实例报错、`zookeeper://` 报错。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-dubbo --test debug_dubbo_registry`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 解析 `registry` URL；`guard::check_host` 校验 Nacos host（允许私网需配置）。
- HTTP GET + JSON 解析；超时 3s；响应大小上限 `max_bytes`。
- 选择策略：第一个 healthy；记录选择结果到 `DebugResponse.headers["x-dubbo-provider"]`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-dubbo --test debug_dubbo_registry`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/dubbo.rs crates/apidoc/tests/debug_dubbo_registry.rs
git commit -m "feat(debug): resolve dubbo providers via nacos http api"
```

---

### Task 6: AsyncAPI 导出

**Files:**
- Create: `crates/apidoc/src/export/asyncapi.rs`
- Modify: `crates/apidoc/src/export/mod.rs`
- Modify: `crates/apidoc/src/route_core.rs`（`format=asyncapi`）
- Create: `crates/apidoc/tests/asyncapi_export.rs`

**Interfaces:**
- `asyncapi::render(doc: &ApiDoc, version: &str) -> serde_json::Value`。
- 输出 AsyncAPI 2.6：`asyncapi: "2.6.0"`、`info`、`channels`。
- 只导出非 HTTP 协议（`websocket`/`ws-binary`/`sse`/`socketio`/`stomp`/`stomp-binary`/`mqtt`/`amqp`/`kafka`/`webhook`）。
- channel 名：`ep.protocol_opts.topic` 或 `ep.protocol_opts.target` 或 `ep.url`；`publish` 对应 `direction:"in"`？约定：
  - `ep.events` 中 `direction:"out"` = 客户端发送 → AsyncAPI `publish`；
  - `direction:"in"` = 客户端接收 → AsyncAPI `subscribe`。
- 消息 payload 用 `ep.params`/`returned` 生成 JSON Schema（复用 swagger 的 schema 函数；若不可复用则提取到 `export/schema.rs`）。
- `GET /apidoc/export?format=asyncapi` → `application/json`。

- [ ] **Step 1: 写快照测试**

构造一个 WS endpoint（两个 events）与一个 MQTT endpoint；断言 `channels` 名字、publish/subscribe 方向、payload schema；HTTP endpoint 不出现。用 `serde_json` 快照。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --test asyncapi_export`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 提取/复用 swagger 的 `schema_for` 到 `export/schema.rs`（若已有私有函数，改为 `pub(crate)`）。
- 生成 AsyncAPI 结构；未知 kind 跳过并 eprintln。
- 路由新增 format 分支与 Content-Type。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --test asyncapi_export && cargo test -p apidoc-rust --all-features`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/export/asyncapi.rs crates/apidoc/src/export/mod.rs crates/apidoc/src/export/schema.rs crates/apidoc/src/route_core.rs crates/apidoc/tests/asyncapi_export.rs
git commit -m "feat(export): add asyncapi export for streaming protocols"
```

---

### Task 7: 2.1.0 发版准备

**Files:**
- Modify: `VERSION`（2.1.0）
- Modify: `Cargo.toml`（workspace version）
- Modify: `crates/apidoc/Cargo.toml`（`apidoc-macros` path 依赖 version）
- Modify: `crates/apidoc-test-fixtures/Cargo.toml`（`apidoc-rust` path 依赖 version）
- Modify: `README.md` 与 `docs/i18n/README-*.md`（协议表、版本号、VERSION 注释）
- Modify: `.github/workflows/release.yml`（如需）

**Interfaces:** 无代码接口；这是发布一致性任务。

- [ ] **Step 1: 版本一致性检查脚本**

```bash
grep -R "2\.0\.0" --include=Cargo.toml --include=VERSION . | grep -v target
```
Expected: 只出现在需要改的位置；逐一改为 2.1.0。

- [ ] **Step 2: README 各语言同步**

中文 README 增加「多协议调试」章节：注解示例、feature 列表、安全默认、浏览器/服务端分工、已支持协议表。其余语言按同一结构翻译（可分批，但发版前必须完成）。

- [ ] **Step 3: 全量回归**

```bash
cargo test -p apidoc-rust --all-features
cargo test -p apidoc-macros
cargo test -p apidoc-idl
node --test crates/apidoc/tests/js/codecs.test.js
cargo check -p apidoc-rust
cargo packaging -p apidoc-rust --allow-dirty --no-verify 2>/dev/null || cargo package -p apidoc-rust --allow-dirty --no-verify
```
Expected: 全部 PASS；默认依赖树无新增重型依赖。

- [ ] **Step 4: 提交**

```bash
git add VERSION Cargo.toml crates/apidoc/Cargo.toml crates/apidoc-test-fixtures/Cargo.toml README.md docs/i18n .github
git commit -m "chore(release): 2.1.0 multi-protocol debug"
```

- [ ] **Step 5: 发版（用户确认后）**

按项目记忆 `release-flow-apidoc-rust`：先发 `apidoc-macros`，再发 `apidoc-rust`，最后打 tag 触发 release。不要在本计划内自动推送 tag。

---

## P6 退出标准

- Webhook 捕获有界、可选 HMAC、路由鉴权正确；UI 可查看入站事件。
- `apidoc-idl` 能从 proto/thrift 生成可编译的注解代码，快照稳定。
- Dubbo Nacos 直连可用；ZooKeeper 明确不支持。
- AsyncAPI 2.6 导出通过快照与解析校验；HTTP 端点不重复出现。
- 版本号、README、crate 依赖版本一致，发版流程就绪。
