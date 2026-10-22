#!/usr/bin/env node
// R1 round-1 smoke test (no playwright, no server). Verifies the pure-logic pieces:
//   R1.1  migration script strips **...** wrappers from buggy code blocks
//   R1.2  backend snippet splitter (port of split_around) returns before/match/after
//   R1.3  image-width parser handles both {w=NN} and <!--w=NN-->; serializer emits <!--w=NN-->
//
// Usage: node scripts/kb-round1-smoke.js
const assert = require('assert');

let failures = 0;
function check(label, fn) {
  try { fn(); console.log('  ok  ' + label); }
  catch (e) { failures++; console.log('  FAIL ' + label + ' :: ' + e.message); }
}

// ─── R1.1: regex pipeline mirroring scripts/kb-migrate-double-asterisk.js ───
console.log('R1.1 migration');
function stripLineWrappers(line) {
  const m = line.match(/^(\s*)\*\*([\s\S]+)\*\*(\s*)$/);
  if (!m) return line;
  if (m[2].includes('**')) return line;
  return m[1] + m[2] + m[3];
}
function cleanCodeBlock(body) { return body.split('\n').map(stripLineWrappers).join('\n'); }
function migrateFile(text) {
  let out = text.replace(/\*\*```/g, '```');
  out = out.replace(/```(\w+)\*\*/g, '```$1');
  out = out.replace(/```\*\*/g, '```');
  out = out.replace(/```([\w-]*)\n([\s\S]*?)```/g, (_m, lang, body) => {
    return '```' + lang + '\n' + cleanCodeBlock(body.replace(/\n$/, '')) + '\n```';
  });
  return out;
}

check('removes **``` fence opener', () => {
  const out = migrateFile('**```js\ncode\n```');
  assert(out.startsWith('```js\n'));
});
check('removes ```hlsl** lang trailer', () => {
  const out = migrateFile('```hlsl**\n**A = 1**\n```');
  assert(out.includes('```hlsl\nA = 1\n```'));
});
check('leaves real markdown bold alone', () => {
  const before = 'this **has bold** inside';
  const after = migrateFile(before);
  assert.strictEqual(after, before);
});
check('handles multi-line code block', () => {
  const before = '```glsl\n**float x = 1.0;**\n**float y = 2.0;**\n```';
  const after = migrateFile(before);
  assert.strictEqual(after, '```glsl\nfloat x = 1.0;\nfloat y = 2.0;\n```');
});

// ─── R1.2: backend split_around, ported 1:1 from kb.rs ───
console.log('R1.2 snippet');
function splitAround(content, idx, qLen) {
  const start = Math.max(0, idx - 60);
  const end = Math.min(content.length, idx + qLen + 60);
  const before = content.slice(start, idx).replace(/\n/g, ' ');
  const m = content.slice(idx, idx + qLen).replace(/\n/g, ' ');
  const after = content.slice(idx + qLen, end).replace(/\n/g, ' ');
  return [before, m, after];
}
check('basic match', () => {
  const c = 'alpha kb beta';
  const i = c.indexOf('kb');
  const [b, m, a] = splitAround(c, i, 2);
  assert.strictEqual(m, 'kb');
  assert.strictEqual(b, 'alpha ');
  assert.strictEqual(a, ' beta');
});
check('flattens newlines', () => {
  const c = 'line1\nline2\nKB found\nline4';
  const i = c.indexOf('KB');
  const [b, m, a] = splitAround(c, i, 2);
  assert.strictEqual(m, 'KB');
  assert.ok(!m.includes('\n'));
  assert.ok(!b.includes('\n'));
  assert.ok(!a.includes('\n'));
});
check('clips to 60 chars around match', () => {
  const pad = 'x'.repeat(200);
  const c = pad + 'HIT' + pad;
  const [b, m, a] = splitAround(c, 200, 3);
  assert.strictEqual(m, 'HIT');
  assert.strictEqual(b.length, 60);
  assert.strictEqual(a.length, 60);
});

// ─── R1.3: image width parsing/serialization, mirrors kb.js ───
console.log('R1.3 image width');
function preprocess(text) {
  const widths = {};
  let out = text.replace(/!\[([^\]]*)\]\(([^)\s]+)\)\{w=(\d+)\}/g, (_m, alt, url, w) => {
    widths[url] = parseInt(w, 10);
    return '![' + alt + '](' + url + ')';
  });
  out = out.replace(/!\[([^\]]*)\]\(([^)\s]+)<!--w=(\d+)-->\)/g, (_m, alt, url, w) => {
    widths[url] = parseInt(w, 10);
    return '![' + alt + '](' + url + ')';
  });
  return { text: out, widths };
}
function serializeImg(alt, url, w) {
  if (w && w > 0) return '![' + alt + '](' + url + '<!--w=' + w + '-->)';
  return '![' + alt + '](' + url + ')';
}

check('parses {w=NN} form', () => {
  const { text, widths } = preprocess('![alt](img.png){w=320}');
  assert.strictEqual(widths['img.png'], 320);
  assert.strictEqual(text, '![alt](img.png)');
});
check('parses <!--w=NN--> form', () => {
  const { text, widths } = preprocess('![alt](img.png<!--w=200-->)');
  assert.strictEqual(widths['img.png'], 200);
  assert.strictEqual(text, '![alt](img.png)');
});
check('parses both forms together', () => {
  const { widths } = preprocess('![a](img.png){w=100}\n![b](img2.png<!--w=250-->)');
  assert.strictEqual(widths['img.png'], 100);
  assert.strictEqual(widths['img2.png'], 250);
});
check('serializer emits <!--w=NN--> when width>0', () => {
  assert.strictEqual(serializeImg('x', 'a.png', 300), '![x](a.png<!--w=300-->)');
});
check('serializer omits width hint when 0', () => {
  assert.strictEqual(serializeImg('x', 'a.png', 0), '![x](a.png)');
});
check('round-trip: parse -> serialize preserves width', () => {
  const src = '![alt](img.png){w=420}';
  const { widths } = preprocess(src);
  const w = widths['img.png'];
  const round = serializeImg('alt', 'img.png', w);
  assert.strictEqual(round, '![alt](img.png<!--w=420-->)');
  const re = preprocess(round);
  assert.strictEqual(re.widths['img.png'], 420);
});

if (failures === 0) { console.log('\nALL OK'); process.exit(0); }
else { console.log('\n' + failures + ' FAILURES'); process.exit(1); }
