# P1 浏览器原生协议 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在文档 UI 内直接调试 SOAP、GraphQL、JSON-RPC、XML-RPC、OData、MCP、A2A、Webhooks、WebSocket（文本/二进制）、SSE、Socket.IO、STOMP（含二进制）、GraphQL 订阅、WebRTC/DataChannel、HLS，不新增任何 Rust 依赖。

**Architecture:** 所有协议都在浏览器侧执行。纯编解码放进 `ui.codecs.js`（浏览器全局 + Node `module.exports` 双用），HTTP 族预设放进 `ui.protocols.js`，流式面板放进 `ui.debug.stream.js`，媒体/对等面板放进 `ui.debug.media.js`。`ui.debug.js` 只保留分发与通用表单。

**Tech Stack:** 原生 JS（无构建步骤）、WebSocket / fetch ReadableStream / RTCPeerConnection / `<video>`、Node 22 `node:test` 仅用于纯函数单测。

**Spec:** [总纲](./2026-10-24-multi-protocol-master.md)；P0 代码见 `crates/apidoc/src/ui.debug.js`、`crates/apidoc/src/ui.js`、`crates/apidoc/src/lib.rs`。

## 全局约束（本阶段）

- 禁用 `innerHTML`；所有对端文本 `textContent` 注入。
- 帧日志上限 `MAX_FRAMES = 1000`；超过时停止接收并提示，不得无限增长。
- 二进制一律走 `{encoding:"hex"|"base64"|"utf8", data}`，不得隐式字符串化。
- 浏览器无法设置 WS 握手 header：认证只能放 URL query；UI 必须提示这一点。
- 不在浏览器 localStorage 保存密码/证书；只保存非敏感开关（沿用 `apidoc_pre_script` 风格）。
- 单文件 ≤ 500 行；`ui.codecs.js` 只放无 DOM 依赖的纯函数。

## Review Focus（本阶段）

1. 二进制帧/消息被 UTF-8 解码破坏（WS binary、STOMP binary、MQTT 未来复用）。
2. SSE `data:` 多行、CRLF、注释、跨 chunk 边界解析错误。
3. STOMP 头转义（`\n` / `\c` / `\\`）与 `content-length` 后的二进制 body。
4. Socket.IO/GraphQL-WS 的握手时序（ping/pong、connection_ack、complete）竞态。
5. 帧日志无界增长或 DOM 节点泄漏（断开/切换接口后仍接收）。

---

## 文件结构

```
crates/apidoc/src/
  ui.html                 # 修改：流式/媒体面板样式
  ui.js                   # 修改：DOC 全局已有；仅加协议分发需要的辅助
  ui.codecs.js            # 新建：纯编解码（STOMP/SSE/Socket.IO/GraphQL-WS/base64/hex）
  ui.protocols.js         # 新建：HTTP 族表单预设与纯 body 构造
  ui.debug.js             # 修改：分发、复用 codecs、注入预设
  ui.debug.stream.js      # 新建：流式面板框架 + 适配器 registry
  ui.debug.media.js       # 新建：WebRTC/DataChannel + HLS
  lib.rs                  # 修改：UI_HTML concat 顺序
crates/apidoc/tests/
  js/codecs.test.js       # 新建：node:test 纯函数单测
  js_codecs.rs            # 新建：cargo test 调 node
  ui_events.rs            # 修改：新增结构断言
```

`UI_HTML` 顺序必须是：`ui.html` → `ui.js` → `ui.codecs.js` → `ui.protocols.js` → `ui.debug.js` → `ui.debug.stream.js` → `ui.debug.media.js`。后面的文件依赖前面的全局函数。

---

### Task 1: 纯编解码 + Node 测试入口（STOMP / SSE / base64）

**Files:**
- Create: `crates/apidoc/src/ui.codecs.js`
- Create: `crates/apidoc/tests/js/codecs.test.js`
- Create: `crates/apidoc/tests/js_codecs.rs`
- Modify: `crates/apidoc/src/lib.rs:37-43`（`UI_HTML` concat）
- Modify: `crates/apidoc/src/ui.debug.js`（删除本地 `decodeBytes`，改用全局函数）

**Interfaces:**
- Produces（浏览器全局 + `module.exports`）：
  - `decodeBytes(encoding: string, text: string): string | Uint8Array`
  - `encodeBytes(bytes: Uint8Array | string, encoding: string): string`
  - `stompEncode(command: string, headers: object, body?: string | Uint8Array): string`
  - `stompDecode(buffer: string): { frames: Array<{command, headers, body}>, rest: string }`
  - `sseDecode(buffer: string): { events: Array<{event, data, id, retry}>, rest: string }`
- `stompDecode` 保留未完成的尾部到 `rest`；`\0` 分帧；`content-length` 存在时按字节数取 body。
- `sseDecode` 支持 `\n`/`\r\n`、多行 `data:`、`:` 注释、空行分派。

- [ ] **Step 1: 写失败测试**

```js
// crates/apidoc/tests/js/codecs.test.js
const test = require('node:test');
const assert = require('node:assert');
const c = require('../../src/ui.codecs.js');

test('stompEncode emits NUL-terminated frame with content-length', () => {
  const out = c.stompEncode('SEND', { destination: '/q', 'content-type': 'text/plain' }, 'hi');
  assert.equal(out, 'SEND\ndestination:/q\ncontent-type:text/plain\ncontent-length:2\n\nhi\0');
});

test('stompDecode handles binary body by content-length and partial tail', () => {
  const wire = 'MESSAGE\ncontent-length:3\n\na\0b\0';
  const { frames, rest } = c.stompDecode(wire);
  assert.equal(frames.length, 1);
  assert.equal(frames[0].command, 'MESSAGE');
  assert.equal(frames[0].body, 'a\0b');
  assert.equal(rest, '');
});

test('sseDecode splits across chunks and joins multiline data', () => {
  const a = c.sseDecode('event: message\ndata: one\ndata: two\n\n');
  assert.deepEqual(a.events, [{ event: 'message', data: 'one\ntwo', id: undefined, retry: undefined }]);
  assert.equal(a.rest, '');
  const b = c.sseDecode('data: half');
  assert.equal(b.events.length, 0);
  assert.equal(b.rest, 'data: half');
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `node --test crates/apidoc/tests/js/codecs.test.js`
Expected: FAIL — `Cannot find module '../../src/ui.codecs.js'`.

- [ ] **Step 3: 实现 `ui.codecs.js`**

实现上述四个函数。`decodeBytes`/`encodeBytes` 从现有 `ui.debug.js` 搬入；Node 下用 `Buffer`，浏览器下用 `atob`/`btoa`。文件末尾：

```js
if (typeof module !== 'undefined') {
  module.exports = { decodeBytes, encodeBytes, stompEncode, stompDecode, sseDecode };
}
```

- [ ] **Step 4: 接入 UI 并加 cargo 入口**

`lib.rs` 的 `UI_HTML` 在 `ui.js` 后插入 `include_str!("ui.codecs.js")`。新增 `tests/js_codecs.rs`：

```rust
#[test]
fn js_codecs_pass() {
    let status = std::process::Command::new("node")
        .args(["--test", "tests/js/codecs.test.js"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status();
    match status {
        Ok(s) => assert!(s.success(), "node codec tests failed"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => eprintln!("node not found, skipping"),
        Err(e) => panic!("failed to run node: {e}"),
    }
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `node --test crates/apidoc/tests/js/codecs.test.js && cargo test -p apidoc-rust --test js_codecs`
Expected: 全部 PASS。

- [ ] **Step 6: 提交**

```bash
git add crates/apidoc/src/ui.codecs.js crates/apidoc/src/lib.rs crates/apidoc/src/ui.debug.js crates/apidoc/tests/js/codecs.test.js crates/apidoc/tests/js_codecs.rs
git commit -m "feat(debug): add browser codecs and node test harness"
```

---

### Task 2: 流式面板框架 + WebSocket / WS Binary

**Files:**
- Create: `crates/apidoc/src/ui.debug.stream.js`
- Modify: `crates/apidoc/src/ui.html`（`.stream-log` / `.stream-row` 样式）
- Modify: `crates/apidoc/src/lib.rs`（concat）
- Modify: `crates/apidoc/src/ui.debug.js`（`debugPanel` 分发到 `streamPanel`）

**Interfaces:**
- Produces: `streamPanel(ep: DocEndpoint, adapterName: string): Node`
- Produces: `registerStreamAdapter(name, adapter)`；`STREAM_ADAPTERS` 可被后续 Task 扩展。
- Adapter 形状：`connect(target, opts, handlers) -> { send({encoding,data}), close() }`；`handlers = {onOpen,onFrame({dir,encoding,data}),onClose(code,reason),onError(msg)}`。
- 约束：`MAX_FRAMES = 1000`；断开时清空定时器/监听器。

- [ ] **Step 1: 写结构测试**

在 `tests/ui_events.rs` 增加：

```rust
#[test]
fn ui_has_stream_panel_and_ws_adapter() {
    let ui = include_str!("../src/ui.debug.stream.js");
    assert!(ui.contains("function streamPanel"));
    assert!(ui.contains("MAX_FRAMES = 1000"));
    assert!(ui.contains("registerStreamAdapter"));
    assert!(ui.contains("'websocket'"));
    assert!(!ui.contains("innerHTML"));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --test ui_events ui_has_stream_panel_and_ws_adapter`
Expected: FAIL — file not found.

- [ ] **Step 3: 实现面板与 WebSocket 适配器**

`streamPanel` 结构：连接条（target 预填 `ep.protocol_opts.target`）、状态行、帧日志、发送区（编码 select + textarea + 发送/断开）。日志行用 `el()` 与 `textContent`，超过 1000 帧停止接收并追加一条提示。WebSocket 适配器：

```js
registerStreamAdapter('websocket', {
  connect(target, opts, h) {
    const ws = new WebSocket(target);
    ws.binaryType = 'arraybuffer';
    ws.onopen = () => h.onOpen();
    ws.onmessage = ev => h.onFrame({ dir: 'in', encoding: typeof ev.data === 'string' ? 'utf8' : 'base64', data: typeof ev.data === 'string' ? ev.data : encodeBytes(new Uint8Array(ev.data), 'base64') });
    ws.onclose = ev => h.onClose(ev.code, ev.reason);
    ws.onerror = () => h.onError('WebSocket 连接失败');
    return {
      send(p) { ws.send(p.encoding === 'utf8' ? p.data : decodeBytes(p.encoding, p.data)); },
      close() { ws.close(); },
    };
  },
});
```

`debugPanel` 在 `ep.protocol` 为 `websocket` / `ws-binary` 时返回 `streamPanel(ep, 'websocket')`。

- [ ] **Step 4: 手工验证**

启动 `examples/axum_demo` 或任意带 WS echo 的服务，打开文档页，连接、发送 text/hex/base64、断开；确认日志方向、时间、字节显示正确且超过 1000 帧后停止。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p apidoc-rust --test ui_events && node --test crates/apidoc/tests/js/codecs.test.js`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add crates/apidoc/src/ui.debug.stream.js crates/apidoc/src/ui.html crates/apidoc/src/lib.rs crates/apidoc/src/ui.debug.js crates/apidoc/tests/ui_events.rs
git commit -m "feat(debug): add stream panel and websocket adapter"
```

---

### Task 3: Socket.IO 适配器

**Files:**
- Modify: `crates/apidoc/src/ui.codecs.js`
- Modify: `crates/apidoc/src/ui.debug.stream.js`
- Modify: `crates/apidoc/tests/js/codecs.test.js`

**Interfaces:**
- Produces: `socketIoEncode(type: string, payload: any): string`；`socketIoDecode(text: string): {type, payload}`。
- Socket.IO packet type：`0` connect、`1` disconnect、`2` event、`3` ack、`4` error；Engine.IO 前缀 `4`（message）。编码 event 为 `42["name",payload]`。
- Adapter `connect` 先 `fetch(base + '/socket.io/?EIO=4&transport=polling')` 取 `0{"sid":...}`，再开 WS `.../socket.io/?EIO=4&transport=websocket&sid=...`；open 后发 `40`；收到 `2` 回 `3`。

- [ ] **Step 1: 写失败测试**

```js
test('socketIo encode/decode event', () => {
  assert.equal(c.socketIoEncode('event', ['chat', { text: 'hi' }]), '42["chat",{"text":"hi"}]');
  assert.deepEqual(c.socketIoDecode('42["chat",{"text":"hi"}]'), { type: 'event', payload: ['chat', { text: 'hi' }] });
  assert.deepEqual(c.socketIoDecode('40'), { type: 'connect', payload: null });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `node --test crates/apidoc/tests/js/codecs.test.js`
Expected: FAIL — `socketIoEncode is not a function`.

- [ ] **Step 3: 实现编解码与适配器**

在 `ui.codecs.js` 实现两个纯函数并导出；在 `ui.debug.stream.js` 注册 `'socketio'` 适配器，处理 polling 握手、`2`→`3`、`40` open、`42` 事件。`ep.protocol_opts` 可提供 `event`（默认 `message`）与 `namespace`（默认 `/`）。

- [ ] **Step 4: 跑测试确认通过**

Run: `node --test crates/apidoc/tests/js/codecs.test.js && cargo test -p apidoc-rust --test ui_events`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/ui.codecs.js crates/apidoc/src/ui.debug.stream.js crates/apidoc/tests/js/codecs.test.js
git commit -m "feat(debug): add socket.io adapter"
```

---

### Task 4: STOMP / STOMP Binary / GraphQL 订阅适配器

**Files:**
- Modify: `crates/apidoc/src/ui.codecs.js`（GraphQL-WS 编解码）
- Modify: `crates/apidoc/src/ui.debug.stream.js`
- Modify: `crates/apidoc/tests/js/codecs.test.js`

**Interfaces:**
- Produces: `graphqlWsEncode(type, id, payload): string`；`graphqlWsDecode(text): {type,id,payload}`。
- STOMP 适配器：连接 target；`CONNECT` 头 `accept-version:1.2`、`host`；收到 `CONNECTED` 后 `SUBSCRIBE`（`destination` 来自 `opts.destination`）；发送 `SEND`；关闭前 `DISCONNECT`。
- GraphQL-WS 适配器：open 后发 `{"type":"connection_init"}`；收到 `connection_ack` 后发 `{"id":"1","type":"subscribe","payload":{query,variables,operationName}}`；`next` 转帧，`complete` 关闭。

- [ ] **Step 1: 写失败测试**

```js
test('graphqlWs encode/decode', () => {
  assert.equal(c.graphqlWsEncode('subscribe', '1', { query: '{x}' }), '{"type":"subscribe","id":"1","payload":{"query":"{x}"}}');
  assert.deepEqual(c.graphqlWsDecode('{"type":"next","id":"1","payload":{"data":{}}}'), { type: 'next', id: '1', payload: { data: {} } });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `node --test crates/apidoc/tests/js/codecs.test.js`
Expected: FAIL — `graphqlWsEncode is not a function`.

- [ ] **Step 3: 实现**

实现 GraphQL-WS 纯函数并导出。在 `ui.debug.stream.js` 注册 `'stomp'` 与 `'graphql-ws'`；STOMP 复用 Task 1 的 `stompEncode`/`stompDecode`，按 `\0` 分帧；GraphQL 订阅面板增加 query/variables/operationName 输入（可复用 `ui.protocols.js` 的 GraphQL 输入构造器，若尚未创建则本任务内联）。

- [ ] **Step 4: 跑测试确认通过**

Run: `node --test crates/apidoc/tests/js/codecs.test.js && cargo test -p apidoc-rust --test ui_events`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/ui.codecs.js crates/apidoc/src/ui.debug.stream.js crates/apidoc/tests/js/codecs.test.js
git commit -m "feat(debug): add stomp and graphql-ws adapters"
```

---

### Task 5: HTTP 族协议表单预设

**Files:**
- Create: `crates/apidoc/src/ui.protocols.js`
- Modify: `crates/apidoc/src/ui.debug.js`
- Modify: `crates/apidoc/src/lib.rs`（concat）
- Create/Modify: `crates/apidoc/tests/js/codecs.test.js`（纯 body 构造器测试）

**Interfaces:**
- Produces: `PROTOCOL_FORMS` 与纯函数：
  - `buildGraphqlBody(query, variablesJson, operationName): string`
  - `buildJsonRpcBody(method, paramsJson, id): string`
  - `buildODataQuery(input): Array<{name,value}>`
  - `buildSoapHeaders(action): object`
- `protocolExtras(ep): { extras: Node[], buildBody: () => string | null, buildHeaders: () => object, buildQuerys: () => Array<{name,value}> } | null`
- `debugForm` 在渲染 headers 之后调用 `protocolExtras`；若 `buildBody()` 返回非空且原始 Body 为空，则用它覆盖通用 JSON 组装。

- [ ] **Step 1: 写失败测试**

```js
test('graphql/jsonrpc/odata builders', () => {
  assert.equal(c.buildGraphqlBody('{ me }', '{"x":1}', 'Q'), '{"query":"{ me }","variables":{"x":1},"operationName":"Q"}');
  assert.equal(c.buildJsonRpcBody('tools/list', '{}', '1'), '{"jsonrpc":"2.0","method":"tools/list","params":{},"id":"1"}');
  assert.deepEqual(c.buildODataQuery({ filter: "a eq 1", top: '5', select: 'id' }), [
    { name: '$filter', value: 'a eq 1' }, { name: '$top', value: '5' }, { name: '$select', value: 'id' },
  ]);
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `node --test crates/apidoc/tests/js/codecs.test.js`
Expected: FAIL — builders not found.

- [ ] **Step 3: 实现预设**

`ui.protocols.js` 提供：
- SOAP：XML textarea、SOAPAction input；默认 `Content-Type: text/xml; charset=utf-8`。
- GraphQL：query / variables / operationName；JSON body。
- JSON-RPC / MCP / A2A：method 输入（MCP 预设 `initialize`/`tools/list`/`tools/call`；A2A 预设 `message/send`/`tasks/get`）、params JSON、id；JSON-RPC 2.0 body。
- XML-RPC：method + params XML 模板 textarea；`Content-Type: text/xml`。
- OData：`$filter/$select/$expand/$orderby/$top/$skip` 输入，并入 query string。
- Webhooks：说明文案 + 默认 JSON 头；body 沿用通用参数组装。

在 `ui.debug.js` 中接入；保留原始 Body 覆盖优先。

- [ ] **Step 4: 跑测试确认通过**

Run: `node --test crates/apidoc/tests/js/codecs.test.js && cargo test -p apidoc-rust --test ui_events`
Expected: PASS。

- [ ] **Step 5: 手工验证**

对本地 mock 的 GraphQL/SOAP/JSON-RPC endpoint 各发一次请求，确认 body 与 Content-Type 正确；OData 确认 query 参数拼接。

- [ ] **Step 6: 提交**

```bash
git add crates/apidoc/src/ui.protocols.js crates/apidoc/src/ui.debug.js crates/apidoc/src/lib.rs crates/apidoc/tests/js/codecs.test.js
git commit -m "feat(debug): add http-family protocol form presets"
```

---

### Task 6: SSE 适配器

**Files:**
- Modify: `crates/apidoc/src/ui.debug.stream.js`
- Modify: `crates/apidoc/tests/ui_events.rs`

**Interfaces:**
- Adapter `'sse'`：使用 `fetch(target, {method, headers, body, signal})`，读到 `response.body.getReader()`，`TextDecoder` 增量解码，交给 `sseDecode`；每个事件回调 `onFrame({dir:'in',encoding:'utf8',data: JSON.stringify({event,id,data})})`。
- 发送区：SSE 通常是 GET；若 `opts.method === 'POST'`，允许发送一次 body（含 query 参数）。
- `close()` 调 `AbortController.abort()`。

- [ ] **Step 1: 写结构测试**

```rust
#[test]
fn ui_has_sse_adapter_and_abort() {
    let ui = include_str!("../src/ui.debug.stream.js");
    assert!(ui.contains("'sse'"));
    assert!(ui.contains("AbortController"));
    assert!(ui.contains("sseDecode"));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --test ui_events ui_has_sse_adapter_and_abort`
Expected: FAIL.

- [ ] **Step 3: 实现**

注册 `'sse'` 适配器；面板在 `ep.protocol === 'sse'` 时使用。注意 `sseDecode` 的 `rest` 必须跨 chunk 保留；`AbortController` 在断开时触发。

- [ ] **Step 4: 手工验证**

对本地 SSE endpoint（可用 `curl` 或测试服务器）连接，确认多行 data、注释、断线后停止。

- [ ] **Step 5: 跑测试确认通过并提交**

Run: `cargo test -p apidoc-rust --test ui_events`
```bash
git add crates/apidoc/src/ui.debug.stream.js crates/apidoc/tests/ui_events.rs
git commit -m "feat(debug): add sse adapter"
```

---

### Task 7: WebRTC / DataChannel 面板

**Files:**
- Create: `crates/apidoc/src/ui.debug.media.js`
- Modify: `crates/apidoc/src/ui.html`（媒体区域样式）
- Modify: `crates/apidoc/src/lib.rs`（concat）

**Interfaces:**
- `webrtcPanel(ep): Node`
- 信令契约（写进面板帮助文本）：`POST ep.protocol_opts.target`，请求 `{ "type": "offer", "sdp": "<local>" }`；响应 `{ "type": "answer", "sdp": "<remote>", "iceServers"?: [...] }`。
- `opts.channel` 默认 `"data"`；`opts.iceServers` 为 JSON 数组。
- 面板：连接、DataChannel 状态、发送文本/hex/base64、接收日志（复用 `MAX_FRAMES`）。

- [ ] **Step 1: 写结构测试**

在 `ui_events.rs` 增加：

```rust
#[test]
fn ui_has_webrtc_and_hls_panels() {
    let ui = include_str!("../src/ui.debug.media.js");
    assert!(ui.contains("function webrtcPanel"));
    assert!(ui.contains("RTCPeerConnection"));
    assert!(ui.contains("createDataChannel"));
    assert!(ui.contains("function hlsPanel"));
    assert!(ui.contains("createElement('video')"));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --test ui_events ui_has_webrtc_and_hls_panels`
Expected: FAIL.

- [ ] **Step 3: 实现 WebRTC 面板**

创建 offer → `setLocalDescription` → POST 信令 → `setRemoteDescription(answer)`；处理 `oniceconnectionstatechange`；DataChannel `onmessage` 转帧；发送按编码调用 `send`。所有错误用 `warnBox`。

- [ ] **Step 4: 手工验证**

用一个最小信令服务器（Node/Python 均可，只要按上述契约回 answer）验证连接与双向消息；确认二进制不乱码。

- [ ] **Step 5: 跑测试确认通过并提交**

Run: `cargo test -p apidoc-rust --test ui_events`
```bash
git add crates/apidoc/src/ui.debug.media.js crates/apidoc/src/ui.html crates/apidoc/src/lib.rs crates/apidoc/tests/ui_events.rs
git commit -m "feat(debug): add webrtc datachannel panel"
```

---

### Task 8: HLS 面板

**Files:**
- Modify: `crates/apidoc/src/ui.debug.media.js`
- Modify: `crates/apidoc/src/ui.debug.js`（分发）

**Interfaces:**
- `hlsPanel(ep): Node`
- 使用原生 `<video controls>`；`src = ep.protocol_opts.target || ep.url`；提供 manifest 文本框（`fetch(target)` 显示原文）与重新加载按钮。
- 不支持的服务端（非 HLS）在 `video.onerror` 提示，不抛异常。

- [ ] **Step 1: 写结构测试**

在 Task 7 的测试中断言 `hlsPanel`、`createElement('video')` 已存在（Task 8 完成后再跑）。

- [ ] **Step 2: 实现**

`video` 元素、`source` URL、`fetch` manifest 文本、错误提示。`debugPanel` 在 `ep.protocol === 'hls'` 时调用。

- [ ] **Step 3: 手工验证**

用一个公开或本地 HLS 流验证播放；用一个非 HLS URL 验证错误提示。

- [ ] **Step 4: 跑测试并提交**

Run: `cargo test -p apidoc-rust --test ui_events && node --test crates/apidoc/tests/js/codecs.test.js`
```bash
git add crates/apidoc/src/ui.debug.media.js crates/apidoc/src/ui.debug.js crates/apidoc/tests/ui_events.rs crates/apidoc/tests/js/codecs.test.js
git commit -m "feat(debug): add hls preview panel"
```

---

### Task 9: 分发收口与回归

**Files:**
- Modify: `crates/apidoc/src/ui.debug.js`（`debugPanel` 完整分发）
- Modify: `crates/apidoc/tests/ui_events.rs`
- Modify: `README.md`（仅增补一段协议调试说明；不放实现细节）

**Interfaces:**
- `debugPanel(ep)` 分发优先级：
  1. `not_debug` 勾选逻辑不变；
  2. `websocket`/`ws-binary` → `streamPanel(ep,'websocket')`；
  3. `sse` → `streamPanel(ep,'sse')`；
  4. `socketio` → `streamPanel(ep,'socketio')`；
  5. `stomp`/`stomp-binary` → `streamPanel(ep,'stomp')`；
  6. `graphql` 且 `protocol_opts.operation === 'subscription'` → `streamPanel(ep,'graphql-ws')`，否则 `debugForm`（带 GraphQL 预设）；
  7. `mcp`/`a2a` 且 `protocol_opts.transport === 'sse'` → `streamPanel(ep,'sse')`，否则 `debugForm`（带 JSON-RPC 预设）；
  8. `webrtc` → `webrtcPanel`；`hls` → `hlsPanel`；
  9. HTTP 族 → `debugForm`；
  10. 其余 → 占位提示（“该协议面板在 P2+ 提供”）。

- [ ] **Step 1: 写失败测试**

在 `ui_events.rs` 断言分发标记：`streamPanel(ep, 'websocket')`、`webrtcPanel(ep)`、`hlsPanel(ep)`、`graphql-ws`、`P2+`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p apidoc-rust --test ui_events`
Expected: FAIL.

- [ ] **Step 3: 实现分发并补 README 一段**

- [ ] **Step 4: 全量回归**

Run:
```bash
cargo test -p apidoc-rust --all-features
cargo test -p apidoc-macros
node --test crates/apidoc/tests/js/codecs.test.js
cargo check -p apidoc-rust
```
Expected: 全部 PASS；默认构建依赖树无变化。

- [ ] **Step 5: 提交**

```bash
git add crates/apidoc/src/ui.debug.js crates/apidoc/tests/ui_events.rs README.md
git commit -m "feat(debug): wire browser protocol panels"
```

---

## P1 退出标准

- 18 个浏览器原生协议条目全部有可用面板或明确占位（Webhooks 复用 HTTP 表单并注明出站语义）。
- `node --test crates/apidoc/tests/js/codecs.test.js` 覆盖 STOMP/SSE/Socket.IO/GraphQL-WS 编解码。
- 无 `innerHTML`；帧日志上限生效；断开后无残留监听器/定时器。
- `cargo test -p apidoc-rust --all-features` 与默认构建 `cargo check` 均通过。
