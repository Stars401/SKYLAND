// ════════════════════════════════════════════
// SKYLAND SPA Router — hash-based view switching
// ════════════════════════════════════════════

const Router = {
  current: '',
  chatInited: false,
  kbInited: false,
  todoInited: false,
  imageInited: false,
  tasksInited: false,
  nodesInited: false,

  init() {
    window.addEventListener('hashchange', () => this.dispatch());
    this.dispatch();
  },

  dispatch() {
    const hash = window.location.hash.slice(1) || '/home';
    const route = hash.split('?')[0];
    this.show(route);
    this.current = route;
  },

  show(route) {
    if (this.current === '/nodes' && route !== '/nodes' &&
        typeof NodesApp !== 'undefined' && NodesApp.onHide) {
      NodesApp.onHide();
    }
    document.querySelectorAll('.view').forEach(v => v.style.display = 'none');
    const id = 'view-' + route.slice(1);
    const view = document.getElementById(id);
    if (view) {
      view.style.display = '';
    } else {
      // 未知路由回退首页（防白屏——托盘/外部链接写入未知 hash 时兜底）
      const home = document.getElementById('view-home');
      if (home) home.style.display = '';
    }
    // 首启数据目录引导（仅 standalone 未配置时弹一次）
    if (route === '/home') maybeShowDataDirGuide();

    if (route === '/chat' && !this.chatInited) {
      if (typeof init === 'function') { init(); this.chatInited = true; }
    }
    if (route === '/chat' && this.chatInited) {
      if (typeof renderChatList === 'function') renderChatList();
    }
    if (route === '/kb' && !this.kbInited) {
      if (typeof KbApp !== 'undefined' && KbApp.init) { KbApp.init(); this.kbInited = true; }
    }
    if (route === '/kb' && typeof KbApp !== 'undefined' && KbApp.onShow) {
      KbApp.onShow();
    }
    if (route === '/todo' && !this.todoInited) {
      if (typeof TodoApp !== 'undefined' && TodoApp.init) { TodoApp.init(); this.todoInited = true; }
    }
    if (route === '/todo' && this.todoInited && typeof TodoApp !== 'undefined' && TodoApp.onShow) {
      TodoApp.onShow();
    }
    if (route === '/tasks' && typeof TasksApp !== 'undefined' && TasksApp.init && !this.tasksInited) {
    TasksApp.init(); this.tasksInited = true;
  }
  if (route === '/tasks' && typeof TasksApp !== 'undefined' && TasksApp.onShow) {
    TasksApp.onShow();
  }
  if (route === '/image' && !this.imageInited) {
      if (typeof ImageApp !== 'undefined' && ImageApp.init) { ImageApp.init(); this.imageInited = true; }
    }
    if (route === '/image' && this.imageInited && typeof ImageApp !== 'undefined' && ImageApp.onShow) {
      ImageApp.onShow();
    }
    if (route === '/nodes' && !this.nodesInited) {
      if (typeof NodesApp !== 'undefined' && NodesApp.init) { NodesApp.init(); this.nodesInited = true; }
    }
    if (route === '/nodes' && this.nodesInited && typeof NodesApp !== 'undefined' && NodesApp.onShow) {
      NodesApp.onShow();
    }
  },

  navigate(route) {
    window.location.hash = route;
  }
};

// ─── 首启数据目录引导（Windows 规范定向 — 程序目录不再落任何数据）───
// 未配置时在首页弹一次：默认位置（%APPDATA%\SKYLAND）/ 自定义位置 / 暂不
function _ddEsc(s) { return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;'); }
async function maybeShowDataDirGuide() {
  if (sessionStorage.getItem('skyland_dd_prompted')) return;
  let info = {};
  try { info = await fetch('/api/server/info').then(r => r.json()); } catch (e) {}
  // server 模式数据目录由管理员决定；client 模式数据在远端 — 都不弹
  if (info && info.mode && info.mode !== 'standalone') { sessionStorage.setItem('skyland_dd_prompted', '1'); return; }
  let d = null;
  try { d = await fetch('/api/datadir').then(r => r.json()); } catch (e) {}
  if (!d || d.status !== 'ok') return;
  if (d.configured) { sessionStorage.setItem('skyland_dd_prompted', '1'); return; }
  sessionStorage.setItem('skyland_dd_prompted', '1');
  // 认证首启引导（server 设置弹窗）优先 — 已有 modal 在屏时不叠加
  if (document.querySelector('.kb-modal-overlay') || document.querySelector('.auth-setup-overlay')) return;
  _showDataDirModal(d);
}
function _showDataDirModal(d) {
  const overlay = document.createElement('div');
  overlay.className = 'kb-modal-overlay';
  const modal = document.createElement('div');
  modal.className = 'kb-modal';
  modal.style.width = 'min(480px, 92vw)';
  modal.innerHTML = `
    <div class="kb-modal-header"><span class="kb-modal-title">欢迎使用 SKYLAND — 选择数据目录</span></div>
    <div class="kb-modal-body">
      <p style="font-size:13px;line-height:1.7;color:var(--text-primary)">检测到首次运行。SKYLAND 需要一个<b>数据目录</b>来存放配置、对话、待办与生成文件 —— 按 Windows 规范放入 AppData，<b>不再写入程序所在目录</b>。</p>
      <div style="padding:10px 12px;background:var(--bg-hover);border-radius:8px;margin:10px 0;font-size:12px;color:var(--text-secondary)">
        <span style="color:var(--text-muted)">默认位置</span><br>
        <span style="word-break:break-all">${_ddEsc(d.default)}</span>
      </div>
      <p style="font-size:11.5px;color:var(--text-muted)">也可以选择自定义位置（如 D:\\SkylandData）。选择后需重启应用生效。</p>
    </div>
    <div class="kb-modal-footer">
      <button class="kb-modal-btn kb-modal-btn-cancel" id="dd-later">暂不</button>
      <button class="kb-modal-btn" id="dd-custom">自定义位置…</button>
      <button class="kb-modal-btn kb-modal-btn-ok" id="dd-default">使用默认位置</button>
    </div>`;
  overlay.appendChild(modal);
  overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
  modal.querySelector('#dd-later').onclick = () => overlay.remove();
  modal.querySelector('#dd-default').onclick = () => _applyDataDir(overlay, modal, d.default);
  modal.querySelector('#dd-custom').onclick = async () => {
    if (typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke) {
      try {
        const picked = await __TAURI__.core.invoke('pick_directory');
        if (picked) { _applyDataDir(overlay, modal, picked); return; }
      } catch (e) { console.error('pick_directory failed:', e); }
    } else {
      // 浏览器模式：文本输入兜底
      const input = window.prompt('请输入数据目录的完整绝对路径（如 D:\\SkylandData）：', d.default);
      if (input && input.trim()) { _applyDataDir(overlay, modal, input.trim()); }
    }
  };
  document.body.appendChild(overlay);
}
async function _applyDataDir(overlay, modal, dir) {
  const body = modal.querySelector('.kb-modal-body');
  const footer = modal.querySelector('.kb-modal-footer');
  try {
    const r = await fetch('/api/datadir', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ dir }),
    });
    const d = await r.json();
    if (!d || d.status !== 'ok') throw new Error((d && d.message) || '设置失败');
    body.innerHTML = `
      <p style="font-size:13px;line-height:1.7;color:var(--text-primary)">✓ 数据目录已创建并记录：</p>
      <div style="padding:10px 12px;background:var(--bg-hover);border-radius:8px;margin:10px 0;font-size:12px;word-break:break-all">${_ddEsc(d.dir)}</div>
      <p style="font-size:12px;color:var(--text-secondary)">重启应用后，全部配置与数据将定向到该位置。</p>`;
    footer.innerHTML = '';
    if (typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke) {
      const btn = document.createElement('button');
      btn.className = 'kb-modal-btn kb-modal-btn-ok';
      btn.textContent = '立即重启';
      btn.onclick = async () => { try { await __TAURI__.core.invoke('restart_app'); } catch (e) { alert('重启失败：' + e); } };
      footer.appendChild(btn);
      const later = document.createElement('button');
      later.className = 'kb-modal-btn kb-modal-btn-cancel';
      later.textContent = '稍后手动重启';
      later.onclick = () => overlay.remove();
      footer.appendChild(later);
    } else {
      const btn = document.createElement('button');
      btn.className = 'kb-modal-btn kb-modal-btn-ok';
      btn.textContent = '我知道了（请手动重启应用）';
      btn.onclick = () => overlay.remove();
      footer.appendChild(btn);
    }
  } catch (e) {
    body.innerHTML = `<p style="font-size:13px;color:var(--error)">设置失败：${_ddEsc(e.message)}</p>`;
    const btn = document.createElement('button');
    btn.className = 'kb-modal-btn kb-modal-btn-cancel';
    btn.textContent = '关闭';
    btn.onclick = () => overlay.remove();
    footer.innerHTML = '';
    footer.appendChild(btn);
  }
}

// Home page button handlers + theme init
document.addEventListener('DOMContentLoaded', () => {
  // Auth boot — 云端模式登录检查 / 首次运行引导（必须在 Router 初始化前）
  if (typeof Auth !== 'undefined' && Auth.boot) {
    Auth.boot();
    if (typeof Auth.hookSettings === 'function') Auth.hookSettings();
  }

  // ─── 全局 Windows 通知助手：重写后的四级降级链 ───
  // 旧链（插件直发→权限协商→rust 命令→浏览器→应用内）十轮迭代失败根因：
  // ① 插件底层 WinRT Toast 需 AUMID 系统注册，未注册被静默丢弃；
  // ② notify_windows 命令从未进 ACL 权限清单，remote WebView 一律静默拒绝；
  // ③ WebView2 不支持浏览器 Notification。新链全部改走原生通道：
  // 1) 本机后端 → POST /api/notify/send（Rust WinRT 直发，不经 WebView、无 ACL）
  // 2) 客户端壳 → invoke notify_windows（Toast 弹在用户本机；本机后端异常时兜底）
  // 3) 浏览器 Notification（局域网真浏览器；WebView2 无此 API 自然跳过）
  // 4) 应用内 toast（最后兜底，任何环境都有可见提醒）
  window.notifyWindows = async function (title, body) {
    const localBackend = ['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname);
    const invoke = (typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke)
      ? __TAURI__.core.invoke : null;
    // ① 本机后端直发（最可靠）
    if (localBackend) {
      try {
        const r = await fetch('/api/notify/send', {
          method: 'POST', headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ title: title, body: body }),
        });
        if (r.ok) {
          const d = await r.json().catch(() => null);
          if (d && d.delivered) { focusShellWindow(); return 'backend'; }
        }
      } catch (e1) { /* 继续降级 */ }
    }
    // ② 客户端壳：Toast 必须弹在用户本机 → 走壳的原生命令
    if (invoke) {
      try {
        await invoke('notify_windows', { title: title, body: body });
        focusShellWindow();
        return 'tauri';
      } catch (e2) { /* 继续降级 */ }
    }
    // ③ 浏览器通知
    try {
      if ('Notification' in window) {
        if (Notification.permission === 'granted') {
          new Notification(title, { body: body });
          return 'web';
        }
        if (Notification.permission !== 'denied') {
          const p = await Notification.requestPermission();
          if (p === 'granted') { new Notification(title, { body: body }); return 'web'; }
        }
      }
    } catch (e3) { /* 最后降级 */ }
    // ④ 应用内 toast（保证任何环境都有可见提醒）
    try {
      let toast = document.getElementById('app-inapp-toast');
      if (!toast) {
        toast = document.createElement('div');
        toast.id = 'app-inapp-toast';
        toast.className = 'app-inapp-toast';
        document.body.appendChild(toast);
      }
      toast.innerHTML = '<strong>' + (title || '') + '</strong>' + (body ? '<div>' + body + '</div>' : '');
      toast.style.display = 'block';
      clearTimeout(window._inappToastTimer);
      window._inappToastTimer = setTimeout(() => { toast.style.display = 'none'; }, 8000);
    } catch (e4) {}
    return 'inapp';
  };

  // 通知送达后把桌面壳窗口拉到前台（浏览器环境静默跳过）
  function focusShellWindow() {
    try {
      if (typeof __TAURI__ !== 'undefined' && __TAURI__.window && __TAURI__.window.getCurrentWindow) {
        const win = __TAURI__.window.getCurrentWindow();
        if (win.show) win.show();
        if (win.setFocus) win.setFocus();
      }
    } catch (e) {}
  }

  // 设置面板 v2 — 导航/通知开关/MCP 管理绑定（一次性）
  if (typeof Settings2 !== 'undefined' && Settings2.init) Settings2.init();

  // 待办提醒引擎 — 应用启动即武装（不依赖用户访问待办页）
  if (typeof TodoApp !== 'undefined' && TodoApp.startNotifyEngine) {
    TodoApp.startNotifyEngine();
  }

  // 定时任务完成通知轮询 — 每 60s 比对 last_run 变化（三模式一致：client 直连远端）
  let lastTaskRunSeen = 0;
  let taskPollBusy = false;
  function pollTaskNotifications() {
    if (taskPollBusy) return;
    if (typeof Settings2 !== 'undefined' && Settings2.pollEnabled && !Settings2.pollEnabled()) return;
    taskPollBusy = true;
    fetch('/api/tasks/list').then(r2 => r2.status === 401 ? null : r2.json()).then(data => {
      if (!data || data.status !== 'ok') return;
      let newest = lastTaskRunSeen;
      for (const t of (data.tasks || [])) {
        const lr = t.last_run || 0;
        if (lr > lastTaskRunSeen && lastTaskRunSeen !== 0) {
          const body = '「' + (t.name || '任务') + '」已完成，结果已写入任务会话';
          if (typeof window.notifyWindows === 'function') {
            window.notifyWindows('SKYLAND 定时任务', body);
          }
        }
        if (lr > newest) newest = lr;
      }
      lastTaskRunSeen = newest;
    }).catch(() => {}).finally(() => { taskPollBusy = false; });
  }
  setInterval(pollTaskNotifications, 60000);
  setTimeout(pollTaskNotifications, 5000);

  // Apply saved theme before anything renders
  // Auto dark mode: check if auto mode is enabled
  try {
    const g = window.SkylandSettings ? window.SkylandSettings.snapshotFlat()
      : (JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.global)) || {});
    if (g.autoDarkMode) {
      // Auto mode — check current time against configured hours
      const now = new Date();
      const hour = now.getHours();
      const startDark = g.darkModeStart || 21;
      const endDark = g.darkModeEnd || 7;
      const isDark = hour >= startDark || hour < endDark;
      document.documentElement.classList.toggle('dark', isDark);
    } else {
      const s = window.SkylandSettings ? window.SkylandSettings.snapshotFlat()
        : JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings));
      if (s && s.theme) {
        document.documentElement.classList.toggle('dark', s.theme === 'dark');
      } else if (s && s.accentColor) {
        document.documentElement.style.setProperty('--accent', s.accentColor);
      }
    }
  } catch(e) {}

  // Apply global font settings
  try {
    const g = window.SkylandSettings ? window.SkylandSettings.snapshotFlat()
      : (JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.global)) || {});
    if (g.fontFamily && typeof window.skyFontStack === 'function') {
      document.documentElement.style.setProperty('--font-body', window.skyFontStack(g.fontFamily));
    }
    if (g.fontSize) {
      document.documentElement.style.setProperty('--font-size-base', g.fontSize + 'px');
    }
    if (g.fontWeight) {
      document.documentElement.style.setProperty('--font-weight-base', g.fontWeight);
    }
  } catch(e) {}

  // Home theme toggle
  const ht = document.getElementById('home-theme-toggle');
  if (ht) ht.addEventListener('click', () => {
    const isDark = document.documentElement.classList.toggle('dark');
    try {
      if (window.SkylandSettings) {
        window.SkylandSettings.set('appearance.theme', isDark ? 'dark' : 'light');
        window.SkylandSettings.save().catch(() => {});
        return;
      }
      const s = JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings)) || {};
      s.theme = isDark ? 'dark' : 'light';
      localStorage.setItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings, JSON.stringify(s));
    } catch(e) {}
  });

  // Global settings — 已并入统一设置面板；首页右下角与实验室悬浮按钮两处入口
  const populateGlobalFields = () => {
    try {
      const s = window.SkylandSettings ? window.SkylandSettings.snapshotFlat()
        : (JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.global)) || {});
      const cp = document.getElementById('global-config-path');
      const ud = document.getElementById('global-userdata-path');
      const convPath = document.getElementById('global-conversations-path');
      const todoPath = document.getElementById('global-todo-path');
      const imgPath = document.getElementById('global-img-path');
      if (cp) cp.value = s.configPath || 'SKYLAND/config/llm.toml';
      if (ud) ud.value = s.userDataPath || 'SKYLAND/users/local';
      if (convPath) convPath.value = s.conversationsPath || 'SKYLAND/users/local/conversations';
      if (todoPath) todoPath.value = s.todoListPath || 'SKYLAND/users/local/todo';
      if (imgPath) imgPath.value = s.imgPath || 'SKYLAND/users/local/images';
      fetch('/api/todo/path').then(r => r.ok ? r.json() : null).then(data => {
        if (data && data.path && todoPath) todoPath.value = data.path;
      }).catch(() => {});
      fetch('/api/image/path')
        .then(r => r.ok ? r.json() : null)
        .then(data => { if (data && imgPath) imgPath.value = data.path || imgPath.value; })
        .catch(() => {});
      // 全局外观
      const ff = document.getElementById('global-font-family');
      const fw = document.getElementById('global-font-weight');
      const fs = document.getElementById('global-font-size');
      const fsv = document.getElementById('global-font-size-value');
      if (ff) ff.value = s.fontFamily || 'system';
      if (fw) fw.value = s.fontWeight || '400';
      if (fs) { fs.value = s.fontSize || 14; if (fsv) fsv.textContent = fs.value; }
      if (fs) fs.addEventListener('input', () => { if (fsv) fsv.textContent = fs.value; });
      const autoToggle = document.getElementById('auto-dark-toggle');
      const autoConfig = document.getElementById('auto-dark-config');
      const darkStart = document.getElementById('dark-start');
      const darkEnd = document.getElementById('dark-end');
      if (s.autoDarkMode) {
        if (autoToggle) autoToggle.classList.add('active');
        if (autoConfig) autoConfig.style.display = 'block';
      } else {
        if (autoConfig) autoConfig.style.display = 'none';
      }
      if (darkStart) darkStart.value = s.darkModeStart || 21;
      if (darkEnd) darkEnd.value = s.darkModeEnd || 7;
      if (!s.autoDarkMode) {
        const theme = (window.SkylandSettings ? window.SkylandSettings.get('appearance.theme') : null)
          || (JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings)) || {}).theme || 'dark';
        document.querySelectorAll('[data-global-theme]').forEach(b => {
          b.classList.toggle('active', b.dataset.globalTheme === theme);
        });
      }
    } catch (e) {}
  };

  const openUnifiedSettings = (target) => {
    const sp = document.getElementById('settings-panel');
    const so = document.getElementById('settings-overlay');
    if (sp) sp.classList.add('open');
    if (so) so.classList.add('active');
    if (typeof Settings2 !== 'undefined' && Settings2.switchTo) Settings2.switchTo(target || 'data');
    populateGlobalFields();
    if (typeof loadPresets === 'function') loadPresets();
  };
  // 供 js/home.js Tab 呼出 / 抽屉联动使用（统一设置唯一开启逻辑）
  window.SkylandOpenSettings = openUnifiedSettings;
  const bindSettingsOpener = (id, target) => {
    const btn = document.getElementById(id);
    if (btn) btn.addEventListener('click', () => openUnifiedSettings(target || 'data'));
  };
  bindSettingsOpener('home-settings-btn');
  // 实验室齿轮与首页一致走统一设置面板（AI 语境落到「模型与搜索」页）。
  // 修复：曾同时被 settings.js 再绑一次打开 AISettings 模态 — 一次点击同时弹出两个窗口。
  bindSettingsOpener('agent-global-settings', 'models');
  // Agent 终端内设置按钮 — 打开时也填充字段
  const chatSettingsBtn = document.getElementById('btn-settings');
  if (chatSettingsBtn) chatSettingsBtn.addEventListener('click', populateGlobalFields);

  // 数据目录保存（server 模式非管理员跳过同步；门禁由后端兜底）
  const gsSave = document.getElementById('home-settings-save');
  const closeGs = () => {
    const sp = document.getElementById('settings-panel');
    const so = document.getElementById('settings-overlay');
    if (sp) sp.classList.remove('open');
    if (so) so.classList.remove('active');
  };
  async function canManagePaths() {
    try {
      const info = await fetch('/api/server/info').then(r => r.json());
      if (info.mode !== 'server') return true;
      const me = await fetch('/api/auth/me').then(r => r.json());
      return !!(me.authenticated && me.role === 'admin');
    } catch (e) { return true; }
  }
  if (gsSave) gsSave.addEventListener('click', async () => {
    const cp = document.getElementById('global-config-path');
    const ud = document.getElementById('global-userdata-path');
    const convPath = document.getElementById('global-conversations-path');
    const todoPath = document.getElementById('global-todo-path');
    const imgPath = document.getElementById('global-img-path');
    const autoActive = document.getElementById('auto-dark-toggle')?.classList.contains('active') || false;
    const darkStart = parseInt(document.getElementById('dark-start')?.value || '21');
    const darkEnd = parseInt(document.getElementById('dark-end')?.value || '7');
    const ff = document.getElementById('global-font-family');
    const fw = document.getElementById('global-font-weight');
    const fs = document.getElementById('global-font-size');
    const ffVal = ff ? ff.value : 'system';
    const fwVal = fw ? fw.value : '400';
    const fsVal = fs ? parseInt(fs.value) : 14;
    try {
      const nextGlobal = {
        configPath: cp ? cp.value : 'SKYLAND/config/llm.toml',
        userDataPath: ud ? ud.value : 'SKYLAND/users/local',
        conversationsPath: convPath ? convPath.value : 'SKYLAND/users/local/conversations',
        todoListPath: todoPath ? todoPath.value : 'SKYLAND/users/local/todo',
        imgPath: imgPath ? imgPath.value : 'SKYLAND/users/local/images',
        autoDarkMode: autoActive,
        darkModeStart: darkStart,
        darkModeEnd: darkEnd,
        fontFamily: ffVal,
        fontWeight: fwVal,
        fontSize: fsVal,
      };
      if (window.SkylandSettings) {
        window.SkylandSettings.replaceFlat({ ...window.SkylandSettings.snapshotFlat(), ...nextGlobal });
        window.SkylandSettings.save().catch(() => {});
      } else {
        localStorage.setItem(SkylandStore.LEGACY_SETTINGS_KEYS.global, JSON.stringify(nextGlobal));
      }
      closeGs();
      if (await canManagePaths()) {
        fetch('/api/image/path', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ path: nextGlobal.imgPath })
        }).then(async response => {
          if (!response.ok) {
            const message = await response.text().catch(() => '');
            throw new Error(message || '生图目录保存失败');
          }
        }).catch(error => {
          console.warn('生图目录未同步到服务端:', error.message);
          alert(error.message || '生图目录保存失败');
        });
        if (todoPath) {
          fetch('/api/todo/path', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ path: todoPath.value })
          }).catch(error => {
            console.warn('待办目录未同步到服务端:', error.message);
          });
        }
      }
      if (window.SkylandSettings) {
        window.SkylandSettings.set('appearance.font', ffVal);
        window.SkylandSettings.set('appearance.fontSize', fsVal);
        window.SkylandSettings.save().catch(() => {});
      } else {
        const ms = JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings)) || {};
        ms.font = ffVal;
        ms.fontSize = fsVal;
        localStorage.setItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings, JSON.stringify(ms));
      }
      if (typeof window.skyFontStack === 'function') document.documentElement.style.setProperty('--font-body', window.skyFontStack(ffVal));
      document.documentElement.style.setProperty('--font-size-base', fsVal + 'px');
      document.documentElement.style.setProperty('--font-weight-base', fwVal);
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('数据目录已保存（路径类改动重启后生效）');
    } catch (e) { closeGs(); }
  });

  document.querySelectorAll('[data-global-theme]').forEach(b => {
    b.addEventListener('click', () => {
      document.querySelectorAll('[data-global-theme]').forEach(x => x.classList.remove('active'));
      const autoToggle = document.getElementById('auto-dark-toggle');
      if (autoToggle) autoToggle.classList.remove('active');
      const autoConfig = document.getElementById('auto-dark-config');
      if (autoConfig) autoConfig.style.display = 'none';
      b.classList.add('active');
      const theme = b.dataset.globalTheme;
      document.documentElement.classList.toggle('dark', theme === 'dark');
      try {
        if (window.SkylandSettings) {
          window.SkylandSettings.set('appearance.theme', theme);
          window.SkylandSettings.set('appearance.autoDarkMode', false);
          window.SkylandSettings.save().catch(() => {});
          return;
        }
        const s = JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings)) || {};
        s.theme = theme;
        localStorage.setItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings, JSON.stringify(s));
        const g = JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.global)) || {};
        g.autoDarkMode = false;
        localStorage.setItem(SkylandStore.LEGACY_SETTINGS_KEYS.global, JSON.stringify(g));
      } catch (e) {}
    });
  });

  const autoToggleEl = document.getElementById('auto-dark-toggle');
  if (autoToggleEl) autoToggleEl.addEventListener('click', () => {
    document.querySelectorAll('[data-global-theme]').forEach(x => x.classList.remove('active'));
    autoToggleEl.classList.add('active');
    const autoConfig = document.getElementById('auto-dark-config');
    if (autoConfig) autoConfig.style.display = 'block';
    const darkStart = parseInt(document.getElementById('dark-start')?.value || '21');
    const darkEnd = parseInt(document.getElementById('dark-end')?.value || '7');
    const hour = new Date().getHours();
    const isDark = hour >= darkStart || hour < darkEnd;
    document.documentElement.classList.toggle('dark', isDark);
  });

  // Navigation buttons
  document.querySelectorAll('[data-route]').forEach(btn => {
    btn.addEventListener('click', () => Router.navigate(btn.dataset.route));
  });
  Router.init();

  // Daily tips — random short phrases shown on home and agent pages
  const TIPS = [
    '先跑通，再跑稳，最后才谈快',
    '改 bug 前，先让 bug 能稳定复现',
    '命名多想三分钟，读代码的人少走十分钟弯路',
    '警告不是噪音，是提前来打招呼的 bug',
    '日志打得好，半夜睡得早',
    '先问"数据长什么样"，再写处理逻辑',
    '接口越简单，用它的人越省心',
    '复杂的问题，先拆成三个小问题',
    '慢不可怕，不知道哪里慢才可怕',
    '小步提交，随时可回退',
    '与其猜测，不如验证',
    '能自动化的重复劳动，今天就把它自动化',
    '多花一分钟读报错，少花一小时瞎猜',
    '如果一段方案解释不清，多半是还没想清',
    '代码评审看设计，不只是挑拼写错误',
    '上线不是终点，是观察的开始',
    '数据结构定对了，算法自然就简单了',
    '每一次"临时方案"，最后都活过了三年',
    '边界情况想清楚，能省掉一半的返工',
    '重构之前，先给现状拍张照——测试',
  ];
  const homeTip = document.getElementById('home-tip');
  const agentTip = document.getElementById('agent-tip');
  const tip1 = TIPS[Math.floor(Math.random() * TIPS.length)];
  let tip2 = TIPS[Math.floor(Math.random() * TIPS.length)];
  while (tip2 === tip1 && TIPS.length > 1) tip2 = TIPS[Math.floor(Math.random() * TIPS.length)];
  if (homeTip) homeTip.textContent = tip1;
  if (agentTip) agentTip.textContent = tip2;
});
