// ═══════════════════════════════════════════════════════════════
// SKYLAND Home Shell — v0.19 视觉迭代
// 主页启动页（hero + 自定义背景）· 抽屉系统（模块/实验室叠加）
// 全局保存反馈 toast · Tab 呼出设置抽屉（仅主页与轻量子页）
// 设计标准：低饱和 · 飞书蓝仅作语义标记 · 深浅双主题自动反色
// ═══════════════════════════════════════════════════════════════

const SkyToast = (() => {
  let timer = null;
  function show(msg, type, ms) {
    const el = document.getElementById('sky-toast');
    if (!el) return;
    el.classList.toggle('error', type === 'error');
    el.innerHTML = '<span class="sky-toast-dot"></span><span></span>';
    el.lastChild.textContent = msg || '';
    el.classList.add('show');
    clearTimeout(timer);
    timer = setTimeout(() => el.classList.remove('show'), ms || 2400);
  }
  return { show };
})();

const HomeShell = (() => {
  const $ = id => document.getElementById(id);
  const BG_KEY = 'skyland.homeBg.v1';

  // ─── 抽屉系统 ───
  // 状态机：closed → home → lab（叠加，一级 parked）。设置抽屉独立（settings-panel）。
  const Drawer = {
    els: {},
    init() {
      this.els = {
        overlay: $('drawer-overlay'),
        home: $('home-drawer'),
        lab: $('lab-drawer'),
      };
      if (!this.els.overlay) return;
      $('home-start-btn')?.addEventListener('click', () => this.openHome());
      $('drawer-home-back')?.addEventListener('click', () => this.closeAll());
      $('drawer-lab-back')?.addEventListener('click', () => this.closeLab());
      $('drawer-card-agent')?.addEventListener('click', () => this.openLab());
      this.els.overlay.addEventListener('click', () => this.closeAll());
      // 主页设置图标与抽屉互斥（router.js 的监听先开面板，此处后收抽屉）
      $('home-settings-btn')?.addEventListener('click', () => this.closeAll());
      $('agent-global-settings')?.addEventListener('click', () => this.closeAll());
      // 路由变化（抽屉卡片 data-route 由 router.js 统一绑定导航）→ 收起全部抽屉
      window.addEventListener('hashchange', () => this.closeAll());
      document.addEventListener('keydown', e => {
        if (e.key !== 'Escape') return;
        const t = e.target;
        if (t && t.closest && t.closest('input, textarea, select, [contenteditable="true"]')) return;
        if (this.els.lab.classList.contains('open')) this.closeLab();
        else if (this.els.home.classList.contains('open')) this.closeAll();
      });
    },
    openHome() {
      this.els.overlay.classList.add('active');
      this.els.home.classList.add('open');
      this.els.home.setAttribute('aria-hidden', 'false');
    },
    openLab() {
      this.els.overlay.classList.add('active');
      this.els.home.classList.add('open', 'parked');
      this.els.home.setAttribute('aria-hidden', 'true');
      // 叠加节奏：一级抽屉先起步，二级随后跟进（动画层叠美感）
      setTimeout(() => {
        this.els.lab.classList.add('open');
        this.els.lab.setAttribute('aria-hidden', 'false');
      }, 90);
    },
    closeLab() {
      this.els.lab.classList.remove('open');
      this.els.lab.setAttribute('aria-hidden', 'true');
      this.els.home.classList.remove('parked');
      this.els.home.setAttribute('aria-hidden', 'false');
    },
    closeAll() {
      this.els.lab.classList.remove('open', 'parked');
      this.els.home.classList.remove('open', 'parked');
      this.els.home.setAttribute('aria-hidden', 'true');
      this.els.lab.setAttribute('aria-hidden', 'true');
      this.els.overlay.classList.remove('active');
    },
    anyOpen() {
      return this.els.home.classList.contains('open') || this.els.lab.classList.contains('open');
    },
  };

  // ─── 设置抽屉（复用统一设置面板，底部抽屉式）───
  function openSettings() {
    if (Drawer.anyOpen()) Drawer.closeAll();
    if (typeof window.SkylandOpenSettings === 'function') {
      window.SkylandOpenSettings('appearance');
    } else {
      $('settings-panel')?.classList.add('open');
      $('settings-overlay')?.classList.add('active');
    }
  }
  function closeSettings() {
    $('settings-panel')?.classList.remove('open');
    $('settings-overlay')?.classList.remove('active');
  }
  function settingsOpen() {
    return $('settings-panel')?.classList.contains('open');
  }

  // ─── Tab 呼出设置 — 仅主页与轻量子页；工作页（nodes 等）保持原交互 ───
  const TAB_ROUTES = ['/home', '/image', '/todo', '/tasks', '/kb', '/chat'];
  function bindTabLauncher() {
    document.addEventListener('keydown', e => {
      if (e.key !== 'Tab' || e.ctrlKey || e.altKey || e.metaKey) return;
      if (!TAB_ROUTES.includes(Router.current)) return;
      const t = e.target;
      if (t && t.closest && t.closest('input, textarea, select, [contenteditable="true"]')) return;
      if (document.querySelector('.slash-menu')) return;           // slash 菜单打开时 Tab=补全
      if (document.querySelector('.effort-menu')) return;          // 思考强度/执行模式菜单导航
      if (document.querySelector('.nodes-search-panel')?.style.display !== 'none') return;
      e.preventDefault();
      if (settingsOpen()) { closeSettings(); return; }
      openSettings();
    });
  }

  // ─── 主页自定义背景（localStorage + canvas 压缩，仅存本机）───
  function applyBackground(dataUrl) {
    const img = $('home-bg-img');
    const box = $('home-bg');
    if (!img || !box) return;
    if (dataUrl) {
      img.style.backgroundImage = `url("${dataUrl}")`;
      img.classList.add('show');
      box.classList.add('custom');
    } else {
      img.style.backgroundImage = '';
      img.classList.remove('show');
      box.classList.remove('custom');
    }
    const prev = $('home-bg-preview');
    if (prev) prev.style.backgroundImage = dataUrl ? `url("${dataUrl}")` : '';
  }

  function loadBackground() {
    let d = null;
    try { d = localStorage.getItem(BG_KEY); } catch (e) {}
    if (d) applyBackground(d);
  }

  // 压缩到最长边 1920、JPEG q0.82 — 控制 localStorage 体积（~5MB 配额）
  function compressImage(file) {
    return new Promise((resolve, reject) => {
      const url = URL.createObjectURL(file);
      const im = new Image();
      im.onload = () => {
        try {
          const scale = Math.min(1, 1920 / Math.max(im.width, im.height));
          const c = document.createElement('canvas');
          c.width = Math.round(im.width * scale);
          c.height = Math.round(im.height * scale);
          c.getContext('2d').drawImage(im, 0, 0, c.width, c.height);
          resolve(c.toDataURL('image/jpeg', 0.82));
        } catch (err) { reject(err); } finally { URL.revokeObjectURL(url); }
      };
      im.onerror = () => { URL.revokeObjectURL(url); reject(new Error('图片读取失败')); };
      im.src = url;
    });
  }

  async function saveBackground(file) {
    try {
      const d = await compressImage(file);
      try { localStorage.setItem(BG_KEY, d); }
      catch (e) { SkyToast.show('背景图过大，压缩后仍超出本机存储配额', 'error'); return; }
      applyBackground(d);
      SkyToast.show('主页背景已更新');
    } catch (e) {
      SkyToast.show('背景图设置失败：' + e.message, 'error');
    }
  }

  function initBackground() {
    loadBackground();
    $('home-bg-btn')?.addEventListener('click', () => $('home-bg-input')?.click());
    $('home-bg-input')?.addEventListener('change', e => {
      const f = e.target.files && e.target.files[0];
      if (f) saveBackground(f);
      e.target.value = '';
    });
    // 设置面板 · 外观页入口
    $('btn-home-bg-select')?.addEventListener('click', () => $('home-bg-input')?.click());
    $('btn-home-bg-reset')?.addEventListener('click', () => {
      try { localStorage.removeItem(BG_KEY); } catch (e) {}
      applyBackground(null);
      SkyToast.show('已恢复默认背景');
    });
  }

  // ─── 设置面板 · 外观页保存（字体 + 自动深色时段；主题/主题色即时生效）───
  function initAppearanceSave() {
    const btn = $('home-appearance-save');
    if (!btn) return;
    btn.addEventListener('click', () => {
      const ff = $('global-font-family'), fw = $('global-font-weight'), fs = $('global-font-size');
      const ffVal = ff ? ff.value : 'system';
      const fwVal = fw ? fw.value : '400';
      const fsVal = fs ? parseInt(fs.value, 10) : 14;
      const autoActive = $('auto-dark-toggle')?.classList.contains('active') || false;
      const darkStart = parseInt($('dark-start')?.value || '21', 10);
      const darkEnd = parseInt($('dark-end')?.value || '7', 10);
      try {
        const next = {
          fontFamily: ffVal, fontWeight: fwVal, fontSize: fsVal,
          autoDarkMode: autoActive, darkModeStart: darkStart, darkModeEnd: darkEnd,
        };
        if (window.SkylandSettings) {
          window.SkylandSettings.replaceFlat({ ...window.SkylandSettings.snapshotFlat(), ...next });
          window.SkylandSettings.set('appearance.font', ffVal);
          window.SkylandSettings.set('appearance.fontSize', fsVal);
          window.SkylandSettings.save().catch(() => {});
        } else {
          const g = JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.global)) || {};
          Object.assign(g, next);
          localStorage.setItem(SkylandStore.LEGACY_SETTINGS_KEYS.global, JSON.stringify(g));
          const ms = JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings)) || {};
          ms.font = ffVal; ms.fontSize = fsVal;
          localStorage.setItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings, JSON.stringify(ms));
        }
        document.documentElement.style.setProperty('--font-body', window.skyFontStack ? window.skyFontStack(ffVal) : ffVal);
        document.documentElement.style.setProperty('--font-size-base', fsVal + 'px');
        document.documentElement.style.setProperty('--font-weight-base', fwVal);
        if (autoActive) {
          const hour = new Date().getHours();
          document.documentElement.classList.toggle('dark', hour >= darkStart || hour < darkEnd);
        }
        SkyToast.show('外观设置已保存');
      } catch (e) {
        SkyToast.show('外观保存失败：' + e.message, 'error');
      }
    });
  }

  function init() {
    if (window._skylandHomeShellInited) return;
    window._skylandHomeShellInited = true;
    Drawer.init();
    bindTabLauncher();
    initBackground();
    initAppearanceSave();
  }

  return { init, Drawer, openSettings, closeSettings };
})();

// 子页（Agent 终端 / AI生图 / 节点流 / 待时任务）左上返回 → 主页 + 直接叠出实验室抽屉
window.SkylandOpenLab = function () {
  if (typeof Router !== 'undefined') Router.navigate('/home');
  setTimeout(() => HomeShell.Drawer.openLab(), 60);
};

document.addEventListener('DOMContentLoaded', () => HomeShell.init());
