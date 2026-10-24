// ---------- 在线调试 ----------

// P0：有 HTTP method + path 语义的协议复用通用调试表单；其余协议后续阶段接入。
const HTTP_FAMILY = new Set([
  'http', 'http2', 'http3', 'http-binary', 'soap', 'graphql', 'jsonrpc',
  'xmlrpc', 'odata', 'mcp', 'a2a', 'webhook',
]);

// 原始 Body 编码：utf8 原样，hex/base64 解成字节供 fetch 直发。
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

function debugRow(name, placeholder) {
  const row = el('div', 'debug-row');
  row.appendChild(el('label', null, name));
  const input = document.createElement('input');
  if (placeholder) input.placeholder = placeholder;
  row.appendChild(input);
  return { row, name, input };
}
// 递归渲染参数输入行：children 按点分路径 parent.child 展开

function paramRowsFor(container, params, kind, prefix) {
  const rows = [];
  const seen = new Set();
  for (const p of params) {
    const path = prefix ? prefix + '.' + p.name : p.name;
    if (seen.has(path)) continue;
    seen.add(path);
    const r = debugRow(path, p.desc || '');
    r.input.dataset.kind = kind;
    r.input.dataset.path = path;
    r.input.dataset.ty = p.type;
    container.appendChild(r.row);
    rows.push(r);
    if (p.children && p.children.length) rows.push(...paramRowsFor(container, p.children, kind, path));
  }
  return rows;
}
// 从 mock 响应按点分路径取值（嵌套对象叶子），取不到返回 undefined。
// array 段先降入首个元素（mock 数组子行预填取 items[0]）。

function mockAt(mock, kind, path) {
  let cur = mock[kind];
  for (const seg of path.split('.')) {
    if (Array.isArray(cur)) cur = cur[0];
    if (cur && typeof cur === 'object' && seg in cur) cur = cur[seg];
    else return undefined;
  }
  return cur;
}
// 一个输入框的 mock 预填（失败静默留空）

function prefillFromMock(mock, kind, path, input) {
  const v = mockAt(mock, kind, path);
  if (v !== undefined) input.value = typeof v === 'string' ? v : JSON.stringify(v);
}

// 结果提示框（琥珀警示）：CORS 失败、前置/后置脚本抛错共用
function warnBox(text) {
  const w = el('div', null, text);
  w.style.background = '#fef3c7';
  w.style.border = '1px solid #f59e0b';
  w.style.borderRadius = '6px';
  w.style.padding = '10px 12px';
  w.style.color = '#92400e';
  w.style.marginTop = '8px';
  return w;
}
// 调试事件（前置/后置脚本）：new Function 执行用户自己输入的脚本。
// 返回 { ctx, ret, error }；error 非空表示脚本抛错，调用方继续正常流程。
// 返回值是对象时浅合并回 ctx（前置改请求）；是字符串时由后置替换展示文本。

function runScript(code, ctx) {
  if (!code || !code.trim()) return { ctx, ret: undefined, error: null };
  try {
    const ret = new Function('ctx', code)(ctx);
    if (ret && typeof ret === 'object') Object.assign(ctx, ret);
    return { ctx, ret, error: null };
  } catch (e) {
    return { ctx, ret: undefined, error: e && e.message ? e.message : String(e) };
  }
}

function debugForm(ep) {
  const form = el('div');
  const baseRow = el('div', 'debug-row');
  baseRow.appendChild(el('label', null, 'Base URL'));
  const baseUrl = document.createElement('input');
  baseUrl.value = location.origin;
  baseRow.appendChild(baseUrl);
  form.appendChild(baseRow);
  // 参数输入区：route_params / querys / params 三类
  const routeRows = [];
  const queryRows = [];
  const paramRows = [];
  if (ep.route_params && ep.route_params.length) {
    const box = el('div', 'debug-row');
    box.appendChild(el('label', null, 'route_params'));
    const inner = el('div');
    inner.style.flex = '1';
    routeRows.push(...paramRowsFor(inner, ep.route_params, 'route_params', ''));
    box.appendChild(inner);
    form.appendChild(box);
  }
  if (ep.querys && ep.querys.length) {
    const box = el('div', 'debug-row');
    box.appendChild(el('label', null, 'querys'));
    const inner = el('div');
    inner.style.flex = '1';
    queryRows.push(...paramRowsFor(inner, ep.querys, 'querys', ''));
    box.appendChild(inner);
    form.appendChild(box);
  }
  if (ep.params && ep.params.length) {
    const box = el('div', 'debug-row');
    box.appendChild(el('label', null, 'params'));
    const inner = el('div');
    inner.style.flex = '1';
    paramRows.push(...paramRowsFor(inner, ep.params, 'params', ''));
    box.appendChild(inner);
    form.appendChild(box);
  }
  const headerRows = [];
  const headersBox = el('div');
  for (const h of (ep.headers || [])) {
    const r = debugRow(h.name, h.desc || '');
    headerRows.push(r);
    headersBox.appendChild(r.row);
  }
  const adder = el('div', 'debug-row');
  const hName = document.createElement('input');
  hName.placeholder = 'Header 名';
  const hVal = document.createElement('input');
  hVal.placeholder = '值';
  const addBtn = el('button', null, '添加');
  addBtn.onclick = () => {
    const name = hName.value.trim();
    if (!name) return;
    const r = debugRow(name, '');
    r.input.value = hVal.value;
    headerRows.push(r);
    headersBox.insertBefore(r.row, adder);
    hName.value = '';
    hVal.value = '';
  };
  adder.appendChild(el('label', null, '自定义 header'));
  adder.appendChild(hName);
  adder.appendChild(hVal);
  adder.appendChild(addBtn);
  headersBox.appendChild(adder);
  {
    const box = el('div', 'debug-row');
    box.appendChild(el('label', null, 'headers'));
    const inner = el('div');
    inner.style.flex = '1';
    inner.appendChild(headersBox);
    box.appendChild(inner);
    form.appendChild(box);
  }
  // 原始 Body：非 JSON 协议（HTTP binary / SOAP / GraphQL…）直接用原文或 hex/base64；
  // 留空时沿用上方参数自动组装的 JSON body。
  const rawBox = el('details', 'debug-script');
  rawBox.appendChild(el('summary', null, '原始 Body（设置后覆盖参数组装）'));
  const rawFmt = document.createElement('select');
  for (const f of ['utf8', 'hex', 'base64']) {
    const o = document.createElement('option');
    o.value = f;
    o.textContent = f;
    rawFmt.appendChild(o);
  }
  const rawTa = document.createElement('textarea');
  rawTa.rows = 4;
  rawTa.placeholder = '留空 = 按上方参数自动组装 JSON';
  rawBox.appendChild(rawFmt);
  rawBox.appendChild(rawTa);
  form.appendChild(rawBox);
  // 服务端代发：仅在 config.debug.enable_server_side 时出现。可强制 HTTP/2、
  // 绕过 CORS 与内网限制；实际可达性由服务端 DebugConfig 白名单决定。
  const dbgCfg = DOC && DOC.config && DOC.config.debug;
  const viaServer = document.createElement('input');
  viaServer.type = 'checkbox';
  if (dbgCfg && dbgCfg.enable_server_side) {
    const row = el('div', 'debug-row');
    const label = el('label', null, '经服务端发送');
    label.prepend(viaServer);
    row.appendChild(label);
    row.appendChild(el('span', 'empty', '可强制 HTTP/2、绕过 CORS/内网限制'));
    form.appendChild(row);
  }
  // 前置/后置调试事件：可折叠文本域，值持久化到 localStorage（键名同 apidoc_token 风格）
  const scriptBox = (label, key, hint) => {
    const det = el('details', 'debug-script');
    det.appendChild(el('summary', null, label));
    const ta = document.createElement('textarea');
    ta.rows = 4;
    ta.placeholder = hint;
    ta.value = localStorage.getItem(key) || '';
    ta.oninput = () => localStorage.setItem(key, ta.value);
    det.appendChild(ta);
    form.appendChild(det);
    return ta;
  };
  const preTa = scriptBox('前置脚本（发送请求前执行）', 'apidoc_pre_script',
    '可用 ctx：url / method / headers / body / ep。改 ctx 即改本次请求；返回对象会浅合并回 ctx。\n例：ctx.headers["X-Trace"] = "t1";');
  const postTa = scriptBox('后置脚本（响应回来后执行）', 'apidoc_post_script',
    '可用 ctx：status / ms / text / url / method / ep（只读）。返回字符串会替换展示的响应文本，抛错只提示、不影响原始响应。\n例：return "状态 " + ctx.status;');
  // 提交：组装 URL/query/body 后 fetch 直发
  const resultBox = el('div');
  resultBox.id = 'debug-result';
  const sendBtn = el('button', null, '发送请求');
  const actionRow = el('div', 'debug-actions');
  actionRow.appendChild(sendBtn);
  form.appendChild(actionRow);
  form.appendChild(resultBox);
  // 非叶子行（名字是其他行前缀）不预填、不参与组装：值由子行逐项提供，
  // 否则父行 JSON 字符串会遮蔽子行、body 变成转义字符串而非嵌套对象
  const leafRows = rows => rows.filter(r => !rows.some(o => o !== r && o.name.startsWith(r.name + '.')));
  const leafRouteRows = leafRows(routeRows);
  const leafQueryRows = leafRows(queryRows);
  const leafParamRows = leafRows(paramRows);
  // mock 预填：一次请求取三类 mock，逐个输入框填值（失败静默留空）
  fetch(base + 'mock?url=' + encodeURIComponent(ep.url) + '&method=' + encodeURIComponent(ep.method) + (authQuery() ? '&' + authQuery() : ''))
    .then(r => (r.ok ? r.json() : null))
    .then(mock => {
      if (!mock) return;
      for (const r of leafRouteRows) prefillFromMock(mock, 'route_params', r.name, r.input);
      for (const r of leafQueryRows) prefillFromMock(mock, 'querys', r.name, r.input);
      for (const r of leafParamRows) prefillFromMock(mock, 'params', r.name, r.input);
    })
    .catch(() => {});
  sendBtn.onclick = async () => {
    const val = r => (r.input.value.trim() ? encodeURIComponent(r.input.value.trim()) : null);
    // route_params：替换 url 中 {name} / :name 占位符（空值保留原占位符）
    let fullUrl = (baseUrl.value || location.origin) + ep.url;
    fullUrl = fullUrl.replace(/\{([^}]+)\}|:([A-Za-z0-9_]+)/g, (m, braced, coloned) => {
      const key = braced || coloned;
      const row = routeRows.find(r => r.name === key);
      const v = row && val(row);
      return v || m;
    });
    // querys + GET/HEAD 的 params 并入 query string
    const qs = [];
    for (const r of leafQueryRows) { const v = val(r); if (v) qs.push(encodeURIComponent(r.name) + '=' + v); }
    const isGet = ep.method === 'GET' || ep.method === 'HEAD';
    if (isGet) {
      for (const r of leafParamRows) { const v = val(r); if (v) qs.push(encodeURIComponent(r.name) + '=' + v); }
    }
    if (qs.length) fullUrl += (fullUrl.includes('?') ? '&' : '?') + qs.join('&');
    // 其余 method：params 按点分路径组装嵌套 JSON body。
    // array 段（含嵌套）生成 [{}] 承载子行，其余段仍为 {}
    let body;
    if (!isGet) {
      const arr = new Set(paramRows.filter(r => r.input.dataset.ty === 'array').map(r => r.name));
      const obj = {};
      for (const r of leafParamRows) {
        const v = r.input.value.trim();
        if (!v) continue;
        const segs = r.name.split('.');
        let cur = obj;
        for (let i = 0; i < segs.length - 1; i++) {
          const segPath = segs.slice(0, i + 1).join('.');
          if (cur[segs[i]] === undefined) cur[segs[i]] = arr.has(segPath) ? [{}] : {};
          cur = Array.isArray(cur[segs[i]]) ? cur[segs[i]][0] : cur[segs[i]];
        }
        cur[segs[segs.length - 1]] = v;
      }
      if (Object.keys(obj).length) body = JSON.stringify(obj);
    }
    // headers：非空值才发送；自动组装的 JSON body 补 Content-Type（原始 Body 由用户自填）
    let headers = {};
    for (const h of headerRows) {
      const v = h.input.value.trim();
      if (v) headers[h.name] = v;
    }
    const rawText = rawTa.value;
    const rawEncoding = rawText.trim() ? rawFmt.value : 'utf8';
    if (body && !rawText.trim()) headers['Content-Type'] = 'application/json';
    if (rawText.trim()) body = rawText;
    const out = el('div');
    // 前置调试事件：脚本抛错只提示，仍按原值发请求
    const preRun = runScript(preTa.value, { url: fullUrl, method: ep.method, headers, body, ep });
    if (preRun.error) out.appendChild(warnBox('前置脚本执行失败：' + preRun.error));
    fullUrl = preRun.ctx.url;
    headers = preRun.ctx.headers;
    body = preRun.ctx.body;
    const sendViaServer = viaServer.checked && dbgCfg && dbgCfg.enable_server_side;
    const t0 = performance.now();
    try {
      let status, ms, text;
      if (sendViaServer) {
        // 服务端代发：body 用 encoding + data 表达，二进制不会被字符串化
        const payload = body === undefined ? null : { encoding: rawEncoding, data: body };
        const opts = ep.protocol === 'http2' ? { version: '2' } : {};
        const res = await fetch(base + 'debug' + (authQuery() ? '?' + authQuery() : ''), {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            kind: ep.protocol || 'http',
            target: fullUrl,
            method: preRun.ctx.method || ep.method,
            headers,
            body: payload,
            opts,
          }),
        });
        const data = await res.json();
        if (!res.ok) {
          out.appendChild(warnBox('服务端调试失败：' + (data.error || res.status)));
          resultBox.textContent = '';
          resultBox.appendChild(out);
          return;
        }
        status = data.status;
        ms = data.ms;
        text = data.body ? (data.body.encoding === 'base64' ? '[base64] ' + data.body.data : data.body.data) : '';
        out.appendChild(el('p', null, '服务端代发 ' + status + '（' + ms + 'ms）'));
      } else {
        let directBody;
        try {
          directBody = body === undefined ? undefined : decodeBytes(rawEncoding, body);
        } catch (err) {
          out.appendChild(warnBox('Body 解码失败：' + err.message));
          resultBox.textContent = '';
          resultBox.appendChild(out);
          return;
        }
        if (isGet && directBody !== undefined) {
          out.appendChild(warnBox('GET/HEAD 不能带 Body：请勾选「经服务端发送」或改用 POST。'));
          resultBox.textContent = '';
          resultBox.appendChild(out);
          return;
        }
        const res = await fetch(fullUrl, { method: preRun.ctx.method || ep.method, headers, body: directBody });
        text = await res.text();
        status = res.status;
        ms = Math.round(performance.now() - t0);
        out.appendChild(el('p', null, res.status + ' ' + res.statusText + '（' + ms + 'ms）'));
      }
      // 后置调试事件：ctx 只读；脚本抛错只提示，仍展示原始响应
      const postRun = runScript(postTa.value, { status, ms, text, url: fullUrl, method: ep.method, ep });
      if (postRun.error) out.appendChild(warnBox('后置脚本执行失败：' + postRun.error));
      const shown = typeof postRun.ret === 'string' ? postRun.ret : text;
      const pre = document.createElement('pre');
      try {
        pre.textContent = JSON.stringify(JSON.parse(shown), null, 2);
      } catch (e) {
        pre.textContent = shown;
      }
      out.appendChild(pre);
    } catch (e) {
      // CORS 与网络失败无法区分，一条提示覆盖两种
      console.error(e);
      const hint = dbgCfg && dbgCfg.enable_server_side ? '也可勾选「经服务端发送」重试。' : '';
      out.appendChild(warnBox('请求失败：目标服务未开启 CORS、或网络不可达。请确认目标接口所在服务调用了 apidoc_axum::cors_layer 且对当前 Origin 放行（含预检 OPTIONS），并检查 Base URL 是否正确。浏览器控制台有详细错误。' + hint));
    }
    resultBox.textContent = '';
    resultBox.appendChild(out);
  };
  return form;
}

function debugPanel(ep) {
  const section = el('section');
  const titleRow = el('div', 'debug-title');
  titleRow.appendChild(el('h3', null, '在线调试'));
  const show = document.createElement('input');
  show.type = 'checkbox';
  const showLabel = el('label', null, '显示 not_debug 接口（本地调试）');
  showLabel.prepend(show);
  titleRow.appendChild(showLabel);
  const rebuild = () => {
    section.textContent = '';
    section.appendChild(titleRow);
    if (ep.not_debug && !show.checked) {
      section.appendChild(el('p', 'empty', '该接口标记为 not_debug，默认不展示调试表单；勾选上方复选框可强制显示。'));
    } else if (ep.protocol && !HTTP_FAMILY.has(ep.protocol)) {
      // 非 HTTP 族的专用面板（WS / gRPC / MQTT / TCP…）在后续阶段接入。
      section.appendChild(el('p', 'empty', '该协议（' + ep.protocol + '）的调试面板将在后续阶段提供。'));
    } else {
      section.appendChild(debugForm(ep));
    }
  };
  show.onchange = rebuild;
  rebuild();
  return section;
}
// ---------- 启动 ----------
load();
</script>
</body>
</html>
