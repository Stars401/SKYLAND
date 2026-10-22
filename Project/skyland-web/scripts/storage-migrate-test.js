// ═══════════════════════════════════════════════════════════════
// 存储键迁移回归测试（moline_* → skyland_*）
// ═══════════════════════════════════════════════════════════════
// 品牌改名会动到**用户真实数据**（聊天记录 / 工作区 / 侧栏状态），
// 一旦漏搬就是不可恢复的数据丢失，且症状隐蔽（表现为"某个功能被重置"）。
// 故用纯 Node + 伪 storage 对搬运动力学做确定性验证，不依赖浏览器与后端。
// 用法: node scripts/storage-migrate-test.js

const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const SCRIPT = path.join(__dirname, '..', 'static', 'js', 'storage-migrate.js');

/** 最小 localStorage 语义实现（Map 保序，可枚举 length/key(i)） */
function makeStorage(initial) {
  const map = new Map(Object.entries(initial || {}));
  return {
    _map: map,
    get length() { return map.size; },
    key(i) { return Array.from(map.keys())[i] ?? null; },
    getItem(k) { return map.has(String(k)) ? map.get(String(k)) : null; },
    setItem(k, v) { map.set(String(k), String(v)); },
    removeItem(k) { map.delete(String(k)); },
    clear() { map.clear(); },
    snapshot() { return Object.fromEntries(map); },
  };
}

/** 在隔离的 vm 上下文里跑一次迁移，返回 { store, local, session } */
function runMigration({ local: localInit, session: sessionInit, throwOnSet = false } = {}) {
  const local = makeStorage(localInit);
  const session = makeStorage(sessionInit);
  if (throwOnSet) {
    // 模拟配额超限：写入抛错，但读取与删除仍可用
    const realSet = local.setItem.bind(local);
    local.setItem = (k, v) => {
      if (!String(k).startsWith('skyland_')) return realSet(k, v);
      throw new Error('QuotaExceededError');
    };
  }
  const window = { localStorage: local, sessionStorage: session };
  const ctx = vm.createContext({ window, localStorage: local, sessionStorage: session, Date, JSON, console });
  vm.runInContext(fs.readFileSync(SCRIPT, 'utf8'), ctx, { filename: SCRIPT });
  return { store: window.SkylandStore, local, session };
}

let passed = 0;
const failures = [];
function check(name, cond, detail) {
  if (cond) { passed++; console.log(`  ok  ${name}`); }
  else { failures.push(name); console.error(`FAIL  ${name}${detail ? ' → ' + detail : ''}`); }
}

console.log('storage-migrate:');

// ── 1. 核心承诺：旧键的用户数据被搬到新键，值原样保留 ──
{
  const chats = JSON.stringify([{ id: 'c1', title: '重要会话' }]);
  const { store, local } = runMigration({ local: { moline_chats: chats } });
  check('聊天记录被迁移且值不变', local.getItem('skyland_chats') === chats, String(local.getItem('skyland_chats')));
  check('旧键已清理（不留双份）', local.getItem('moline_chats') === null);
  check('键名表指向新键', store.KEYS.chats === 'skyland_chats', store.KEYS.chats);
}

// ── 2. 绝不能覆盖用户在新版本里已写入的值 ──
{
  const { local } = runMigration({
    local: { moline_model: 'old-model', skyland_model: 'new-model' },
  });
  check('新键已有值时不覆盖', local.getItem('skyland_model') === 'new-model', local.getItem('skyland_model'));
  check('新键有值后旧键仍被清理', local.getItem('moline_model') === null);
}

// ── 3. 动态拼接键：moline_vision_ack_<model> ──
{
  const { local } = runMigration({
    local: { 'moline_vision_ack_gpt-4o': '1', 'moline_vision_ack_claude': '1' },
  });
  check('动态前缀键 1 已迁移', local.getItem('skyland_vision_ack_gpt-4o') === '1');
  check('动态前缀键 2 已迁移', local.getItem('skyland_vision_ack_claude') === '1');
  check('动态前缀键无残留',
    Object.keys(local.snapshot()).filter(k => k.startsWith('moline_')).length === 0,
    Object.keys(local.snapshot()).join(','));
}

// ── 4. 关键陷阱：遍历中删除会导致漏搬 ──
// 故意让"先删后搬"与"边遍历边删"都无法通过，只有先收集再删除才正确。
{
  const many = {};
  for (let i = 0; i < 12; i++) many[`moline_vision_ack_m${i}`] = String(i);
  const { local } = runMigration({ local: many });
  const left = Object.keys(local.snapshot()).filter(k => k.startsWith('moline_'));
  check('12 个动态键全部搬完（无索引前移漏搬）', left.length === 0, `残留 ${left.join(',')}`);
}

// ── 5. 单一 owner：三个设置键归 settings-store.js 管，此处不得碰 ──
{
  const { local } = runMigration({
    local: { moline_settings: '{"theme":"dark"}', moline_global: '{"fontSize":14}', moline_kb_settings: '{"mdTheme":"x"}' },
  });
  check('moline_settings 不被本模块搬走', local.getItem('moline_settings') === '{"theme":"dark"}');
  check('moline_global 不被本模块搬走', local.getItem('moline_global') === '{"fontSize":14}');
  check('moline_kb_settings 不被本模块搬走', local.getItem('moline_kb_settings') === '{"mdTheme":"x"}');
  check('未擅自创建 skyland_settings（否则与分组结构打架）', local.getItem('skyland_settings') === null);
}

// ── 6. sessionStorage 同样要搬（auth.js 的重载断路器） ──
{
  const { session } = runMigration({ session: { moline_last_reload: '123' } });
  check('sessionStorage 也被迁移', session.getItem('skyland_last_reload') === '123');
  check('sessionStorage 旧键已清理', session.getItem('moline_last_reload') === null);
}

// ── 7. 幂等：重复执行不得改动数据（用户每次启动都会跑） ──
{
  const first = runMigration({ local: { moline_chats: 'v1', moline_workdir: '{"path":"D:/a"}' } });
  const snap1 = JSON.stringify(first.local.snapshot());
  // 模拟第二次启动：把脚本再跑一遍在同一 store 上
  const ctx = vm.createContext({
    window: { localStorage: first.local, sessionStorage: first.session },
    Date, JSON, console,
  });
  vm.runInContext(fs.readFileSync(SCRIPT, 'utf8'), ctx, { filename: SCRIPT });
  const snap2 = JSON.stringify(first.local.snapshot());
  check('重复执行幂等', snap1 === snap2, `\n  1st=${snap1}\n  2nd=${snap2}`);
  check('幂等后聊天记录仍在', first.local.getItem('skyland_chats') === 'v1');
  check('幂等后工作区仍在', first.local.getItem('skyland_workdir') === '{"path":"D:/a"}');
}

// ── 8. 写失败（配额超限）必须保住旧键，而不是删掉唯一副本 ──
{
  const { local } = runMigration({ local: { moline_chats: 'precious' }, throwOnSet: true });
  check('写入失败时旧键保留（下次启动可重试）', local.getItem('moline_chats') === 'precious', String(local.snapshot()['moline_chats']));
}

// ── 9. 存储完全不可用（隐私模式）不得抛错阻断页面 ──
{
  let ok = false;
  try {
    const ctx = vm.createContext({
      window: {}, Date, JSON, console,
    });
    vm.runInContext(fs.readFileSync(SCRIPT, 'utf8'), ctx, { filename: SCRIPT });
    ok = true;
  } catch (e) {
    ok = false;
    console.error('  异常: ' + e.message);
  }
  check('localStorage 缺失时静默降级', ok);
}

console.log(`\n${failures.length ? 'FAIL' : 'PASS'}  ${passed} passed, ${failures.length} failed`);
if (failures.length) { failures.forEach(f => console.error('  - ' + f)); process.exit(1); }
