// ═══════════════════════════════════════════════════════════════
// SKYLAND runtime smoke — headless Edge/Chrome via CDP
// 捕获 DOMContentLoaded 运行时异常，并验证主页/设置面板核心交互
// 用法: node scripts/runtime-smoke.js [baseUrl]      (默认 http://127.0.0.1:3000)
// 可选: --shots <dir>  保存关键截图; --light 以浅色主题+用户偏好注入
// 依赖: 本机 Edge/Chrome + Node >=22 (全局 WebSocket)
// ═══════════════════════════════════════════════════════════════
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const http = require('node:http');
const net = require('node:net');
const path = require('node:path');

const argv = process.argv.slice(2);
const flags = new Set(argv.filter(a => a.startsWith('--')));
const SHOT_DIR = argv.includes('--shots') ? argv[argv.indexOf('--shots') + 1] : null;
const BASE = argv.find((a, i) => !a.startsWith('--') && argv[i - 1] !== '--shots') || 'http://127.0.0.1:3000';
const LIGHT = flags.has('--light');
const EDGE_CANDIDATES = [
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
];
const sleep = ms => new Promise(r => setTimeout(r, ms));

function getFreePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      server.close(error => error ? reject(error) : resolve(port));
    });
  });
}

function withTimeout(promise, label, ms = 10000) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} 超时（${ms}ms）`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function httpGetJson(urlPath, port) {
  return new Promise((resolve, reject) => {
    http.get({ host: '127.0.0.1', port, path: urlPath }, res => {
      let d = '';
      res.on('data', c => d += c);
      res.on('end', () => { try { resolve(JSON.parse(d)); } catch (e) { reject(e); } });
    }).on('error', reject);
  });
}

async function main() {
  const exe = EDGE_CANDIDATES.find(p => fs.existsSync(p));
  if (!exe) { console.error('FATAL: 未找到 Edge/Chrome'); process.exit(2); }
  if (SHOT_DIR) fs.mkdirSync(SHOT_DIR, { recursive: true });

  const debugPort = await getFreePort();
  const profileDir = path.join(__dirname, `.edge-profile-${process.pid}-${debugPort}`);
  fs.rmSync(profileDir, { recursive: true, force: true });
  const browser = spawn(exe, [
    '--headless', `--remote-debugging-port=${debugPort}`,
    '--remote-debugging-address=127.0.0.1', '--remote-allow-origins=*',
    `--user-data-dir=${profileDir}`,
    '--no-first-run', '--disable-gpu', '--window-size=1400,900',
    'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  let browserStderr = '';
  let browserExit = null;
  browser.stderr.on('data', chunk => { browserStderr += chunk.toString(); });
  browser.once('exit', (code, signal) => { browserExit = { code, signal }; });
  browser.on('error', error => console.error('浏览器进程错误:', error.message));
  let cleanedUp = false;
  const cleanup = () => {
    if (cleanedUp) return;
    cleanedUp = true;
    try { browser.kill(); } catch (e) {}
    try { fs.rmSync(profileDir, { recursive: true, force: true }); } catch (e) {}
  };
  process.on('exit', cleanup);

  let targets = null;
  for (let i = 0; i < 60; i++) {
    if (browserExit) break;
    await sleep(250);
    try { targets = await httpGetJson('/json/list', debugPort); if (targets.some(t => t.type === 'page')) break; } catch (e) {}
  }
  const page = targets?.find(t => t.type === 'page');
  if (!page) {
    console.error('FATAL: CDP 未就绪', browserExit || '', browserStderr.trim());
    cleanup();
    process.exitCode = 2;
    return;
  }

  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await withTimeout(new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error('CDP WebSocket 连接失败'));
    ws.onclose = () => reject(new Error('CDP WebSocket 已关闭'));
  }), 'CDP WebSocket 连接');

  let msgId = 0;
  const pending = new Map();
  const logs = [];
  ws.onmessage = ev => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); return; }
    if (m.method === 'Runtime.consoleAPICalled' && (m.params.type === 'error' || m.params.type === 'warning')) {
      logs.push(`[console.${m.params.type}] ` + m.params.args.map(a => a.value ?? a.description ?? '').join(' '));
    }
    if (m.method === 'Runtime.exceptionThrown') {
      const d = m.params.exceptionDetails;
      logs.push(`[exception] ${d.text} ${d.exception?.description || ''} @${d.url || ''}:${d.lineNumber ?? '?'}`);
    }
  };
  const send = (method, params = {}) => withTimeout(new Promise(resolve => {
    const id = ++msgId;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  }), `CDP ${method}`);
  const evalJs = async expr => {
    const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true });
    if (r.result?.exceptionDetails) return { __err: r.result.exceptionDetails.text + ' ' + (r.result.exceptionDetails.exception?.description || '') };
    return r.result?.result?.value;
  };

  await send('Runtime.enable');
  await send('Page.enable');

  // 浅色主题 + 用户偏好注入（模拟长期使用的 localStorage 状态）
  if (LIGHT) {
    await send('Page.addScriptToEvaluateOnNewDocument', { source:
      `try {
        // 刻意注入**历史键名**（moline_*）：这三个键由 settings-store.js 独占迁移
        // （扁平 → 分组结构变换），storage-migrate.js 不接管它们。
        // 注入旧名可顺带验证"老用户升级后主题仍被正确接管"这条真实路径。
        localStorage.setItem('moline_settings', JSON.stringify({ theme: 'light', font: 'HYRY', fontSize: 14 }));
        localStorage.setItem('moline_global', JSON.stringify({ fontFamily: 'HYRY', fontWeight: '400', fontSize: 14, autoDarkMode: false }));
      } catch (e) {}` });
  }

  await send('Page.navigate', { url: BASE + '/#/home' });
  await sleep(4000);

  const results = [];
  const check = (name, actual, expected = true) =>
    results.push({ name, pass: JSON.stringify(actual) === JSON.stringify(expected), actual });
  const shot = async name => {
    if (!SHOT_DIR) return;
    const r = await send('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync(path.join(SHOT_DIR, name + '.png'), Buffer.from(r.result.data, 'base64'));
  };

  check('页面加载（标题）', await evalJs('document.title'), 'SKYLAND');
  check('无运行时异常（加载阶段）', logs.filter(l => l.startsWith('[exception]')).length === 0, true);
  if (LIGHT) check('浅色主题已应用', await evalJs(`!document.documentElement.classList.contains('dark')`), true);

  // 1. 登录遮罩未遮挡
  check('登录遮罩未显示', await evalJs(`(document.getElementById('login-overlay')?.style.display || 'none') === 'none'`), true);

  // 2. 主页卡片路由
  await evalJs(`document.querySelector('#view-home .home-card[data-route="/agent"]').click()`);
  await sleep(300);
  check('点击主页卡片 → 跳转 /agent', await evalJs('location.hash'), '#/agent');
  await evalJs(`window.location.hash = '#/home'`);
  await sleep(250);

  // 3. 主页右下角全局设置按钮 — 本脚本核心断言
  check('全局设置按钮存在', await evalJs(`!!document.getElementById('home-settings-btn')`), true);
  check('按钮 auth 钩子已挂', await evalJs(`document.getElementById('home-settings-btn')?.dataset.authHook || '未挂'`), '1');
  // 命中测试：按钮中心点的 elementFromPoint 必须落在按钮自身（或其子元素）内
  check('按钮无遮挡（elementFromPoint）', await evalJs(`(function(){
    const b = document.getElementById('home-settings-btn');
    const r = b.getBoundingClientRect();
    const el = document.elementFromPoint(r.left + r.width/2, r.top + r.height/2);
    return b === el || b.contains(el);
  })()`), true);
  await evalJs(`document.getElementById('home-settings-btn').click()`);
  await sleep(600);
  check('点击后 settings-panel.open', await evalJs(`document.getElementById('settings-panel').classList.contains('open')`), true);
  check('点击后 settings-overlay.active', await evalJs(`document.getElementById('settings-overlay').classList.contains('active')`), true);
  check('面板已滑入（transform 归零）', await evalJs(`['none','matrix(1, 0, 0, 1, 0, 0)'].includes(getComputedStyle(document.getElementById('settings-panel')).transform)`), true);
  check('激活页 = data', await evalJs(`Array.from(document.querySelectorAll('.settings-page')).filter(p => p.style.display !== 'none').map(p => p.dataset.spage).join(',')`), 'data');
  await shot('after-open');
  await evalJs(`document.getElementById('btn-settings-close').click()`);
  await sleep(400);
  check('关闭后 panel 无 open', await evalJs(`document.getElementById('settings-panel').classList.contains('open')`), false);

  // 4. 主题切换按钮
  await evalJs(`document.getElementById('home-theme-toggle').click()`);
  const themeAfter = await evalJs(`document.documentElement.classList.contains('dark')`);
  await evalJs(`document.getElementById('home-theme-toggle').click()`);
  const themeRestored = await evalJs(`document.documentElement.classList.contains('dark')`);
  check('主题切换往返生效', themeAfter === !themeRestored, true);

  // 5. 全程运行时异常
  check('无运行时异常（全程）', logs.filter(l => l.startsWith('[exception]')).length === 0, true);

  let failed = 0;
  console.log('\n═══ SKYLAND runtime smoke ═══');
  for (const r of results) {
    if (!r.pass) failed++;
    console.log(`${r.pass ? '✅ PASS' : '❌ FAIL'}  ${r.name}${r.pass ? '' : '  → 实际: ' + JSON.stringify(r.actual)}`);
  }
  if (logs.length) {
    console.log('── console 输出 ──');
    logs.forEach(l => console.log('  ' + l));
  }
  console.log(failed === 0 ? '\n全部通过' : `\n${failed} 项失败`);
  ws.close();
  cleanup();
  process.exitCode = failed === 0 ? 0 : 1;
}

main().catch(e => { console.error('FATAL', e); process.exitCode = 2; });
