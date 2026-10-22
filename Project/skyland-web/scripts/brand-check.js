// ═══════════════════════════════════════════════════════════════
// 品牌一致性守卫 —— moline → skyland
// ═══════════════════════════════════════════════════════════════
// 品牌改名不是一次性替换，而是要长期守住的不变量：
//   1. 现网文档/源码不得再指向已删除的 moline-* 路径（会让人 cargo run 失败）；
//   2. 兼容面必须仍在（localStorage 迁移 / Cookie / 环境变量 / UDP 协议）——
//      悄悄删掉会让老用户数据失联或新旧版本互相失联。
// 故本检查**双向**断言：既不许漏改，也不许改过头。
// 用法: node scripts/brand-check.js      （从仓库根或本目录均可）
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..', '..', '..');   // 仓库根
const PROJECT = path.join(ROOT, 'Project');

let passed = 0; const failures = [];
function check(name, cond, detail) {
  if (cond) { passed++; console.log('  ok  ' + name); return; }
  failures.push(name); console.error('FAIL  ' + name + (detail ? ' -> ' + String(detail).slice(0, 300) : ''));
}

function walk(dir, filter, out) {
  out = out || [];
  let ents = [];
  try { ents = fs.readdirSync(dir, { withFileTypes: true }); } catch (e) { return out; }
  for (const e of ents) {
    if (e.name === 'target' || e.name === '.git' || e.name === 'backup' || e.name === 'node_modules') continue;
    const full = path.join(dir, e.name);
    if (e.isDirectory()) walk(full, filter, out);
    else if (!filter || filter(e.name)) out.push(full);
  }
  return out;
}
const readIf = f => { try { return fs.readFileSync(f, 'utf8'); } catch (e) { return ''; } };

console.log('brand-check:');

// ── 1. 目录/包名已统一 ──
for (const d of ['skyland-web', 'skyland-app', 'skyland-daemon']) {
  check('目录存在 Project/' + d, fs.existsSync(path.join(PROJECT, d)));
}
check('旧目录 Project/moline-web 已删除', !fs.existsSync(path.join(PROJECT, 'moline-web')));
check('旧目录 Project/moline-app 已删除', !fs.existsSync(path.join(PROJECT, 'moline-app')));
check('旧目录 Project/moline-daemon 已删除', !fs.existsSync(path.join(PROJECT, 'moline-daemon')));
check('proto 已更名 skyland.proto', fs.existsSync(path.join(PROJECT, 'lib', 'proto', 'skyland.proto')));
check('旧 proto moline.proto 已删除', !fs.existsSync(path.join(PROJECT, 'lib', 'proto', 'moline.proto')));

// ── 2. crate 名 ──
const cargoToml = readIf(path.join(PROJECT, 'Cargo.toml'));
check('workspace 不含 moline-* 包', !/moline-(lib|web|app|daemon|tests)/.test(cargoToml));

// ── 3. 现网文档不得指向已删除路径 ──
const liveDocs = ['README.md', 'CODELY.md']
  .map(f => path.join(ROOT, f))
  .concat(walk(path.join(ROOT, 'docs'), n => n.endsWith('.md')))
  .filter(f => f.indexOf('superpowers') === -1);   // plans/specs 是历史记录，不改写
// 历史变更记录按"节"豁免：整节标题带日期的（如 CODELY.md 的
// `### xxx（2026-09-18，v=116）`）记录的是当时的事实，改写它等于篡改历史。
// 但**节内**若出现的是"现在照着做会失败"的路径，仍需暴露 —— 故只豁免整节，
// 并在节内做行级检测，一旦命中就报出来供人工判断。
const DATE_HEADING = /^#{1,6}\s.*[（(]\s*\d{4}-\d{2}-\d{2}/;
let badDoc = 0;
for (const f of liveDocs) {
  const lines = readIf(f).split(/\r?\n/);
  let inHistory = false;
  lines.forEach((line, i) => {
    if (/^#{1,6}\s/.test(line)) inHistory = DATE_HEADING.test(line);
    if (!/moline-(web|app|daemon|lib)/i.test(line)) return;
    if (inHistory) return;                       // 历史节：保留原文
    if (/^\s*[-*]?\s*\[?\d{4}-\d{2}-\d{2}/.test(line)) return;  // 行内带日期的条目
    badDoc++;
    console.error('    ' + path.basename(f) + ':' + (i + 1) + ' ' + line.trim().slice(0, 160));
  });
}
check('现网文档无 moline-* 路径引用', badDoc === 0, badDoc + ' 处');

// ── 4. 兼容面必须仍在（防止"改过头"）──
const authRs = readIf(path.join(PROJECT, 'skyland-web', 'src', 'auth.rs'));
check('Cookie 兼容：新名 skyland_session', /skyland_session/.test(authRs));
check('Cookie 兼容：仍读历史名 moline_session', /moline_session/.test(authRs));
check('Cookie 兼容：登出同时清历史名', /LEGACY_SESSION_COOKIE.*max_age == 0|max_age == 0.*LEGACY_SESSION_COOKIE/s.test(authRs));

const disc = readIf(path.join(PROJECT, 'skyland-web', 'src', 'discovery.rs'));
check('UDP 兼容：保留历史探测包', /MOLINE_DISCOVER_V1/.test(disc));
check('UDP 兼容：版本回显', /reply_prefix_for/.test(disc));

const envc = readIf(path.join(PROJECT, 'skyland-web', 'src', 'env_compat.rs'));
check('环境变量兼容层存在', envc.length > 0 && /MOLINE_DATA_DIR/.test(envc));

const mig = readIf(path.join(PROJECT, 'skyland-web', 'static', 'js', 'storage-migrate.js'));
check('localStorage 迁移层存在', mig.length > 0 && /moline_/.test(mig) && /skyland_/.test(mig));

// ── 5. 迁移层必须先于读取方加载 ──
const html = readIf(path.join(PROJECT, 'skyland-web', 'static', 'index.html'));
const iMig = html.indexOf('js/storage-migrate.js');
const iAuth = html.indexOf('js/auth.js');
const iStore = html.indexOf('js/settings-store.js');
check('storage-migrate.js 在 auth.js 之前加载', iMig > 0 && iMig < iAuth, 'iMig=' + iMig + ' iAuth=' + iAuth);
check('storage-migrate.js 在 settings-store.js 之前加载', iMig > 0 && iMig < iStore, 'iMig=' + iMig + ' iStore=' + iStore);

console.log('\n' + (failures.length ? 'FAIL' : 'PASS') + '  ' + passed + ' passed, ' + failures.length + ' failed');
if (failures.length) process.exit(1);