// ---------- P1 WebRTC / DataChannel 媒体面板 ----------
// 信令契约：POST <Signaling URL> {type:"offer", sdp} → {type:"answer", sdp, iceServers?}
// WebRTC 需要 HTTPS 或 localhost；跨域信令需目标服务放行 CORS。

function waitForIce(pc, timeoutMs) {
  if (pc.iceGatheringState === 'complete') return Promise.resolve();
  return new Promise(resolve => {
    const timer = setTimeout(done, timeoutMs);
    function done() {
      clearTimeout(timer);
      pc.removeEventListener('icegatheringstatechange', check);
      resolve();
    }
    function check() {
      if (pc.iceGatheringState === 'complete') done();
    }
    pc.addEventListener('icegatheringstatechange', check);
  });
}

function webrtcPanel(ep) {
  const o = ep.protocol_opts || {};
  const box = el('div');
  const target = protocolField('Signaling URL', 'input', 'https://example.com/signal', o.target || '');
  const label = protocolField('DataChannel', 'input', 'data', o.channel || 'data');
  const ice = protocolField('ICE Servers JSON', 'textarea', '[]', o.ice_servers || '[]');
  box.appendChild(target.row);
  box.appendChild(label.row);
  box.appendChild(ice.row);
  box.appendChild(el('p', 'empty',
    '信令契约：POST {type:"offer",sdp} → {type:"answer",sdp}。WebRTC 需要 HTTPS 或 localhost；跨域信令需目标放行 CORS。'));

  const status = el('p', 'empty', '未连接');
  box.appendChild(status);
  const log = el('div', 'stream-log');
  box.appendChild(log);

  const sendRow = el('div', 'debug-row');
  sendRow.appendChild(el('label', null, '发送'));
  const fmt = document.createElement('select');
  for (const f of ['utf8', 'hex', 'base64']) {
    const opt = document.createElement('option');
    opt.value = f;
    opt.textContent = f;
    fmt.appendChild(opt);
  }
  const ta = document.createElement('textarea');
  ta.rows = 3;
  ta.placeholder = 'DataChannel 消息（按左侧编码解释）';
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
  sendBtn.disabled = true;
  closeBtn.disabled = true;
  actions.appendChild(connectBtn);
  actions.appendChild(sendBtn);
  actions.appendChild(closeBtn);
  box.appendChild(actions);

  let pc = null;
  let channel = null;
  let frames = 0;
  let stopped = false;
  const setStatus = text => { status.textContent = text; };
  const append = frame => {
    if (stopped) return;
    if (frames >= MAX_FRAMES) {
      stopped = true;
      log.appendChild(el('p', 'empty', '已达 ' + MAX_FRAMES + ' 帧上限，连接已断开。'));
      if (pc) { try { pc.close(); } catch (e) {} }
      pc = null;
      channel = null;
      sendBtn.disabled = true;
      closeBtn.disabled = true;
      return;
    }
    frames++;
    log.appendChild(streamFrameRow(frame));
    log.scrollTop = log.scrollHeight;
  };

  connectBtn.onclick = async () => {
    if (pc) return;
    if (!target.input.value.trim()) { setStatus('请填写 Signaling URL'); return; }
    let iceServers = [];
    try {
      iceServers = JSON.parse(ice.input.value || '[]');
    } catch (e) {
      setStatus('ICE Servers JSON 非法');
      return;
    }
    frames = 0;
    stopped = false;
    log.textContent = '';
    setStatus('连接中…');
    try {
      pc = new RTCPeerConnection({ iceServers });
      pc.onconnectionstatechange = () => {
        if (pc && (pc.connectionState === 'failed' || pc.connectionState === 'disconnected')) {
          setStatus('连接状态：' + pc.connectionState);
        }
      };
      channel = pc.createDataChannel(label.input.value || 'data');
      channel.binaryType = 'arraybuffer';
      channel.onopen = () => {
        setStatus('已连接');
        sendBtn.disabled = false;
        closeBtn.disabled = false;
      };
      channel.onclose = () => {
        setStatus('DataChannel 已关闭');
        sendBtn.disabled = true;
        closeBtn.disabled = true;
      };
      channel.onmessage = ev => {
        if (typeof ev.data === 'string') {
          append({ dir: 'in', encoding: 'utf8', data: ev.data });
        } else {
          append({ dir: 'in', encoding: 'base64', data: encodeBytes(new Uint8Array(ev.data), 'base64') });
        }
      };
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      await waitForIce(pc, 2000);
      const res = await fetch(target.input.value.trim(), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ type: 'offer', sdp: pc.localDescription.sdp }),
      });
      if (!res.ok) throw new Error('signaling HTTP ' + res.status);
      const answer = await res.json();
      await pc.setRemoteDescription({ type: answer.type || 'answer', sdp: answer.sdp });
      setStatus('等待 DataChannel…');
    } catch (e) {
      setStatus('连接失败：' + (e.message || e));
      if (pc) { try { pc.close(); } catch (e2) {} }
      pc = null;
      channel = null;
      sendBtn.disabled = true;
      closeBtn.disabled = true;
    }
  };

  sendBtn.onclick = () => {
    if (!channel || channel.readyState !== 'open') return;
    try {
      const data = fmt.value === 'utf8' ? ta.value : decodeBytes(fmt.value, ta.value);
      channel.send(data);
      append({ dir: 'out', encoding: fmt.value, data: ta.value });
    } catch (e) {
      setStatus('发送失败：' + (e.message || e));
    }
  };

  closeBtn.onclick = () => {
    if (pc) { try { pc.close(); } catch (e) {} }
    pc = null;
    channel = null;
    sendBtn.disabled = true;
    closeBtn.disabled = true;
    setStatus('已断开');
  };

  return box;
}
