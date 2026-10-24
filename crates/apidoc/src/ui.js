// 文档内容全部经 textContent 注入，api.json 里的字符串永不按 HTML 处理。

const base = location.pathname.endsWith('/') ? location.pathname : location.pathname + '/';

const COLORS = { GET:'#16a34a', POST:'#2563eb', PUT:'#d97706', DELETE:'#dc2626', PATCH:'#7c3aed', HEAD:'#64748b', OPTIONS:'#64748b' };
// P0 协议徽标配色：按协议族分组，未知协议落灰色。
const PROTOCOL_COLORS = {
  websocket:'#0ea5e9', 'ws-binary':'#0ea5e9', socketio:'#0ea5e9', stomp:'#0284c7', 'stomp-binary':'#0284c7',
  sse:'#8b5cf6', graphql:'#e10098', grpc:'#0891b2', thrift:'#0891b2', dubbo:'#0891b2', hessian:'#0891b2',
  jsonrpc:'#2563eb', xmlrpc:'#2563eb', soap:'#d97706', odata:'#d97706', mcp:'#7c3aed', a2a:'#7c3aed',
  mqtt:'#f59e0b', amqp:'#f59e0b', kafka:'#111827', xmpp:'#0f766e', sip:'#0f766e',
  tcp:'#64748b', udp:'#64748b', raw:'#64748b', unix:'#64748b', coap:'#059669', lwm2m:'#059669',
  modbus:'#b45309', dnp3:'#b45309', iec104:'#b45309', bacnet:'#b45309', opcua:'#b45309',
  rtp:'#be123c', rtcp:'#be123c', rtsp:'#be123c', rtmp:'#be123c', hls:'#be123c', webrtc:'#7c3aed',
  webhook:'#d97706', iso8583:'#334155', 'http-binary':'#2563eb', http2:'#2563eb', http3:'#2563eb',
};

// 最近一次加载的 api.json：调试面板读取 config.debug 决定是否显示服务端发送开关。
let DOC = null;

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function badge(method) {
  const b = el('span', 'badge', method);
  b.style.background = COLORS[method] || '#64748b';
  return b;
}

function protocolBadge(protocol) {
  const b = el('span', 'badge', protocol.toUpperCase());
  b.style.background = PROTOCOL_COLORS[protocol] || '#64748b';
  return b;
}

// HTTP 族沿用 method 徽标；非 HTTP 协议显示协议名。
function epBadge(ep) {
  return !ep.protocol || ep.protocol === 'http' ? badge(ep.method) : protocolBadge(ep.protocol);
}

function firstSeg(ep) {
  return ep.url.replace(/^\/+/, '').split('/').filter(Boolean)[0] || '';
}
// 分组优先读 group 注解（renderEps 里 ep.group || groupOf(ep)）；未标注的
// endpoint 走启发式：所有 endpoint 首段相同（如全是 /api/...）时跳过公共首段
// 用第二段分组（/api/user/info -> "user"）；否则用各自首段。

let allShare = false;

function groupOf(ep) {
  const segs = ep.url.replace(/^\/+/, '').split('/').filter(Boolean);
  if (!segs.length) return '未分组';
  if (allShare && segs.length > 1) return segs[1];
  return segs[0];
}
// 灰色小徽标：response_status / 示例 code

function statusBadge(s) {
  const b = el('span', 'badge', s);
  b.style.background = '#9ca3af';
  return b;
}
// 成功/失败示例：code 徽标 + 响应体原文（textContent 注入，pre-wrap）

function exampleSection(title, list) {
  const section = el('section');
  section.appendChild(el('h3', null, title));
  for (const ex of list) {
    section.appendChild(statusBadge(ex.code));
    const pre = document.createElement('pre');
    pre.style.whiteSpace = 'pre-wrap';
    pre.textContent = ex.example;
    section.appendChild(pre);
  }
  return section;
}

function paramTable(title, list) {
  const section = el('section');
  section.appendChild(el('h3', null, title));
  if (!list || !list.length) {
    section.appendChild(el('p', 'empty', '无'));
    return section;
  }
  const table = el('table');
  const head = el('tr');
  for (const h of ['名称', '类型', '必填', '默认值', '描述', '示例']) head.appendChild(el('th', null, h));
  table.appendChild(head);
  const tbody = el('tbody');
  const addRows = (params, depth) => {
    for (const p of params) {
      const tr = el('tr');
      const name = el('td', 'name', p.name);
      name.style.paddingLeft = depth * 18 + 'px';
      tr.appendChild(name);
      tr.appendChild(el('td', null, p.type));
      tr.appendChild(el('td', null, p.required ? '是' : '否'));
      tr.appendChild(el('td', null, p.default ?? ''));
      tr.appendChild(el('td', null, p.desc ?? ''));
      tr.appendChild(el('td', null, p.mock ?? ''));
      tbody.appendChild(tr);
      if (p.children && p.children.length) addRows(p.children, depth + 1);
    }
  };
  addRows(list, 0);
  table.appendChild(tbody);
  section.appendChild(table);
  return section;
}

function render(ep) {
  const detail = document.getElementById('detail');
  detail.textContent = '';
  detail.appendChild(el('h2', null, ep.title));
  if (ep.author) detail.appendChild(el('p', 'desc', '作者：' + ep.author));
  if (ep.ref) detail.appendChild(el('p', 'desc', '参考接口：' + ep.ref));
  const line = el('div', 'endpoint-line');
  line.appendChild(epBadge(ep));
  if (ep.response_status) for (const s of ep.response_status) line.appendChild(statusBadge(s));
  line.appendChild(el('code', null, ep.url));
  line.appendChild(shareButton(ep));
  detail.appendChild(line);
  // 非 HTTP 协议的连接/操作参数：target 由 protocol_opts 提供，其余作为标签展示
  if (ep.protocol && ep.protocol !== 'http' && ep.protocol_opts) {
    const optline = el('div', 'endpoint-line');
    for (const key of ['target', 'service', 'method', 'topic', 'version']) {
      if (ep.protocol_opts[key]) optline.appendChild(el('code', null, key + '=' + ep.protocol_opts[key]));
    }
    if (optline.childNodes.length) detail.appendChild(optline);
  }
  if (ep.desc) detail.appendChild(el('p', 'desc', ep.desc));
  if (ep.tags && ep.tags.length) {
    const tagline = el('div', 'endpoint-line');
    for (const t of ep.tags) tagline.appendChild(el('code', null, t));
    detail.appendChild(tagline);
  }
  detail.appendChild(paramTable('请求参数', ep.params));
  detail.appendChild(paramTable('Query 参数', ep.querys));
  detail.appendChild(paramTable('返回字段', ep.returned));
  if (ep.success && ep.success.length) detail.appendChild(exampleSection('成功示例', ep.success));
  if (ep.error && ep.error.length) detail.appendChild(exampleSection('失败示例', ep.error));
  if (ep.md) {
    const s = el('section');
    s.appendChild(el('h3', null, '补充说明'));
    const pre = document.createElement('pre');
    pre.style.whiteSpace = 'pre-wrap';
    pre.textContent = ep.md;
    s.appendChild(pre);
    detail.appendChild(s);
  }
  // 在线调试：调试数据从 /apidoc/mock 按需取，api.json 零变化
  detail.appendChild(debugPanel(ep));
}
// ---------- M6a 密码鉴权 ----------
// 密码以 POST body 交给 /apidoc/auth（前端不做摘要，摘要本身会被当成新密码）；
// token 按应用分别存 localStorage，后续所有数据请求（api.json / mock）自动附带
// token 与 appKey。

let APIDOC_APP = ''; // 当前选中的应用 key（'' = 默认应用）

const tokKey = () => 'apidoc_token' + (APIDOC_APP ? '_' + APIDOC_APP : '');

const authQuery = () => {
  const p = [];
  const t = localStorage.getItem(tokKey());
  if (t) p.push('token=' + encodeURIComponent(t));
  if (APIDOC_APP) p.push('appKey=' + encodeURIComponent(APIDOC_APP));
  return p.join('&');
};

const mask = document.getElementById('mask');

// v2 分享深链：?app=<key>&ep=<接口 url>&method=<方法>&token=<t>
// token 先落盘（按 appKey 分键）→ 选中目标应用 → 首次渲染后定位到该接口。
const shareParams = new URLSearchParams(location.search);
let deepAppDone = false;
let deepEpDone = false;

// v2 分享按钮：向 /apidoc/share 换取深链，优先写剪贴板；失败则展示可复制的输入框
function shareButton(ep) {
  const btn = el('button', null, '分享');
  btn.onclick = async () => {
    btn.textContent = '生成中…';
    const q = new URLSearchParams();
    if (APIDOC_APP) q.set('app', APIDOC_APP);
    q.set('url', ep.url);
    q.set('method', ep.method);
    q.set('base', location.origin);
    const aq = authQuery();
    const res = await fetch(base + 'share?' + q.toString() + (aq ? '&' + aq : ''));
    if (!res.ok) { btn.textContent = '分享失败'; return; }
    const link = (await res.json()).url;
    try {
      await navigator.clipboard.writeText(link);
      btn.textContent = '已复制';
    } catch (e) {
      btn.textContent = '分享';
      const box = document.createElement('input');
      box.readOnly = true;
      box.value = link;
      btn.parentNode.appendChild(box);
      box.select();
    }
    setTimeout(() => { btn.textContent = '分享'; }, 1500);
  };
  return btn;
}


const maskMsg = document.getElementById('mask-msg');

const maskPw = document.getElementById('mask-pw');

function showMask(msg) {
  maskMsg.textContent = msg || '';
  mask.style.display = 'flex';
  maskPw.focus();
}

async function doAuth() {
  // 原样提交：服务端比的是配置里的原始串，前端 trim 会让"首尾带空格的密码"永远登不进去
  const pw = maskPw.value;
  if (!pw.trim()) return;
  maskMsg.textContent = '';
  // 密码走 POST body：不进 URL、不进访问日志与浏览器历史；服务端恒定时间比对。
  const res = await fetch(base + 'auth', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(APIDOC_APP ? { password: pw, appKey: APIDOC_APP } : { password: pw }),
  });
  if (res.status === 200) {
    localStorage.setItem(tokKey(), (await res.json()).token);
    location.reload();
  } else if (res.status === 401) {
    showMask('密码错误');
  } else {
    mask.style.display = 'none'; // 404：服务端已关闭鉴权，按公开文档加载
    load();
  }
}
document.getElementById('mask-btn').onclick = doAuth;
maskPw.addEventListener('keydown', e => { if (e.key === 'Enter') doAuth(); });
// ---------- M6b 多应用/版本 ----------
// doc.apps 为配置树（key/title/items/endpoints）；未配置 apps 时选择器隐藏，
// 行为与 M5 完全一致。切换应用/版本后按该节点渲染 endpoints，并重拉数据
// （不同应用可能有独立密码，token 按 appKey 区分）。

const appsSel = document.getElementById('apps-sel');

const verSel = document.getElementById('ver-sel');

const selRow = document.getElementById('sel-row');

let appsList = []; // [{ title, node, versions: [{node, label}] }]，node=null 表示默认应用

function flattenVersions(node, depth, out) {
  for (const it of (node.items || [])) {
    out.push({ node: it, label: '　'.repeat(depth) + it.title });
    flattenVersions(it, depth + 1, out);
  }
}

function buildSelector(doc) {
  if (!(doc.apps || []).length) return;
  selRow.style.display = 'flex';
  if (doc.endpoints.length) appsList.push({ title: '默认', node: null, versions: [] });
  for (const app of doc.apps) {
    const versions = [];
    flattenVersions(app, 1, versions);
    appsList.push({ title: app.title, node: app, versions });
  }
  for (const a of appsList) appsSel.appendChild(el('option', null, a.title));
  appsSel.onchange = () => { fillVersions(); load(); };
  verSel.onchange = () => { load(); };
  fillVersions();
}

function fillVersions() {
  verSel.textContent = '';
  const a = appsList[appsSel.selectedIndex];
  if (!a || !a.versions.length) { verSel.style.display = 'none'; return; }
  verSel.style.display = '';
  for (const v of a.versions) verSel.appendChild(el('option', null, v.label));
}

function selectedEps(doc) {
  const a = appsList[appsSel.selectedIndex];
  if (!a) return doc.endpoints;
  const node = a.versions.length ? a.versions[verSel.selectedIndex].node : a.node;
  APIDOC_APP = node ? node.key : '';
  return node ? node.endpoints : doc.endpoints;
}
// 拉取 api.json：401 视为需要鉴权（token 缺失/过期），展示密码遮罩

async function load() {
  // v2 深链：先把 ?app= 落成"当前应用"再存 token —— token 的 localStorage 键、
  // 首次请求带的 appKey 都取决于它。顺序反了就会：应用 token 存进应用键、
  // 首次 api.json 却按全局键读取（空）→ 401 → 遮罩又不带 appKey → 密码永远不对，
  // 应用分享链接直接打不开。
  if (!APIDOC_APP && shareParams.get('app')) APIDOC_APP = shareParams.get('app');
  // v2 深链：token 先落盘（按 appKey 分键，与 mask 登录后的键一致）
  const st = shareParams.get('token');
  if (st) {
    const ak = APIDOC_APP;
    localStorage.setItem('apidoc_token' + (ak ? '_' + ak : ''), st);
    // 落盘后立刻把 token 从地址栏与历史里摘掉（delete 只改对象，不改 location）：
    // 留着等于把 bearer token 挂在浏览历史里。深链参数保留，刷新仍定位同一接口。
    shareParams.delete('token');
    const rest = shareParams.toString();
    history.replaceState(null, '', location.pathname + (rest ? '?' + rest : '') + location.hash);
  }
  const res = await fetch(base + 'api.json' + (authQuery() ? '?' + authQuery() : ''));
  if (res.status === 401) { showMask(''); return; }
  const doc = await res.json();
  DOC = doc;
  document.getElementById('title').textContent = doc.config.title || 'API Documentation';
  document.getElementById('subtitle').textContent = doc.config.description || '';
  if (!appsList.length) buildSelector(doc);
  // v2 深链：首次加载后按 ?app= 选中目标应用（只做一次，避免与切换逻辑互相触发）
  if (!deepAppDone && shareParams.get('app')) {
    deepAppDone = true;
    const i = appsList.findIndex(a => a.node && a.node.key === shareParams.get('app'));
    if (i >= 0) { appsSel.selectedIndex = i; fillVersions(); load(); return; }
  }
  renderEps(selectedEps(doc));
}

function renderEps(eps) {
  const nav = document.getElementById('groups');
  const detail = document.getElementById('detail');
  // 每次渲染清空：切换应用/版本时 nav 不累积旧 anchor（事件残留 + 索引错位）
  nav.textContent = '';
  detail.textContent = '';
  if (!eps.length) {
    detail.appendChild(el('p', 'empty', '暂无接口文档'));
    return;
  }
  allShare = eps.every(e => firstSeg(e) === firstSeg(eps[0]));
  const groups = {}; // 组名 -> endpoint 列表
  for (const ep of eps) {
    const g = ep.group || groupOf(ep); // group 注解优先，启发式兜底
    (groups[g] || (groups[g] = [])).push(ep);
  }
  const names = Object.keys(groups).sort((a, b) => a.localeCompare(b));
  // 各组的链接单独记：nav 里所有 <a> 是整篇文档的先后顺序，拿组内下标 ei 去索引
  // 会点到别的组（gi>0 必然点错，深链与首次渲染都走 pick）
  const groupAnchors = [];
  const pick = (gi, ei) => {
    for (const x of nav.querySelectorAll('a')) x.classList.remove('active');
    groupAnchors[gi][ei].classList.add('active');
    render(groups[names[gi]][ei]);
  };
  names.forEach((name, gi) => {
    nav.appendChild(el('h2', null, name));
    // sort 注解权重优先（大者在前），未标注视为 0；相同权重时稳定排序
    // 保留声明序（seq），再按 method/url 兜底
    groups[name].sort((a, b) => (b.sort || 0) - (a.sort || 0) || a.method.localeCompare(b.method) || a.url.localeCompare(b.url));
    const anchors = [];
    groupAnchors.push(anchors);
    groups[name].forEach((ep, ei) => {
      const a = el('a');
      a.appendChild(epBadge(ep));
      a.appendChild(el('span', null, ep.title));
      a.onclick = () => {
        location.hash = '#g' + gi + '/e' + ei;
        pick(gi, ei);
      };
      anchors.push(a);
      nav.appendChild(a);
    });
  });
  const m = location.hash.match(/^#g(\d+)\/e(\d+)$/);
  const gi = m ? Math.min(+m[1], names.length - 1) : 0;
  const ei = m ? Math.min(+m[2], groups[names[gi]].length - 1) : 0;
  pick(gi, ei);
  // v2 深链：首次渲染后按 ?ep=&method= 定位接口（找不到则停在默认选中）
  if (!deepEpDone && shareParams.get('ep')) {
    deepEpDone = true;
    const wantUrl = shareParams.get('ep');
    const wantM = shareParams.get('method') || '';
    outer: for (let g2 = 0; g2 < names.length; g2++) {
      const list = groups[names[g2]];
      for (let e2 = 0; e2 < list.length; e2++) {
        if (list[e2].url === wantUrl && (!wantM || list[e2].method === wantM)) {
          location.hash = '#g' + g2 + '/e' + e2;
          pick(g2, e2);
          break outer;
        }
      }
    }
  }
}
