// P1 纯编解码测试：node --test crates/apidoc/tests/js/codecs.test.js
const test = require('node:test');
const assert = require('node:assert');
const c = require('../../src/ui.codecs.js');

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

test('socketIo encode/decode event', () => {
  assert.equal(c.socketIoEncode('event', ['chat', { text: 'hi' }]), '42["chat",{"text":"hi"}]');
  assert.deepEqual(c.socketIoDecode('42["chat",{"text":"hi"}]'), { type: 'event', payload: ['chat', { text: 'hi' }] });
  assert.deepEqual(c.socketIoDecode('40'), { type: 'connect', payload: null });
});

test('graphqlWs encode/decode', () => {
  assert.equal(c.graphqlWsEncode('subscribe', '1', { query: '{x}' }), '{"type":"subscribe","id":"1","payload":{"query":"{x}"}}');
  assert.deepEqual(c.graphqlWsDecode('{"type":"next","id":"1","payload":{"data":{}}}'), { type: 'next', id: '1', payload: { data: {} } });
});

test('sseDecode splits across chunks and joins multiline data', () => {
  const a = c.sseDecode('event: message\ndata: one\ndata: two\n\n');
  assert.deepEqual(a.events, [{ event: 'message', data: 'one\ntwo', id: undefined, retry: undefined }]);
  assert.equal(a.rest, '');
  const b = c.sseDecode('data: half');
  assert.equal(b.events.length, 0);
  assert.equal(b.rest, 'data: half');
});
