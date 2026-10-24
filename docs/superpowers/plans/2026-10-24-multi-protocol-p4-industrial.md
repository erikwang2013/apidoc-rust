# P4 IoT / 工业协议 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 服务端调试 CoAP、LwM2M、Modbus TCP、DNP3、IEC 60870-5-104、BACnet/IP、OPC UA Binary；覆盖读/写/发现/轮询等最常见操作，不做完整协议栈。

**Architecture:** 复用 P2 的 `guard::check_host`、`sockets` 超时/上限工具与 `DebugResponse`。每个协议一个 `debug/<proto>.rs`；报文编解码优先手写（可测试、无 C 依赖），必要时用纯 Rust crate（`coap-lite`、`tokio-modbus`、`opcua`）并先做 MSRV spike。

**Tech Stack:** tokio UDP/TCP、`coap-lite`、`tokio-modbus`、`opcua`；DNP3/IEC104/BACnet 视 spike 结果使用 crate 或手写子集。

**Spec:** [总纲](./2026-10-24-multi-protocol-master.md)；P2 sockets 见 `crates/apidoc/src/debug/sockets.rs`。

## 全局约束（本阶段）

- 工业协议默认端口固定（CoAP 5683/5684、Modbus 502、DNP3 20000、IEC104 2404、BACnet 47808、OPC UA 4840）；`target` 必须显式给 host:port。
- 所有读写有超时（默认 3000ms）；无响应返回明确错误，不无限等待。
- 广播（BACnet Who-Is）默认禁止；需 `allow_private_networks=true` 且 `opts.broadcast=true`，仍拒绝全局广播地址。
- 不做设备发现扫描（不遍历网段）；只对用户给定的单目标操作。
- 报文编解码不得依赖系统库；纯 Rust。

## Review Focus（本阶段）

1. 协议帧长度/字节序错误（Modbus 寄存器大端、DNP3 CRC、IEC104 APDU 长度、BACnet BVLC）。
2. 无响应/半包导致挂死；期望读超时与帧上限。
3. 广播/组播绕过 SSRF 守卫；期望显式开关 + 私网允许 + 拒绝 255.255.255.255/组播。
4. 写入类操作误触生产设备；期望 UI 与文档标注“写操作”，默认只读示例。
5. 资源泄漏（未关闭 TCP/UDP、未取消订阅）。

---

## 文件结构

```
crates/apidoc/src/debug/
  coap.rs
  lwm2m.rs
  modbus.rs
  dnp3.rs
  iec104.rs
  bacnet.rs
  opcua.rs
  mod.rs
crates/apidoc/tests/
  debug_coap.rs
  debug_lwm2m.rs
  debug_modbus.rs
  debug_dnp3.rs
  debug_iec104.rs
  debug_bacnet.rs
  debug_opcua.rs
```

---

### Task 1: CoAP

**Files:**
- Create: `crates/apidoc/src/debug/coap.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`debug-coap = ["debug-http", "dep:coap-lite"]`）
- Create: `crates/apidoc/tests/debug_coap.rs`

**Interfaces:**
- `coap::execute(cfg, req, ep)`。
- `opts`：`method`（`GET`/`POST`/`PUT`/`DELETE`，默认 GET）、`path`（默认 `/`）、`content_format`（默认 `text/plain`）、`observe`（`"true"` 时收 N 个通知）、`messages`（默认 1）、`duration_ms`（默认 3000）。
- `target`：`coap://host:port` 或 `host:port`（默认 5683；`coaps` 未实现，返回明确错误）。
- 响应：`{"code":"2.05","payload":{encoding,data},"options":{...}}`。

- [ ] **Step 1: 写失败测试**

本地 UDP CoAP server：解析请求，回 `2.05 Content` + `"ok"`。断言 `execute` 返回 code `2.05` 与 payload `"ok"`。再加超时测试：无 server 时 100ms 内返回错误。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-coap --test debug_coap`
Expected: FAIL。

- [ ] **Step 3: 实现**

- `guard::check_host` 校验目标；UDP bind 0.0.0.0:0。
- `coap_lite::Request` 构造 method/path/options/payload；`Message::from(request)` 编码；发送后按 `duration_ms` 收包。
- 解析 `Response`，code 转 `"2.05"` 字符串；payload 用 `Payload::from_bytes`。
- Observe：设置 Observe option 0，收到 N 条或超时后发送 RST/取消。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-coap --test debug_coap`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/coap.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_coap.rs
git commit -m "feat(debug): add coap executor"
```

---

### Task 2: LwM2M

**Files:**
- Create: `crates/apidoc/src/debug/lwm2m.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Create: `crates/apidoc/tests/debug_lwm2m.rs`

**Interfaces:**
- `lwm2m::execute(cfg, req, ep)`。
- `opts`：`endpoint`（LwM2M client name）、`operation`（`register`/`update`/`deregister`/`read`/`write`）、`object_id`、`instance_id`、`resource_id`、`value`、`lifetime`（默认 300）。
- target：LwM2M server 的 CoAP host:port。
- 响应：`{"code":"2.01","location":"/rd/1"}` 等。
- 只实现注册/更新/注销与单资源读写；不做 Bootstrap/固件升级。

- [ ] **Step 1: 写失败测试**

本地 CoAP server 对 `POST /rd?ep=...` 回 `2.01 Created` + `Location-Path: /rd/1`；断言 `execute` 返回该 location。TLV 编解码单测：一个整数资源 0/1/2 字节三种编码。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-coap --test debug_lwm2m`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 复用 `coap.rs` 的发送/接收 helper（提取 `coap::exchange`）。
- Register：`POST /rd?ep=&lt=&lwm2m=1.1&b=U`，Content-Format `application/vnd.oma.lwm2m+tlv`。
- Update：`POST /rd/{id}`；Deregister：`DELETE /rd/{id}`。
- Read/Write：`GET/PUT /{obj}/{inst}/{res}`，TLV 编解码整数/字符串/布尔。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-coap --test debug_lwm2m`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/lwm2m.rs crates/apidoc/src/debug/mod.rs crates/apidoc/tests/debug_lwm2m.rs
git commit -m "feat(debug): add lwm2m register and resource operations"
```

---

### Task 3: Modbus TCP

**Files:**
- Create: `crates/apidoc/src/debug/modbus.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`debug-industrial` 先只加 `tokio-modbus`）
- Create: `crates/apidoc/tests/debug_modbus.rs`

**Interfaces:**
- `modbus::execute(cfg, req, ep)`。
- `opts`：`unit_id`（默认 1）、`operation`（`read_coils`/`read_discrete`/`read_holding`/`read_input`/`write_coil`/`write_register`）、`address`、`quantity`（读，默认 1）、`value`（写）。
- 响应：`{"values":[...]}`。
- feature：`debug-industrial = ["debug-http","dep:tokio-modbus"]`。

- [ ] **Step 1: 写失败测试**

用 `tokio-modbus` 的 server 在随机端口提供 holding registers；`execute` 读 address=0 quantity=2，断言 `values`。再测写寄存器后读回。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_modbus`
Expected: FAIL。

- [ ] **Step 3: 实现**

- `tcp::connect(SocketAddr)`；`context.set_unit_id`。
- 调用 `read_coils/read_holding_registers/...`；写操作 `write_single_coil/register`。
- 地址/数量边界校验（quantity 1..=125）；错误映射。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_modbus`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/modbus.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_modbus.rs
git commit -m "feat(debug): add modbus tcp executor"
```

---

### Task 4: DNP3

**Files:**
- Create: `crates/apidoc/src/debug/dnp3.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`dep:dnp3`）
- Create: `crates/apidoc/tests/debug_dnp3.rs`

**Interfaces:**
- `dnp3::execute(cfg, req, ep)`。
- `opts`：`link_address`（默认 1）、`operation`（`integrity`/`class0`/`class1`/`class2`/`class3`/`read`）、`start`/`stop`（read 时）、`duration_ms`（默认 3000）。
- 响应：`{"points":[{"group","variation","index","value","flags"}...]}`。
- **先 spike** `dnp3` crate（StepFun 的 `dnp3`）是否支持 Rust 1.82 与 client master；不满足则手写链路层 + 应用层 Integrity Poll 子集。

- [ ] **Step 1: 写失败测试**

若 crate 可用：用其 test server/mock 回一组 Class 0 点；断言解析。若手写：黄金向量覆盖 CRC、链路头、应用头。测试先按 crate 路径写，spike 失败再改为黄金向量。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_dnp3`
Expected: FAIL。

- [ ] **Step 3: 实现/spike**

- 连接 target（默认 20000），发送 Integrity Poll，等待响应，按 `duration_ms` 收集。
- 解析 group/variation，映射为 JSON；未知 variation 以十六进制展示。
- 明确只做只读轮询。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_dnp3`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/dnp3.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_dnp3.rs
git commit -m "feat(debug): add dnp3 integrity poll executor"
```

---

### Task 5: IEC 60870-5-104

**Files:**
- Create: `crates/apidoc/src/debug/iec104.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`
- Create: `crates/apidoc/tests/debug_iec104.rs`

**Interfaces:**
- `iec104::execute(cfg, req, ep)`。
- `opts`：`common_address`（默认 1）、`operation`（`interrogation`/`read`）、`ioa`（read 时）、`duration_ms`（默认 3000）。
- 响应：`{"asdu":[{"type_id","cause","ioa","value"}...]}`。
- **先 spike** `iec60870` crate；不满足则手写 APDU（I/S/U 帧）+ STARTDT + 总召唤子集。

- [ ] **Step 1: 写失败测试**

本地 TCP server 实现 STARTDT 确认与总召唤响应；断言 `execute` 解析出至少一个 ASDU。手写路径用黄金向量（I 帧头、ASDU 类型 100）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_iec104`
Expected: FAIL。

- [ ] **Step 3: 实现/spike**

- 连接 2404；发送 STARTDT act；等 STARTDT con；发送总召唤（C_IC_NA_1, type 100, cause 6）。
- 收集 I 帧到 `duration_ms`；解析 ASDU 类型/原因/IOA/值。
- 只读，不做遥控。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_iec104`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/iec104.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_iec104.rs
git commit -m "feat(debug): add iec104 interrogation executor"
```

---

### Task 6: BACnet/IP

**Files:**
- Create: `crates/apidoc/src/debug/bacnet.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`
- Create: `crates/apidoc/tests/debug_bacnet.rs`

**Interfaces:**
- `bacnet::execute(cfg, req, ep)`。
- `opts`：`operation`（`who_is`/`read_property`）、`device_instance`、`object_type`、`object_instance`、`property`（默认 85 `present-value`）、`broadcast`（`"true"` 仅 who_is）。
- 响应：`{"devices":[{"instance","address"}...]}` 或 `{"value":...}`。
- 广播规则：`broadcast=true` 需要 `allow_private_networks=true`，且目标必须是子网广播地址而非 255.255.255.255；否则拒绝。

- [ ] **Step 1: 写失败测试**

本地 UDP responder 解析 Who-Is 后回 I-Am；断言设备列表。ReadProperty 回一个 `present-value`（REAL），断言值。广播拒绝测试：默认配置下 `broadcast=true` 返回错误。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_bacnet`
Expected: FAIL。

- [ ] **Step 3: 实现**

- BVLC/NPDU/APDU 编解码：Who-Is (0x10 0x08)、I-Am (0x10 0x00)、ReadProperty (0x10 0x0c)。
- 跳过厂商私有扩展；只解析常见 tag（unsigned、real、boolean、character-string）。
- 广播地址校验 + 超时收集。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_bacnet`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/bacnet.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_bacnet.rs
git commit -m "feat(debug): add bacnet who-is and read-property executor"
```

---

### Task 7: OPC UA Binary

**Files:**
- Create: `crates/apidoc/src/debug/opcua.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`
- Create: `crates/apidoc/tests/debug_opcua.rs`

**Interfaces:**
- `opcua::execute(cfg, req, ep)`。
- `opts`：`operation`（`browse`/`read`/`write`）、`node_id`、`attribute`（默认 13 `Value`）、`value`、`username`、`password`。
- 响应：`{"value":...,"status":"Good"}` 或 `{"references":[...]}`。
- feature：`debug-industrial` 加 `dep:opcua`；先 spike MSRV 与纯 Rust 状态。

- [ ] **Step 1: 写失败测试**

用 `opcua` crate 自带的 server 起一个本地 endpoint（随机端口，匿名或用户名密码）；`execute` browse/read 一个已知节点；断言值。凭据缺失且 server 要求时返回认证错误。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_opcua`
Expected: FAIL。

- [ ] **Step 3: 实现**

- `opcua::Client::new(endpoint)` → `connect`（或 `connect_and_activate_session`）。
- Browse/Read/Write 按 `opts`；value 转 JSON（数字/字符串/布尔；复杂类型转字符串）。
- 超时与错误映射；不实现订阅/历史读取。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-industrial --test debug_opcua`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/opcua.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_opcua.rs
git commit -m "feat(debug): add opcua browse/read/write executor"
```

---

### Task 8: feature 收口与只读警示

**Files:**
- Modify: `crates/apidoc/Cargo.toml`（`debug-industrial = ["debug-http","dep:tokio-modbus","dep:dnp3","dep:iec60870","dep:opcua"]`，BACnet 手写无 dep）
- Modify: `crates/apidoc/src/debug/mod.rs`（分发）
- Modify: UI：`ui.debug.js` 对工业协议显示“写操作有风险”提示（结构测试）
- Modify: `README.md`（协议表与只读说明）

- [ ] **Step 1: 验证组合**

```bash
cargo check -p apidoc-rust --features debug-coap
cargo check -p apidoc-rust --features debug-industrial
cargo check -p apidoc-rust --features debug-all
```

- [ ] **Step 2: 默认依赖检查**

Run: `cargo tree -p apidoc-rust -e normal | grep -E "coap-lite|tokio-modbus|dnp3|iec60870|opcua"` → 无输出。

- [ ] **Step 3: 全量回归**

Run: `cargo test -p apidoc-rust --all-features`
Expected: PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/apidoc/Cargo.toml crates/apidoc/src/debug/mod.rs crates/apidoc/src/ui.debug.js README.md
git commit -m "docs(debug): document industrial executors and write-operation warnings"
```

---

## P4 退出标准

- 7 个协议都有可执行操作与本地回环/黄金向量测试。
- 所有操作有超时、帧/字节上限；无广播扫描；写操作在 UI 有警示。
- DNP3/IEC104/BACnet/OPC UA 的 spike 结论记录在代码注释与本计划附录；降级路径明确。
- 默认构建依赖树不变；`debug-coap` / `debug-industrial` 可独立启用。
