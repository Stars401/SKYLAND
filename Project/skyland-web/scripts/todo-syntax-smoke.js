// 待办速记语法解析器冒烟测试 — node scripts/todo-syntax-smoke.js
// todo.js 是 IIFE 且加载期无 DOM 副作用，可直接 eval 提取 TodoApp
const fs = require('fs');
const path = require('path');
const src = fs.readFileSync(path.join(__dirname, '..', 'static', 'js', 'todo.js'), 'utf8');
global.window = {};
global.document = undefined;
// todo.js 加载期读取 localStorage（日历收放偏好）；Node ≥22 暴露的该全局在未开
// --experimental-webstorage 时不可用，必须 stub 掉再 eval
try { delete globalThis.localStorage; } catch {}
globalThis.localStorage = { getItem: () => null, setItem: () => {}, removeItem: () => {} };
// todo.js 模块级初始化读取 SkylandStore.KEYS.todoCal（浏览器由 settings-store.js 先行注入）
globalThis.SkylandStore = { KEYS: new Proxy({}, { get: () => 'skyland_stub' }), LEGACY_SETTINGS_KEYS: { global: 'g', settings: 's' } };
eval(src + '\nglobalThis.__TodoApp = TodoApp;');
const parse = globalThis.__TodoApp._parseQuickSyntax;
if (typeof parse !== 'function') { console.error('FAIL: TodoApp._parseQuickSyntax 未暴露'); process.exit(1); }
let failed = 0;
const ok = (name, cond) => { if (!cond) { failed++; console.error('FAIL:', name, JSON.stringify(cond === false ? parse : cond)); } };

const r1 = parse('买菜');
ok('纯标题', r1.title === '买菜' && r1.priority === 1 && !r1.category && !r1.notify_enabled);
const r2 = parse('!! 修复登录bug #工作');
ok('高优先+分类', r2.title === '修复登录bug' && r2.priority === 2 && r2.category === '工作');
const r3 = parse('! 买菜 #生活');
ok('低优先+分类', r3.title === '买菜' && r3.priority === 0 && r3.category === '生活');
const r4 = parse('!! 交周报 @09:30 #工作');
ok('时间+优先+分类乱序', r4.title === '交周报' && r4.priority === 2 && r4.category === '工作'
  && r4.notify_enabled && r4.notify_times.length === 1 && r4.notify_times[0] === '09:30');
const r5 = parse('开会 @9:05 @14:40');
ok('多时间补零', r5.title === '开会' && r5.notify_times.join(',') === '09:05,14:40');
const r6 = parse('#工作 前缀标签在前的标题');
ok('前置分类', r6.title === '前缀标签在前的标题' && r6.category === '工作');
const r7 = parse('@25:00 无效时间保留');
ok('无效时间不解析', r7.title.includes('@25:00') && !r7.notify_enabled);
const r8 = parse('   !!   ');
ok('空标题', r8.error === 'empty');
const r9 = parse('#工作 #次要 双标签只取首个');
ok('双标签取首个', r9.category === '工作' && r9.title.includes('#次要'));
const r10 = parse('!! 标题 @09:00 #工作');
ok('中文兼容', r10.title === '标题' && r10.category === '工作' && r10.priority === 2);
const r11 = parse('#工作 !! 交周报 @09:00');
ok('分类在前优先级', r11.title === '交周报' && r11.priority === 2 && r11.category === '工作' && r11.notify_times[0] === '09:00');
const r12 = parse('!!空格优先级 #空分类##');
ok('边界字符不炸', r11.error === 'empty' || typeof r11.error === 'string' || r11.title.length >= 0);

const r13 = parse('#学习 复习记忆卡 @2026-09-25');
ok('长日期token', r13.title === '复习记忆卡' && r13.category === '学习' && r13.date === '2026-09-25');
const r14 = parse('项目评审 @10-08');
ok('短日期token当年', r14.title === '项目评审' && r14.date === `${new Date().getFullYear()}-10-08`);
const r15 = parse('@02-30 非法日期保留');
ok('非法日期不解析', r15.title.includes('@02-30') && !r15.date);
const r16 = parse('多日期 @2026-09-25 @09-30');
ok('多日期取首个', r16.title === '多日期' && r16.date === '2026-09-25');
const r17 = parse('明天汇报 @2026-09-20 @09:00');
ok('日期+时间混合', r17.title === '明天汇报' && r17.date === '2026-09-20' && r17.notify_times[0] === '09:00');

if (failed) { console.error(`${failed} 个用例失败`); process.exit(1); }
console.log('todo-syntax-smoke: 全部通过');
