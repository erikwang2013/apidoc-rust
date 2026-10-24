// ---------- P1 流式调试 ----------
// 适配器 registry：每个适配器实现 connect(target, opts, handlers) -> { send, close }。
// handlers = { onOpen, onFrame({dir,encoding,data}), onClose(code,reason), onError(msg) }。

const MAX_FRAMES = 1000;
const STREAM_ADAPTERS = {};

function registerStreamAdapter(name, adapter) {
  STREAM_ADAPTERS[name] = adapter;
}

function streamFrameRow(frame) {
  const row = el('div', 'stream-row ' + (frame.dir === 'in' ? 'in' : 'out'));
  row.appendChild(el('span', 'dir', frame.dir === 'in' ? 'IN' : 'OUT'));
  row.appendChild(el('span', 'ts', new Date().toLocaleTimeString()));
  row.appendChild(el('span', 'enc', frame.encoding));
  row.appendChild(el('span', 'data', frame.data));
  return row;
}

function streamPanel(ep, adapterName) {
  const box = el('div');
  const adapter = STREAM_ADAPTERS[adapterName];
  const opts = ep.protocol_opts || {};
  const targetRow = el('div', 'debug-row');
  targetRow.appendChild(el('label', null, 'Target'));
  const target = document.createElement('input');
  target.value = opts.target || opts.url || ep.url || '';
  targetRow.appendChild(target);
  box.appendChild(targetRow);
  if (adapter && adapter.hint) box.appendChild(el('p', 'empty', adapter.hint));
  const status = el('p', 'empty', '未连接');
  box.appendChild(status);
  const log = el('div', 'stream-log');
  box.appendChild(log);

  const sendRow = el('div', 'debug-row');
  sendRow.appendChild(el('label', null, '发送'));
  const fmt = document.createElement('select');
  for (const f of ['utf8', 'hex', 'base64']) {
    const o = document.createElement('option');
    o.value = f;
    o.textContent = f;
    fmt.appendChild(o);
  }
  if (ep.protocol === 'ws-binary' || ep.protocol === 'stomp-binary') fmt.value = 'base64';
  const ta = document.createElement('textarea');
  ta.rows = 3;
  ta.placeholder = '发送内容（按左侧编码解释）';
  const sendBox = el('div');
  sendBox.style.flex = '1';
  sendBox.appendChild(fmt);
  sendBox.appendChild(ta);
  sendRow.appendChild(sendBox);
  box.appendChild(sendRow);

  const actions = el('div', 'debug-actions');
  const connectBtn = el('button', null, '连接');
  const sendBtn = el('button', null, '发送');
  const closeBtn = el('button', null, '断开');
  closeBtn.disabled = true;
  sendBtn.disabled = true;
  actions.appendChild(connectBtn);
  actions.appendChild(sendBtn);
  actions.appendChild(closeBtn);
  box.appendChild(actions);
  box.appendChild(el('p', 'empty', '帧日志上限 ' + MAX_FRAMES + ' 条；断开后停止接收。'));

  let session = null;
  let frames = 0;
  let stopped = false;
  const setStatus = text => { status.textContent = text; };
  const append = frame => {
    if (stopped) return;
    if (frames >= MAX_FRAMES) {
      stopped = true;
      log.appendChild(el('p', 'empty', '已达 ' + MAX_FRAMES + ' 帧上限，连接已断开。'));
      if (session) {
        try { session.close(); } catch (e) {}
        session = null;
      }
      closeBtn.disabled = true;
      sendBtn.disabled = true;
      return;
    }
    frames++;
    log.appendChild(streamFrameRow(frame));
    log.scrollTop = log.scrollHeight;
  };

  connectBtn.onclick = () => {
    if (!adapter) { setStatus('适配器未实现：' + adapterName); return; }
    if (session) return;
    if (!target.value.trim()) { setStatus('请填写 Target'); return; }
    frames = 0;
    stopped = false;
    log.textContent = '';
    setStatus('连接中…');
    try {
      session = adapter.connect(target.value.trim(), { protocol: ep.protocol, opts }, {
        onOpen: () => {
          setStatus('已连接');
          closeBtn.disabled = false;
          sendBtn.disabled = false;
        },
        onFrame: f => append({ dir: f.dir || 'in', encoding: f.encoding || 'utf8', data: f.data }),
        onClose: (code, reason) => {
          setStatus('已断开' + (code !== undefined ? ' (' + code + (reason ? ' ' + reason : '') + ')' : ''));
          session = null;
          closeBtn.disabled = true;
          sendBtn.disabled = true;
        },
        onError: msg => setStatus('错误：' + msg),
      });
    } catch (e) {
      session = null;
      setStatus('连接失败：' + (e && e.message ? e.message : e));
    }
  };
  sendBtn.onclick = () => {
    if (!session) return;
    try {
      session.send({ encoding: fmt.value, data: ta.value });
      append({ dir: 'out', encoding: fmt.value, data: ta.value });
    } catch (e) {
      setStatus('发送失败：' + (e && e.message ? e.message : e));
    }
  };
  closeBtn.onclick = () => {
    if (session) {
      try { session.close(); } catch (e) {}
    }
  };
  return box;
}

registerStreamAdapter('websocket', {
  hint: '浏览器 WebSocket 不能自定义握手 header；认证请放入 URL query。',
  connect(targetUrl, opts, handlers) {
    const ws = new WebSocket(targetUrl);
    ws.binaryType = 'arraybuffer';
    let closed = false;
    ws.onopen = () => { if (!closed) handlers.onOpen(); };
    ws.onmessage = ev => {
      if (closed) return;
      if (typeof ev.data === 'string') {
        handlers.onFrame({ dir: 'in', encoding: 'utf8', data: ev.data });
      } else {
        handlers.onFrame({
          dir: 'in',
          encoding: 'base64',
          data: encodeBytes(new Uint8Array(ev.data), 'base64'),
        });
      }
    };
    ws.onclose = ev => {
      closed = true;
      handlers.onClose(ev.code, ev.reason);
    };
    ws.onerror = () => { if (!closed) handlers.onError('WebSocket 连接失败'); };
    return {
      send(payload) {
        if (ws.readyState !== WebSocket.OPEN) throw new Error('WebSocket 未连接');
        ws.send(payload.encoding === 'utf8' ? payload.data : decodeBytes(payload.encoding, payload.data));
      },
      close() {
        closed = true;
        try { ws.close(); } catch (e) {}
      },
    };
  },
});
