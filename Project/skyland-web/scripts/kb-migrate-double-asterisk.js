#!/usr/bin/env node
// kb-migrate-double-asterisk.js
// R1.1: One-shot cleanup of legacy code blocks where every line was wrapped in **...**
// due to a historical WYSIWYG bug. Backups go into <file-dir>/.history/migrate-<ts>/.
//
// Usage: node kb-migrate-double-asterisk.js [UserData-root]
//   default UserData-root = <repo>/UserData

const fs = require('fs');
const path = require('path');

const repoRoot = path.resolve(__dirname, '..', '..', '..');
const userData = process.argv[2] || path.join(repoRoot, 'UserData');

if (!fs.existsSync(userData)) {
  console.error('[migrate] UserData not found at', userData);
  process.exit(1);
}

// Strip a leading "**" + trailing "**" from a line the OLD bug wrapped.
// Refuse if the inner content already contains another "**" (real markdown bold).
function stripLineWrappers(line) {
  const m = line.match(/^(\s*)\*\*([\s\S]+)\*\*(\s*)$/);
  if (!m) return line;
  if (m[2].includes('**')) return line;
  return m[1] + m[2] + m[3];
}

function cleanCodeBlock(body) {
  return body.split('\n').map(stripLineWrappers).join('\n');
}

function migrateFile(text) {
  // Pattern 1: fence opener with stray "**": "**```OpenGL" -> "```OpenGL"
  let out = text.replace(/\*\*```/g, '```');
  // Pattern 2: fence opener with language + trailing "**": "```OpenGL**" -> "```OpenGL"
  out = out.replace(/```(\w+)\*\*/g, '```$1');
  // Pattern 3: stray "**" right after closing fence: "```**" -> "```"
  out = out.replace(/```\*\*/g, '```');
  // Pattern 4: per-line wrappers inside any fenced block.
  out = out.replace(/```([\w-]*)\n([\s\S]*?)```/g, (_m, lang, body) => {
    return '```' + lang + '\n' + cleanCodeBlock(body.replace(/\n$/, '')) + '\n```';
  });
  return out;
}

function walk(dir, cb) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith('.')) continue;
    if (entry.name === 'images') continue;
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(p, cb);
    else if (entry.isFile() && entry.name.endsWith('.md')) cb(p);
  }
}

const stamp = new Date().toISOString().replace(/[:.]/g, '-');
let touched = 0, linesFixed = 0;

walk(userData, file => {
  const orig = fs.readFileSync(file, 'utf8');
  const next = migrateFile(orig);
  if (next === orig) return;

  const before = (orig.match(/^\s*\*\*[\s\S]+\*\*\s*$/gm) || []).length;

  const backupDir = path.join(path.dirname(file), '.history', `migrate-${stamp}`);
  fs.mkdirSync(backupDir, { recursive: true });
  fs.writeFileSync(path.join(backupDir, path.basename(file)), orig, 'utf8');

  fs.writeFileSync(file, next, 'utf8');
  touched++;
  linesFixed += before;
  console.log(`[migrate] ${path.relative(userData, file)}  (~${before} wrapped lines)`);
});

console.log(`[migrate] done. files=${touched}, wrapped-lines-cleared~=${linesFixed}`);
