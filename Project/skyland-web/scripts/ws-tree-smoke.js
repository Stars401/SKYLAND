// ═══════════════════════════════════════════════════════════════
// 工作区会话树 — 真实浏览器验证（headless Edge + CDP）
// ═══════════════════════════════════════════════════════════════
// 复现用户报告的原始缺陷并验证已修复：
//   1) 先后设置工作区 A、B
//   2) 停在 A，切到 B
//   3) 断言 A 的会话**依然可见**（旧实现这里会整片消失）
// 用法: node scripts/ws-tree-smoke.js [baseUrl] [--shots <dir>]
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const http = require('node:http');
const net = require('node:net');
const path = require('node:path');

const argv = process.argv.slice(2);
const SHOT_DIR = argv.includes('--shots') ? argv[argv.indexOf('--shots') + 1] : null;
const BASE = argv.find((a, i) => !a.startsWith('--') && argv[i - 1] !== '--shots') || 'http://127.0.0.1:3000';
const EDGE = [
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
].find(p => fs.existsSync(p));
const sleep = ms => new Promise(r => setTimeout(r, ms));
// 夹具工作区。显示名取路径末段（与 ws-tree 的 displayName 一致）。
// 断言一律用这两个常量派生，避免"改了夹具名、断言忘了改"这类假失败。
const WD_A = 'Project/fixture-A';
const WD_B = 'Project/fixture-B';
const NAME_A = WD_A.split('/').pop();
const NAME_B = WD_B.split('/').pop();
let passed = 0; const failures = [];
function check(name, cond, detail) {
  if (cond) { passed++; console.log('  ok  ' + name); return; }
  failures.push(name); console.error('FAIL  ' + name + (detail ? ' -> ' + String(detail).slice(0, 300) : ''));
}
function freePort() {
  return new Promise((res, rej) => { const s = net.createServer(); s.once('error', rej); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(e => e ? rej(e) : res(p)); }); });
}
function getJson(p, port) {
  return new Promise((res, rej) => { http.get({ host: '127.0.0.1', port, path: p }, r => { let d = ''; r.on('data', c => d += c); r.on('end', () => { try { res(JSON.parse(d)); } catch (e) { rej(e); } }); }).on('error', rej); });
}
async function main() {
  if (!EDGE) { console.error('FATAL: 未找到 Edge/Chrome'); process.exit(2); }
  const dp = await freePort();
  const profile = path.join(__dirname, '.ws-profile-' + process.pid);
  fs.rmSync(profile, { recursive: true, force: true });
  if (SHOT_DIR) fs.mkdirSync(SHOT_DIR, { recursive: true });
  const browser = spawn(EDGE, ['--headless', '--remote-debugging-port=' + dp, '--remote-debugging-address=127.0.0.1',
    '--remote-allow-origins=*', '--user-data-dir=' + profile, '--no-first-run', '--disable-gpu',
    '--window-size=1400,900', 'about:blank'], { stdio: 'ignore' });
  const cleanup = () => { try { browser.kill(); } catch (e) {} try { fs.rmSync(profile, { recursive: true, force: true }); } catch (e) {} };
  process.on('exit', cleanup);
  let targets = null;
  for (let i = 0; i < 60; i++) { await sleep(250); try { targets = await getJson('/json/list', dp); if (targets.some(t => t.type === 'page')) break; } catch (e) {} }
  const page = targets && targets.find(t => t.type === 'page');
  if (!page) { console.error('FATAL: CDP 未就绪'); cleanup(); process.exit(2); }
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error('CDP 连接失败')); });
  let id = 0; const pending = new Map(); const errors = [];
  ws.onmessage = ev => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); return; }
    if (m.method === 'Runtime.exceptionThrown') {
      const d = m.params.exceptionDetails;
      errors.push(d.text + ' ' + ((d.exception && d.exception.description) || ''));
    }
    if (m.method === 'Runtime.consoleAPICalled' && m.params.type === 'error') {
      errors.push('console.error: ' + m.params.args.map(a => a.value || a.description || '').join(' '));
    }
  };
  const send = (method, params) => new Promise(res => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params: params || {} })); });
  const evaluate = async expr => {
    const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.result && r.result.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
    return r.result && r.result.result ? r.result.result.value : undefined;
  };
  await send('Page.enable'); await send('Runtime.enable');
  // 自建夹具：先经 API 写入两个工作区的会话，再开页面。
  // 不能依赖"环境里恰好有数据" —— 那会让用例在干净机器上假失败。
  // 调用方（run-checks.ps1）会为浏览器阶段单独指定 SKYLAND_DATA_DIR，
  // 故这些写入落在临时目录，不会污染用户真实数据。
  async function seed(workdir, id, title) {
    const body = { id, title, messages: [{ role: 'user', content: 'q' }, { role: 'assistant', content: 'a' }] };
    const r = await fetch(BASE + '/api/conversations/save?workdir=' + encodeURIComponent(workdir), {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    });
    if (!r.ok) throw new Error('seed ' + workdir + ' HTTP ' + r.status);
  }
  await seed(WD_A, 1, '报告大纲讨论');
  await seed(WD_A, 2, '图表配色确认');
  await seed(WD_B, 1, '首页布局方案');
  await seed(WD_B, 2, '接口联调记录');
  await send('Page.navigate', { url: BASE + '/#/chat' });
  await sleep(2500);
  // 先注册两个工作区（走真实 Workdir.set 路径，会触发 ws-tree 登记）
  await evaluate('Workdir.set("' + WD_A + '", false)'); await sleep(900);
  await evaluate('Workdir.set("' + WD_B + '", false)'); await sleep(900);
  // 停回 A
  await evaluate('Workdir.set("' + WD_A + '", false)'); await sleep(1400);
  const snapA = await evaluate('JSON.stringify({html: document.getElementById("ws-tree").innerHTML, cur: Workdir.display()})');
  const a1 = JSON.parse(snapA);
  check('树已渲染', a1.html && a1.html.length > 40, a1.html);
  check('工作区A 可见', a1.html.includes(NAME_A), a1.html.slice(0, 200));
  check('工作区B 同时可见（未隐藏）', a1.html.includes(NAME_B), a1.html.slice(0, 200));
  check('A 的会话列出', /报告大纲讨论/.test(a1.html), a1.html.slice(0, 300));
  check('B 的会话列出', /首页布局方案/.test(a1.html), a1.html.slice(0, 300));
  check('当前标记唯一', (a1.html.match(/ws-tree-badge">当前/g) || []).length === 1, String((a1.html.match(/ws-tree-badge">当前/g) || []).length));
  if (SHOT_DIR) {
    const s = await send('Page.captureScreenshot', { format: 'png' });
    if (s.result && s.result.data) fs.writeFileSync(path.join(SHOT_DIR, 'ws-tree-A.png'), Buffer.from(s.result.data, 'base64'));
  }
  // ★ 关键一步：切到 B，验证 A 的会话不消失
  await evaluate('Workdir.set("' + WD_B + '", false)'); await sleep(1500);
  const snapB = await evaluate('JSON.stringify({html: document.getElementById("ws-tree").innerHTML, cur: Workdir.display()})');
  const b1 = JSON.parse(snapB);
  check('切到 B 后当前工作区是 B', (b1.cur || '').includes(NAME_B), b1.cur);
  check('★ 切走后 A 的会话仍然可见', /报告大纲讨论/.test(b1.html), b1.html.slice(0, 300));
  check('★ 切走后 B 的会话可见', /首页布局方案/.test(b1.html), b1.html.slice(0, 300));
  check('★ 侧栏仍同时列出两个工作区', b1.html.includes(NAME_A) && b1.html.includes(NAME_B));
  check('无未捕获异常', errors.length === 0, errors.join(' | '));
  if (SHOT_DIR) {
    const s = await send('Page.captureScreenshot', { format: 'png' });
    if (s.result && s.result.data) fs.writeFileSync(path.join(SHOT_DIR, 'ws-tree-B.png'), Buffer.from(s.result.data, 'base64'));
  }
  console.log('\n' + (failures.length ? 'FAIL' : 'PASS') + '  ' + passed + ' passed, ' + failures.length + ' failed');
  cleanup();
  try { ws.close(); } catch (e) {}
  process.exit(failures.length ? 1 : 0);
}
main().catch(e => { console.error('FATAL: ' + e.stack); process.exit(1); });
