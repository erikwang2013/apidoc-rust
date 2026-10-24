// P1 纯编解码：浏览器全局函数 + Node module.exports 双用。
// 约束：不依赖 DOM；二进制以 char code 0-255 的字符串或 Uint8Array 表达。

function bytesOf(input) {
  if (typeof input !== 'string') return input;
  const out = new Uint8Array(input.length);
  for (let i = 0; i < input.length; i++) out[i] = input.charCodeAt(i) & 0xff;
  return out;
}

function binaryString(bytes) {
  let out = '';
  for (let i = 0; i < bytes.length; i++) out += String.fromCharCode(bytes[i]);
  return out;
}

function byteLength(text) {
  if (typeof TextEncoder !== 'undefined') return new TextEncoder().encode(text).length;
  return unescape(encodeURIComponent(text)).length;
}

// utf8 原样返回字符串；hex/base64 返回 Uint8Array（fetch/WebSocket 可直接发送）。
function decodeBytes(encoding, text) {
  if (encoding === 'base64') {
    const bin = atob(text.trim());
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  if (encoding === 'hex') {
    const clean = text.replace(/\s+/g, '');
    if (clean.length % 2 || /[^0-9a-fA-F]/.test(clean)) throw new Error('hex 格式不合法');
    const out = new Uint8Array(clean.length / 2);
    for (let i = 0; i < out.length; i++) out[i] = parseInt(clean.substr(i * 2, 2), 16);
    return out;
  }
  return text;
}

function encodeBytes(input, encoding) {
  const bytes = bytesOf(input);
  if (encoding === 'base64') return btoa(binaryString(bytes));
  if (encoding === 'hex') {
    return Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
  }
  return new TextDecoder('utf-8').decode(bytes);
}

// STOMP 1.2 头转义：\ 换行 回车 冒号。
function stompEsc(s) {
  return String(s)
    .replace(/\\/g, '\\\\')
    .replace(/\r/g, '\\r')
    .replace(/\n/g, '\\n')
    .replace(/:/g, '\\c');
}

function stompUnesc(s) {
  let out = '';
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '\\' && i + 1 < s.length) {
      const n = s[++i];
      out += n === 'n' ? '\n' : n === 'r' ? '\r' : n === 'c' ? ':' : n === '\\' ? '\\' : n;
    } else {
      out += s[i];
    }
  }
  return out;
}

// body 允许 string（按 UTF-8 计长）或 Uint8Array（按字节计长）。
// 返回以 \0 结尾的整帧；二进制 body 以 char code 0-255 的字符串承载。
function stompEncode(command, headers, body) {
  let out = command + '\n';
  for (const key of Object.keys(headers || {})) {
    out += stompEsc(key) + ':' + stompEsc(headers[key]) + '\n';
  }
  if (body === undefined || body === null) {
    return out + '\n\0';
  }
  if (typeof body === 'string') {
    out += 'content-length:' + byteLength(body) + '\n\n' + body;
  } else {
    const bytes = bytesOf(body);
    out += 'content-length:' + bytes.length + '\n\n' + binaryString(bytes);
  }
  return out + '\0';
}

// 增量解码：返回完整帧与未消费的尾部（跨 WebSocket 消息/网络 chunk）。
function stompDecode(buffer) {
  const frames = [];
  let rest = buffer;
  for (;;) {
    const lf = rest.indexOf('\n\n');
    const crlf = rest.indexOf('\r\n\r\n');
    let sep;
    let sepLen;
    if (lf < 0 && crlf < 0) break;
    if (lf >= 0 && (crlf < 0 || lf <= crlf)) {
      sep = lf;
      sepLen = 2;
    } else {
      sep = crlf;
      sepLen = 4;
    }
    const lines = rest.slice(0, sep).replace(/\r/g, '').split('\n');
    const command = lines.shift() || '';
    if (!command) {
      rest = rest.slice(sep + sepLen);
      continue;
    }
    const headers = {};
    for (const line of lines) {
      if (!line) continue;
      const at = line.indexOf(':');
      if (at < 0) continue;
      headers[stompUnesc(line.slice(0, at))] = stompUnesc(line.slice(at + 1));
    }
    const bodyStart = sep + sepLen;
    if (headers['content-length'] !== undefined) {
      const len = parseInt(headers['content-length'], 10);
      const end = bodyStart + len;
      if (rest.length < end + 1) break;
      if (rest[end] !== '\0') {
        // 帧损坏：跳到下一个 NUL 重新同步
        const nul = rest.indexOf('\0', bodyStart);
        if (nul < 0) break;
        rest = rest.slice(nul + 1);
        continue;
      }
      frames.push({ command, headers, body: rest.slice(bodyStart, end) });
      rest = rest.slice(end + 1);
    } else {
      const nul = rest.indexOf('\0', bodyStart);
      if (nul < 0) break;
      frames.push({ command, headers, body: rest.slice(bodyStart, nul) });
      rest = rest.slice(nul + 1);
    }
  }
  return { frames, rest };
}

// Socket.IO 协议（Engine.IO message 前缀 4）：0 connect / 1 disconnect /
// 2 event / 3 ack / 4 error / 5 binary_event / 6 binary_ack。
const SOCKETIO_CODES = {
  connect: '0', disconnect: '1', event: '2', ack: '3', error: '4',
  connect_error: '4', binary_event: '5', binary_ack: '6',
};
const SOCKETIO_NAMES = {
  '0': 'connect', '1': 'disconnect', '2': 'event', '3': 'ack',
  '4': 'error', '5': 'binary_event', '6': 'binary_ack',
};

function socketIoEncode(type, payload) {
  const code = SOCKETIO_CODES[type];
  if (code === undefined) throw new Error('unknown socket.io packet type: ' + type);
  return '4' + code + (payload === undefined || payload === null ? '' : JSON.stringify(payload));
}

function socketIoDecode(text) {
  if (!text || text[0] !== '4') return { type: 'unknown', payload: text };
  const type = SOCKETIO_NAMES[text[1]] || 'unknown';
  const rest = text.slice(2);
  let payload = null;
  if (rest) {
    const start = rest.search(/[\[{]/);
    if (start >= 0) {
      const json = rest.slice(start);
      try { payload = JSON.parse(json); } catch (e) { payload = json; }
    } else {
      payload = rest;
    }
  }
  return { type, payload };
}

// SSE 增量解析：空行分派，data: 多行拼 \n，支持注释与 CRLF。
function sseDecode(buffer) {
  const normalized = buffer.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
  const events = [];
  let idx = 0;
  for (;;) {
    const end = normalized.indexOf('\n\n', idx);
    if (end < 0) break;
    const block = normalized.slice(idx, end);
    idx = end + 2;
    const ev = { event: undefined, data: undefined, id: undefined, retry: undefined };
    const data = [];
    for (const line of block.split('\n')) {
      if (!line || line.startsWith(':')) continue;
      const at = line.indexOf(':');
      const field = at < 0 ? line : line.slice(0, at);
      let value = at < 0 ? '' : line.slice(at + 1);
      if (value.startsWith(' ')) value = value.slice(1);
      if (field === 'data') data.push(value);
      else if (field === 'event') ev.event = value;
      else if (field === 'id') ev.id = value;
      else if (field === 'retry') ev.retry = value;
    }
    if (data.length) ev.data = data.join('\n');
    events.push(ev);
  }
  return { events, rest: normalized.slice(idx) };
}

if (typeof module !== 'undefined' && module.exports) {
  module.exports = {
    decodeBytes, encodeBytes, stompEncode, stompDecode, sseDecode,
    socketIoEncode, socketIoDecode,
  };
}
