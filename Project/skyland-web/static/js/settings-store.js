(function () {
  const STORAGE_KEY = 'skyland_settings';
  // 统一设置落地前的三个扁平键。键名常量来自 storage-migrate.js（唯一登记处），
  // 迁移顺序上 storage-migrate.js 先加载，故此处可安全引用。
  const L = window.SkylandStore.LEGACY_SETTINGS_KEYS;
  const LEGACY_KEYS = [L.settings, L.global, L.kb];
  const defaults = {
    version: 1,
    appearance: {},
    editor: {},
    execution: {},
    notifications: {},
    paths: {},
  };
  let state = clone(defaults);
  const listeners = new Set();

  function clone(value) {
    return JSON.parse(JSON.stringify(value));
  }

  function readJson(key) {
    try {
      const value = JSON.parse(localStorage.getItem(key) || 'null');
      return value && typeof value === 'object' ? value : {};
    } catch (_) {
      return {};
    }
  }

  function mergeGroup(target, source, keys) {
    keys.forEach((key) => {
      if (Object.prototype.hasOwnProperty.call(source, key)) target[key] = source[key];
    });
  }

  function fromFlat(flat) {
    const next = clone(defaults);
    mergeGroup(next.appearance, flat, [
      'theme', 'accentColor', 'font', 'fontSize', 'autoDarkMode',
      'darkModeStart', 'darkModeEnd',
    ]);
    mergeGroup(next.editor, flat, [
      'editorWidth', 'sidebarWidth', 'font', 'fontSize', 'mdTheme', 'outlineCollapsed',
    ]);
    mergeGroup(next.execution, flat, [
      'executionMode', 'permissionMode', 'thinkingEffort', 'webSearchEnabled',
    ]);
    mergeGroup(next.notifications, flat, ['taskPoll']);
    mergeGroup(next.paths, flat, [
      'model', 'compactModel', 'contextLength', 'imageSize', 'imageCustom',
    ]);
    return next;
  }

  function toFlat(next) {
    return {
      ...(next.appearance || {}),
      ...(next.editor || {}),
      ...(next.execution || {}),
      ...(next.notifications || {}),
      ...(next.paths || {}),
    };
  }

  function readLegacy() {
    return LEGACY_KEYS.reduce((all, key) => ({ ...all, ...readJson(key) }), {});
  }

  function readCached() {
    const modern = readJson(STORAGE_KEY);
    const base = Object.keys(modern).length ? modern : readLegacy();
    state = fromFlat(toFlat(base));
    Object.keys(base).forEach((key) => {
      if (key in defaults) state[key] = { ...(state[key] || {}), ...base[key] };
    });
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    return state;
  }

  function notify() {
    listeners.forEach((listener) => {
      try { listener(state); } catch (_) { /* listener errors must not break the page */ }
    });
  }

  function get(path) {
    return path.split('.').reduce((value, key) => value && value[key], state);
  }

  function set(path, value) {
    const parts = path.split('.');
    const root = parts.shift();
    if (!state[root] || typeof state[root] !== 'object') state[root] = {};
    let target = state[root];
    parts.forEach((key, index) => {
      if (index === parts.length - 1) target[key] = value;
      else {
        if (!target[key] || typeof target[key] !== 'object') target[key] = {};
        target = target[key];
      }
    });
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    notify();
  }

  function replaceFlat(flat) {
    state = fromFlat(flat || {});
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    notify();
  }

  function snapshotFlat() {
    return toFlat(state);
  }

  async function save() {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    const response = await fetch('/api/settings/user', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(state),
    });
    if (!response.ok) throw new Error('设置保存失败');
    notify();
    return true;
  }

  async function load() {
    try {
      const response = await fetch('/api/settings/user');
      if (response.ok) {
        const remote = await response.json();
        if (remote && typeof remote === 'object') {
          state = {
            ...clone(defaults),
            ...remote,
            appearance: { ...(remote.appearance || {}) },
            editor: { ...(remote.editor || {}) },
            execution: { ...(remote.execution || {}) },
            notifications: { ...(remote.notifications || {}) },
            paths: { ...(remote.paths || {}) },
          };
        }
      }
    } catch (_) {
      // 首屏仍使用同步缓存，网络恢复后由下一次保存重试。
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    notify();
    return state;
  }

  function subscribe(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  }

  readCached();
  window.SkylandSettings = {
    STORAGE_KEY,
    LEGACY_KEYS,
    get,
    set,
    replaceFlat,
    snapshotFlat,
    save,
    load,
    subscribe,
  };
})();
