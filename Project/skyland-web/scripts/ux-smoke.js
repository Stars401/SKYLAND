// ═══════════════════════════════════════════════════════════════
// 设置/模型配置 交互验证（headless Edge + CDP）
// 覆盖目标 2（设置改为中部弹窗）与目标 3（模型【修改】进入二级编辑页）
// 用法: node scripts/ux-smoke.js [baseUrl] [--shots <dir>]
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
let passed = 0; const failures = [];
function check(name, cond, detail) {
  if (cond) { passed++; console.log('  ok  ' + name); return; }
  failures.push(name); console.error('FAIL  ' + name + (detail !== undefined ? ' -> ' + String(detail).slice(0, 260) : ''));
}
function freePort() { return new Promise((res, rej) => { const s = net.createServer(); s.once('error', rej); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(e => e ? rej(e) : res(p)); }); }); }
function getJson(p, port) { return new Promise((res, rej) => { http.get({ host: '127.0.0.1', port, path: p }, r => { let d = ''; r.on('data', c => d += c); r.on('end', () => { try { res(JSON.parse(d)); } catch (e) { rej(e); } }); }).on('error', rej); }); }
async function main() {
  if (!EDGE) { console.error('FATAL: 未找到 Edge/Chrome'); process.exit(2); }
  const dp = await freePort();
  const profile = path.join(__dirname, '.ux-profile-' + process.pid);
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
    if (m.method === 'Runtime.exceptionThrown') { const d = m.params.exceptionDetails; errors.push(d.text + ' ' + ((d.exception && d.exception.description) || '')); }
  };
  const send = (method, params) => new Promise(res => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params: params || {} })); });
  const ev = async expr => {
    const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.result && r.result.exceptionDetails) return { __err: JSON.stringify(r.result.exceptionDetails) };
    return (r.result && r.result.result) ? r.result.result.value : undefined;
  };
  await send('Page.enable'); await send('Runtime.enable');
  await send('Page.navigate', { url: BASE + '/#/chat' });
  await sleep(3000);
  console.log('ux:');
  // ---- 目标 2：设置面板为居中弹窗 ----
  const g0 = JSON.parse(await ev('JSON.stringify((function(){var p=document.getElementById("settings-panel");var o=document.getElementById("settings-overlay");return {panel:p?getComputedStyle(p).display:"none",exists:!!p};})())'));
  check('设置面板存在', g0.exists);
  await ev('document.getElementById("btn-settings").click()');
  await sleep(700);
  const g1 = JSON.parse(await ev('JSON.stringify((function(){var p=document.getElementById("settings-panel");var cs=getComputedStyle(p);var r=p.getBoundingClientRect();return {display:cs.display,position:cs.position,left:cs.left,top:cs.top,vw:window.innerWidth,vh:window.innerHeight,rect:{x:r.x,y:r.y,w:r.width,h:r.height},overlayActive:document.getElementById("settings-overlay").className};})())'));
  check('点击设置后面板可见', g1.display !== 'none' && g1.display !== '', g1.display);
  check('面板为 fixed 定位（非左下角抽屉）', g1.position === 'fixed', g1.position);
  const cx = g1.rect.x + g1.rect.w / 2, cy = g1.rect.y + g1.rect.h / 2;
  const dcx = Math.abs(cx - g1.vw / 2), dcy = Math.abs(cy - g1.vh / 2);
  check('★ 面板水平居中（容差 40px）', dcx <= 40, 'dx=' + dcx.toFixed(1));
  // v0.19 起设置面板改为底部抽屉（用户指定的统一交互语言）—— 断言贴底而非居中
  check('★ 面板水平居中（容差 40px）', dcx <= 40, 'dx=' + dcx.toFixed(1));
  check('★ 面板贴底抽屉（底边距 ≤ 2px）', Math.abs((g1.rect.y + g1.rect.h) - g1.vh) <= 2, 'bottomGap=' + ((g1.rect.y + g1.rect.h) - g1.vh).toFixed(1));
  check('★ 面板顶部留白（非全高）', g1.rect.y > g1.vh * 0.1, 'top=' + g1.rect.y.toFixed(1));
  check('遮罩已激活', /active/.test(g1.overlayActive || ''), g1.overlayActive);
  if (SHOT_DIR) { const s = await send('Page.captureScreenshot', { format: 'png' }); if (s.result && s.result.data) fs.writeFileSync(path.join(SHOT_DIR, 'settings-modal.png'), Buffer.from(s.result.data, 'base64')); }
  // 目标 3：模型【修改】进入二级编辑页
  const g2 = JSON.parse(await ev('JSON.stringify((function(){return {list:document.getElementById("provider-list-page").style.display,edit:document.getElementById("provider-edit-page").style.display,editShown:document.getElementById("provider-edit-page").offsetParent!==null};})())'));
  check('初始显示模型列表页', g2.edit === 'none', JSON.stringify(g2));
  const clicked = await ev('(function(){var b=document.querySelector(\'[data-act="edit"]\');if(!b)return "NO_EDIT_BTN";b.click();return "clicked:"+b.dataset.key;})()');
  await sleep(800);
  if (String(clicked).indexOf('NO_EDIT_BTN') === 0) {
    // 无已配置模型时，退而验证「添加模型配置」进入编辑页的同一条通路
    await ev('document.getElementById("btn-provider-add").click()'); await sleep(600);
    const g3 = JSON.parse(await ev('JSON.stringify((function(){return {list:document.getElementById("provider-list-page").style.display,edit:document.getElementById("provider-edit-page").style.display,title:document.getElementById("provider-edit-title").textContent};})())'));
    check('进入模型编辑页（列表隐藏）', g3.list === 'none' && g3.edit !== 'none', JSON.stringify(g3));
  } else {
    const g3 = JSON.parse(await ev('JSON.stringify((function(){var p=document.getElementById("provider-list-page").style.display;var e=document.getElementById("provider-edit-page").style.display;return {list:p,edit:e,title:document.getElementById("provider-edit-title").textContent,name:document.getElementById("provider-name").value,url:document.getElementById("provider-baseurl").value,model:document.getElementById("provider-model-name").value,cancel:document.getElementById("btn-cancel-edit").style.display};})())'));
    check('★ 点击修改后列表隐藏', g3.list === 'none', g3.list);
    check('★ 点击修改后进入编辑页', g3.edit !== 'none', g3.edit);
    check('编辑页带回原配置名', !!g3.name, g3.name);
    check('编辑页带回 Base URL', !!g3.url, g3.url);
    check('编辑页带回模型名', !!g3.model, g3.model);
    check('「放弃/取消修改」按钮可见', g3.cancel !== 'none', g3.cancel);
    check('「保存」按钮存在', await ev('!!document.getElementById("btn-save-provider")'));
  }
  if (SHOT_DIR) { const s = await send('Page.captureScreenshot', { format: 'png' }); if (s.result && s.result.data) fs.writeFileSync(path.join(SHOT_DIR, 'provider-edit.png'), Buffer.from(s.result.data, 'base64')); }
  // 返回通路
  await ev('var b=document.getElementById("btn-provider-edit-back");if(b)b.click();');
  await sleep(500);
  const g4 = JSON.parse(await ev('JSON.stringify((function(){return {list:document.getElementById("provider-list-page").style.display};})())'));
  check('「返回模型列表」回到列表页', g4.list !== 'none', g4.list);
  check('无未捕获异常', errors.length === 0, errors.join(' | '));
  console.log('\n' + (failures.length ? 'FAIL' : 'PASS') + '  ' + passed + ' passed, ' + failures.length + ' failed');
  cleanup();
  try { ws.close(); } catch (e) {}
  process.exit(failures.length ? 1 : 0);
}
main().catch(e => { console.error('FATAL: ' + e.stack); process.exit(1); });