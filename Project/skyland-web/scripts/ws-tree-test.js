// ═══════════════════════════════════════════════════════════════
// 工作区会话树回归测试
// ═══════════════════════════════════════════════════════════════
// 守护用户明确指出的缺陷：「设置其他工作区后，原先工作区的历史对话全部消失」。
// 该缺陷本质是**信息架构**问题，故断言的是"多工作区同时可见 + 切换不隐藏"。
// 用法: node scripts/ws-tree-test.js
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const SCRIPT = path.join(__dirname, '..', 'static', 'js', 'ws-tree.js');
const SRC = fs.readFileSync(SCRIPT, 'utf8');
const CHATS_KEY = 'skyland_chats';
let passed = 0;
const failures = [];
function check(name, cond, detail) {
  if (cond) { passed++; console.log('  ok  ' + name); return; }
  failures.push(name);
  console.error('FAIL  ' + name + (detail ? ' -> ' + detail : ''));
}
function makeStorage(initial) {
  const map = new Map(Object.entries(initial || {}));
  return {
    get length() { return map.size; },
    key(i) { const a = Array.from(map.keys()); return a[i] === undefined ? null : a[i]; },
    getItem(k) { return map.has(String(k)) ? map.get(String(k)) : null; },
    setItem(k, v) { map.set(String(k), String(v)); },
    removeItem(k) { map.delete(String(k)); },
    snapshot() { return Object.fromEntries(map); },
  };
}
function makeHost() {
  const host = { _html: '' };
  host.style = {};
  host.addEventListener = function () {};
  Object.defineProperty(host, 'innerHTML', {
    get() { return host._html; },
    set(v) { host._html = v; },
  });
  return host;
}
function load(opts) {
  opts = opts || {};
  const local = makeStorage(opts.local);
  local.setItem(CHATS_KEY, JSON.stringify(opts.chats || []));
  const host = makeHost();
  const setCalls = [];
  let current = opts.workdir || { path: '', cloud: false };
  const Workdir = {
    get: function () { return current; },
    set: function (p, c) { setCalls.push({ path: p, cloud: !!c }); current = { path: p, cloud: !!c }; },
    onChange: function (cb) { Workdir._cb = cb; },
    _cb: null,
  };
  const fetchCalls = [];
  async function fetchStub(url) {
    fetchCalls.push(url);
    const m = /workdir=([^&]*)/.exec(url);
    const wd = m ? decodeURIComponent(m[1]) : '';
    const list = (opts.conversations || {})[wd] || [];
    return { ok: true, status: 200, json: async function () { return { status: 'ok', conversations: list }; } };
  }
  const document = { getElementById: function (id) { return id === 'ws-tree' ? host : null; } };
  const win = { localStorage: local, SkylandStore: { KEYS: { chats: CHATS_KEY } } };
  // ws-tree.js 以**裸全局**方式引用 Workdir / SkylandStore（与 app.js 同惯例，
  // 浏览器里 classic script 的顶层 const 即全局）。vm 上下文必须显式注入，
  // 否则 typeof Workdir === 'undefined' 会让模块静默退化成"无工作区"。
  const ctx = vm.createContext({
    Workdir: Workdir,
    SkylandStore: win.SkylandStore,
    window: win, localStorage: local, document: document, fetch: fetchStub,
    confirm: function () { return true; },
    setTimeout: function (fn) { try { fn(); } catch (e) { /* ignore */ } },
    clearTimeout: function () {},
    Date: Date, JSON: JSON, Math: Math, Object: Object, String: String,
    Number: Number, Promise: Promise, isNaN: isNaN, console: console,
    encodeURIComponent: encodeURIComponent, decodeURIComponent: decodeURIComponent,
  });
  win.window = win;
  win.document = document;
  vm.runInContext(SRC, ctx, { filename: SCRIPT });
  return {
    tree: win.SkylandWsTree, local: local, host: host, Workdir: Workdir,
    setCalls: setCalls, fetchCalls: fetchCalls,
    setWorkdir: function (p, c) { current = { path: p, cloud: !!c }; },
  };
}
async function main() {
  console.log('ws-tree:');
  // 1. 稳定 id：同一路径反复切换必须落回同一分组（地基）
  {
    const t = load({}).tree;
    const id1 = t.makeId(t.normalizeIdentity('Project/报告', false));
    const id2 = t.makeId(t.normalizeIdentity('Project/报告', false));
    check('同一路径 id 稳定', id1 === id2, id1 + ' vs ' + id2);
    check('不同路径 id 不同', id1 !== t.makeId(t.normalizeIdentity('Project/其他', false)));
  }
  // 2. Windows 路径归一：大小写/分隔符/尾斜杠差异不能分裂成两个工作区
  {
    const t = load({}).tree;
    const n = function (p) { return t.normalizeIdentity(p, false); };
    check('反斜杠与正斜杠等价', n('C:\\a\\b') === n('C:/a/b'), n('C:\\a\\b') + ' vs ' + n('C:/a/b'));
    check('尾斜杠被去除', n('C:/a/b/') === n('C:/a/b'), n('C:/a/b/'));
    check('盘符大小写归一', n('c:\\a') === n('C:\\a'), n('c:\\a') + ' vs ' + n('C:\\a'));
    check('云工作独立于本地路径', t.normalizeIdentity('', true) === '@cloud');
  }
  // 3. 核心修复：多工作区**同时**渲染；切换只改"当前"标记
  {
    const e = load({
      workdir: { path: 'A', cloud: false },
      conversations: {
        A: [{ id: 1, title: 'A的会话', modified: 3000, messages: 2 }],
        B: [{ id: 1, title: 'B的会话', modified: 2000, messages: 5 },
            { id: 2, title: 'B的另一个', modified: 1000, messages: 1 }],
      },
    });
    e.tree.ensure('A');
    e.tree.ensure('B');
    await e.tree.refresh();
    e.setWorkdir('B', false);
    await e.tree.refresh();
    const html = e.host.innerHTML;
    check('切换后 A 仍在树中（不被隐藏）', /A的会话/.test(html), html.slice(0, 240));
    check('切换后 B 的会话也在', /B的会话/.test(html));
    check('存在"当前"标记', /ws-tree-badge">当前/.test(html));
    check('"当前"标记唯一', (html.match(/ws-tree-badge">当前/g) || []).length === 1,
      String((html.match(/ws-tree-badge">当前/g) || []).length));
    check('最近会话跨工作区聚合', /最近会话/.test(html));
  }
  // 4. 跨工作区打开会话必须先切工作区（会话 id 每工作区各自从 1 开始）
  {
    const e = load({
      workdir: { path: 'B', cloud: false },
      conversations: {
        A: [{ id: 1, title: 'A的会话', modified: 3000, messages: 2 }],
        B: [{ id: 1, title: 'B的会话', modified: 2000, messages: 5 }],
      },
    });
    e.tree.ensure('A');
    e.tree.ensure('B');
    await e.tree.refresh();
    e.setCalls.length = 0;
    e.tree.openConversation(e.tree.makeId('A'), 1);
    check('跨工作区打开会话会切换工作区', e.setCalls.length === 1, JSON.stringify(e.setCalls));
    check('切换到的是目标工作区 A', e.setCalls[0] && e.setCalls[0].path === 'A', JSON.stringify(e.setCalls[0]));
  }
  // 5. 只拉 list 元信息，不做 N+1 详情请求
  {
    const e = load({ workdir: { path: 'A', cloud: false },
      conversations: { A: [{ id: 1, title: 'x', modified: 1, messages: 9 }] } });
    e.tree.ensure('A');
    await e.tree.refresh();
    check('仅 1 次请求（无 N+1 详情拉取）', e.fetchCalls.length === 1, String(e.fetchCalls.length));
    check('请求携带 workdir 参数', /workdir=A/.test(e.fetchCalls[0] || ''), e.fetchCalls[0] || 'none');
  }
  // 6. 单个工作区失败不得拖垮整棵树
  {
    const e = load({ workdir: { path: 'A', cloud: false }, conversations: { A: [] } });
    e.tree.ensure('A');
    e.tree.ensure('MISSING');
    await e.tree.refresh();
    check('有工作区失败时仍渲染出骨架', e.host.innerHTML.length > 0);
  }
  // 7. 无工作区时 localStorage 会话归入"未分组"，一个都不丢
  {
    const e = load({ workdir: { path: '', cloud: false },
      chats: [{ id: 111, title: '旧的本地会话', messages: [{ role: 'user', content: 'x' }] }],
      conversations: {} });
    await e.tree.refresh();
    e.tree.render(null);
    check('无工作区会话出现在最近会话', /旧的本地会话/.test(e.host.innerHTML), e.host.innerHTML.slice(0, 240));
  }
  // 8. XSS：会话标题来自用户，必须转义
  {
    const e = load({ workdir: { path: 'A', cloud: false },
      conversations: { A: [{ id: 1, title: '<img src=x onerror=alert(1)>', modified: 1, messages: 0 }] } });
    e.tree.ensure('A');
    await e.tree.refresh();
    const h = e.host.innerHTML;
    check('恶意标题被转义（不产生裸 img 标签）', !/<img src=x/.test(h) && /&lt;img/.test(h), h.slice(0, 300));
  }
  // 9. 云工作区走 @cloud 通道
  {
    const e = load({ workdir: { path: '', cloud: true },
      conversations: { '@cloud': [{ id: 1, title: '云会话', modified: 10, messages: 1 }] } });
    await e.tree.refresh();
    check('云工作区会话可拉取并渲染', /云会话/.test(e.host.innerHTML), e.host.innerHTML.slice(0, 240));
  }
  console.log('\n' + (failures.length ? 'FAIL' : 'PASS') + '  ' + passed + ' passed, ' + failures.length + ' failed');
  if (failures.length) { failures.forEach(function (f) { console.error('  - ' + f); }); process.exit(1); }
}
main().catch(function (e) { console.error('FATAL: ' + e.stack); process.exit(1); });