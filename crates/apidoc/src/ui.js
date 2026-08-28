// 文档内容全部经 textContent 注入，api.json 里的字符串永不按 HTML 处理。

const base = location.pathname.endsWith('/') ? location.pathname : location.pathname + '/';

const COLORS = { GET:'#16a34a', POST:'#2563eb', PUT:'#d97706', DELETE:'#dc2626', PATCH:'#7c3aed', HEAD:'#64748b', OPTIONS:'#64748b' };

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
  line.appendChild(badge(ep.method));
  if (ep.response_status) for (const s of ep.response_status) line.appendChild(statusBadge(s));
  line.appendChild(el('code', null, ep.url));
  detail.appendChild(line);
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
// 密码在前端 md5 后提交 /apidoc/auth；token 按应用分别存 localStorage，
// 后续所有数据请求（api.json / mock）自动附带 token 与 appKey。

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

const maskMsg = document.getElementById('mask-msg');

const maskPw = document.getElementById('mask-pw');

function showMask(msg) {
  maskMsg.textContent = msg || '';
  mask.style.display = 'flex';
  maskPw.focus();
}

async function doAuth() {
  const pw = maskPw.value.trim();
  if (!pw) return;
  maskMsg.textContent = '';
  const res = await fetch(base + 'auth?password=' + md5(pw) + (APIDOC_APP ? '&appKey=' + encodeURIComponent(APIDOC_APP) : ''));
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
// MD5（标准实现，公开领域算法）：仅用于本地对密码做摘要，绝不落盘
function md5(input) {
  const rotl = (x, n) => (x << n) | (x >>> (32 - n));
  const bytes = [];
  for (let i = 0; i < input.length; i++) {
    const c = input.charCodeAt(i);
    if (c < 0x80) bytes.push(c);
    else if (c < 0x800) bytes.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
    else bytes.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
  }
  const bitLen = bytes.length * 8;
  const padded = bytes.slice();
  padded.push(0x80);
  while (padded.length % 64 !== 56) padded.push(0);
  // 64 位长度低 4 字节（>>> 移位计数按 32 取模，i>=4 时 (bitLen >>> 8*i) 恒等于 bitLen，必须拆开写）
  for (let i = 0; i < 4; i++) padded.push((bitLen >>> (8 * i)) & 0xff);
  padded.push(0, 0, 0, 0); // 高 4 字节：消息 <4GiB 时恒为 0（ponytail: 超 4GiB 需 2^32 拆位）
  const S = [7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,5,9,14,20,5,9,14,20,5,9,14,20,5,9,14,20,4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21];
  const K = [];
  for (let i = 0; i < 64; i++) K[i] = Math.floor(Math.abs(Math.sin(i + 1)) * 0x100000000) >>> 0;
  let a0 = 0x67452301, b0 = 0xefcdab89, c0 = 0x98badcfe, d0 = 0x10325476;
  for (let off = 0; off < padded.length; off += 64) {
    const M = [];
    for (let i = 0; i < 16; i++) M[i] = padded[off + i*4] | (padded[off+i*4+1] << 8) | (padded[off+i*4+2] << 16) | (padded[off+i*4+3] << 24);
    let A = a0, B = b0, C = c0, D = d0;
    for (let i = 0; i < 64; i++) {
      let F, g;
      if (i < 16) { F = (B & C) | (~B & D); g = i; }
      else if (i < 32) { F = (D & B) | (~D & C); g = (5*i + 1) % 16; }
      else if (i < 48) { F = B ^ C ^ D; g = (3*i + 5) % 16; }
      else { F = C ^ (B | ~D); g = (7*i) % 16; }
      F = (F + A + K[i] + M[g]) >>> 0;
      A = D; D = C; C = B;
      B = (B + rotl(F, S[i])) >>> 0;
    }
    a0 = (a0 + A) >>> 0; b0 = (b0 + B) >>> 0; c0 = (c0 + C) >>> 0; d0 = (d0 + D) >>> 0;
  }
  const hex = n => {
    let s = '';
    for (let i = 0; i < 4; i++) s += ((n >>> (8 * i)) & 0xff).toString(16).padStart(2, '0');
    return s;
  };
  return hex(a0) + hex(b0) + hex(c0) + hex(d0);
}
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
  const res = await fetch(base + 'api.json' + (authQuery() ? '?' + authQuery() : ''));
  if (res.status === 401) { showMask(''); return; }
  const doc = await res.json();
  document.getElementById('title').textContent = doc.config.title || 'API Documentation';
  document.getElementById('subtitle').textContent = doc.config.description || '';
  if (!appsList.length) buildSelector(doc);
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
  const pick = (gi, ei) => {
    const links = nav.querySelectorAll('a');
    for (const x of links) x.classList.remove('active');
    links[ei].classList.add('active');
    render(groups[names[gi]][ei]);
  };
  names.forEach((name, gi) => {
    nav.appendChild(el('h2', null, name));
    // sort 注解权重优先（大者在前），未标注视为 0；相同权重时稳定排序
    // 保留声明序（seq），再按 method/url 兜底
    groups[name].sort((a, b) => (b.sort || 0) - (a.sort || 0) || a.method.localeCompare(b.method) || a.url.localeCompare(b.url));
    groups[name].forEach((ep, ei) => {
      const a = el('a');
      a.appendChild(badge(ep.method));
      a.appendChild(el('span', null, ep.title));
      a.onclick = () => {
        location.hash = '#g' + gi + '/e' + ei;
        pick(gi, ei);
      };
      nav.appendChild(a);
    });
  });
  const m = location.hash.match(/^#g(\d+)\/e(\d+)$/);
  const gi = m ? Math.min(+m[1], names.length - 1) : 0;
  const ei = m ? Math.min(+m[2], groups[names[gi]].length - 1) : 0;
  pick(gi, ei);
}
