// P1 纯编解码测试：node --test crates/apidoc/tests/js/codecs.test.js
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const c = require('../../src/ui.codecs.js');
const p = require('../../src/ui.protocols.js');

test('base64/hex roundtrip preserves binary bytes', () => {
  const bytes = new Uint8Array([0, 1, 255, 65]);
  assert.deepEqual(c.decodeBytes('base64', c.encodeBytes(bytes, 'base64')), bytes);
  assert.deepEqual(c.decodeBytes('hex', c.encodeBytes(bytes, 'hex')), bytes);
  assert.equal(c.encodeBytes(bytes, 'hex'), '0001ff41');
});

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

test('stomp header escaping roundtrips', () => {
  const out = c.stompEncode('SEND', { 'x-note': 'a\nb:c' }, '');
  const { frames } = c.stompDecode(out);
  assert.equal(frames[0].headers['x-note'], 'a\nb:c');
});

test('mockFromEvents filters direction and falls back by type', () => {
  const evs = [
    { direction: 'out', name: 'join', type: 'json', mock: '{"room":1}' },
    { direction: 'in', name: 'msg', type: 'text' },
    { direction: 'out', name: 'raw', type: 'binary' },
  ];
  assert.deepEqual(c.mockFromEvents(evs, 'out'), [
    { name: 'join', payload: { encoding: 'utf8', data: '{"room":1}' } },
    { name: 'raw', payload: { encoding: 'hex', data: '00' } },
  ]);
  assert.deepEqual(c.mockFromEvents(evs, 'in'), [
    { name: 'msg', payload: { encoding: 'utf8', data: 'string' } },
  ]);
});

test('socketIo encode/decode event', () => {
  assert.equal(c.socketIoEncode('event', ['chat', { text: 'hi' }]), '42["chat",{"text":"hi"}]');
  assert.deepEqual(c.socketIoDecode('42["chat",{"text":"hi"}]'), { type: 'event', payload: ['chat', { text: 'hi' }] });
  assert.deepEqual(c.socketIoDecode('40'), { type: 'connect', payload: null });
});

test('graphqlWs encode/decode', () => {
  assert.equal(c.graphqlWsEncode('subscribe', '1', { query: '{x}' }), '{"type":"subscribe","id":"1","payload":{"query":"{x}"}}');
  assert.deepEqual(c.graphqlWsDecode('{"type":"next","id":"1","payload":{"data":{}}}'), { type: 'next', id: '1', payload: { data: {} } });
});

test('http-family protocol body builders', () => {
  assert.equal(p.buildGraphqlBody('{ me }', '{"x":1}', 'Q'), '{"query":"{ me }","variables":{"x":1},"operationName":"Q"}');
  assert.equal(p.buildJsonRpcBody('tools/list', '{}', '1'), '{"jsonrpc":"2.0","method":"tools/list","params":{},"id":"1"}');
  assert.deepEqual(p.buildODataQuery({ filter: 'a eq 1', top: '5', select: 'id' }), [
    { name: '$filter', value: 'a eq 1' },
    { name: '$top', value: '5' },
    { name: '$select', value: 'id' },
  ]);
  assert.deepEqual(p.buildSoapHeaders('urn:Op'), {
    'Content-Type': 'text/xml; charset=utf-8',
    SOAPAction: 'urn:Op',
  });
});

test('sseDecode splits across chunks and joins multiline data', () => {
  const a = c.sseDecode('event: message\ndata: one\ndata: two\n\n');
  assert.deepEqual(a.events, [{ event: 'message', data: 'one\ntwo', id: undefined, retry: undefined }]);
  assert.equal(a.rest, '');
  const b = c.sseDecode('data: half');
  assert.equal(b.events.length, 0);
  assert.equal(b.rest, 'data: half');
});

test('sseDecode handles CRLF, comments, and cross-chunk dispatch', () => {
  const a = c.sseDecode(': keep-alive\r\nevent: message\r\ndata: one\r\ndata: two\r\n\r\n');
  assert.deepEqual(a.events, [{ event: 'message', data: 'one\ntwo', id: undefined, retry: undefined }]);
  assert.equal(a.rest, '');
  const partial = c.sseDecode('data: par');
  assert.equal(partial.events.length, 0);
  const done = c.sseDecode(partial.rest + 'tial\n\n');
  assert.equal(done.events.length, 1);
  assert.equal(done.events[0].data, 'partial');
});

// ---------- 流式适配器握手时序（假 WebSocket，无网络） ----------

class FakeWebSocket {
  static OPEN = 1;
  constructor(url, protocol) {
    this.url = url;
    this.protocol = protocol;
    this.readyState = 0;
    this.sent = [];
    FakeWebSocket.last = this;
  }
  send(data) { this.sent.push(data); }
  close() { this.readyState = 3; if (this.onclose) this.onclose({ code: 1000, reason: 'client close' }); }
  open() { this.readyState = 1; if (this.onopen) this.onopen(); }
  receive(data) { if (this.onmessage) this.onmessage({ data }); }
}

// ui.debug.stream.js 是浏览器脚本：在 vm 里注入它依赖的全局函数后导出 registry。
function loadStreamAdapters(env = {}) {
  const src = fs.readFileSync(path.join(__dirname, '../../src/ui.debug.stream.js'), 'utf8');
  const ctx = {
    URL, setTimeout, clearTimeout, TextEncoder, TextDecoder, Uint8Array, ArrayBuffer,
    encodeBytes: c.encodeBytes, decodeBytes: c.decodeBytes,
    stompEncode: c.stompEncode, stompDecode: c.stompDecode,
    socketIoEncode: c.socketIoEncode, socketIoDecode: c.socketIoDecode,
    graphqlWsEncode: c.graphqlWsEncode, graphqlWsDecode: c.graphqlWsDecode,
    WebSocket: FakeWebSocket,
    module: { exports: {} },
    ...env,
  };
  vm.runInNewContext(src + '\nmodule.exports = STREAM_ADAPTERS;', ctx);
  return ctx.module.exports;
}

function recorder() {
  const events = [];
  return {
    events,
    handlers: {
      onOpen: () => events.push({ type: 'open' }),
      onFrame: f => events.push({ type: 'frame', ...f }),
      onClose: (code, reason) => events.push({ type: 'close', code, reason }),
      onError: e => events.push({ type: 'error', error: e }),
    },
  };
}

test('graphql-ws handshake: init, ack, subscribe, ping/pong, complete', () => {
  const adapters = loadStreamAdapters();
  const { events, handlers } = recorder();
  const conn = adapters['graphql-ws'].connect(
    'ws://example.com/graphql', { opts: { query: 'subscription { x }' } }, handlers);
  const ws = FakeWebSocket.last;
  assert.equal(ws.protocol, 'graphql-transport-ws');
  ws.open();
  assert.deepEqual(ws.sent.map(s => JSON.parse(s).type), ['connection_init']);
  ws.receive(JSON.stringify({ type: 'connection_ack' }));
  assert.deepEqual(events.map(e => e.type), ['open']);
  assert.equal(JSON.parse(ws.sent[1]).type, 'subscribe');
  assert.equal(JSON.parse(ws.sent[1]).payload.query, 'subscription { x }');
  ws.receive(JSON.stringify({ type: 'ping' }));
  assert.equal(JSON.parse(ws.sent[2]).type, 'pong');
  conn.send({ encoding: 'utf8', data: 'subscription { y }' });
  assert.equal(JSON.parse(ws.sent[3]).payload.query, 'subscription { y }');
  ws.receive(JSON.stringify({ type: 'complete', id: '1' }));
  assert.deepEqual(events.at(-1), { type: 'close', code: 1000, reason: 'complete' });
});

test('stomp handshake: CONNECT -> CONNECTED -> SUBSCRIBE', () => {
  const adapters = loadStreamAdapters();
  const { events, handlers } = recorder();
  const conn = adapters.stomp.connect(
    'ws://example.com/stomp', { opts: { destination: '/queue/a' } }, handlers);
  const ws = FakeWebSocket.last;
  ws.open();
  const connect = c.stompDecode(ws.sent[0]).frames[0];
  assert.equal(connect.command, 'CONNECT');
  assert.equal(connect.headers['accept-version'], '1.2');
  assert.equal(connect.headers.host, 'example.com');
  ws.receive('CONNECTED\nversion:1.2\n\n\0');
  assert.deepEqual(events.map(e => e.type), ['open']);
  const sub = c.stompDecode(ws.sent[1]).frames[0];
  assert.equal(sub.command, 'SUBSCRIBE');
  assert.equal(sub.headers.destination, '/queue/a');
  conn.send({ encoding: 'utf8', data: 'hello' });
  const sent = c.stompDecode(ws.sent[2]).frames[0];
  assert.equal(sent.command, 'SEND');
  assert.equal(sent.body, 'hello');
});

test('socket.io handshake: polling sid, upgrade, 40, ping/pong, event framing', async () => {
  const requests = [];
  const adapters = loadStreamAdapters({
    fetch: async url => {
      requests.push(url);
      return { ok: true, text: async () => '0{"sid":"abc"}' };
    },
  });
  const { events, handlers } = recorder();
  const pending = adapters.socketio.connect(
    'ws://example.com/chat', { opts: { event: 'chat' } }, handlers);
  await new Promise(resolve => setImmediate(resolve));
  const ws = FakeWebSocket.last;
  assert.ok(requests[0].includes('EIO=4&transport=polling'), requests[0]);
  assert.ok(ws.url.includes('transport=websocket') && ws.url.includes('sid=abc'), ws.url);
  ws.open();
  assert.equal(ws.sent[0], '40');
  ws.receive('40{"sid":"abc"}');
  const conn = await pending;
  assert.deepEqual(events.map(e => e.type), ['open']);
  ws.receive('2');
  assert.equal(ws.sent[1], '3');
  ws.receive('42["chat",{"text":"hi"}]');
  assert.equal(events.at(-1).type, 'frame');
  assert.equal(events.at(-1).data, JSON.stringify(['chat', { text: 'hi' }]));
  conn.send({ encoding: 'utf8', data: 'yo' });
  assert.equal(ws.sent[2], '42["chat","yo"]');
});
