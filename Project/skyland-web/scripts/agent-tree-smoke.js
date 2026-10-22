// 子智能体状态显示 回归测试（headless Edge + CDP）
// 守护三个真实缺陷（均为「看起来正常、实际骗人」类）：
//   1. 状态缺失时前端默认成「完成」→ 谎报子任务成功；
//   2. 按 name 回退匹配 → 模型派生两个同名子代理时，更新会打到别人节点上；
//   3. 后端一直在发 rounds/elapsed_ms，前端丢弃 → 用户无从判断子代理是否在推进。
// 直接调用页面里真实的 addAgentNode/updateAgentNode/renderAgentTree（app.js 顶层函数
// 在 classic script 下即全局），不是复刻一份逻辑来测。
// 用法: node scripts/agent-tree-smoke.js [baseUrl]
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const http = require('node:http');
const net = require('node:net');
const path = require('node:path');

const argv = process.argv.slice(2);
const BASE = argv[0] && !argv[0].startsWith('--') ? argv[0] : 'http://127.0.0.1:3000';
const EDGE = [
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
].find(p => fs.existsSync(p));
const sleep = ms => new Promise(r => setTimeout(r, ms));
let passed = 0; const failures = [];
function check(name, cond, detail) {
  if (cond) { passed++; console.log('  ok  ' + name); return; }
  failures.push(name); console.error('FAIL  ' + name + (detail !== undefined ? ' -> ' + String(detail).slice(0, 300) : ''));
}
function freePort() { return new Promise((res, rej) => { const s = net.createServer(); s.once('error', rej); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(e => e ? rej(e) : res(p)); }); }); }
function getJson(p, port) { return new Promise((res, rej) => { http.get({ host: '127.0.0.1', port, path: p }, r => { let d = ''; r.on('data', c => d += c); r.on('end', () => { try { res(JSON.parse(d)); } catch (e) { rej(e); } }); }).on('error', rej); }); }

async function main() {
  if (!EDGE) { console.error('FATAL: 未找到 Edge/Chrome'); process.exit(2); }
  const dp = await freePort();
  const profile = path.join(__dirname, '.agt-profile-' + process.pid);
  fs.rmSync(profile, { recursive: true, force: true });
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
  await sleep(2800);

  console.log('agent-tree:');
  const fns = JSON.parse(await ev('JSON.stringify({a:typeof addAgentNode,u:typeof updateAgentNode,r:typeof renderAgentTree,t:typeof clearAgentTree})'));
  check('页面暴露真实子代理函数', fns.a === 'function' && fns.u === 'function' && fns.r === 'function', JSON.stringify(fns));

  // 1) 状态缺失/未知 → 绝不能显示「完成」
  const g1 = JSON.parse(await ev(`(function(){
    clearAgentTree();
    addAgentNode('c1','x','t1');
    updateAgentNode('c1', undefined, '', '结果');
    var n = agentNodes[0];
    return JSON.stringify({state:n.state, html:document.getElementById('task-list').innerHTML});
  })()`));
  check('未知状态不落到 completed', g1.state !== 'completed', g1.state);
  check('未知状态按失败呈现', /失败/.test(g1.html), g1.html.slice(0, 160));

  // 2) 两个同名子代理：按 id 结束其中一个，另一个必须仍是执行中
  const g2 = JSON.parse(await ev(`(function(){
    clearAgentTree();
    addAgentNode('idA','dup','任务A');
    addAgentNode('idB','dup','任务B');
    updateAgentNode('idA','completed','','ok',3,1200);
    return JSON.stringify(agentNodes.map(function(n){return {id:n.id,state:n.state,rounds:n.rounds,elapsed:n.elapsed};}));
  })()`));
  check('同名子代理按 id 精确命中', g2.length === 2 && g2[0].state === 'completed' && g2[1].state === 'running', JSON.stringify(g2));
  check('轮次被记录', g2[0] && g2[0].rounds === 3, JSON.stringify(g2[0]));
  check('耗时被记录', g2[0] && g2[0].elapsed === 1200, JSON.stringify(g2[0]));

  // 3) 轮次/耗时确实渲染到界面
  const g3 = JSON.parse(await ev(`(function(){
    clearAgentTree();
    addAgentNode('m1','m','任务');
    updateAgentNode('m1','completed','','done',4,2500);
    return JSON.stringify(document.getElementById('task-list').innerHTML);
  })()`));
  check('界面显示轮数', /4 轮/.test(g3), g3.slice(0, 200));
  check('界面显示耗时（秒）', /2\.5s/.test(g3), g3.slice(0, 200));

  // 4) 执行中展示进度文案，且不误判为完成
  const g4 = JSON.parse(await ev(`(function(){
    clearAgentTree();
    addAgentNode('r1','r','任务');
    updateAgentNode('r1','running','第2轮 · read_file、grep');
    return JSON.stringify({state:agentNodes[0].state, html:document.getElementById('task-list').innerHTML});
  })()`));
  check('running 状态正确', g4.state === 'running', g4.state);
  check('执行中展示轮次文案', /第2轮/.test(g4.html), g4.html.slice(0, 200));
  check('执行中不显示完成', !/>完成</.test(g4.html), g4.html.slice(0, 200));

  // 5) 终态不被迟到的 status 事件覆盖（并行派生时事件会交错）
  const g5 = JSON.parse(await ev(`(function(){
    clearAgentTree();
    addAgentNode('f1','f','任务');
    updateAgentNode('f1','completed','','ok');
    updateAgentNode('f1','running','第9轮 · read_file');
    return JSON.stringify({state:agentNodes[0].state});
  })()`));
  check('终态不被迟到 status 覆盖', g5.state === 'completed', g5.state);

  // 6) 失败/取消各自独立呈现
  const g6 = JSON.parse(await ev(`(function(){
    clearAgentTree();
    addAgentNode('x1','x','t'); addAgentNode('x2','y','t'); addAgentNode('x3','z','t');
    updateAgentNode('x1','failed','','err'); updateAgentNode('x2','cancelled','','stop'); updateAgentNode('x3','completed','','ok');
    var h=document.getElementById('task-list').innerHTML;
    return JSON.stringify({states:agentNodes.map(function(n){return n.state;}), fail:/失败/.test(h), cancel:/已取消/.test(h)});
  })()`));
  check('failed 正确', g6.states[0] === 'failed', JSON.stringify(g6));
  check('cancelled 正确', g6.states[1] === 'cancelled', JSON.stringify(g6));
  check('completed 正确', g6.states[2] === 'completed', JSON.stringify(g6));
  check('失败文案呈现', g6.fail);
  check('取消文案呈现', g6.cancel);

  // 7. 切换会话必须清空子代理面板 —— agentNodes 不随会话持久化，
  //    留着会被读成"新会话也派生了这些子代理"，属张冠李戴的数据错误。
  const g7raw = await ev(`(function(){
    if (typeof selectChat !== 'function' || typeof closeChatView !== 'function') return JSON.stringify({skip:true});
    try {
      clearAgentTree();
      chats = [{id:1,title:'A',messages:[]},{id:2,title:'B',messages:[]}];
      selectChat(1);                       // 先建立基线：应用启动时已自动选中某条真实会话
      clearAgentTree();
      addAgentNode('s1','sub','任务A');
      selectChat(1);
      var afterFirst = agentNodes.length;
      addAgentNode('s2','sub2','任务A续');
      selectChat(2);
      var afterSwitch = agentNodes.length;
      addAgentNode('s3','sub3','任务B');
      selectChat(2);
      var afterReselect = agentNodes.length;
      closeChatView();
      return JSON.stringify({skip:false, a:afterFirst, b:afterSwitch, c:afterReselect, d:agentNodes.length});
    } catch (e) { return JSON.stringify({evalError: String((e && e.stack) || (e && e.message) || e)}); }
  })()`);
  // ev() 在页面内抛错时返回错误对象；这里显式区分，避免 JSON.parse 抛 SyntaxError 掩盖真实原因
  let g7;
  if (typeof g7raw === 'string') { g7 = JSON.parse(g7raw); }
  else { g7 = { evalError: (g7raw && g7raw.__err) || JSON.stringify(g7raw) }; }
  if (g7.evalError) { check('切换会话清空子代理（页面内异常）', false, g7.evalError); }
  else if (g7.skip) { check('切换会话清空子代理（前置函数缺失）', false, 'selectChat/closeChatView 未定义'); }
  else {
    check('重复点同一条不误清（树仍在）', g7.a === 1, JSON.stringify(g7));
    check('★ 切换会话后子代理被清空', g7.b === 0, JSON.stringify(g7));
    check('重复点同一会话不误清进行中的树', g7.c === 1, JSON.stringify(g7));
    check('关闭会话后子代理被清空', g7.d === 0, JSON.stringify(g7));
  }
  // 8. 「可写」徽标：会改文件的子代理必须与只读分析一眼可分，否则用户无从判断风险
  const g8 = JSON.parse(await ev(`(function(){
    clearAgentTree();
    addAgentNode('w1','coder','写实现', true);
    addAgentNode('r1','explore','读架构', false);
    var h=document.getElementById('task-list').innerHTML;
    return JSON.stringify({tags:document.querySelectorAll('.agent-write-tag').length, html:h});
  })()`));
  check('★ 可写子代理显示「可写」徽标', g8.tags === 1, g8.html.slice(0,240));
  check('只读子代理不显示可写徽标', !/explore[^<]*可写/.test(g8.html), g8.html.slice(0,240));
  check('无未捕获异常', errors.length === 0, errors.join(' | '));

  console.log('\n' + (failures.length ? 'FAIL' : 'PASS') + '  ' + passed + ' passed, ' + failures.length + ' failed');
  cleanup();
  try { ws.close(); } catch (e) {}
  process.exit(failures.length ? 1 : 0);
}
main().catch(e => { console.error('FATAL: ' + e.stack); process.exit(1); });