// todo-cal-smoke.js — 时间线日历渲染回归冒烟测试
//
// 背景：renderCalendar 旧实现把表头 .map().join('') 拼成字符串后 .push，
// 运行时 TypeError → 月份标题已更新而日网格永空白（"只显示月不显示日"事故）。
// 本测试直接驱动重构出的纯函数 TodoApp._cal.buildCalCells：
//   1) 必须返回数组（能 push 不会抛）；
//   2) 单元数 = 7 表头 + 当月首日星期偏移空位 + 当月天数；
//   3) 日格按钮携带 data-date、today 高亮、计数徽标；
//   4) 覆盖三种首日星期（周二/周日）与平年二月。
// 运行：node scripts/todo-cal-smoke.js（零依赖，Node 内置断言）

'use strict';
const fs = require('fs');
const path = require('path');
const assert = require('assert');

// todo.js 在 IIFE 装载期只碰 localStorage（calState 可见性持久化），stub 掉即可
global.localStorage = { getItem: () => null, setItem: () => {} };
// todo.js 模块级初始化读取 SkylandStore.KEYS.todoCal（同 todo-syntax-smoke 的修复）
global.SkylandStore = { KEYS: new Proxy({}, { get: () => 'skyland_stub' }), LEGACY_SETTINGS_KEYS: { global: 'g', settings: 's' } };

const src = fs.readFileSync(path.join(__dirname, '..', 'static', 'js', 'todo.js'), 'utf8');
const TodoApp = eval(src + '\n;TodoApp;');
assert(TodoApp && TodoApp._cal && typeof TodoApp._cal.buildCalCells === 'function',
  'todo.js 必须导出 _cal.buildCalCells（供本冒烟测试）');

const { buildCalCells } = TodoApp._cal;

let pass = 0;
function check(name, date, year, month, startWeek, daysInMonth, today, counts) {
  const cells = buildCalCells(year, month, counts, today, null);
  assert(Array.isArray(cells), `${name}: cells 必须是数组（旧事故：字符串不可 push）`);
  assert.strictEqual(cells.length, 7 + startWeek + daysInMonth,
    `${name}: 期望 ${7 + startWeek + daysInMonth} 个单元，实际 ${cells.length}`);
  // 表头 7 个
  for (let i = 0; i < 7; i++) {
    assert(cells[i].includes('todo-cal-wd'), `${name}: 第 ${i} 个表头缺失`);
  }
  // 空位
  for (let i = 0; i < startWeek; i++) {
    assert(cells[7 + i].includes('todo-cal-day empty'), `${name}: 第 ${i} 个空位缺失`);
  }
  // 首日按钮
  const first = cells[7 + startWeek];
  assert(first.includes(`data-date="${date}-01"`), `${name}: 首日 data-date 缺失`);
  assert(first.includes('<span class="todo-cal-num">1</span>'), `${name}: 首日数字缺失`);
  // today 高亮 + 计数徽标
  if (today && today.startsWith(date)) {
    const day = String(Number(today.slice(8, 10)));
    const todayCell = cells.find(c => c.includes(`data-date="${today}"`));
    assert(todayCell && todayCell.includes('today'), `${name}: today 高亮缺失`);
    const n = counts && counts[today];
    if (n) {
      assert(todayCell.includes(`todo-cal-count">${n}`), `${name}: 计数徽标缺失`);
    }
  }
  // 末尾无字符串拼接残留（旧 bug 的形态是整个网格静默丢失，这里兜底验证 join 成功）
  const html = cells.join('');
  assert(html.includes('data-date'), `${name}: join 后网格无日格按钮`);
  pass++;
}

// 2026-09：1 日为周二 → 2 空位；30 天；today=17 带计数
check('2026-09', '2026-09', 2026, 9, 2, 30, '2026-09-17', { '2026-09-17': 3 });
// 2026-11：1 日为周日 → 0 空位；30 天
check('2026-11', '2026-11', 2026, 11, 0, 30, null, {});
// 2026-02（平年）：1 日为周日 → 0 空位；28 天；today=14 无计数
check('2026-02', '2026-02', 2026, 2, 0, 28, '2026-02-14', {});
// 2026-05：1 日为周五 → 5 空位；31 天（最大偏移）
check('2026-05', '2026-05', 2026, 5, 5, 31, null, { '2026-05-20': 12 });

console.log(`todo-cal-smoke: 通过（${pass}/4 组日历网格断言全部命中）`);
