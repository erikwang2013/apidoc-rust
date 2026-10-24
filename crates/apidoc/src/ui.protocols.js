// P1 HTTP 族协议预设：纯 body 构造（Node 可测）+ 少量 DOM 输入。
// 纯函数不依赖 DOM；DOM 部分只在浏览器调用。

function buildGraphqlBody(query, variablesJson, operationName) {
  const body = { query: query || '' };
  if (variablesJson && variablesJson.trim()) body.variables = JSON.parse(variablesJson);
  if (operationName && operationName.trim()) body.operationName = operationName.trim();
  return JSON.stringify(body);
}

function buildJsonRpcBody(method, paramsJson, id) {
  if (!method || !method.trim()) throw new Error('JSON-RPC method 不能为空');
  const params = paramsJson && paramsJson.trim() ? JSON.parse(paramsJson) : {};
  return JSON.stringify({ jsonrpc: '2.0', method: method.trim(), params, id });
}

function buildODataQuery(input) {
  const map = {
    filter: '$filter', select: '$select', expand: '$expand',
    orderby: '$orderby', top: '$top', skip: '$skip',
  };
  const out = [];
  for (const key of Object.keys(input || {})) {
    const value = input[key];
    if (map[key] && value !== undefined && value !== null && String(value).trim() !== '') {
      out.push({ name: map[key], value: String(value).trim() });
    }
  }
  return out;
}

function buildSoapHeaders(action) {
  const headers = { 'Content-Type': 'text/xml; charset=utf-8' };
  if (action && action.trim()) headers.SOAPAction = action.trim();
  return headers;
}

function protocolField(label, tag, placeholder, value) {
  const row = el('div', 'debug-row');
  row.appendChild(el('label', null, label));
  const input = document.createElement(tag || 'input');
  if (tag === 'textarea') input.rows = 3;
  if (placeholder) input.placeholder = placeholder;
  if (value !== undefined) input.value = value;
  row.appendChild(input);
  return { row, input };
}

function graphqlForm(ep) {
  const o = ep.protocol_opts || {};
  const query = protocolField('GraphQL Query', 'textarea', '{ __typename }', o.query || '');
  const variables = protocolField('Variables JSON', 'textarea', '{}', o.variables || '{}');
  const operation = protocolField('Operation Name', 'input', '', o.operation_name || '');
  const box = el('div');
  box.appendChild(query.row);
  box.appendChild(variables.row);
  box.appendChild(operation.row);
  return {
    extras: box,
    buildBody() {
      if (!query.input.value.trim()) return null;
      return buildGraphqlBody(query.input.value, variables.input.value, operation.input.value);
    },
    buildHeaders() {
      return { 'Content-Type': 'application/json' };
    },
  };
}

function soapForm(ep) {
  const o = ep.protocol_opts || {};
  const xml = protocolField('XML Body', 'textarea', '<soapenv:Envelope>…</soapenv:Envelope>', o.body || '');
  const action = protocolField('SOAPAction', 'input', '', o.soap_action || '');
  const box = el('div');
  box.appendChild(xml.row);
  box.appendChild(action.row);
  return {
    extras: box,
    buildBody() {
      return xml.input.value.trim() ? xml.input.value : null;
    },
    buildHeaders() {
      return buildSoapHeaders(action.input.value);
    },
  };
}

function jsonRpcForm(ep) {
  const o = ep.protocol_opts || {};
  const defaults = { mcp: 'tools/list', a2a: 'message/send' };
  const method = protocolField('Method', 'input', 'method', o.method || defaults[ep.protocol] || '');
  const params = protocolField('Params JSON', 'textarea', '{}', o.params || '{}');
  const id = protocolField('ID', 'input', '1', o.id || '1');
  const box = el('div');
  box.appendChild(method.row);
  box.appendChild(params.row);
  box.appendChild(id.row);
  return {
    extras: box,
    buildBody() {
      return buildJsonRpcBody(method.input.value, params.input.value, id.input.value);
    },
    buildHeaders() {
      return { 'Content-Type': 'application/json' };
    },
  };
}

function xmlRpcForm(ep) {
  const o = ep.protocol_opts || {};
  const template =
    '<?xml version="1.0"?>\n<methodCall>\n  <methodName>method</methodName>\n  <params>\n  </params>\n</methodCall>';
  const xml = protocolField('XML-RPC Body', 'textarea', '<?xml …', o.body || template);
  const box = el('div');
  box.appendChild(xml.row);
  return {
    extras: box,
    buildBody() {
      return xml.input.value.trim() ? xml.input.value : null;
    },
    buildHeaders() {
      return { 'Content-Type': 'text/xml' };
    },
  };
}

function odataForm(ep) {
  const o = ep.protocol_opts || {};
  const fields = ['filter', 'select', 'expand', 'orderby', 'top', 'skip'];
  const inputs = {};
  const box = el('div');
  for (const f of fields) {
    const r = protocolField('$' + f, 'input', '', o[f] || '');
    inputs[f] = r.input;
    box.appendChild(r.row);
  }
  return {
    extras: box,
    buildQuerys() {
      const values = {};
      for (const f of fields) values[f] = inputs[f].value;
      return buildODataQuery(values);
    },
  };
}

function webhookForm() {
  const box = el('div');
  box.appendChild(el('p', 'empty', 'Webhook 出站调试：按普通 HTTP 请求发送；入站捕获在 P6 提供。'));
  return { extras: box };
}

const PROTOCOL_FORMS = {
  soap: soapForm,
  graphql: graphqlForm,
  jsonrpc: jsonRpcForm,
  xmlrpc: xmlRpcForm,
  odata: odataForm,
  mcp: jsonRpcForm,
  a2a: jsonRpcForm,
  webhook: webhookForm,
};

function protocolExtras(ep) {
  if (ep.protocol === 'graphql' && ep.protocol_opts && ep.protocol_opts.operation === 'subscription') {
    return null;
  }
  const make = PROTOCOL_FORMS[ep.protocol];
  return make ? make(ep) : null;
}

if (typeof module !== 'undefined' && module.exports) {
  module.exports = { buildGraphqlBody, buildJsonRpcBody, buildODataQuery, buildSoapHeaders };
}
