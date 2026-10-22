// ═══════════════════════════════════════════════════════════════════
// SKYLAND ToDoList v2 — 智能待办事项 + 优先级 + 分类 + 截止日期 + 统计
// ═══════════════════════════════════════════════════════════════════

const TodoApp = (() => {
  const $ = id => document.getElementById(id);
  let currentDate = '';
  let items = [];
  let notifyCheckTimer = null;
  let notifyEngineStarted = false;
  let activeCategory = 'all';
  let allCategories = [];
  let searchMode = false;
  let carryoverData = null;
  // ── v3: 三 Tab + memos 式速记 ──
  let activeTab = 'today';
  let serverMode = false;
  let timelineState = { groups: [], nextBefore: null, hasMore: false, tag: 'all', loaded: false };
  // ── v3.1: 日历（memos 风）──
let calState = { year: 0, month: 0, counts: {}, dateFilter: null, visible: localStorage.getItem(SkylandStore.KEYS.todoCal) !== 'off' };
  let exploreState = { groups: [], nextBefore: null, hasMore: false, tag: 'all', mode: 'solo', tags: [], stats: null, loaded: false };

  // ─── 初始化 ───
  function init() {
    bindEvents();
    bindTabs();
    bindKeyboardShortcuts();
    loadToday();
    startNotifyEngine();
    setInterval(checkExpiry, 60000);
    // Load stats periodically
    loadStats();
    setInterval(loadStats, 120000);
    detectServerMode();
  }

  // ─── 通知引擎（远端兜底武装，幂等）───
  // 本机后端（localhost）有自己的 Rust 通知引擎（todo_notify.rs）直发系统 Toast，
  // 前端引擎只在远端页面（局域网浏览器 / 客户端壳连接云端）武装，
  // 双方共享 last_notified 判据天然不双发。
  function isLocalBackend() {
    return ['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname);
  }

  function startNotifyEngine() {
    if (notifyEngineStarted) return;
    notifyEngineStarted = true; // 先置位：本机/远端任一分支都不会重复武装
    if (isLocalBackend()) return; // 本机由后端引擎负责
    notifyCheckTimer = setInterval(checkAndFireNotifications, 30000);
    setTimeout(checkAndFireNotifications, 2000);
  }

  function onShow() {
    // 切走再切回时清理搜索态，避免列表残留隐藏
    if (searchMode) toggleSearch(false);
    if (activeTab === 'timeline') { loadTimeline(true); }
    else if (activeTab === 'explore') { loadExplore(true); }
    else if (!currentDate) { loadToday(); }
    else { loadList(currentDate); }
    loadStats();
    // 打开待办页 → 向云端同步源拉取一次（未配置时后端 noop；失败静默不打扰）
    if (typeof window.__skylandLastAutoPull !== 'number' || Date.now() - window.__skylandLastAutoPull > 60000) {
      window.__skylandLastAutoPull = Date.now();
      fetch('/api/todo/sync/pull', { method: 'POST' }).then(r => r.json()).then(d => {
        if (d && d.status === 'ok' && d.imported > 0) {
          if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show(`同步：从云端拉取 ${d.imported} 条新待办`);
          loadToday();
        }
      }).catch(() => {});
    }
  }

  function bindEvents() {
    const addBtn = $('todo-add-btn');
    if (addBtn) addBtn.addEventListener('click', () => showAddDialog(null, currentDate));

    const prevBtn = $('todo-prev-day');
    if (prevBtn) prevBtn.addEventListener('click', () => changeDay(-1));
    const nextBtn = $('todo-next-day');
    if (nextBtn) nextBtn.addEventListener('click', () => changeDay(1));
    const todayBtn = $('todo-today-btn');
    if (todayBtn) todayBtn.addEventListener('click', () => loadToday());

    const backBtn = $('todo-back-home');
    if (backBtn) backBtn.addEventListener('click', () => Router.navigate('/home'));

    // 速记编辑器（memos 式）：Ctrl+Enter 提交，输入实时解析预览
    const quickInput = $('todo-quick-input');
    if (quickInput) {
      quickInput.addEventListener('keydown', e => {
        // IME 组词中的回车是确认候选词，不是提交
        if (e.isComposing || e.keyCode === 229) return;
        if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); quickAdd(); }
      });
      quickInput.addEventListener('input', () => { autoGrowEditor(quickInput); renderEditorPreview(); });
    }
    // 日历导航
    const calPrev = $('todo-cal-prev');
    if (calPrev) calPrev.addEventListener('click', () => calShiftMonth(-1));
    const calNext = $('todo-cal-next');
    if (calNext) calNext.addEventListener('click', () => calShiftMonth(1));
    const calClear = $('todo-cal-clear');
    if (calClear) calClear.addEventListener('click', () => { calState.dateFilter = null; renderCalendar(); loadTimeline(true); });

    const quickBtn = $('todo-quick-add-btn');
    if (quickBtn) quickBtn.addEventListener('click', () => quickAdd());

    // 编辑器工具栏（memos 式）— 分类 / 优先级 / 提醒时间
    $('todo-ed-cat')?.addEventListener('click', e => { e.stopPropagation(); toggleEditorMenu('cat', e.currentTarget); });
    $('todo-ed-pri')?.addEventListener('click', e => { e.stopPropagation(); toggleEditorMenu('pri', e.currentTarget); });
    $('todo-ed-time')?.addEventListener('click', e => { e.stopPropagation(); toggleEditorMenu('time', e.currentTarget); });

    // 通知链路测试（header 铃铛 + 编辑弹窗按钮共用）—— 一键验证系统 Toast 是否真的弹出
    const testBtn = $('todo-notify-test-btn');
    if (testBtn) testBtn.addEventListener('click', () => sendTestNotification());

    // Search toggle
    const searchToggle = $('todo-search-toggle');
    if (searchToggle) searchToggle.addEventListener('click', () => toggleSearch());
    const searchClose = $('todo-search-close');
    if (searchClose) searchClose.addEventListener('click', () => toggleSearch(false));
    const searchInput = $('todo-search-input');
    if (searchInput) {
      let searchTimer = null;
      searchInput.addEventListener('input', () => {
        clearTimeout(searchTimer);
        searchTimer = setTimeout(() => doSearch(searchInput.value.trim()), 300);
      });
    }
  }

  function bindKeyboardShortcuts() {
    document.addEventListener('keydown', e => {
      // Only active when todo view is visible
      const view = $('view-todo');
      if (!view || view.style.display === 'none') return;
      // 弹窗打开时：快捷键静默，Escape 关闭最上层弹窗（走取消按钮以 resolve Promise）
      if (document.querySelector('.kb-modal-overlay')) {
        if (e.key === 'Escape') {
          const modals = document.querySelectorAll('.kb-modal-overlay');
          const top = modals[modals.length - 1];
          const cancel = top.querySelector('.kb-modal-btn-cancel, #todo-alert-ok');
          if (cancel) cancel.click(); else top.remove();
        }
        return;
      }
      // 登录 / 首启引导遮罩打开时不响应快捷键（中心化守卫，新增遮罩自动生效）
      if (typeof Auth !== 'undefined' && Auth.isOverlayActive && Auth.isOverlayActive()) return;
      // 设置抽屉打开时快捷键静默（避免设置面板上再叠出新建弹窗）
      if (document.getElementById('settings-panel')?.classList.contains('open')) return;
      // Don't intercept when typing in inputs
      const tag = e.target.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      // Ctrl+B 日历收放 — 必须在修饰键拦截之前判断（否则永不触达）
      if ((e.ctrlKey || e.metaKey) && !e.altKey && (e.key === 'b' || e.key === 'B')) {
        e.preventDefault();
        calState.visible = !calState.visible;
        localStorage.setItem(SkylandStore.KEYS.todoCal, calState.visible ? 'on' : 'off');
        applyCalendarVisibility();
        return;
      }
      if (e.ctrlKey || e.altKey || e.metaKey) return;

      switch(e.key) {
        case 'n': case 'N':
          e.preventDefault(); showAddDialog(null, currentDate); break;
        case '/':
          e.preventDefault();
          if (activeTab !== 'today') break; // 搜索栏在今日视图内 — 其他 Tab 静默忽略
          toggleSearch(true); break;
        case 'Escape':
          if (searchMode) toggleSearch(false);
          break;
      }
    });
  }

  // ─── v3: 三 Tab 切换 + 运行模式 ───
  function bindTabs() {
    document.querySelectorAll('#todo-tabs .todo-tab').forEach(btn => {
      btn.addEventListener('click', () => showTab(btn.dataset.todoTab));
    });
  }

  function showTab(tab) {
    if (!['today', 'timeline', 'explore'].includes(tab)) return;
    if (tab === 'timeline') refreshCalendarCounts();
    setTimeout(applyCalendarVisibility, 0);
    activeTab = tab;
    document.querySelectorAll('#todo-tabs .todo-tab').forEach(b =>
      b.classList.toggle('active', b.dataset.todoTab === tab));
    const sub = $('todo-subheader');
    const editorWrap = $('todo-editor-wrap');
    const todayView = $('todo-view-today');
    const tlView = $('todo-view-timeline');
    const exView = $('todo-view-explore');
    if (sub) sub.style.display = tab === 'today' ? '' : 'none';
    if (editorWrap) editorWrap.style.display = tab === 'explore' ? 'none' : '';
    if (todayView) todayView.style.display = tab === 'today' ? '' : 'none';
    if (tlView) tlView.style.display = tab === 'timeline' ? '' : 'none';
    if (exView) exView.style.display = tab === 'explore' ? '' : 'none';
    if (tab === 'timeline') loadTimeline(true);
    if (tab === 'explore') loadExplore(true);
  }

  async function detectServerMode() {
    try {
      const r = await fetch('/api/server/info');
      const data = await r.json();
      serverMode = data.mode === 'server';
    } catch (e) { serverMode = false; }
  }

  // ─── v3: memos 式速记语法 ───
  // 语法：#分类（首个生效，其余保留原文）、!!/! 优先级前缀、@HH:MM 提醒时间（可多个）、
  //       @YYYY-MM-DD / @MM-DD（当年）目标日期（首个生效，条目落到那天清单）
  function _parseQuickSyntax(raw) {
    let text = (raw || '').trim();
    const result = { title: '', category: '', priority: 1, notify_times: [], notify_enabled: false, date: null, error: null };
    const times = [];
    let date = null;
    // @日期：先长格式再短格式（当年）；JS Date 对非法日期会静默回滚（2月30日→3月2日），
    // 必须用"构造后往返比对"校验而非 isNaN
    const validYmd = (y, mo, d) => {
      const dt = new Date(y, mo - 1, d);
      return dt.getFullYear() === +y && dt.getMonth() === mo - 1 && dt.getDate() === +d;
    };
    text = text.replace(/@(\d{4})-(\d{1,2})-(\d{1,2})/g, (m, y, mo, d) => {
      if (!validYmd(y, mo, d)) return m;
      if (!date) date = `${y}-${String(mo).padStart(2, '0')}-${String(d).padStart(2, '0')}`;
      return '';
    });
    const yearNow = new Date().getFullYear();
    text = text.replace(/@(\d{1,2})-(\d{1,2})/g, (m, mo, d) => {
      if (!validYmd(yearNow, mo, d)) return m;
      if (!date) date = `${yearNow}-${String(mo).padStart(2, '0')}-${String(d).padStart(2, '0')}`;
      return '';
    });
    text = text.replace(/@(\d{1,2}):([0-5]\d)/g, (m, h, mm) => {
      if (parseInt(h, 10) > 23) return m; // 无效时间保留原文
      times.push(`${String(h).padStart(2, '0')}:${mm}`);
      return '';
    });
    let category = '';
    text = text.replace(/#([^\s#@]+)/, (m, t) => { category = t; return ''; }); // 仅首个
    text = text.trimStart(); // 分类在前的输入移除标签后残留前导空格，保证 !!/! 前缀可匹配
    let priority = 1;
    text = text.replace(/^(!{1,2})\s*/, (m, bangs) => { priority = bangs.length === 2 ? 2 : 0; return ''; });
    result.title = text.replace(/\s+/g, ' ').trim();
    result.category = category;
    result.priority = priority;
    result.date = date;
    if (times.length) { result.notify_times = times; result.notify_enabled = true; }
    if (!result.title) result.error = 'empty';
    return result;
  }

  function autoGrowEditor(el) {
    el.style.height = 'auto';
    el.style.height = Math.min(Math.max(el.scrollHeight, 58), 220) + 'px';
  }

  // ─── 速记编辑器工具栏（memos 式）───
  // 设计约束：按钮只负责「往输入框写 token」，语义解析永远走 _parseQuickSyntax
  // 单一真源 —— 预览 chips 与提交共用同一解析，避免按钮与语法两套状态漂移。
  // 第三方同步适配（memos/flomo/Notion 导入）也以该解析为边界：外部数据
  // 转 token 文本或直接走 /api/todo/import 结构化通道，见 todo_api.rs。

  function insertToken(token) {
    const input = $('todo-quick-input');
    if (!input) return;
    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? start;
    const before = input.value.slice(0, start);
    const after = input.value.slice(end);
    const pad = before && !/\s$/.test(before) ? ' ' : '';
    input.value = before + pad + token + ' ' + after.replace(/^\s+/, '');
    const pos = (before + pad + token + ' ').length;
    input.focus();
    input.setSelectionRange(pos, pos);
    autoGrowEditor(input);
    renderEditorPreview();
  }

  function stripPriorityTokens() {
    const input = $('todo-quick-input');
    if (!input) return;
    input.value = input.value.replace(/^\s*(!{1,2})\s*/, '').replace(/(^|\s)!(\s|$)/g, '$1').trimStart();
    input.focus();
    autoGrowEditor(input);
    renderEditorPreview();
  }

  // 优先级是行首语法（解析器 ^(!{1,2}) 只匹配开头）— 必须写入行首而非光标处，
  // 否则 token 落进标题（真机踩过：mid-text !! 原样进了标题）
  function setPriorityToken(token) {
    const input = $('todo-quick-input');
    if (!input) return;
    const rest = input.value.replace(/^\s*(!{1,2})\s*/, '').trimStart();
    input.value = rest ? token + ' ' + rest : token + ' ';
    const pos = input.value.length;
    input.focus();
    input.setSelectionRange(pos, pos);
    autoGrowEditor(input);
    renderEditorPreview();
  }

  function closeEditorMenu() {
    const m = $('todo-ed-menu');
    if (m) m.style.display = 'none';
    document.querySelectorAll('.todo-ed-btn.active').forEach(b => b.classList.remove('active'));
    document.removeEventListener('click', onEditorMenuDocClick, true);
  }
  function onEditorMenuDocClick(e) {
    const m = $('todo-ed-menu');
    if (m && !m.contains(e.target) && !e.target.closest('.todo-ed-btn')) closeEditorMenu();
  }

  function toggleEditorMenu(kind, btn) {
    const m = $('todo-ed-menu');
    if (!m) return;
    const wasOpen = m.style.display !== 'none' && m.dataset.kind === kind;
    closeEditorMenu();
    if (wasOpen) return;
    m.dataset.kind = kind;
    m.innerHTML = buildMenuHtml(kind);
    m.style.display = 'block';
    btn.classList.add('active');
    bindMenuActions(kind);
    setTimeout(() => document.addEventListener('click', onEditorMenuDocClick, true), 0);
  }

  function buildMenuHtml(kind) {
    const esc2 = esc;
    if (kind === 'cat') {
      const cats = (allCategories || []).slice(0, 12);
      const items = cats.length
        ? cats.map(c => `<button class="todo-ed-item" data-cat="${esc2(c)}"><span class="todo-ed-item-tok">#</span>${esc2(c)}</button>`).join('')
        : '<div class="todo-ed-menu-hint">暂无分类 — 直接新建一个</div>';
      return `<div class="todo-ed-menu-title">分类</div>${items}
        <div class="todo-ed-newrow">
          <span class="todo-ed-item-tok">#</span>
          <input type="text" id="todo-ed-newcat" class="todo-ed-newcat" placeholder="新建分类，回车插入" maxlength="20">
        </div>`;
    }
    if (kind === 'pri') {
      const cur = _parseQuickSyntax($('todo-quick-input')?.value).priority;
      const opt = (label, desc, v) => `<button class="todo-ed-item${cur === v ? ' active' : ''}" data-pri="${v}">
        <span class="todo-ed-item-tok">${v === 2 ? '!!' : v === 0 ? '!' : '—'}</span>${label}
        ${cur === v ? '<span class="todo-ed-check">✓</span>' : ''}</button>`;
      return `<div class="todo-ed-menu-title">优先级</div>
        ${opt('高优先', '红色标记 + 置顶', 2)}
        ${opt('普通', '默认', 1)}
        ${opt('低优先', '弱化显示', 0)}`;
    }
    if (kind === 'time') {
      // 极简相对时间（用户裁定方案）— label 相对、右侧动态算出确切时刻，
      // 兼顾「快」与「可预期」；精确时刻走条目编辑弹窗（已有完整通知字段）
      const fmt = d => String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
      const opts = [
        { min: 30, label: '30 分钟后' },
        { min: 60, label: '1 小时后' },
        { min: 180, label: '3 小时后' },
        { min: 480, label: '8 小时后' },
      ];
      return `<div class="todo-ed-menu-title">提醒时间</div>` + opts.map(o => {
        const at = fmt(new Date(Date.now() + o.min * 60 * 1000));
        return `<button class="todo-ed-item todo-ed-rel" data-time="${at}">${o.label}<span class="todo-ed-rel-at">${at}</span></button>`;
      }).join('');
    }
    return '';
  }

  function bindMenuActions(kind) {
    const m = $('todo-ed-menu');
    if (!m) return;
    const input = $('todo-quick-input');
    if (kind === 'cat') {
      m.querySelectorAll('[data-cat]').forEach(b => b.addEventListener('click', () => { insertToken('#' + b.dataset.cat); closeEditorMenu(); }));
      const newCat = m.querySelector('#todo-ed-newcat');
      if (newCat) newCat.addEventListener('keydown', e => {
        if (e.key === 'Enter' && newCat.value.trim()) { insertToken('#' + newCat.value.trim()); closeEditorMenu(); }
        if (e.key === 'Escape') closeEditorMenu();
        e.stopPropagation();
      });
    } else if (kind === 'pri') {
      m.querySelectorAll('[data-pri]').forEach(b => b.addEventListener('click', () => {
        const v = b.dataset.pri;
        if (v === '1') { stripPriorityTokens(); }
        else { setPriorityToken(v === '2' ? '!!' : '!'); }
        closeEditorMenu();
      }));
    } else if (kind === 'time') {
      m.querySelectorAll('[data-time]').forEach(b => b.addEventListener('click', () => { insertToken('@' + b.dataset.time); closeEditorMenu(); }));
      const tIn = m.querySelector('#todo-ed-newtime');
      m.querySelector('[data-time-input]')?.addEventListener('click', () => {
        if (tIn && tIn.value) { insertToken('@' + tIn.value); closeEditorMenu(); }
      });
      tIn?.addEventListener('keydown', e => { if (e.key === 'Escape') closeEditorMenu(); e.stopPropagation(); });
    }
    void input;
  }

  function renderEditorPreview() {
    const input = $('todo-quick-input');
    const box = $('todo-editor-preview');
    if (!input || !box) return;
    if (!input.value.trim()) { box.innerHTML = ''; return; }
    const v = _parseQuickSyntax(input.value);
    const chips = [];
    if (v.priority === 2) chips.push('<span class="todo-editor-chip pri-high">!! 高优先</span>');
    if (v.priority === 0) chips.push('<span class="todo-editor-chip pri-low">! 低优先</span>');
    if (v.category) chips.push(`<span class="todo-editor-chip">#${esc(v.category)}</span>`);
    if (v.date) chips.push(`<span class="todo-editor-chip">${ICONS.calendar} ${esc(v.date)}</span>`);
    for (const t of v.notify_times) chips.push(`<span class="todo-editor-chip">${ICONS.clock} ${esc(t)}</span>`);
    if (v.error === 'empty') chips.push(`<span class="todo-editor-chip">${ICONS.warn} 缺少标题内容</span>`);
    box.innerHTML = chips.join('');
  }

  // ─── 日期工具 ───
  function formatDate(d) {    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${y}-${m}-${day}`;
  }

  function formatDateDisplay(dateStr) {
    const d = new Date(dateStr + 'T00:00:00');
    const today = new Date();
    today.setHours(0,0,0,0);
    const diff = Math.round((d - today) / 86400000);
    if (diff === 0) return '今日待办';
    if (diff === -1) return '昨日待办';
    if (diff === 1) return '明日待办';
    const weekday = ['周日','周一','周二','周三','周四','周五','周六'][d.getDay()];
    return `${dateStr} ${weekday}`;
  }

  async function loadToday() {
    currentDate = formatDate(new Date());
    // today 端点附带跨天延续数据（carryover）
    await loadList(currentDate, true);
  }

  async function changeDay(delta) {
    const d = new Date(currentDate + 'T00:00:00');
    d.setDate(d.getDate() + delta);
    currentDate = formatDate(d);
    await loadList(currentDate);
  }

  // ─── 数据加载 ───
  async function loadList(date, isToday = false) {
    $('todo-date-display').textContent = formatDateDisplay(date);
    // 切换日期时重置分类筛选，避免假"暂无待办"
    activeCategory = 'all';
    try {
      const r = await fetch(isToday ? '/api/todo/today' : `/api/todo/${date}`);
      // 会话过期/未登录 → 显示登录遮罩（切勿 reload：未登录时视图重载会再触发 401，无限循环）
      if (r.status === 401) {
        if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost();
        return;
      }
      const text = await r.text();
      let data;
      try { data = JSON.parse(text); } catch(e) {
        items = []; render(); hideCarryoverPrompt(); return;
      }
      if (data.status === 'ok' && data.list) {
        items = data.list.items || [];
      } else {
        items = [];
      }

      // Handle carryover (only for today)
      if (data.carryover && data.carryover.length > 0) {
        carryoverData = { items: data.carryover, dates: data.carryover_dates || [] };
        showCarryoverPrompt();
      } else {
        carryoverData = null;
        hideCarryoverPrompt();
      }

      // Extract categories
      updateCategories();
      checkExpiry();
      render();
    } catch(e) {
      console.error('loadList error:', e);
      items = []; render(); hideCarryoverPrompt();
    }
  }

  function updateCategories() {
    const cats = new Set();
    for (const item of items) {
      if (item.category) cats.add(item.category);
    }
    allCategories = Array.from(cats).sort();
  }

  function checkExpiry() {
    let changed = false;
    const todayStr = formatDate(new Date());
    for (const item of items) {
      if (!item.done && !item.expired) {
        // Only mark expired if the item's date is before today
        if (currentDate < todayStr) {
          item.expired = true;
          changed = true;
        }
      }
    }
    if (changed) render();
  }

  function isTimePassed(timeStr) {
    const [h, m] = timeStr.split(':').map(Number);
    const now = new Date();
    return now.getHours() > h || (now.getHours() === h && now.getMinutes() >= m);
  }

  // ─── 渲染 ───
  function render() {
    renderProgress();
    renderCategoryFilter();
    renderItems();
  }

  function renderProgress() {
    const total = items.length;
    const done = items.filter(i => i.done).length;
    const pct = total > 0 ? (done / total * 100) : 0;
    const fill = $('todo-progress-fill');
    const text = $('todo-progress-text');
    if (fill) fill.style.width = pct + '%';
    if (text) text.textContent = `${done}/${total}`;
  }

  function renderCategoryFilter() {
    const bar = $('todo-filter-bar');
    if (!bar) return;
    if (allCategories.length === 0) { bar.style.display = 'none'; return; }
    bar.style.display = 'flex';
    const chips = [`<button class="todo-filter-chip ${activeCategory === 'all' ? 'active' : ''}" data-cat="all">全部</button>`];
    for (const cat of allCategories) {
      chips.push(`<button class="todo-filter-chip ${activeCategory === cat ? 'active' : ''}" data-cat="${esc(cat)}">${esc(cat)}</button>`);
    }
    bar.innerHTML = chips.join('');
    bar.querySelectorAll('.todo-filter-chip').forEach(btn => {
      btn.addEventListener('click', () => {
        activeCategory = btn.dataset.cat;
        render();
      });
    });
  }

  function isItemOverdue(item) {
    if (item.done) return false;
    if (item.due_date) {
      const today = formatDate(new Date());
      if (item.due_date < today) return true;
      if (item.due_date === today && item.due_time && isTimePassed(item.due_time)) return true;
    }
    return false;
  }

  function getSortedItems() {
    let filtered = items;
    if (activeCategory !== 'all') {
      filtered = items.filter(i => i.category === activeCategory);
    }
    return filtered.slice().sort((a, b) => {
      // Overdue items to top
      const aOverdue = !a.done && (a.expired || isItemOverdue(a));
      const bOverdue = !b.done && (b.expired || isItemOverdue(b));
      if (aOverdue && !bOverdue) return -1;
      if (!aOverdue && bOverdue) return 1;
      // Done items to bottom
      if (a.done && !b.done) return 1;
      if (!a.done && b.done) return -1;
      // Priority desc
      const aPri = a.priority ?? 1;
      const bPri = b.priority ?? 1;
      if (aPri !== bPri) return bPri - aPri;
      // Sort order asc
      return (a.sort_order || 0) - (b.sort_order || 0);
    });
  }

  function renderItems() {
    const container = $('todo-items');
    if (!container) return;
    const sorted = getSortedItems();
    if (sorted.length === 0) {
      container.innerHTML = '<div class="todo-empty">暂无待办事项<br><span class="todo-empty-hint">点击右下角 + 或在上方输入框快速添加</span></div>';
      return;
    }
    container.innerHTML = sorted.map(item => renderItemCard(item, currentDate)).join('');
    bindCardActions(container);
  }

  // 卡片操作统一委托 — 卡片根节点携带 data-date，供时间线/发现页跨日期操作
  function bindCardActions(container) {
    container.querySelectorAll('[data-action]').forEach(btn => {
      btn.addEventListener('click', e => {
        e.stopPropagation();
        const action = btn.dataset.action;
        const id = btn.dataset.id;
        const card = btn.closest('.todo-card');
        const date = (card && card.dataset.date) || currentDate;
        if (action === 'toggle') toggleItem(id, date);
        else if (action === 'edit') editItem(id, date);
        else if (action === 'delete') deleteItem(id, date);
        else if (action === 'visibility') toggleVisibility(id, date);
        else if (action === 'mdcheck' && card) toggleMemoCheck(card.dataset.id, date, parseInt(btn.dataset.ci, 10));
      });
    });
  }

  // 在当前激活视图的数据源里定位事项
  function findItemAnywhere(id, date) {
    if (date === currentDate) return items.find(i => i.id === id);
    const state = activeTab === 'timeline' ? timelineState : exploreState;
    for (const g of state.groups) {
      if (g.date !== date) continue;
      const f = g.items.find(i => i.id === id);
      if (f) return f;
    }
    return null;
  }

  // 变更后刷新当前视图（今日视图本地渲染；时间线/发现整页重载保证一致性）
  function refreshActiveView() {
    if (activeTab === 'timeline') loadTimeline(true);
    else if (activeTab === 'explore') loadExplore(true);
    else { updateCategories(); render(); }
  }

  function renderItemCard(item, date) {
    const d = date || currentDate;
    const priority = item.priority ?? 1;
    const priorityClass = ['todo-pri-low', 'todo-pri-mid', 'todo-pri-high'][priority] || 'todo-pri-mid';
    const overdue = isItemOverdue(item);
    const statusClass = item.done ? 'done' : item.expired ? 'expired' : overdue ? 'overdue' : 'pending';
    const checkIcon = item.done ? '✓' : item.expired ? '✗' : '○';
    const notifyBadge = item.notify_enabled ? '<span class="todo-notify-badge" title="通知已开启">' + ICONS.bell + '</span>' : '';
    const overdueBadge = (overdue && !item.expired) ? '<span class="todo-overdue-badge">已逾期</span>' : '';
    const categoryTag = item.category ? `<span class="todo-cat-tag">${esc(item.category)}</span>` : '';
    const authorChip = item.author ? `<span class="todo-author-chip">@${esc(item.author)}</span>` : '';
    // 发现页的他人公开条目：只读展示（本地用户目录里没有这条数据，
    // 勾选/编辑/删除只会拿到 200+error 的假成功）——隐藏全部操作按钮
    const mine = !item.author;
    const checkCtl = mine
      ? `<button class="todo-check ${statusClass}" data-action="toggle" data-id="${item.id}">${checkIcon}</button>`
      : `<span class="todo-check ${statusClass}" title="来自 ${esc(item.author)} 的公开事项">${checkIcon}</span>`;
    // server 模式下展示公私徽章（点击切换，仅自己的条目）
    const visBadge = (serverMode && mine)
      ? `<button class="todo-vis-badge${item.visibility === 'public' ? ' is-public' : ''}" data-action="visibility" data-id="${item.id}" title="${item.visibility === 'public' ? '公开 — 点击设为私有' : '私有 — 点击设为公开'}">${item.visibility === 'public' ? '公开' : '私有'}</button>`
      : '';
    const dueInfo = formatDueInfo(item, overdue);
    const notifyInfo = formatNotifyInfo(item);

    return `
      <div class="todo-card ${statusClass} ${priorityClass}" data-id="${item.id}" data-date="${d}">
        <div class="todo-card-pri-bar"></div>
        <div class="todo-card-content">
          <div class="todo-card-header">
            ${checkCtl}
            <span class="todo-card-title">${esc(item.title)}</span>
            ${overdueBadge}
            ${categoryTag}
            ${authorChip}
            ${visBadge}
            ${notifyBadge}
            <span class="todo-card-status">${item.done ? '已完成' : item.expired ? '已逾期' : '待办'}</span>
            ${mine ? `<button class="todo-card-action" data-action="edit" data-id="${item.id}" title="编辑">${ICONS.pencil}</button>
            <button class="todo-card-action" data-action="delete" data-id="${item.id}" title="删除">${ICONS.trash}</button>` : ''}
          </div>
          ${item.content ? `<div class="todo-card-body">${renderTodoMarkdown(item.content)}</div>` : ''}
          ${dueInfo}
          ${notifyInfo}
        </div>
      </div>
    `;
  }

  // ─── 轻量 Markdown 渲染（同步内容保真）───
  // 先整体转义再生成受限标签 —— XSS 安全；支持 换行/加粗/斜体/行内代码/
  // 图片/链接/checkbox 列表/标题行。图片 URL 仅接受 /api/ 与 http(s)。
  function renderTodoMarkdown(src) {
    let t = esc(src);
    t = t.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (m, alt, url) => {
      if (!/^(https?:\/\/|\/api\/)/.test(url)) return m;
      return `<img class="todo-md-img" src="${url}" alt="${alt}" loading="lazy">`;
    });
    t = t.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (m, text, url) => {
      if (!/^(https?:\/\/|\/api\/)/.test(url)) return m;
      return `<a href="${url}" target="_blank" rel="noopener">${text}</a>`;
    });
    t = t.replace(/(^|\s)(https?:\/\/[^\s<]+)/g, '$1<a href="$2" target="_blank" rel="noopener">$2</a>');
    // checkbox 渲染为可交互元素（memos 式：点击切换并持久回写 content）
    let _ci = 0;
    t = t.replace(/^(\s*)([-*])(\s+)\[([ xX])\]/gm, (m, ind, dash, sp, mark) => {
      const checked = mark !== ' ';
      const idx = _ci++;
      return `${ind}<span class="todo-md-check${checked ? ' checked' : ''}" data-action="mdcheck" data-ci="${idx}" role="checkbox" aria-checked="${checked}" tabindex="0"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" width="10" height="10"><polyline points="20 6 9 17 4 12"/></svg></span>${sp}`;
    });
    t = t.replace(/^#{1,4}\s+(.+)$/gm, '<strong>$1</strong>');
    t = t.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    t = t.replace(/(^|[^*])\*([^*\s][^*]*)\*/g, '$1<em>$2</em>');
    t = t.replace(/`([^`]+)`/g, '<code>$1</code>');
    return t;
  }

  function formatDueInfo(item, overdue) {
    if (!item.due_date) return '';
    const parts = [ICONS.clock];
    parts.push(esc(item.due_date));
    if (item.due_time) parts.push(esc(item.due_time));
    return `<div class="todo-card-due ${overdue ? 'overdue' : ''}">${parts.join(' ')}</div>`;
  }

  function formatNotifyInfo(item) {
    if (!item.notify_enabled) return '';
    const times = collectNotifyTimes(item);
    if (times.length === 0) return '';
    const parts = [];
    parts.push(`提醒: ${times.join('、')}`);
    if (item.advance_minutes && item.advance_minutes.length > 0) {
      parts.push(`提前${item.advance_minutes.join('/')}分钟`);
    }
    if (item.repeat_enabled && item.repeat_interval > 0) {
      parts.push(`每${item.repeat_interval}分钟重复`);
    }
    return `<div class="todo-card-notify">${parts.join(' · ')}</div>`;
  }

  function collectNotifyTimes(item) {
    const times = [];
    if (item.notify_times && item.notify_times.length > 0) times.push(...item.notify_times);
    if (item.notify_time && !times.includes(item.notify_time)) times.push(item.notify_time);
    return times;
  }

  // ─── 快速添加（速记语法解析）───
  async function quickAdd() {
    const input = $('todo-quick-input');
    if (!input) return;
    const parsed = _parseQuickSyntax(input.value);
    if (parsed.error === 'empty') { renderEditorPreview(); return; }

    // 失败恢复：先快照原文再清空 —— 所有失败分支必须把文本还给用户
    // （曾发生过：清空后请求失败静默 return，文字没了条目也没建）
    const draft = input.value;
    input.value = ''; autoGrowEditor(input); renderEditorPreview();
    input.disabled = true;
    const restore = (msg) => {
      input.value = draft; autoGrowEditor(input); renderEditorPreview();
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show(msg, 'error');
    };
    try {
      // @日期 token → 条目落到该日期清单（不进今日数组，时间线可见）
      const targetDate = parsed.date || currentDate;
      const r = await fetch(`/api/todo/${targetDate}/item`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          title: parsed.title, content: '', priority: parsed.priority,
          category: parsed.category, visibility: 'private',
          notify_enabled: parsed.notify_enabled,
          notify_times: parsed.notify_times,
          notify_time: parsed.notify_times[0] || '',
          advance_minutes: [],
        }),
      });
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); restore('登录已过期，内容已恢复'); return; }
      const text = await r.text();
      let data; try { data = JSON.parse(text); } catch (e) { restore('服务器响应异常，内容已恢复'); return; }
      if (data.status === 'ok' && data.item) {
        if (targetDate !== currentDate) {
          // 未来/改期条目：落在目标日期清单 — 今日视图不插入（避免"明天的事今天出现"）
          if (activeTab === 'timeline') loadTimeline(true);
          else if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show(`已添加到 ${targetDate} 的清单`);
        } else if (activeTab === 'timeline') { loadTimeline(true); }
        else { items.push(data.item); updateCategories(); render(); }
      } else {
        restore((data && data.message) ? ('添加失败：' + data.message) : '添加失败，内容已恢复');
      }
    } catch (e) {
      console.error('quickAdd error:', e);
      restore('网络异常，内容已恢复');
    } finally {
      input.disabled = false;
      input.focus();
    }
  }

  // ─── 操作（日期感知：今日视图本地维护，时间线/发现按卡片日期调用 API）───
  async function toggleItem(id, date) {
    const item = findItemAnywhere(id, date);
    if (!item) return;
    const newDone = !item.done;
    const cardEl = document.querySelector(`.todo-card[data-id="${CSS.escape(String(id))}"]`);
    try {
      const r = await fetch(`/api/todo/${date}/item/${id}`, {
        method: 'PUT', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ done: newDone }),
      });
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      if (!r.ok) { console.error('toggleItem 服务器拒绝:', r.status); return; }
      // 后端对「事项不存在」回 200+status:error（多端删除/会话过期等陈旧页面场景）：
      // 必须给可见反馈并刷新列表 — 静默吞掉曾让用户以为「单击无响应」
      const d = await r.json().catch(() => null);
      if (d && d.status === 'error') {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('该事项已不存在，列表已刷新', 'error');
        // 今日视图必须真正重拉（本地数组里还有陈旧条目，refreshActiveView 只重渲染）
        if (date === currentDate) loadToday(); else refreshActiveView();
        return;
      }
      item.done = newDone;
      if (newDone) { item.expired = false; item.last_notified = null; }
      // 完成动效：划线横扫 + 标题转灰（取消完成则直接刷新，无需仪式感）
      if (newDone && cardEl && date === currentDate) {
        cardEl.classList.add('todo-completing');
        setTimeout(() => { if (date === currentDate) render(); else refreshActiveView(); }, 680);
      } else if (date === currentDate) {
        render();
      } else {
        refreshActiveView();
      }
    } catch (e) { console.error('toggleItem error:', e); }
  }

  async function deleteItem(id, date) {
    const confirmed = await showConfirm('删除事项', '确定删除此事项？此操作不可恢复。');
    if (!confirmed) return;
    try {
      const r = await fetch(`/api/todo/${date}/item/${id}`, { method: 'DELETE' });
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      if (!r.ok) { console.error('deleteItem 服务器拒绝:', r.status); return; }
      // 200+error（条目已在别处删除）→ 提示 + 重拉，与 toggleItem 同款防护
      const d = await r.json().catch(() => null);
      if (d && d.status === 'error') {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('该事项已不存在，列表已刷新', 'error');
        if (date === currentDate) loadToday(); else refreshActiveView();
        return;
      }
      if (date === currentDate) {
        items = items.filter(i => i.id !== id);
        updateCategories();
        render();
      } else {
        refreshActiveView();
      }
    } catch (e) { console.error('deleteItem error:', e); }
  }

  async function toggleVisibility(id, date) {
    const item = findItemAnywhere(id, date);
    if (!item) return;
    const next = item.visibility === 'public' ? 'private' : 'public';
    try {
      const r = await fetch(`/api/todo/${date}/item/${id}`, {
        method: 'PUT', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ visibility: next }),
      });
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      if (!r.ok) { console.error('toggleVisibility 服务器拒绝:', r.status); return; }
      const d = await r.json().catch(() => null);
      if (d && d.status === 'error') {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('该事项已不存在，列表已刷新', 'error');
        if (date === currentDate) loadToday(); else refreshActiveView();
        return;
      }
      item.visibility = next;
      if (date === currentDate && activeTab === 'today') render();
      else refreshActiveView();
    } catch (e) { console.error('toggleVisibility error:', e); }
  }

  // memo checkbox 勾选回写 — 第 ci 个 `- [ ]`/`- [x]` token 切换后 PUT 持久化
  //（渲染与写回共用同一 token 顺序，保证 idx 对应）
  async function toggleMemoCheck(id, date, ci) {
    const item = findItemAnywhere(id, date);
    if (!item || typeof item.content !== 'string') return;
    let count = -1;
    let hit = false;
    const newContent = item.content.replace(/^(\s*)([-*])(\s+)\[([ xX])\]/gm, (m, ind, dash, sp, mark) => {
      count++;
      if (count !== ci) return m;
      hit = true;
      return ind + dash + sp + (mark === ' ' ? '[x]' : '[ ]');
    });
    if (!hit) return;
    try {
      const r = await fetch(`/api/todo/${date}/item/${id}`, {
        method: 'PUT', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content: newContent }),
      });
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      const d = await r.json().catch(() => null);
      if (!r.ok || (d && d.status === 'error')) {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('该事项已不存在，列表已刷新', 'error');
        if (date === currentDate) loadToday(); else refreshActiveView();
        return;
      }
      item.content = newContent;
      render();
    } catch (e) { console.error('toggleMemoCheck error:', e); }
  }

  function editItem(id, date) {
    const item = findItemAnywhere(id, date);
    if (item) showAddDialog(item, date);
  }

  // ─── 统计面板 ───
  async function loadStats() {
    try {
      const r = await fetch('/api/todo/stats');
      const data = await r.json();
      if (data.status === 'ok') renderStats(data);
    } catch(e) {}
  }

  function renderStats(data) {
    const container = $('todo-stats');
    if (!container) return;
    const today = data.today || {};
    const week = data.week || {};
    const streak = data.streak || 0;
    const rate = week.completion_rate || 0;

    container.innerHTML = `
      <div class="todo-stat-card">
        <div class="todo-stat-value">${today.done}/${today.total}</div>
        <div class="todo-stat-label">今日完成</div>
      </div>
      <div class="todo-stat-card">
        <div class="todo-stat-value">${today.overdue}</div>
        <div class="todo-stat-label">已逾期</div>
      </div>
      <div class="todo-stat-card">
        <div class="todo-stat-value">${rate}%</div>
        <div class="todo-stat-label">7日完成率</div>
      </div>
      <div class="todo-stat-card">
        <div class="todo-stat-value">${streak}天</div>
        <div class="todo-stat-label">连续完成</div>
      </div>
    `;
  }

  // ─── 跨天延续 ───
  function showCarryoverPrompt() {
    const el = $('todo-carryover');
    if (!el || !carryoverData) return;
    el.style.display = 'flex';
    const dateLabel = carryoverData.dates.length > 1
      ? `${esc(carryoverData.dates[0])} ~ ${esc(carryoverData.dates[carryoverData.dates.length - 1])}`
      : (esc(carryoverData.dates[0]) || '过去');
    el.innerHTML = `
      <div class="todo-carryover-info">
        <span class="todo-carryover-icon">${ICONS.clipboard}</span>
        <span>${carryoverData.items.length} 个未完成事项来自 ${dateLabel}</span>
      </div>
      <div class="todo-carryover-actions">
        <button class="todo-carryover-btn" id="todo-carryover-yes">延续到今天</button>
        <button class="todo-carryover-btn todo-carryover-dismiss" id="todo-carryover-no">忽略</button>
      </div>
    `;
    el.querySelector('#todo-carryover-yes').addEventListener('click', () => doCarryover());
    el.querySelector('#todo-carryover-no').addEventListener('click', () => hideCarryoverPrompt());
  }

  function hideCarryoverPrompt() {
    const el = $('todo-carryover');
    if (el) el.style.display = 'none';
  }

  async function doCarryover() {
    if (!carryoverData) return;
    try {
      const itemIds = carryoverData.items.map(i => i.id);
      const r = await fetch(`/api/todo/${currentDate}/carryover`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ source_dates: carryoverData.dates, item_ids: itemIds }),
      });
      const data = await r.json();
      if (data.status === 'ok') {
        items = data.list.items || items;
        updateCategories();
        hideCarryoverPrompt();
        carryoverData = null;
        render();
      }
    } catch(e) { console.error('carryover error:', e); }
  }

  // ─── 搜索 ───
  function toggleSearch(force) {
    const bar = $('todo-search-bar');
    const results = $('todo-search-results');
    const itemsEl = $('todo-items');
    const input = $('todo-search-input');
    if (force === false || searchMode) {
      searchMode = false;
      if (bar) bar.style.display = 'none';
      if (results) results.style.display = 'none';
      if (itemsEl) itemsEl.style.display = '';
      if (input) input.value = '';
    } else {
      searchMode = true;
      if (bar) bar.style.display = 'flex';
      if (input) input.focus();
    }
  }

  async function doSearch(q) {
    const resultsEl = $('todo-search-results');
    const itemsEl = $('todo-items');
    if (!q) {
      if (resultsEl) resultsEl.style.display = 'none';
      if (itemsEl) itemsEl.style.display = '';
      return;
    }
    if (itemsEl) itemsEl.style.display = 'none';
    if (resultsEl) resultsEl.style.display = 'block';
    try {
      const r = await fetch(`/api/todo/search?q=${encodeURIComponent(q)}`);
      const data = await r.json();
      if (data.status === 'ok') {
        renderSearchResults(data.results || []);
      }
    } catch(e) {}
  }

  function renderSearchResults(results) {
    const el = $('todo-search-results');
    if (!el) return;
    if (results.length === 0) {
      el.innerHTML = '<div class="todo-empty">未找到匹配的待办事项</div>';
      return;
    }
    el.innerHTML = `<div class="todo-search-count">${results.length} 条结果 · 点击卡片跳转到对应日期</div>` + results.map(r => {
      const item = r.item;
      const priority = item.priority ?? 1;
      const priColors = ['todo-pri-low', 'todo-pri-mid', 'todo-pri-high'];
      const overdue = isItemOverdue(item);
      const statusClass = item.done ? 'done' : item.expired ? 'expired' : overdue ? 'overdue' : 'pending';
      const statusText = item.done ? '已完成' : item.expired ? '已逾期' : overdue ? '已逾期' : '待办';
      return `
        <div class="todo-card ${statusClass} ${priColors[priority]} todo-search-card" data-date="${r.date}" data-id="${item.id}">
          <div class="todo-card-pri-bar"></div>
          <div class="todo-card-content">
            <div class="todo-card-header">
              <span class="todo-search-date">${r.date}</span>
              <span class="todo-card-title">${esc(item.title)}</span>
              ${item.category ? `<span class="todo-cat-tag">${esc(item.category)}</span>` : ''}
              <span class="todo-card-status">${statusText}</span>
            </div>
            ${item.content ? `<div class="todo-card-body">${renderTodoMarkdown(item.content)}</div>` : ''}
          </div>
        </div>
      `;
    }).join('');

    // Make search results clickable — navigate to that date
    el.querySelectorAll('.todo-search-card').forEach(card => {
      card.style.cursor = 'pointer';
      card.addEventListener('click', () => {
        const date = card.dataset.date;
        if (date) {
          toggleSearch(false);
          currentDate = date;
          loadList(date);
        }
      });
    });
  }

  // ─── v3: 时间线视图（memos Journal 风格卡片流）───
  async function loadTimeline(reset) {
    const listEl = $('todo-timeline');
    if (!listEl) return;
    if (reset) { timelineState.nextBefore = null; timelineState.hasMore = false; }
    try {
      const params = new URLSearchParams({ limit: '7' });
      if (timelineState.tag !== 'all') params.set('tag', timelineState.tag);
      if (timelineState.nextBefore) params.set('before', timelineState.nextBefore);
      const r = await fetch(`/api/todo/timeline?${params}`);
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      const data = await r.json();
      if (data.status !== 'ok') return;
      const groups = data.groups || [];
      timelineState.groups = reset ? groups : timelineState.groups.concat(groups);
      timelineState.hasMore = !!data.has_more;
      timelineState.nextBefore = data.next_before || null;
      timelineState.loaded = true;
      refreshCalendarCounts();
      renderTimelineTags();
      renderTimeline(listEl, timelineState, () => loadTimeline(false));
    } catch (e) { console.error('loadTimeline error:', e); }
  }

  function renderTimelineTags() {
    const bar = $('todo-timeline-tags');
    if (!bar) return;
    const cats = new Set();
    for (const g of timelineState.groups) for (const i of g.items) if (i.category) cats.add(i.category);
    const chips = ['<button class="todo-filter-chip ' + (timelineState.tag === 'all' ? 'active' : '') + '" data-tag="all">全部</button>'];
    for (const c of Array.from(cats).sort()) {
      chips.push(`<button class="todo-filter-chip ${timelineState.tag === c ? 'active' : ''}" data-tag="${esc(c)}">${esc(c)}</button>`);
    }
    bar.innerHTML = chips.join('');
    bar.style.display = cats.size ? 'flex' : 'none';
    bar.querySelectorAll('.todo-filter-chip').forEach(btn => {
      btn.addEventListener('click', () => { timelineState.tag = btn.dataset.tag; loadTimeline(true); });
    });
  }

  function renderTimeline(container, state, onLoadMore) {
    if (state.groups.length === 0) {
      container.innerHTML = '<div class="todo-empty">暂无内容<br><span class="todo-empty-hint">在上方输入框随手记一条吧</span></div>';
    } else {
      const parts = [];
      for (const g of state.groups) {
        parts.push(`<div class="todo-timeline-date-head">${esc(formatDateDisplay(g.date))}</div>`);
        for (const item of g.items) parts.push(renderItemCard(item, g.date));
      }
      container.innerHTML = parts.join('');
      bindCardActions(container);
    }
    const moreBtn = container.id === 'todo-timeline' ? $('todo-timeline-more') : $('todo-explore-more');
    if (moreBtn) {
      moreBtn.style.display = state.hasMore ? 'block' : 'none';
      moreBtn.onclick = onLoadMore;
    }
  }

  // ─── v3.1: 日历面板（memos 风：按日计数 + 点击筛选）───
  function calKey(y, m) { return y + '-' + String(m).padStart(2, '0'); }

  async function refreshCalendarCounts() {
    if (!calState.year) {
      const now = new Date();
      calState.year = now.getFullYear();
      calState.month = now.getMonth() + 1;
    }
    renderCalendar(); // 先画网格（fetch 失败也不空白）
    try {
      const r = await fetch('/api/todo/list');
      if (r.status === 401) return;
      const data = await r.json();
      const counts = {};
      for (const d of (data.dates || [])) counts[d.date] = d.items || 0;
      calState.counts = counts;
    } catch (e) {}
    renderCalendar();
    applyCalendarVisibility();
  }

  function applyCalendarVisibility() {
    const panel = $('todo-calendar');
    if (panel) panel.style.display = (activeTab === 'timeline' && calState.visible) ? '' : 'none';
  }

  function renderCalendar() {
    const title = $('todo-cal-title');
    const grid = $('todo-cal-grid');
    if (!title || !grid) return;
    title.textContent = calState.year + ' 年 ' + calState.month + ' 月';
    // 修复：旧实现把表头 .map().join('') 拼成字符串后再 .push → 运行时 TypeError，
    // 标题已更新而日网格永不渲染（"只显示月不显示日"）。现抽纯函数返回数组。
    grid.innerHTML = buildCalCells(calState.year, calState.month, calState.counts,
      formatDate(new Date()), calState.dateFilter).join('');
    grid.querySelectorAll('.todo-cal-day[data-date]').forEach(btn => {
      btn.addEventListener('click', () => filterTimelineByDate(btn.dataset.date));
    });
    const clearBtn = $('todo-cal-clear');
    if (clearBtn) clearBtn.style.display = calState.dateFilter ? '' : 'none';
  }

  // 纯函数：生成日历网格 HTML 单元数组（7 表头 + startWeek 空位 + 当月天数）。
  // 导出供 scripts/todo-cal-smoke.js 冒烟测试（v3.1 日历空白事故的回归防线）。
  function buildCalCells(year, month, counts, todayStr, dateFilter) {
    const first = new Date(year, month - 1, 1);
    const startWeek = first.getDay(); // 0=周日
    const daysInMonth = new Date(year, month, 0).getDate();
    const cells = ['日', '一', '二', '三', '四', '五', '六'].map(w =>
      '<span class="todo-cal-wd">' + w + '</span>');
    for (let i = 0; i < startWeek; i++) cells.push('<span class="todo-cal-day empty"></span>');
    for (let d = 1; d <= daysInMonth; d++) {
      const dateStr = calKey(year, month) + '-' + String(d).padStart(2, '0');
      const count = counts[dateStr] || 0;
      const cls = ['todo-cal-day'];
      if (dateStr === todayStr) cls.push('today');
      if (dateFilter === dateStr) cls.push('selected');
      if (count > 0) cls.push('has-items');
      cells.push('<button class="' + cls.join(' ') + '" data-date="' + dateStr + '" title="' + count + ' 条">' +
        '<span class="todo-cal-num">' + d + '</span>' +
        (count > 0 ? '<span class="todo-cal-count">' + count + '</span>' : '') + '</button>');
    }
    return cells;
  }

  async function filterTimelineByDate(dateStr) {
    calState.dateFilter = (calState.dateFilter === dateStr) ? null : dateStr;
    renderCalendar();
    const listEl = $('todo-timeline');
    if (!listEl) return;
    if (!calState.dateFilter) { loadTimeline(true); return; }
    try {
      const r = await fetch('/api/todo/' + calState.dateFilter);
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      const data = await r.json();
      const items = (data.status === 'ok' && data.list) ? data.list.items : [];
      timelineState.groups = items.length ? [{ date: calState.dateFilter, items }] : [];
      timelineState.hasMore = false;
      timelineState.nextBefore = null;
      renderTimeline(listEl, timelineState, () => {});
    } catch (e) { console.error('filterTimelineByDate error:', e); }
  }

  function calShiftMonth(delta) {
    let m = calState.month + delta, y = calState.year;
    if (m < 1) { m = 12; y--; }
    if (m > 12) { m = 1; y++; }
    calState.year = y; calState.month = m;
    renderCalendar();
    refreshCalendarCounts();
  }

  // ─── v3: 发现视图（solo: 历史回溯 + 标签云；social: 他人公开流）───
  async function loadExplore(reset) {
    const listEl = $('todo-explore-list');
    if (!listEl) return;
    if (reset) { exploreState.nextBefore = null; exploreState.hasMore = false; }
    try {
      const params = new URLSearchParams({ limit: '14' });
      if (exploreState.tag !== 'all') params.set('tag', exploreState.tag);
      if (exploreState.nextBefore) params.set('before', exploreState.nextBefore);
      const r = await fetch(`/api/todo/explore?${params}`);
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      const data = await r.json();
      if (data.status !== 'ok') return;
      exploreState.mode = data.mode || 'solo';
      exploreState.tags = data.tags || [];
      exploreState.stats = data.stats || null;
      const groups = data.groups || [];
      exploreState.groups = reset ? groups : exploreState.groups.concat(groups);
      exploreState.hasMore = !!data.has_more;
      exploreState.nextBefore = data.next_before || null;
      exploreState.loaded = true;
      renderExploreHead();
      renderExploreTags();
      renderTimeline(listEl, exploreState, () => loadExplore(false));
    } catch (e) { console.error('loadExplore error:', e); }
  }

  function renderExploreHead() {
    const head = $('todo-explore-head');
    if (!head) return;
    if (exploreState.mode === 'social') {
      head.innerHTML = '<div class="todo-explore-note">' + ICONS.globe + ' 来自其他用户的公开事项 — 在编辑事项中可将条目设为公开分享</div>';
    } else {
      const s = exploreState.stats || {};
      head.innerHTML = `
        <div class="todo-stat-card"><div class="todo-stat-value">${s.total ?? 0}</div><div class="todo-stat-label">全部事项</div></div>
        <div class="todo-stat-card"><div class="todo-stat-value">${s.done ?? 0}</div><div class="todo-stat-label">已完成</div></div>
        <div class="todo-stat-card"><div class="todo-stat-value">${s.days ?? 0}</div><div class="todo-stat-label">记录天数</div></div>
        <div class="todo-stat-card"><div class="todo-stat-value">${exploreState.tags.length}</div><div class="todo-stat-label">分类数</div></div>`;
    }
  }

  function renderExploreTags() {
    const bar = $('todo-explore-tags');
    if (!bar) return;
    const chips = ['<button class="todo-filter-chip ' + (exploreState.tag === 'all' ? 'active' : '') + '" data-tag="all">全部</button>'];
    for (const t of exploreState.tags) {
      chips.push(`<button class="todo-filter-chip ${exploreState.tag === t.name ? 'active' : ''}" data-tag="${esc(t.name)}">${esc(t.name)} · ${t.count}</button>`);
    }
    bar.innerHTML = chips.join('');
    bar.style.display = exploreState.tags.length ? 'flex' : 'none';
    bar.querySelectorAll('.todo-filter-chip').forEach(btn => {
      btn.addEventListener('click', () => { exploreState.tag = btn.dataset.tag; loadExplore(true); });
    });
  }

  // ─── 通知引擎 ───
  let notifyCheckInProgress = false;

  async function checkAndFireNotifications() {
    if (notifyCheckInProgress) return;
    notifyCheckInProgress = true;
    try {
      const now = new Date();
      const nowTs = Math.floor(now.getTime() / 1000);
      const todayStr = formatDate(now);
      if (currentDate === todayStr) {
        for (const item of items) {
          if (!item.notify_enabled || item.done) continue;
          processItemNotifications(item, now, nowTs, todayStr);
        }
      } else {
        await checkTodayItemsRemotely(todayStr, now, nowTs);
      }
    } finally {
      notifyCheckInProgress = false;
    }
  }

  async function checkTodayItemsRemotely(todayStr, now, nowTs) {
    try {
      const r = await fetch(`/api/todo/${todayStr}`);
      const text = await r.text();
      let data; try { data = JSON.parse(text); } catch(e) { return; }
      if (data.status === 'ok' && data.list) {
        for (const item of data.list.items || []) {
          if (!item.notify_enabled || item.done) continue;
          processItemNotifications(item, now, nowTs, todayStr);
        }
      }
    } catch(e) {}
  }

  function processItemNotifications(item, now, nowTs, dateStr) {
    const times = collectNotifyTimes(item);
    const advanceOffsets = item.advance_minutes || [];
    const lastNotified = item.last_notified || 0;
    const [yr, mo, dy] = dateStr.split('-').map(Number);
    const scheduledTimes = [];

    // Build scheduled notification times from notify_times
    for (const timeStr of times) {
      const [h, m] = timeStr.split(':').map(Number);
      if (isNaN(h) || isNaN(m)) continue;
      const mainTime = new Date(yr, mo - 1, dy, h, m, 0, 0);
      scheduledTimes.push({ time: mainTime, label: timeStr });
      for (const advMin of advanceOffsets) {
        const advTime = new Date(yr, mo - 1, dy, h, m, 0, 0);
        advTime.setMinutes(advTime.getMinutes() - advMin);
        scheduledTimes.push({ time: advTime, label: `${timeStr} 提前${advMin}分钟` });
      }
    }

    // Also add due_date/due_time as a notification trigger if no explicit notify_times cover it
    if (item.due_date && item.due_time) {
      const [dh, dm] = item.due_time.split(':').map(Number);
      if (!isNaN(dh) && !isNaN(dm)) {
        const [dYr, dMo, dDy] = item.due_date.split('-').map(Number);
        const dueTime = new Date(dYr, dMo - 1, dDy, dh, dm, 0, 0);
        // Only add if not already in the list
        const exists = scheduledTimes.some(st => Math.abs(st.time.getTime() - dueTime.getTime()) < 60000);
        if (!exists) {
          scheduledTimes.push({ time: dueTime, label: `截止时间 ${item.due_time}` });
        }
      }
    }

    if (scheduledTimes.length === 0) return;

    scheduledTimes.sort((a, b) => a.time - b.time);
    let triggered = false;
    let triggerLabel = '';
    for (const st of scheduledTimes) {
      const stTs = Math.floor(st.time.getTime() / 1000);
      if (stTs <= nowTs && stTs > lastNotified) {
        triggered = true;
        // 同一窗口多个触发点都过期时取最晚者：正点提醒取代其提前提醒（语义已被覆盖，不双发）
        triggerLabel = st.label;
      }
    }
    if (triggered) {
      sendNotification(item, triggerLabel);
      playNotifySound();
      updateLastNotified(item.id, nowTs);
      return;
    }
    if (item.repeat_enabled && item.repeat_interval > 0) {
      const lastScheduled = scheduledTimes[scheduledTimes.length - 1];
      const lastScheduledTs = Math.floor(lastScheduled.time.getTime() / 1000);
      if (lastScheduledTs < nowTs) {
        // P2-3 修复：重复提醒基准 = max(最后计划时间, 上次通知时刻)。
        // 旧实现 lastNotified===0 时打开页面立即补发；且只从上次通知起算，
        // 上次只发了提前提醒时基准错位。现在：首次重复 = 最后计划时间 + interval。
        const repeatBase = Math.max(lastNotified, lastScheduledTs);
        if (nowTs - repeatBase >= item.repeat_interval * 60) {
          sendNotification(item, `重复提醒 (每${item.repeat_interval}分钟)`);
          playNotifySound();
          updateLastNotified(item.id, nowTs);
        }
      }
    }
  }

  async function updateLastNotified(itemId, ts) {
    const item = items.find(i => i.id === itemId);
    if (item) item.last_notified = ts === null ? null : ts;
    const todayStr = formatDate(new Date());
    try {
      await fetch(`/api/todo/${todayStr}/item/${itemId}`, {
        method: 'PUT', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ last_notified: ts }),
      });
    } catch(e) {}
  }

  // ─── 通知声音 (Web Audio API) ───
  let audioCtx = null;
  function playNotifySound() {
    try {
      if (!audioCtx) audioCtx = new (window.AudioContext || window.webkitAudioContext)();
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.connect(gain);
      gain.connect(audioCtx.destination);
      osc.frequency.value = 880;
      osc.type = 'sine';
      gain.gain.setValueAtTime(0.15, audioCtx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.01, audioCtx.currentTime + 0.3);
      osc.start(audioCtx.currentTime);
      osc.stop(audioCtx.currentTime + 0.3);
    } catch(e) {}
  }

  async function sendNotification(item, label) {
    const body = label ? `${item.title} (${label})` : item.title;
    // 通知送达后的窗口拉起由 notifyWindows 统一负责（见 router.js）
    if (typeof window.notifyWindows === 'function') {
      await window.notifyWindows('SKYLAND 待办提醒', body);
    }
  }

  // 发送测试通知并把实际走的通道如实回报给用户
  async function sendTestNotification() {
    if (typeof window.notifyWindows !== 'function') {
      showAlert('通知测试', '通知组件未就绪（router.js 未加载）');
      return;
    }
    const via = await window.notifyWindows('SKYLAND 通知测试', '通知链路已就绪 — 此提醒经由 Windows 系统通知中心发出。');
    const viaText = {
      backend: '本机后端 · WinRT 直发',
      tauri: '桌面壳原生命令',
      web: '浏览器通知',
      inapp: '应用内提醒（系统通知不可用）',
    }[via] || String(via);
    const ok = via === 'backend' || via === 'tauri' || via === 'web';
    await showAlert(
      ok ? '通知测试：已发送' : '通知测试：系统通知未送达',
      ok
        ? `已通过【${viaText}】发送，请查看屏幕右下角与通知中心。`
        : `当前仅能显示【${viaText}】。请检查：Windows 设置 → 系统 → 通知 是否开启、勿扰模式是否关闭。`
    );
  }

  // ─── v3.2: 全量时间选择器 ───
  // 原生 <input type="time"> 钟表弹层难点击、时间范围看不全 → 自绘行内面板：
  // 时 00-23 全量 6×4 网格 + 分 00-55 步5 全量 6×2 网格一键选；
  // "现在"时刻实时显示并在网格中高亮，方便在附近时间范围直接点选；
  // 预设按当前时间递增（30分钟/1/2/3/5/10小时/明天此时），跨天自动联动所属日期。
  const TP_PRESETS = [
    { label: '现在', minutes: 0 },
    { label: '30分钟后', minutes: 30 },
    { label: '1小时后', minutes: 60 },
    { label: '2小时后', minutes: 120 },
    { label: '3小时后', minutes: 180 },
    { label: '5小时后', minutes: 300 },
    { label: '10小时后', minutes: 600 },
    { label: '明天此时', minutes: 1440 },
  ];

  function tpHHMM(h, m) { return String(h).padStart(2, '0') + ':' + String(m).padStart(2, '0'); }

  // 相对日期的中文标签（今天/明天/后天，更远回退为日期本身）
  function tpDateLabel(dateStr) {
    const today = formatDate(new Date());
    if (dateStr === today) return '今天';
    if (dateStr === formatDate(new Date(Date.now() + 86400000))) return '明天';
    if (dateStr === formatDate(new Date(Date.now() + 2 * 86400000))) return '后天';
    return dateStr;
  }

  // 建议时刻：当前时间的下一个半点（深夜跨天时收敛到当日 23:59）
  function suggestTime() {
    const n = new Date();
    let h = n.getHours(), m = n.getMinutes();
    m = m < 30 ? 30 : 0;
    if (m === 0) h += 1;
    if (h > 23) return '23:59';
    return tpHHMM(h, m);
  }

  function closeTimePickers() {
    document.querySelectorAll('.todo-tp').forEach(p => { if (p._tpClose) p._tpClose(); });
  }

  // 打开时间选择器。anchor: 触发按钮（面板插在其后）；opts:
  //   value 初始 'HH:MM'；getDate/setDate 日期联动（预设跨天时改写所属日期）；
  //   onConfirm(time)；onClear() 可选（无则不显示清空钮）
  function openTimePicker(anchor, opts) {
    closeTimePickers();
    const getD = opts.getDate || (() => formatDate(new Date()));
    const setD = opts.setDate || (() => {});

    let h, m;
    if (opts.value && /^\d{2}:\d{2}$/.test(opts.value)) {
      [h, m] = opts.value.split(':').map(Number);
    } else {
      const now = new Date();
      const d0 = new Date(now.getTime() + 5 * 60000); // 默认 = 5 分钟后收整到 5 分钟步
      if (d0.getDate() !== now.getDate()) { h = 23; m = 55; }
      else { h = d0.getHours(); m = Math.floor(d0.getMinutes() / 5) * 5; }
    }
    const st = { h, m, date: getD() || formatDate(new Date()) };

    const panel = document.createElement('div');
    panel.className = 'todo-tp';
    panel.innerHTML = `
      <div class="todo-tp-head">
        <span class="todo-tp-now">现在 <b class="todo-tp-nowtime">--:--</b></span>
        <span class="todo-tp-sel">已选 <b class="todo-tp-seltime">--:--</b><span class="todo-tp-seldate"></span></span>
      </div>
      <div class="todo-tp-grid todo-tp-hours"></div>
      <div class="todo-tp-grid todo-tp-mins"></div>
      <div class="todo-tp-steppers">
        <button type="button" class="todo-tp-step" data-step="-5">−5′</button>
        <button type="button" class="todo-tp-step" data-step="-1">−1′</button>
        <button type="button" class="todo-tp-step" data-step="1">+1′</button>
        <button type="button" class="todo-tp-step" data-step="5">+5′</button>
      </div>
      <div class="todo-tp-presets">
        ${TP_PRESETS.map((p, i) => `<button type="button" class="todo-tp-preset" data-pi="${i}">${p.label}</button>`).join('')}
      </div>
      <div class="todo-tp-actions">
        ${opts.onClear ? '<button type="button" class="todo-tp-btn todo-tp-clear">清空</button>' : ''}
        <button type="button" class="todo-tp-btn todo-tp-cancel">取消</button>
        <button type="button" class="todo-tp-btn todo-tp-ok">确定</button>
      </div>`;
    // 面板插入：提醒时间行是 flex 行 → 面板插在整行之后（插进行内会被 flex 挤压）；
    // 其他宿主（截止时间字段）→ 插在按钮之后。
    const rowHost = anchor.closest('.todo-time-row');
    if (rowHost) rowHost.after(panel);
    else anchor.after(panel);

    const elNow = panel.querySelector('.todo-tp-nowtime');
    const elSel = panel.querySelector('.todo-tp-seltime');
    const elSelDate = panel.querySelector('.todo-tp-seldate');
    const elHours = panel.querySelector('.todo-tp-hours');
    const elMins = panel.querySelector('.todo-tp-mins');

    function renderSel() {
      elSel.textContent = tpHHMM(st.h, st.m);
      elSelDate.textContent = ' · ' + tpDateLabel(st.date);
    }

    function renderGrids() {
      const n = new Date();
      elNow.textContent = tpHHMM(n.getHours(), n.getMinutes());
      let html = '';
      for (let i = 0; i < 24; i++) {
        html += `<button type="button" class="todo-tp-cell${i === st.h ? ' active' : ''}${i === n.getHours() ? ' now' : ''}" data-h="${i}">${String(i).padStart(2, '0')}</button>`;
      }
      elHours.innerHTML = html;
      html = '';
      const nowM5 = Math.floor(n.getMinutes() / 5) * 5;
      for (let i = 0; i < 60; i += 5) {
        html += `<button type="button" class="todo-tp-cell${i === st.m ? ' active' : ''}${i === nowM5 ? ' now' : ''}" data-m="${i}">${String(i).padStart(2, '0')}</button>`;
      }
      elMins.innerHTML = html;
      elHours.querySelectorAll('.todo-tp-cell').forEach(b =>
        b.addEventListener('click', () => { st.h = +b.dataset.h; renderGrids(); renderSel(); }));
      elMins.querySelectorAll('.todo-tp-cell').forEach(b =>
        b.addEventListener('click', () => { st.m = +b.dataset.m; renderGrids(); renderSel(); }));
    }

    // 预设：目标 = 当前时刻 + 偏移；跨天自动改写所属日期（用户当前时间递增的值）
    function applyPreset(mins) {
      const target = new Date(Date.now() + mins * 60000);
      st.h = target.getHours(); st.m = target.getMinutes();
      st.date = formatDate(target);
      setD(st.date);
      renderGrids(); renderSel();
    }

    // 微调：±1/±5 分钟，跨 0 点时日期跟随
    function stepMinutes(delta) {
      const d = new Date(st.date + 'T' + tpHHMM(st.h, st.m) + ':00');
      d.setMinutes(d.getMinutes() + delta);
      st.date = formatDate(d);
      st.h = d.getHours(); st.m = d.getMinutes();
      setD(st.date);
      renderGrids(); renderSel();
    }

    const clockTimer = setInterval(renderGrids, 30000);
    function close() {
      clearInterval(clockTimer);
      document.removeEventListener('mousedown', onDocDown, true);
      document.removeEventListener('keydown', onKey, true);
      panel.remove();
    }
    panel._tpClose = close;
    function onDocDown(e) {
      if (!panel.contains(e.target) && !anchor.contains(e.target)) close();
    }
    function onKey(e) {
      if (e.key === 'Escape') { e.stopPropagation(); close(); }
    }
    document.addEventListener('mousedown', onDocDown, true);
    document.addEventListener('keydown', onKey, true);

    panel.querySelectorAll('.todo-tp-step').forEach(b =>
      b.addEventListener('click', () => stepMinutes(parseInt(b.dataset.step, 10))));
    panel.querySelectorAll('.todo-tp-preset').forEach(b =>
      b.addEventListener('click', () => applyPreset(TP_PRESETS[+b.dataset.pi].minutes)));
    const clearBtn = panel.querySelector('.todo-tp-clear');
    if (clearBtn) clearBtn.addEventListener('click', () => { const f = opts.onClear; close(); if (f) f(); });
    panel.querySelector('.todo-tp-cancel').addEventListener('click', close);
    panel.querySelector('.todo-tp-ok').addEventListener('click', () => {
      const t = tpHHMM(st.h, st.m);
      close();
      if (opts.onConfirm) opts.onConfirm(t, st.date);
    });

    renderGrids();
    renderSel();
  }

  // ─── 弹窗 ───
  function showConfirm(title, message) {
    return new Promise(resolve => {
      const overlay = document.createElement('div');
      overlay.className = 'kb-modal-overlay';
      const modal = document.createElement('div');
      modal.className = 'kb-modal';
      modal.innerHTML = `
        <div class="kb-modal-header"><span class="kb-modal-title">${esc(title)}</span></div>
        <div class="kb-modal-body"><p class="kb-modal-message">${esc(message)}</p></div>
        <div class="kb-modal-footer">
          <button class="kb-modal-btn kb-modal-btn-cancel" id="todo-confirm-cancel">取消</button>
          <button class="kb-modal-btn kb-modal-btn-ok" id="todo-confirm-ok">确定</button>
        </div>`;
      overlay.appendChild(modal);
      overlay.addEventListener('click', e => { if (e.target === overlay) { overlay.remove(); resolve(false); } });
      modal.querySelector('#todo-confirm-cancel').onclick = () => { overlay.remove(); resolve(false); };
      modal.querySelector('#todo-confirm-ok').onclick = () => { overlay.remove(); resolve(true); };
      document.body.appendChild(overlay);
    });
  }

  function showAlert(title, message) {
    return new Promise(resolve => {
      const overlay = document.createElement('div');
      overlay.className = 'kb-modal-overlay';
      const modal = document.createElement('div');
      modal.className = 'kb-modal';
      modal.innerHTML = `
        <div class="kb-modal-header"><span class="kb-modal-title">${esc(title)}</span></div>
        <div class="kb-modal-body"><p class="kb-modal-message">${esc(message)}</p></div>
        <div class="kb-modal-footer"><button class="kb-modal-btn kb-modal-btn-ok" id="todo-alert-ok">知道了</button></div>`;
      overlay.appendChild(modal);
      overlay.addEventListener('click', e => { if (e.target === overlay) { overlay.remove(); resolve(); } });
      modal.querySelector('#todo-alert-ok').onclick = () => { overlay.remove(); resolve(); };
      document.body.appendChild(overlay);
    });
  }

  function showAddDialog(item, date) {
    const existing = item || null;
    const targetDate = date || currentDate;
    let notifyTimes = [];
    if (existing) {
      notifyTimes = (existing.notify_times && existing.notify_times.length > 0)
        ? [...existing.notify_times] : (existing.notify_time ? [existing.notify_time] : []);
    }
    if (notifyTimes.length === 0) notifyTimes = [suggestTime()];
    const advanceMinutes = existing ? (existing.advance_minutes || []) : [];
    const repeatEnabled = existing ? (existing.repeat_enabled || false) : false;
    const repeatInterval = existing ? (existing.repeat_interval || 10) : 10;
    const notifyEnabled = existing ? (existing.notify_enabled || false) : false;
    const priority = existing ? (existing.priority ?? 1) : 1;
    const category = existing ? (existing.category || '') : '';
    const dueDate = existing ? (existing.due_date || '') : '';
    const dueTime = existing ? (existing.due_time || '') : '';
    const vis = existing ? (existing.visibility || 'private') : 'private';
    let selectedVis = vis;

    // Suggest categories from existing items
    const categorySuggestions = allCategories.filter(c => c !== category).slice(0, 10);

    const overlay = document.createElement('div');
    overlay.className = 'kb-modal-overlay';
    const modal = document.createElement('div');
    modal.className = 'kb-modal todo-modal-wide';
    modal.innerHTML = `
      <div class="kb-modal-header"><span class="kb-modal-title">${existing ? '编辑事项' : '新建事项'}</span></div>
      <div class="kb-modal-body">
        <input class="kb-modal-input" id="todo-input-title" type="text" placeholder="事项标题" value="${existing ? esc(existing.title) : ''}" style="margin-bottom:8px">
        <textarea class="kb-modal-input" id="todo-input-content" placeholder="详细描述（可选）" style="width:100%;min-height:60px;resize:vertical;margin-bottom:12px">${existing ? esc(existing.content) : ''}</textarea>

        <div class="todo-dialog-row">
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">优先级</label>
            <div class="todo-priority-options">
              ${[0, 1, 2].map(p => `
                <label class="todo-priority-chip ${priority === p ? 'active' : ''}" data-pri="${p}">
                  <span class="todo-pri-dot ${['todo-pri-low', 'todo-pri-mid', 'todo-pri-high'][p]}"></span>
                  ${['低', '中', '高'][p]}
                </label>
              `).join('')}
            </div>
          </div>
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">分类</label>
            <input class="kb-modal-input todo-category-input" id="todo-input-category" type="text" list="todo-category-list" value="${esc(category)}" placeholder="如：工作、个人、学习">
            <datalist id="todo-category-list">
              ${categorySuggestions.map(c => `<option value="${esc(c)}">`).join('')}
            </datalist>
          </div>
          ${serverMode ? `
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">可见性</label>
            <div class="todo-priority-options">
              <label class="todo-vis-chip ${selectedVis !== 'public' ? 'active' : ''}" data-vis="private">私有</label>
              <label class="todo-vis-chip ${selectedVis === 'public' ? 'active' : ''}" data-vis="public">公开</label>
            </div>
          </div>` : ''}
        </div>

        <div class="todo-dialog-row">
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">所属日期</label>
            <div class="todo-date-quick">
              <button type="button" class="todo-qdate-chip" data-qd="0">今天</button>
              <button type="button" class="todo-qdate-chip" data-qd="1">明天</button>
              <button type="button" class="todo-qdate-chip" data-qd="2">后天</button>
              <input class="kb-modal-input todo-date-input" id="todo-input-date" type="date" value="${targetDate}">
            </div>
          </div>
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">截止日期</label>
            <div class="todo-date-quick">
              <button type="button" class="todo-qdate-chip" data-qd="0">今天</button>
              <button type="button" class="todo-qdate-chip" data-qd="1">明天</button>
              <button type="button" class="todo-qdate-chip todo-qdate-clear" data-qd="-1">清除</button>
              <input class="kb-modal-input todo-date-input" id="todo-input-due-date" type="date" value="${dueDate}">
            </div>
          </div>
        </div>

        <div class="todo-dialog-row">
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">截止时间</label>
            <button type="button" class="kb-modal-input todo-time-btn todo-due-time-btn" id="todo-input-due-time" data-time="${dueTime}">${ICONS.clock} <span class="todo-time-btn-text">${dueTime || '未设置'}</span></button>
          </div>
        </div>

        <div class="todo-notify-section">
          <label class="todo-notify-toggle">
            <input type="checkbox" id="todo-input-notify" ${notifyEnabled ? 'checked' : ''}>
            <span>${ICONS.bell} 开启通知提醒</span>
          </label>
          <div class="todo-notify-config" id="todo-notify-config" style="${notifyEnabled ? '' : 'display:none'}">
            <div class="todo-notify-times" id="todo-notify-times">
              ${notifyTimes.map((t, i) => renderTimeRow(t, i)).join('')}
            </div>
            <button class="todo-add-time-btn" id="todo-add-time-btn">+ 添加提醒时间</button>
            <div class="todo-notify-test-row">
              <button class="todo-add-time-btn" id="todo-notify-test2" type="button">测试系统通知</button>
              <span class="todo-notify-test-hint">保存前先确认提醒能弹出</span>
            </div>
            <div class="todo-advance-section">
              <label class="todo-advance-label">提前提醒：</label>
              <div class="todo-advance-options">
                ${[5, 10, 30].map(m => `
                  <label class="todo-advance-chip">
                    <input type="checkbox" class="todo-advance-cb" value="${m}" ${advanceMinutes.includes(m) ? 'checked' : ''}>
                    <span>${m}分钟</span>
                  </label>`).join('')}
              </div>
            </div>
            <div class="todo-repeat-section">
              <label class="todo-repeat-toggle">
                <input type="checkbox" id="todo-input-repeat" ${repeatEnabled ? 'checked' : ''}>
                <span>到时间后重复提醒</span>
              </label>
              <div class="todo-repeat-interval" id="todo-repeat-interval" style="${repeatEnabled ? '' : 'display:none'}">
                <span>每隔</span>
                <input type="number" id="todo-input-repeat-interval" value="${repeatInterval}" min="1" max="1440" class="kb-modal-input todo-repeat-input">
                <span>分钟提醒一次，直到勾选完成</span>
              </div>
            </div>
          </div>
        </div>
      </div>
      <div class="kb-modal-footer">
        <button class="kb-modal-btn kb-modal-btn-cancel" id="todo-modal-cancel">取消</button>
        <button class="kb-modal-btn kb-modal-btn-ok" id="todo-modal-save">保存</button>
      </div>`;
    overlay.appendChild(modal);
    overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
    modal.querySelector('#todo-modal-cancel').onclick = () => overlay.remove();

    // Priority selection
    let selectedPriority = priority;
    modal.querySelectorAll('.todo-priority-chip').forEach(chip => {
      chip.addEventListener('click', () => {
        modal.querySelectorAll('.todo-priority-chip').forEach(c => c.classList.remove('active'));
        chip.classList.add('active');
        selectedPriority = parseInt(chip.dataset.pri);
      });
    });

    // Visibility selection (server mode only)
    modal.querySelectorAll('.todo-vis-chip').forEach(chip => {
      chip.addEventListener('click', () => {
        modal.querySelectorAll('.todo-vis-chip').forEach(c => c.classList.remove('active'));
        chip.classList.add('active');
        selectedVis = chip.dataset.vis;
      });
    });

    // Toggle notification config
    const notifyCb = modal.querySelector('#todo-input-notify');
    notifyCb.addEventListener('change', () => {
      modal.querySelector('#todo-notify-config').style.display = notifyCb.checked ? '' : 'none';
    });

    // 添加提醒时间行（默认 = 当前时间的下一个半点）
    modal.querySelector('#todo-add-time-btn').addEventListener('click', () => {
      const container = modal.querySelector('#todo-notify-times');
      const idx = container.children.length;
      const div = document.createElement('div');
      div.innerHTML = renderTimeRow(suggestTime(), idx);
      container.appendChild(div.firstElementChild);
    });

    // 提醒时间行：删除按钮 + 点击时间打开全量选择器（事件委托，覆盖动态新增行）。
    // 预设跨天时自动改写"所属日期"，提醒落在正确的日期清单上。
    modal.querySelector('#todo-notify-times').addEventListener('click', (e) => {
      const rm = e.target.closest('.todo-time-remove');
      if (rm) { rm.closest('.todo-time-row').remove(); return; }
      const btn = e.target.closest('.todo-time-btn');
      if (btn) {
        openTimePicker(btn, {
          value: btn.dataset.time,
          getDate: () => modal.querySelector('#todo-input-date').value || targetDate,
          setDate: d => { const di = modal.querySelector('#todo-input-date'); if (di) di.value = d; },
          onConfirm: t => { btn.dataset.time = t; btn.querySelector('.todo-time-btn-text').textContent = t; },
          onClear: () => btn.closest('.todo-time-row').remove(),
        });
      }
    });

    // 截止时间选择器（跨天预设联动截止日期）
    const dueTimeBtn = modal.querySelector('#todo-input-due-time');
    dueTimeBtn.addEventListener('click', () => {
      openTimePicker(dueTimeBtn, {
        value: dueTimeBtn.dataset.time,
        getDate: () => modal.querySelector('#todo-input-due-date').value
          || modal.querySelector('#todo-input-date').value || targetDate,
        setDate: d => { const di = modal.querySelector('#todo-input-due-date'); if (di) di.value = d; },
        onConfirm: t => { dueTimeBtn.dataset.time = t; dueTimeBtn.querySelector('.todo-time-btn-text').textContent = t; },
        onClear: () => { dueTimeBtn.dataset.time = ''; dueTimeBtn.querySelector('.todo-time-btn-text').textContent = '未设置'; },
      });
    });

    // 日期快捷 chips：今天/明天/后天（截止日期组另有"清除"）
    modal.querySelectorAll('.todo-qdate-chip').forEach(chip => {
      chip.addEventListener('click', () => {
        const qd = parseInt(chip.dataset.qd, 10);
        const input = chip.closest('.todo-date-quick').querySelector('input[type="date"]');
        if (!input) return;
        input.value = qd < 0 ? '' : formatDate(new Date(Date.now() + qd * 86400000));
      });
    });

    // 弹窗内测试通知（与 header 铃铛共用 sendTestNotification）
    const testBtn2 = modal.querySelector('#todo-notify-test2');
    if (testBtn2) testBtn2.addEventListener('click', () => sendTestNotification());

    // Toggle repeat interval
    const repeatCb = modal.querySelector('#todo-input-repeat');
    repeatCb.addEventListener('change', () => {
      modal.querySelector('#todo-repeat-interval').style.display = repeatCb.checked ? '' : 'none';
    });

    // Save handler
    modal.querySelector('#todo-modal-save').onclick = async () => {
      const saveBtn = modal.querySelector('#todo-modal-save');
      if (saveBtn && saveBtn.disabled) return;
      const title = modal.querySelector('#todo-input-title').value.trim();
      const content = modal.querySelector('#todo-input-content').value.trim();
      const notify = modal.querySelector('#todo-input-notify').checked;
      const categoryVal = modal.querySelector('#todo-input-category').value.trim();
      const dueDateVal = modal.querySelector('#todo-input-due-date').value.trim();
      const dueTimeVal = (modal.querySelector('#todo-input-due-time').dataset.time || '').trim();
      // 所属日期：预设跨天/用户手动改动后可能 ≠ 打开弹窗时的日期
      const itemDate = modal.querySelector('#todo-input-date').value.trim() || targetDate;
      if (!title) return;
      if (saveBtn) { saveBtn.textContent = '保存中...'; saveBtn.disabled = true; }

      const timeBtns = modal.querySelectorAll('#todo-notify-times .todo-time-btn');
      const collectedTimes = [];
      timeBtns.forEach(btn => {
        const val = (btn.dataset.time || '').trim();
        if (val && /^\d{2}:\d{2}$/.test(val)) collectedTimes.push(val);
      });
      const advanceCbs = modal.querySelectorAll('.todo-advance-cb:checked');
      const collectedAdvance = [];
      advanceCbs.forEach(cb => collectedAdvance.push(parseInt(cb.value)));
      const repeat = modal.querySelector('#todo-input-repeat').checked;
      const repeatInterval = parseInt(modal.querySelector('#todo-input-repeat-interval').value) || 10;

      const payload = {
        title, content,
        notify_enabled: notify,
        notify_times: notify ? collectedTimes : [],
        advance_minutes: notify ? collectedAdvance : [],
        repeat_enabled: notify ? repeat : false,
        repeat_interval: notify ? repeatInterval : 0,
        priority: selectedPriority,
        // 空字符串 = 清除字段（后端 is_empty→None）；null = 保持不变
        category: categoryVal,
        due_date: dueDateVal,
        due_time: dueTimeVal,
        // 同步维护 legacy 单值字段：有则对齐第一个，无则清除（防旧数据残留触发）
        notify_time: (notify && collectedTimes.length > 0) ? collectedTimes[0] : '',
        visibility: selectedVis,
      };

      if (existing && notify) {
        const oldTimes = collectNotifyTimes(existing).slice().sort();
        const newTimes = collectedTimes.slice().sort();
        const oldAdvance = (existing.advance_minutes || []).slice().sort();
        const newAdvance = collectedAdvance.slice().sort();
        const notifyWasEnabled = existing.notify_enabled || false;
        if (!notifyWasEnabled || JSON.stringify(oldTimes) !== JSON.stringify(newTimes) || JSON.stringify(oldAdvance) !== JSON.stringify(newAdvance)) {
          payload.last_notified = 0;
        }
      }

      // 编辑时日期被改（跨天预设/手动调整）→ 走后端 move_to 迁移
      if (existing && itemDate !== targetDate) payload.move_to = itemDate;

      try {
        if (existing) {
          const r = await fetch(`/api/todo/${targetDate}/item/${existing.id}`, {
            method: 'PUT', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
          });
          if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
          const text = await r.text();
          let data; try { data = JSON.parse(text); } catch(e) { await showAlert('保存失败', '服务器返回非 JSON'); if (saveBtn) { saveBtn.textContent = '保存'; saveBtn.disabled = false; } return; }
          if (data.status === 'ok') {
            Object.assign(existing, payload);
            if (data.moved_to && data.moved_to !== targetDate) {
              // 跨日期迁移成功 — 本视图若持有该事项则剔除并刷新
              if (activeTab === 'today') {
                items = items.filter(i => i.id !== existing.id);
                updateCategories(); render();
              } else { refreshActiveView(); }
            } else if (targetDate === currentDate && activeTab === 'today') { updateCategories(); render(); }
            else { refreshActiveView(); }
            overlay.remove();
          } else { await showAlert('保存失败', data.message || '保存失败'); if (saveBtn) { saveBtn.textContent = '保存'; saveBtn.disabled = false; } }
        } else {
          const r = await fetch(`/api/todo/${itemDate}/item`, {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
          });
          if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
          const text = await r.text();
          let data; try { data = JSON.parse(text); } catch(e) { await showAlert('保存失败', '服务器返回非 JSON'); if (saveBtn) { saveBtn.textContent = '保存'; saveBtn.disabled = false; } return; }
          if (data.status === 'ok' && data.item) {
            if (itemDate === currentDate && activeTab === 'today') {
              items.push(data.item);
              updateCategories();
              render();
            } else {
              refreshActiveView();
            }
            overlay.remove();
          } else { await showAlert('保存失败', data.message || '保存失败'); if (saveBtn) { saveBtn.textContent = '保存'; saveBtn.disabled = false; } }
        }
      } catch (e) {
        await showAlert('保存失败', e.message);
        if (saveBtn) { saveBtn.textContent = '保存'; saveBtn.disabled = false; }
      }
    };

    document.body.appendChild(overlay);
    setTimeout(() => modal.querySelector('#todo-input-title')?.focus(), 50);
  }

  function renderTimeRow(time, index) {
    // v3.2：原生 <input type="time"> → 点击打开全量时间选择器（见 openTimePicker）
    return `
      <div class="todo-time-row">
        <button type="button" class="kb-modal-input todo-time-btn" data-time="${time}">${ICONS.clock} <span class="todo-time-btn-text">${time}</span></button>
        ${index > 0 ? '<button class="todo-time-remove" title="删除">✕</button>' : ''}
      </div>`;
  }

  // SVG 图标（iconfont 风格线性图标 — 避免 emoji 渲染差异）
  const ICONS = {
    bell: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg>',
    clock: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 3"/></svg>',
    clipboard: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="13" height="13"><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><rect x="8" y="2" width="8" height="4" rx="1"/></svg>',
    trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>',
    pencil: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg>',
    play: '<svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><polygon points="6 3 20 12 6 21"/></svg>',
    external: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>',
    send: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="14" height="14" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>',
    warn: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="13" height="13"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>',
    calendar: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><rect x="3" y="4" width="18" height="18" rx="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg>',
    globe: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>',
  };

  function esc(s) { return (s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;'); }

  return { init, onShow, startNotifyEngine, showTab, _parseQuickSyntax, _cal: { buildCalCells } };
})();
