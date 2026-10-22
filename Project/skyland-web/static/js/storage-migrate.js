// ═══════════════════════════════════════════════════════════════
// SKYLAND 本地存储键迁移（moline_* → skyland_*）
// ═══════════════════════════════════════════════════════════════
// 品牌迭代把 localStorage / sessionStorage 的键前缀从 moline_ 换成 skyland_。
// 但这些键里装的是**用户真实数据**（聊天记录、工作区、侧栏状态、设置偏好），
// 直接改名 = 用户数据集体消失且不可恢复。
//
// 为什么在这里集中做，而不是让各模块自己回落读取旧键：
//   1. 回落读取要在 8 个模块里各写一遍，漏一处就是"某个功能悄悄重置"，
//      而且未来新增模块的人根本不知道还有旧键要照顾；
//   2. 一次性搬完 + 删除旧键，存储里不会同时存在两套键，
//      避免"旧键和新键各存一份、用户改了一处另一处没跟着变"的分裂状态。
//
// 刻意**不**接管 moline_settings / moline_global / moline_kb_settings：
// 那三个键由 settings-store.js 负责，它不只是改名，还要做 扁平 → 分组 的
// 结构变换（fromFlat）。若此处也去搬，会把扁平结构塞进分组的 skyland_settings，
// 造成两处迁移互相打架。单一职责原则：一个键只能有一个 owner。

(function () {
  'use strict';

  const LEGACY_PREFIX = 'moline_';
  const CURRENT_PREFIX = 'skyland_';
  // 迁移版本号：结构或映射规则变更时 +1，用来触发一次重扫
  const MIGRATION_FLAG = 'skyland_storage_migrated_v1';

  // 键名权威表 —— 各模块从这里取常量，避免键名字符串散落全项目
  const KEYS = Object.freeze({
    chats: 'skyland_chats',
    model: 'skyland_model',
    ctxLength: 'skyland_ctx_length',
    compactModel: 'skyland_compact_model',
    sidebarRail: 'skyland_sidebar_rail',
    lastReload: 'skyland_last_reload',
    imgSize: 'skyland_img_size',
    imgCustom: 'skyland_img_custom',
    taskPoll: 'skyland_task_poll',
    todoCal: 'skyland_todo_cal',
    workdir: 'skyland_workdir',
    wsRecent: 'skyland_ws_recent',
    visionAckPrefix: 'skyland_vision_ack_',
  });

  /**
   * 统一设置面板落地前遗留的三个扁平键。
   * 它们的迁移**不是简单改名** —— settings-store.js 还要做 扁平 → 分组
   * 的结构变换（fromFlat 把 theme/fontSize 等摊进 appearance/editor/... 分组）。
   * 所以由 settings-store.js 独占，本模块跳过（见 SETTINGS_STORE_OWNED）。
   *
   * 改名必须留在这里集中登记，不要在业务代码里裸写字符串：
   * 这些键在 router.js / kb.js 的 `SkylandSettings ? … : …` 兜底分支里
   * 仍会被读取（settings-store.js 若加载失败，页面仍要能显示主题）。
   * 一旦有人"顺手"把它们改成 skyland_*，那条兜底路径就会读到 undefined。
   */
  const LEGACY_SETTINGS_KEYS = Object.freeze({
    settings: 'moline_settings',
    global: 'moline_global',
    kb: 'moline_kb_settings',
  });

  // 由 settings-store.js 独占迁移的键（扁平→分组结构变换），此处必须跳过
  const SETTINGS_STORE_OWNED = new Set([
    LEGACY_SETTINGS_KEYS.settings,
    LEGACY_SETTINGS_KEYS.global,
    LEGACY_SETTINGS_KEYS.kb,
  ]);

  function toCurrentKey(legacyKey) {
    return CURRENT_PREFIX + legacyKey.slice(LEGACY_PREFIX.length);
  }

  /**
   * 把一个旧键搬到新键。
   * 新键已存在时**不覆盖** —— 用户在新版本里改过的值比旧残留更可信。
   * @returns {boolean} 是否真的发生了搬运
   */
  function migrateOne(store, legacyKey) {
    if (SETTINGS_STORE_OWNED.has(legacyKey)) return false;
    const currentKey = toCurrentKey(legacyKey);
    if (currentKey === legacyKey) return false;
    let value;
    try {
      value = store.getItem(legacyKey);
    } catch (e) {
      return false; // 隐私模式 / 存储被禁用：静默跳过，不阻断页面启动
    }
    if (value === null) return false;
    try {
      if (store.getItem(currentKey) === null) {
        store.setItem(currentKey, value);
      }
      store.removeItem(legacyKey);
      return true;
    } catch (e) {
      // 配额超限等：保留旧键不动，下次启动还有机会重试，绝不删掉唯一副本
      return false;
    }
  }

  /** 处理 `moline_vision_ack_<model>` 这类动态拼接的键（无法静态枚举） */
  function migratePrefixed(store, legacyPrefix, currentPrefix) {
    let moved = 0;
    let stale;
    try {
      stale = [];
      for (let i = 0; i < store.length; i++) {
        const key = store.key(i);
        if (key && key.startsWith(legacyPrefix)) stale.push(key);
      }
    } catch (e) {
      return 0;
    }
    // 先收集再删除：边遍历边删会跳过元素（索引前移），导致漏搬
    stale.forEach((key) => {
      if (migrateOne(store, key)) moved++;
    });
    return moved;
  }

  function runMigrations() {
    const stores = [];
    try { stores.push(window.localStorage); } catch (e) { /* 不可用则跳过 */ }
    try { stores.push(window.sessionStorage); } catch (e) { /* 不可用则跳过 */ }
    if (!stores.length) return;

    let moved = 0;
    for (const store of stores) {
      const stale = [];
      try {
        for (let i = 0; i < store.length; i++) {
          const key = store.key(i);
          if (key && key.startsWith(LEGACY_PREFIX)) stale.push(key);
        }
      } catch (e) {
        continue;
      }
      stale.forEach((key) => { if (migrateOne(store, key)) moved++; });
      moved += migratePrefixed(store, LEGACY_PREFIX + 'vision_ack_', KEYS.visionAckPrefix);
    }
    return moved;
  }

  let migratedCount = 0;
  try {
    // 幂等：搬过的键已不存在，重复执行是 no-op。仍每次跑一遍而非只靠 flag，
    // 这样即使 flag 丢失或用户手动改回旧键，也能自愈。
    migratedCount = runMigrations() || 0;
    try {
      // 只在首次成功时落标记：它是"何时完成迁移"的诊断信息，
      // 不是心跳。若每次启动都重写，等于每次加载都无意义地改动用户存储。
      if (!window.localStorage.getItem(MIGRATION_FLAG)) {
        window.localStorage.setItem(MIGRATION_FLAG, String(Date.now()));
      }
    } catch (e) {}
  } catch (e) {
    // 迁移失败绝不能阻断页面启动
  }

  window.SkylandStore = { KEYS, LEGACY_SETTINGS_KEYS, migratedCount };
})();
