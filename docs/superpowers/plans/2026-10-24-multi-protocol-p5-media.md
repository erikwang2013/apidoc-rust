# P5 媒体 / 信令 / HTTP3 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 服务端调试 RTP/RTCP、RTSP、SIP、RTMP，并支持 HTTP/3；媒体协议只做握手、信令与包级查看，不做转码/播放服务。

**Architecture:** RTP/RTCP 手写头编解码（RFC 3550），走 UDP；RTSP 手写文本客户端（OPTIONS/DESCRIBE/SETUP/PLAY/TEARDOWN）；SIP 优先 `rsip` 解析，失败则手写最小子集；RTMP 手写握手 + AMF0 子集；HTTP/3 用 `quinn` + `h3`，独立 feature。

**Tech Stack:** tokio UDP/TCP、`rsip`（SIP 解析）、`quinn` + `h3`（HTTP/3）；RTP/RTCP/RTMP 手写。

**Spec:** [总纲](./2026-10-24-multi-protocol-master.md)；P2 sockets 见 `crates/apidoc/src/debug/sockets.rs`。

## 全局约束（本阶段）

- 媒体协议默认只发控制/信令，不自动拉流；拉流必须显式 `opts.play=true` 且有帧/时长上限。
- UDP 接收包数上限 1000、单包上限 65507 字节；TCP 读取上限 `max_bytes`。
- TLS 一律 rustls；HTTP/3 不提供跳过证书校验。
- 所有目标先过 `guard::check_host`；RTP/RTSP 的 UDP 端口与 RTSP TCP 端口分别校验。
- feature 独立：`debug-media`、`debug-http3`；默认构建零新增依赖。

## Review Focus（本阶段）

1. RTP/RTCP 头扩展、CSRC 计数、padding 导致偏移错误。
2. RTSP 状态机（CSeq、Session、Transport）与 401 Digest 认证缺失；期望明确报错而非死等。
3. SIP 事务匹配（branch/Call-ID/CSeq）与 UDP 重传。
4. RTMP 握手（C0/C1/C2/S0/S1/S2）与 AMF0 编码边界。
5. HTTP/3 ALPN/证书校验与连接迁移；期望失败可见。

---

## 文件结构

```
crates/apidoc/src/debug/
  rtp.rs
  rtcp.rs
  rtsp.rs
  sip.rs
  rtmp.rs
  http3.rs
  mod.rs
crates/apidoc/tests/
  debug_rtp.rs
  debug_rtsp.rs
  debug_sip.rs
  debug_rtmp.rs
  debug_http3.rs
```

---

### Task 1: RTP / RTCP

**Files:**
- Create: `crates/apidoc/src/debug/rtp.rs`
- Create: `crates/apidoc/src/debug/rtcp.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`debug-media = ["debug-http"]`）
- Create: `crates/apidoc/tests/debug_rtp.rs`

**Interfaces:**
- `rtp::execute(cfg, req, ep)`：kind `rtp`。
  - `opts`：`payload_type`（默认 96）、`ssrc`、`sequence`（默认 0）、`timestamp`、`count`（发送包数，默认 1）、`recv`（`"true"` 时接收）、`duration_ms`。
  - 响应：`{"packets":[{"version","payload_type","sequence","timestamp","ssrc","payload":{encoding,data}}...]}`。
- `rtcp::execute`：kind `rtcp`，解析/构造 SR/RR/SDES/BYE 子集；`opts.packet_type`。
- 复用 `guard::check_host` + UDP。

- [ ] **Step 1: 写黄金向量测试**

```rust
#[test]
fn rtp_header_roundtrip() {
    let bytes = rtp::encode_packet(96, 7, 12345, 0xdead_beef, b"hi");
    assert_eq!(&bytes[..12], &[0x80, 96, 0x00, 0x07, 0x00,0x00,0x30,0x39, 0xde,0xad,0xbe,0xef]);
    let p = rtp::decode_packet(&bytes).unwrap();
    assert_eq!(p.sequence, 7);
    assert_eq!(p.payload, b"hi");
}
```

再加本地 UDP 回环：`rtp::execute` 发送，测试端接收并回一个包，断言解析。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-media --test debug_rtp`
Expected: FAIL。

- [ ] **Step 3: 实现**

- RTP 头：V/P/X/CC、M/PT、sequence、timestamp、ssrc；CSRC 跳过；扩展头按长度跳过；padding 截断。
- RTCP：版本/包类型/长度；SR/RR 只解析通用头与 sender info；BYE 文本。
- UDP 收发与超时；包数上限。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-media --test debug_rtp`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/rtp.rs crates/apidoc/src/debug/rtcp.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_rtp.rs
git commit -m "feat(debug): add rtp and rtcp packet executors"
```

---

### Task 2: RTSP

**Files:**
- Create: `crates/apidoc/src/debug/rtsp.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Create: `crates/apidoc/tests/debug_rtsp.rs`

**Interfaces:**
- `rtsp::execute(cfg, req, ep)`。
- `opts`：`method`（`OPTIONS`/`DESCRIBE`/`SETUP`/`PLAY`/`TEARDOWN`，默认 `OPTIONS`）、`url`、`cseq`（默认 1）、`session`、`transport`（默认 `RTP/AVP/TCP;interleaved=0-1`）。
- 响应：`{"status":200,"reason":"OK","headers":{...},"sdp"?:{...},"body":{...}}`。
- 只实现无认证的 RTSP；401 时返回错误并提示需要 Digest（P6）。

- [ ] **Step 1: 写失败测试**

本地 TCP server 解析 `OPTIONS rtsp://... RTSP/1.0`，回 `RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: ...\r\n\r\n`；断言 `execute` 的 status/headers。再测 DESCRIBE 返回 SDP 文本，断言 `sdp` 字段或原文。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-media --test debug_rtsp`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 请求行 + 头 + `\r\n\r\n`；`CSeq` 递增；读响应头与 `Content-Length` body。
- 解析状态行/头；SDP 作为 `body` 原文返回（不做完整 SDP 模型）。
- SETUP 的 Transport 头、PLAY 的 Session 头透传。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-media --test debug_rtsp`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/rtsp.rs crates/apidoc/src/debug/mod.rs crates/apidoc/tests/debug_rtsp.rs
git commit -m "feat(debug): add rtsp control client"
```

---

### Task 3: SIP

**Files:**
- Create: `crates/apidoc/src/debug/sip.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`dep:rsip`）
- Create: `crates/apidoc/tests/debug_sip.rs`

**Interfaces:**
- `sip::execute(cfg, req, ep)`。
- `opts`：`method`（`OPTIONS`/`REGISTER`/`INVITE`，默认 OPTIONS）、`uri`、`from`、`to`、`call_id`、`cseq`、`transport`（`udp` 默认 / `tcp`）。
- 响应：`{"status":200,"reason":"OK","headers":{...},"body":{...}}`。
- 只做单次事务；不做注册续期/媒体协商。

- [ ] **Step 1: 写失败测试**

本地 UDP server 解析请求行，回 `SIP/2.0 200 OK` + `Via`/`From`/`To`/`Call-ID`/`CSeq`；断言 `execute` 返回 200。再用 `rsip` 解析一条 INVITE 黄金文本，断言 method/uri。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-media --test debug_sip`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 用 `rsip` 构造/解析消息；`Via` branch 随机但可复现（`opts.branch`）。
- UDP/TCP 发送；等匹配 `Call-ID` + `CSeq` 的响应；超时。
- 401/407 返回错误并提示认证未实现。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-media --test debug_sip`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/sip.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_sip.rs
git commit -m "feat(debug): add sip single-transaction executor"
```

---

### Task 4: RTMP

**Files:**
- Create: `crates/apidoc/src/debug/rtmp.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Create: `crates/apidoc/tests/debug_rtmp.rs`

**Interfaces:**
- `rtmp::execute(cfg, req, ep)`。
- `opts`：`app`、`stream`、`operation`（`connect`/`publish`/`play`，默认 `connect`）、`duration_ms`。
- 响应：`{"connected":true,"stream":"..."}`。
- 实现：简单握手（C0/C1/C2/S0/S1/S2）、AMF0 connect/createStream/publish/play 子集；chunk stream 只处理单块/单 message（不实现分块重组？必须实现基本分块，否则真实服务器不兼容）。

- [ ] **Step 1: 写黄金向量测试**

- 握手：C0 版本 3、C1 1536 字节、S0/S1/S2 校验。
- AMF0：`encode_string("connect")`、`encode_number(1)`、`encode_object({"app":"live"})` 的字节断言。
- 本地 TCP server 回握手与 `_result`，`execute` 断言 connected。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-media --test debug_rtmp`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 握手与 chunk 基础（fmt 0/1/2/3、chunk size、basic header）。
- AMF0 常用类型：number/boolean/string/object/null/ecma array。
- connect 命令、createStream、publish/play；只读命令响应，不实现媒体编码。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-media --test debug_rtmp`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/rtmp.rs crates/apidoc/src/debug/mod.rs crates/apidoc/tests/debug_rtmp.rs
git commit -m "feat(debug): add rtmp handshake and command executor"
```

---

### Task 5: HTTP/3

**Files:**
- Create: `crates/apidoc/src/debug/http3.rs`
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: `crates/apidoc/Cargo.toml`（`debug-http3 = ["debug-http","dep:quinn","dep:h3","dep:rustls"]`）
- Create: `crates/apidoc/tests/debug_http3.rs`

**Interfaces:**
- `http3::execute(cfg, req, ep)`，kind `http3`。
- `target` 必须是 `https://`；`opts.version` 忽略。
- 响应与 HTTP 执行器一致：status/headers/body。
- 使用 `quinn::Endpoint::client` + rustls 默认根证书；ALPN `h3`。
- 不提供跳过证书校验。

- [ ] **Step 1: 写失败测试**

本地 h3 server（`h3` + `quinn`，自签证书）在随机 UDP 端口；测试客户端需要信任该证书——用测试专用 `rustls::RootCertStore` 注入自签 CA，生产代码仍用系统根。断言 status/body。再测非 https target 返回错误。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --features debug-http3 --test debug_http3`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 解析 target host/port；`guard::check_host`。
- quinn client config：`with_root_certificates`（测试可注入）、ALPN `h3`。
- `h3::client::new` → `send_request` → 读 body，受 `max_bytes` 限制。
- 错误映射（QUIC 连接失败、证书错误、ALPN）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --features debug-http3 --test debug_http3`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/debug/http3.rs crates/apidoc/src/debug/mod.rs crates/apidoc/Cargo.toml crates/apidoc/tests/debug_http3.rs
git commit -m "feat(debug): add http3 executor"
```

---

### Task 6: feature 收口与媒体警示

**Files:**
- Modify: `crates/apidoc/Cargo.toml`（`debug-media = ["debug-http","dep:rsip"]`；`debug-all` 加入 media/http3）
- Modify: `crates/apidoc/src/debug/mod.rs`
- Modify: UI：`ui.debug.js` 对 `rtsp`/`rtmp`/`rtp`/`rtcp`/`sip` 显示“控制/包级调试，不转码”提示
- Modify: `README.md`

- [ ] **Step 1: 验证组合**

```bash
cargo check -p apidoc-rust --features debug-media
cargo check -p apidoc-rust --features debug-http3
cargo check -p apidoc-rust --features debug-all
```

- [ ] **Step 2: 默认依赖检查**

Run: `cargo tree -p apidoc-rust -e normal | grep -E "quinn|h3|rsip"` → 无输出。

- [ ] **Step 3: 全量回归**

Run: `cargo test -p apidoc-rust --all-features`
Expected: PASS。

- [ ] **Step 4: 提交**

```bash
git add crates/apidoc/Cargo.toml crates/apidoc/src/debug/mod.rs crates/apidoc/src/ui.debug.js README.md
git commit -m "docs(debug): document media and http3 executors"
```

---

## P5 退出标准

- RTP/RTCP/RTSP/SIP/RTMP/HTTP3 都有本地回环或黄金向量测试。
- 媒体协议不在未显式请求时拉流；UDP/TCP 都有包数/字节/超时上限。
- HTTP/3 只信任系统根证书（测试可注入自签 CA），无跳过校验开关。
- 默认构建依赖树不变；`debug-media` / `debug-http3` 可独立启用。
