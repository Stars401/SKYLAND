#!/usr/bin/env node
// index.html 结构校验：div 平衡 + 每个 .view 必须位于深度 0（文档级兄弟）
// 用法：node scripts/structure-check.js   （非零退出 = 结构损坏）
const fs = require('fs');
const path = require('path');
const html = fs.readFileSync(path.join(__dirname, '..', 'static', 'index.html'), 'utf8');
const lines = html.split('\n');
let depth = 0;
const viewDepths = [];
const opens = (html.match(/<div\b/g) || []).length;
const closes = (html.match(/<\/div>/g) || []).length;
lines.forEach((line, idx) => {
  const before = depth;
  depth += (line.match(/<div\b/g) || []).length - (line.match(/<\/div>/g) || []).length;
  if (/class="view"/.test(line)) viewDepths.push({ line: idx + 1, depth: before });
});
let ok = true;
if (opens !== closes) { console.error(`FAIL: div 不平衡 开${opens} 闭${closes}`); ok = false; }
for (const v of viewDepths) {
  if (v.depth !== 0) { console.error(`FAIL: 行 ${v.line} 的 .view 位于深度 ${v.depth}（应为 0）`); ok = false; }
}
for (const id of ['view-home', 'view-agent', 'view-chat', 'view-tasks', 'view-todo', 'view-image', 'view-nodes', 'view-kb', 'settings-panel', 'settings-nav']) {
  if (!html.includes(`id="${id}"`)) { console.error(`FAIL: 缺少 #${id}`); ok = false; }
}
console.log(ok ? `structure-check: 通过（div ${opens}/${closes}，${viewDepths.length} 个视图）` : 'structure-check: 失败');
process.exit(ok ? 0 : 1);
