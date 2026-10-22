// ════════════════════════════════════════════
// SKYLAND Web — Chat Logic
// ════════════════════════════════════════════

// ─── DOM refs (lazy-init to survive element-not-found) ───
const $ = id => document.getElementById(id);
function el(id) { return document.getElementById(id); }

// ─── State ───
let chats = [];
let currentChat = null;
let isStreaming = false;
let abortController = null;
let selectedModel = '';
let autoScroll = true;
let messageQueue = [];
let pendingImages = []; // base64 data URL 数组 — 支持多图随消息发送
let pendingAttachments = [];

// Streaming context
let streamChat = null;
let streamAiEl = null;
let streamFullResponse = '';
let streamTextEl = null;   // 当前文本段元素 — 文本与工具调用按发生顺序分块穿插
let streamSegText = '';    // 当前文本段的累积文本（跨工具调用分段）
let streamStartedAt = 0;
let streamReasoningStart = 0;
let reasonTimer = null; // 思维链实时计时器
let workTimer = null;
let effortUnsupported = false; // 模型思考能力感知 — 后端 warning 事件置位，右栏「思考」行警示
let runDurTimer = null;       // 右栏「时长」实时计时器
let sessionRound = 0;         // 当前执行轮次（round_start 事件）
let sessionToolCount = 0;     // 本次流工具调用总数（tool_start 事件）
let sessionPlan = [];         // 会话级执行计划（plan_update 事件，todo_write 工具维护）

// ─── Settings ───
let settings = {
  accentColor: '#3370FF',
  theme: 'dark',
  font: 'system',
  fontSize: 14,
  executionMode: 'auto',
  thinkingEffort: 'medium',
  roundsLimited: false,
  maxToolRounds: 5,
  systemPrompt: '',
  searchApiKey: '',
  providerApiKey: '',
  providerBaseUrl: '',
  providerPreset: '',
  providerFullUrl: false,
  webSearchEnabled: true,
  workspace: '',
};

const K_CHATS = SkylandStore.KEYS.chats;
const K_MODEL = SkylandStore.KEYS.model;
const K_SETTINGS = SkylandStore.LEGACY_SETTINGS_KEYS.settings;  // 由 settings-store.js 独占迁移（扁平→分组），此处只读

const FONT_FAMILIES = {
  'system': "system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
};

const EXEC_MODES = ['auto', 'edit', 'full', 'plan', 'readonly'];
const EXEC_LABELS = { auto: '自动', edit: '编辑', full: '完全', plan: '计划', readonly: '只读' };
const EFFORT_MODES = ['low', 'medium', 'high', 'max', 'ultra', 'off'];
const EFFORT_LABELS = { low: '低', medium: '中', high: '高', max: '最深', ultra: 'Ultra', off: '关闭' };

// ─── SVG Icons ───
const SVG = {
  brain: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M9.5 2A2.5 2.5 0 0 1 12 4.5v15a2.5 2.5 0 0 1-4.96.44 2.5 2.5 0 0 1-2.96-3.08 3 3 0 0 1-.34-5.58 2.5 2.5 0 0 1 1.32-4.24 2.5 2.5 0 0 1 1.98-3A2.5 2.5 0 0 1 9.5 2Z"/><path d="M14.5 2A2.5 2.5 0 0 0 12 4.5v15a2.5 2.5 0 0 0 4.96.44 2.5 2.5 0 0 0 2.96-3.08 3 3 0 0 0 .34-5.58 2.5 2.5 0 0 0-1.32-4.24 2.5 2.5 0 0 0-1.98-3A2.5 2.5 0 0 0 14.5 2Z"/></svg>',
  warn: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="16" height="16"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>',
  bolt: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>',
  pencil: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg>',
  clipboard: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><rect x="8" y="2" width="8" height="4" rx="1"/></svg>',
  wrench: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>',
};
const EXEC_SVGS = { auto: SVG.bolt, edit: SVG.pencil, plan: SVG.clipboard, readonly: SVG.clipboard };

marked.setOptions({ breaks: true, gfm: true });

// ════════════════════════════════════════════
// Persistence — defensive
// ════════════════════════════════════════════
const MAX_CHATS = 50;
const MAX_MSG_PER_CHAT = 200;
// ─── 工作区会话存储（对齐 Claude Code .claude）───
// 工作区激活 → 会话不再写 localStorage，改存 {工作区}/.skyland/conversations（服务端）；
// 无工作区 → 原有 localStorage 行为完全不变
function wsActive() { return typeof Workdir !== 'undefined' && !!Workdir.value(); }
let wsSyncTimer = null;
let wsPendingSync = false;
function scheduleWorkspaceSync() {
  if (wsSyncTimer) return;
  // 流式期间挂起并在流结束后补排 —— 「流结束的 saveChats 会触发完整同步」
  // 的前提不成立（结束路径执行 saveChats 时 isStreaming 仍为 true），曾致
  // 最后一条 AI 回复不落盘
  if (isStreaming) { wsPendingSync = true; return; }
  wsSyncTimer = setTimeout(() => { wsSyncTimer = null; syncWorkspaceChats(); }, 1500);
}
function syncWorkspaceChats() {
  if (!wsActive()) return;
  const wd = encodeURIComponent(Workdir.value());
  chats.slice(0, MAX_CHATS).forEach(c => {
    const body = { id: c.id, title: c.title || '对话', messages: Array.isArray(c.messages) ? c.messages.slice(-MAX_MSG_PER_CHAT) : [] };
    ['_ctx', '_round', '_toolCount', '_outcome', '_dur', '_plan'].forEach(k => { if (c[k] !== undefined) body[k] = c[k]; });
    fetch('/api/conversations/save?workdir=' + wd, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }).catch(() => {});
  });
}
async function loadWorkspaceChats() {
  if (!wsActive()) return false;
  try {
    const wd = encodeURIComponent(Workdir.value());
    const r = await fetch('/api/conversations/list?workdir=' + wd);
    const d = await r.json();
    if (!d || !Array.isArray(d.conversations)) return false;
    chats = await Promise.all(d.conversations.map(async (ci) => {
      let data = null;
      try {
        const rr = await fetch(`/api/conversations/${ci.id}?workdir=${wd}`);
        const dd = await rr.json();
        if (dd && dd.status === 'ok') data = dd.data;
      } catch (e) {}
      return {
        id: ci.id, title: ci.title || '对话',
        messages: (data && Array.isArray(data.messages)) ? data.messages.filter(m => m && m.role) : [],
        _ctx: data && data._ctx, _round: data && data._round, _toolCount: data && data._toolCount,
        _outcome: data && data._outcome, _dur: data && data._dur, _plan: data && data._plan,
      };
    }));
    if (typeof Workdir !== 'undefined') Workdir.lastLoadedAt = Date.now();
    return true;
  } catch (e) { return false; }
}
function saveChats() {
  if (wsActive()) { scheduleWorkspaceSync(); return; }  // 工作区会话只存 .skyland（防污染 localStorage）
  try {
    // Prune: limit total chats and messages per chat
    let toSave = chats;
    if (toSave.length > MAX_CHATS) toSave = toSave.slice(0, MAX_CHATS);
    const serialized = JSON.stringify(toSave.map(c => ({
      ...c,
      messages: Array.isArray(c.messages) ? c.messages.slice(-MAX_MSG_PER_CHAT) : [],
    })));
    localStorage.setItem(K_CHATS, serialized);
  } catch(e) {
    try {
      localStorage.removeItem(K_CHATS);
      const reduced = chats.slice(-5).map(c => ({
        ...c,
        messages: Array.isArray(c.messages) ? c.messages.slice(-50) : [],
      }));
      localStorage.setItem(K_CHATS, JSON.stringify(reduced));
    } catch(e2) {}
  }
}
function loadChats() {
  try {
    const raw = JSON.parse(localStorage.getItem(K_CHATS));
    if (Array.isArray(raw)) {
      chats = raw.filter(c => c && (c.id !== undefined)).map(c => ({
        id: c.id,
        title: c.title || '对话',
        messages: Array.isArray(c.messages) ? c.messages.filter(m => m && m.role) : [],
        // 右栏会话状态字段透传（_ctx/_plan 等；旧数据缺字段自然回落默认值，undefined 序列化时自动丢弃）
        _ctx: c._ctx, _round: c._round, _toolCount: c._toolCount,
        _outcome: c._outcome, _dur: c._dur, _plan: c._plan,
      }));
    }
  } catch(e) { chats = []; }
}
function saveSettings() {
  if (window.SkylandSettings) {
    window.SkylandSettings.replaceFlat(settings);
    window.SkylandSettings.save().catch(() => {});
    return;
  }
  try { localStorage.setItem(K_SETTINGS, JSON.stringify(settings)); } catch(e) {}
}
function loadSettings() {
  if (window.SkylandSettings) {
    settings = { ...settings, ...window.SkylandSettings.snapshotFlat() };
  } else {
    try {
      const s = JSON.parse(localStorage.getItem(K_SETTINGS));
      if (s && typeof s === 'object') settings = { ...settings, ...s };
    } catch(e) {}
  }
  try {
    if (settings.permissionMode && !settings.executionMode) {
      settings.executionMode = settings.permissionMode === 'readonly' ? 'readonly' : settings.permissionMode;
    }
  } catch(e) {}
  // Sanitize
  if (!settings.executionMode) settings.executionMode = 'auto';
  if (!settings.thinkingEffort) settings.thinkingEffort = 'medium';
  if (!settings.fontSize) settings.fontSize = 14;
  if (!settings.accentColor) settings.accentColor = '#00BFFF';
  if (!settings.theme) settings.theme = 'dark';
}

// ════════════════════════════════════════════
// Theme
// ════════════════════════════════════════════
function applyTheme() {
  document.documentElement.classList.toggle('dark', settings.theme === 'dark');
  const root = document.documentElement.style;
  root.setProperty('--accent', settings.accentColor);
  const c = settings.accentColor;
  const r = parseInt(c.slice(1,3),16), g = parseInt(c.slice(3,5),16), b = parseInt(c.slice(5,7),16);
  root.setProperty('--accent-dim', '#' + [r,g,b].map(x => Math.max(0, x-30).toString(16).padStart(2,'0')).join(''));
  root.setProperty('--accent-light', `rgba(${r},${g},${b},0.10)`);
  root.setProperty('--font-body', (typeof window.skyFontStack === 'function' ? window.skyFontStack(settings.font) : (FONT_FAMILIES[settings.font] || FONT_FAMILIES['system'])));
  root.setProperty('--font-size-base', settings.fontSize + 'px');
  const fs = el('font-select'); if (fs) fs.value = settings.font;
  updateCtrlButtons();
  updateRoundsUI();
}

function updateCtrlButtons() {
  const tb = el('btn-thinking');
  const eb = el('btn-execution');
  const effort = settings.thinkingEffort || 'medium';
  const mode = settings.executionMode || 'auto';
  if (tb) {
    tb.innerHTML = SVG.brain + '<span class="ctrl-label">' + (EFFORT_LABELS[effort] || effort) + '</span>';
    tb.dataset.label = '思考强度: ' + (EFFORT_LABELS[effort] || effort);
  }
  if (eb) {
    eb.innerHTML = (EXEC_SVGS[mode] || SVG.bolt) + '<span class="ctrl-label">' + (EXEC_LABELS[mode] || 'auto') + '</span>';
    eb.dataset.label = '执行模式: ' + (EXEC_LABELS[mode] || 'auto');
    eb.className = 'ctrl-btn mode-' + mode;
  }
  // 完全模式 — 全局红色警示（输入框边框着色），低饱和红避免刺眼
  document.body.classList.toggle('exec-full-mode', mode === 'full');
  // 顶栏模式 chip 同步（只读展示，与执行模式按钮同数据源）
  const hm = el('hdr-mode');
  if (hm) hm.textContent = EXEC_LABELS[mode] || 'auto';
  // Web search button — visible only if configured
  const ws = el('btn-websearch');
  if (ws) {
    if (settings.searchConfigured) {
      ws.style.display = 'flex';
      ws.classList.toggle('active', settings.webSearchEnabled);
      const wsLabel = ws.querySelector('.ctrl-label');
      if (wsLabel) wsLabel.textContent = settings.webSearchEnabled ? '搜索' : '已关';
    } else {
      ws.style.display = 'none';
    }
  }
  updateInfoBar();
}

function updateRoundsUI() {
  const sw = el('rounds-slider-wrap'); if (sw) sw.style.display = settings.roundsLimited ? 'block' : 'none';
  const rs = el('rounds-slider'); if (rs) rs.value = settings.maxToolRounds;
  const rv = el('rounds-value'); if (rv) rv.textContent = settings.maxToolRounds;
  document.querySelectorAll('[data-rounds]').forEach(b => {
    b.classList.toggle('active', (b.dataset.rounds === 'on') === settings.roundsLimited);
  });
}

// ════════════════════════════════════════════
// Helpers
// ════════════════════════════════════════════
function scrollToBottom() { const c = el('chat'); if (c) c.scrollTop = c.scrollHeight; }
function escapeHtml(s) { return (s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;'); }
function md(text) {
  if (typeof marked !== 'undefined' && marked.parse) {
    let html = marked.parse(text);
    // XSS 防护: 移除危险标签和事件处理器
    return html.replace(/<script[\s\S]*?<\/script>/gi, '')
               .replace(/<iframe[\s\S]*?<\/iframe>/gi, '')
               .replace(/\son\w+\s*=\s*"[^"]*"/gi, '')
               .replace(/\son\w+\s*=\s*'[^']*'/gi, '')
               .replace(/\son\w+\s*=\s*[^\s>]+/gi, '')
               .replace(/javascript:/gi, '');
  }
  return escapeHtml(text);
}
function isViewingStream() { return streamChat && currentChat && currentChat.id === streamChat.id; }

// ─── 审批条：智能体请求覆盖/删除用户知识库文档（全模式强制人审）───
// ─── 操作授权窗口（对齐 Claude 权限对话框：允许一次 / 本次对话允许 / 拒绝）───
// approval_request 事件触发；桌面壳同时发 Windows 通知并置前窗口。
let approvalCountdown = null;
let approvalKeyHandler = null;
function showApprovalModal(evt) {
  closeApprovalModal();
  const overlay = el('approval-overlay');
  if (!overlay) return;
  const aEl = el('approval-action');
  const gEl = el('approval-target');
  const dEl = el('approval-detail');
  if (aEl) aEl.textContent = evt.action || '智能体请求执行操作';
  if (gEl) gEl.textContent = evt.target || '';
  if (dEl) dEl.textContent = evt.detail || '';
  overlay.dataset.token = evt.token || '';
  overlay.dataset.decided = '';
  overlay.style.display = 'flex';
  // 倒计时（后端 180s 超时自动拒绝，前端同步展示）
  let remain = 180;
  const cd = el('approval-countdown');
  if (cd) cd.textContent = remain + ' 秒后自动拒绝';
  approvalCountdown = setInterval(() => {
    remain -= 1;
    if (cd) cd.textContent = Math.max(remain, 0) + ' 秒后自动拒绝';
    if (remain <= 0) decideApproval(false, 'once');
  }, 1000);
  // Windows 通知 + 窗口置前（桌面壳；复用待办通知链）
  (async () => {
    try {
      if (typeof window.notifyWindows === 'function') {
        await window.notifyWindows('SKYLAND 需要授权', (evt.action || '智能体请求执行操作') + (evt.target ? '：' + evt.target : ''));
        if (typeof __TAURI__ !== 'undefined' && __TAURI__.window && __TAURI__.window.getCurrentWindow) {
          const win = __TAURI__.window.getCurrentWindow();
          if (win.show) await win.show();
          if (win.setFocus) await win.setFocus();
        }
      }
    } catch (e) {}
  })();
  const onceBtn = el('approval-btn-once');
  const sessBtn = el('approval-btn-session');
  const denyBtn = el('approval-btn-deny');
  if (onceBtn) onceBtn.onclick = () => decideApproval(true, 'once');
  if (sessBtn) sessBtn.onclick = () => decideApproval(true, 'session');
  if (denyBtn) denyBtn.onclick = () => decideApproval(false, 'once');
  overlay.onclick = (e) => { if (e.target === overlay) decideApproval(false, 'once'); };
  // 快捷键：1=允许一次 2=本次对话允许 3/Esc=拒绝（Claude 数字键习惯）
  approvalKeyHandler = (e) => {
    if (e.key === '1') decideApproval(true, 'once');
    else if (e.key === '2') decideApproval(true, 'session');
    else if (e.key === '3' || e.key === 'Escape') decideApproval(false, 'once');
  };
  document.addEventListener('keydown', approvalKeyHandler);
}
async function decideApproval(allow, scope) {
  const overlay = el('approval-overlay');
  if (!overlay || overlay.dataset.decided) return;
  overlay.dataset.decided = '1';
  const token = overlay.dataset.token || '';
  closeApprovalModal();
  try {
    await fetch('/api/kb/approval', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token, allow, scope }),
    });
  } catch (e) { console.error('approval decide error:', e); }
}
function closeApprovalModal() {
  const overlay = el('approval-overlay');
  if (overlay) overlay.style.display = 'none';
  if (approvalCountdown) { clearInterval(approvalCountdown); approvalCountdown = null; }
  if (approvalKeyHandler) { document.removeEventListener('keydown', approvalKeyHandler); approvalKeyHandler = null; }
}

// ─── 对话回溯导轨（ChatGPT 式）：右侧刻度 = 每条用户消息；悬停预览；点击跳转 ───
let railTooltipEl = null;

function refreshRail() {
  const rail = el('chat-rail');
  const chat = el('chat');
  if (!rail || !chat) return;
  const view = el('view-chat');
  if (!view || view.style.display === 'none') { rail.classList.remove('visible'); return; }
  const userMsgs = chat.querySelectorAll('.message-user');
  if (userMsgs.length < 2) { rail.classList.remove('visible'); rail.innerHTML = ''; return; }
  rail.classList.add('visible');
  rail.innerHTML = '';
  const scrollable = chat.scrollHeight - chat.clientHeight;
  userMsgs.forEach((m, i) => {
    const rect = m.getBoundingClientRect();
    const chatRect = chat.getBoundingClientRect();
    const offset = rect.top - chatRect.top + chat.scrollTop;
    const pct = Math.max(2, Math.min(96, (offset / Math.max(chat.scrollHeight, 1)) * 100));
    const text = (m.querySelector('.msg-user-text')?.textContent || '').trim();
    const tick = document.createElement('button');
    tick.className = 'rail-tick';
    tick.style.top = pct + '%';
    tick.title = '';
    tick.addEventListener('mouseenter', () => showRailTooltip(rail, tick, text, i + 1, userMsgs.length));
    tick.addEventListener('mouseleave', hideRailTooltip);
    tick.addEventListener('click', () => {
      m.scrollIntoView({ behavior: 'smooth', block: 'start' });
      m.classList.add('msg-flash');
      setTimeout(() => m.classList.remove('msg-flash'), 1200);
      hideRailTooltip();
    });
    rail.appendChild(tick);
  });
  syncRailActive();
}

function showRailTooltip(rail, tick, text, idx, total) {
  hideRailTooltip();
  if (!text) text = '(图片消息)';
  railTooltipEl = document.createElement('div');
  railTooltipEl.className = 'rail-tooltip';
  railTooltipEl.innerHTML = '<div class="rail-tooltip-head">' + idx + ' / ' + total + '</div>' +
    '<div class="rail-tooltip-text">' + escapeHtml(text.slice(0, 200)) + (text.length > 200 ? '…' : '') + '</div>';
  const tickTop = parseFloat(tick.style.top) || 0;
  if (tickTop > 62) {
    // 刻度靠近导轨底部 → 提示窗向上展开（底边对齐刻度上方）
    railTooltipEl.style.top = 'auto';
    railTooltipEl.style.bottom = 'calc(' + (100 - tickTop) + '% + 12px)';
  } else {
    railTooltipEl.style.top = 'max(8px, calc(' + tickTop + '% + 14px))';
    railTooltipEl.style.bottom = 'auto';
  }
  rail.appendChild(railTooltipEl);
}

function hideRailTooltip() {
  if (railTooltipEl) { railTooltipEl.remove(); railTooltipEl = null; }
}

function syncRailActive() {
  const rail = el('chat-rail');
  const chat = el('chat');
  if (!rail || !chat || !rail.classList.contains('visible')) return;
  const ticks = rail.querySelectorAll('.rail-tick');
  const userMsgs = chat.querySelectorAll('.message-user');
  if (ticks.length !== userMsgs.length) return;
  const chatRect = chat.getBoundingClientRect();
  let activeIdx = -1;
  userMsgs.forEach((m, i) => {
    if (m.getBoundingClientRect().top <= chatRect.top + chatRect.height * 0.35) activeIdx = i;
  });
  ticks.forEach((t, i) => t.classList.toggle('active', i === activeIdx));
}

// KaTeX 公式渲染（完成后一次性调用；流式过程中逐帧渲染太耗性能）
function renderMath(container) {
  try {
    if (typeof renderMathInElement === 'function' && container) {
      renderMathInElement(container, {
        delimiters: [
          { left: '$$', right: '$$', display: true },
          { left: '\[', right: '\]', display: true },
          { left: '$', right: '$', display: false },
          { left: '\(', right: '\)', display: false },
        ],
        throwOnError: false,
      });
    }
  } catch (e) {}
}

// ─── Scroll tracking ───
(function() {
  const c = el('chat'); if (!c) return;
  c.addEventListener('scroll', () => {
    autoScroll = (c.scrollHeight - c.scrollTop - c.clientHeight) < 100;
    syncRailActive();
  });
  if (typeof ResizeObserver !== 'undefined') {
    new ResizeObserver(() => refreshRail()).observe(el('messages'));
  }
  window.addEventListener('resize', () => refreshRail());
})();

// ════════════════════════════════════════════
// Code blocks
// ════════════════════════════════════════════
function renderCodeBlocks(container) {
  if (!container) return;
  container.querySelectorAll('pre').forEach(pre => {
    if (pre.querySelector('.code-header')) return;
    const code = pre.querySelector('code'); if (!code) return;
    let lang = ''; const m = code.className.match(/language-(\w+)/); if (m) lang = m[1];
    if (!code.classList.contains('hljs')) { try { hljs.highlightElement(code); } catch(e) {} }
    const header = document.createElement('div'); header.className = 'code-header';
    const langLabel = document.createElement('span'); langLabel.className = 'code-lang'; langLabel.textContent = lang || 'code';
    const copyBtn = document.createElement('button'); copyBtn.className = 'copy-btn'; copyBtn.textContent = 'Copy';
    copyBtn.onclick = (e) => { e.stopPropagation(); navigator.clipboard.writeText(code.textContent).then(() => { copyBtn.textContent='Copied'; copyBtn.classList.add('copied'); setTimeout(()=>{copyBtn.textContent='Copy';copyBtn.classList.remove('copied');},2000); }).catch(()=>{}); };
    header.appendChild(langLabel); header.appendChild(copyBtn); pre.insertBefore(header, code);
  });
}

// ════════════════════════════════════════════
// Messages — Card-based
// ════════════════════════════════════════════
function addMessage(role, text, image, attachments) {
  const wel = el('welcome'); const msgs = el('messages');
  if (wel) wel.style.display = 'none';
  if (!msgs) return null;
  msgs.style.display = 'flex';
  const div = document.createElement('div');
  const cssRole = role === 'assistant' ? 'ai' : role;
  div.className = `message message-${cssRole}`;

  if (cssRole === 'ai') {
    const header = document.createElement('div');
    header.className = 'msg-card-header';
    const time = new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
    header.innerHTML = `<span class="msg-card-label">AI</span><div class="msg-card-meta"><span class="msg-badge">${escapeHtml(selectedModel || 'model')}</span><span class="msg-badge">${time}</span></div>`;
    div.appendChild(header);
    const body = document.createElement('div');
    body.className = 'msg-card-body';
    body.innerHTML = md(text || '');
    div.appendChild(body);
    renderCodeBlocks(body);
    addMessageActions(div, text);
  } else {
    // User message — render images (多图，兼容旧单图字段) + text
    const imgs = Array.isArray(image) ? image : (image ? [image] : null);
    if (imgs && imgs.length) {
      const wrap = document.createElement('div');
      wrap.className = 'msg-user-images';
      imgs.forEach(src => {
        const imgEl = document.createElement('img');
        imgEl.className = 'msg-user-image';
        imgEl.src = src;
        imgEl.alt = '图片';
        wrap.appendChild(imgEl);
      });
      div.appendChild(wrap);
    }
    if (text) {
      const textEl = document.createElement('div');
      textEl.className = 'msg-user-text';
      textEl.textContent = text;
      div.appendChild(textEl);
    }
  }
  if (role === 'user' && typeof attachments !== 'undefined' && attachments && attachments.length) {
    const list = document.createElement('div');
    list.className = 'msg-user-attachments';
    list.textContent = '附件: ' + attachments.map(x => x.name).join('、');
    div.appendChild(list);
  }

  msgs.appendChild(div);
  if (autoScroll) scrollToBottom();
  return div;
}

function addMessageActions(div, text) {
  // Remove existing actions if any (prevents duplicates from streaming re-add)
  const existing = div.querySelector('.message-actions');
  if (existing) existing.remove();
  const actions = document.createElement('div'); actions.className = 'message-actions';
  const copyBtn = document.createElement('button');
  copyBtn.className = 'msg-action-btn'; copyBtn.title = '复制'; copyBtn.setAttribute('aria-label', '复制');
  copyBtn.innerHTML = SVG.clipboard;
  copyBtn.onclick = () => { navigator.clipboard.writeText(text||'').then(() => { copyBtn.title = '已复制'; copyBtn.classList.add('copied'); setTimeout(() => { copyBtn.title = '复制'; copyBtn.classList.remove('copied'); }, 2000); }).catch(() => {}); };
  actions.appendChild(copyBtn); div.appendChild(actions);
}

function showThinking() {
  const msgs = el('messages'); if (!msgs) return null;
  const div = document.createElement('div');
  div.className = 'message message-thinking';
  div.innerHTML = '<span class="spinner"></span>思考中...';
  msgs.appendChild(div);
  if (autoScroll) scrollToBottom();
  return div;
}

// 工作中计时行（无气泡排版：独立于消息的活动状态行）
function showWorking() {
  const msgs = el('messages'); if (!msgs) return null;
  const div = document.createElement('div');
  div.className = 'agent-working';
  div.innerHTML = '<span class="status-spinner"></span><span class="aw-text">工作中</span><span class="aw-time" id="aw-time"></span>';
  msgs.appendChild(div);
  workTimer = setInterval(() => {
    const t = document.getElementById('aw-time');
    if (t && streamStartedAt) t.textContent = fmtDuration(Date.now() - streamStartedAt);
  }, 1000);
  if (autoScroll) scrollToBottom();
  return div;
}

function fmtDuration(ms) {
  const s = Math.floor(ms / 1000);
  if (s < 60) return s + ' 秒';
  return Math.floor(s / 60) + ' 分 ' + (s % 60) + ' 秒';
}

// ════════════════════════════════════════════
// Tool cards
// ════════════════════════════════════════════
// 工具动作的中文名 + 图标（活动线用）
const TOOL_META = {
  read_file: { label: '读取文件', icon: 'file' }, write_file: { label: '写入文件', icon: 'pencil' },
  edit_file: { label: '编辑文件', icon: 'pencil' }, bash: { label: '执行命令', icon: 'bolt' },
  glob: { label: '查找文件', icon: 'search' }, grep: { label: '搜索代码', icon: 'search' },
  web_search: { label: '搜索网络', icon: 'globe' }, kb_list: { label: '列出知识库', icon: 'clipboard' },
  kb_search: { label: '检索知识库', icon: 'search' }, kb_read: { label: '阅读知识库文档', icon: 'file' },
  todo_read: { label: '查看待办', icon: 'clipboard' }, todo_create: { label: '创建待办', icon: 'clipboard' },
  use_skill: { label: '加载技能', icon: 'brain' }, create_task: { label: '创建定时任务', icon: 'clock' },
  create_document: { label: '生成 Word', icon: 'file' }, create_spreadsheet: { label: '生成表格', icon: 'file' },
  create_presentation: { label: '生成演示文稿', icon: 'file' }, create_pdf: { label: '生成 PDF', icon: 'file' },
  spawn_agent: { label: '派出子智能体', icon: 'bolt' },
  kb_create_kb: { label: '创建知识库', icon: 'clipboard' }, kb_create_doc: { label: '创建知识库文档', icon: 'pencil' },
  kb_delete_doc: { label: '删除知识库文档', icon: 'trash' }, kb_edit_doc: { label: '编辑知识库文档', icon: 'pencil' },
};
function toolIcon(name) {
  const meta = TOOL_META[name];
  const key = meta ? meta.icon : 'wrench';
  const svgs = {
    file: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>',
    pencil: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg>',
    bolt: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>',
    search: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>',
    globe: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>',
    clipboard: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><rect x="8" y="2" width="8" height="4" rx="1"/></svg>',
    brain: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><path d="M9.5 2A2.5 2.5 0 0 1 12 4.5v15a2.5 2.5 0 0 1-4.96.44 2.5 2.5 0 0 1-2.96-3.08 3 3 0 0 1-.34-5.58 2.5 2.5 0 0 1 1.32-4.24 2.5 2.5 0 0 1 1.98-3A2.5 2.5 0 0 1 9.5 2Z"/><path d="M14.5 2A2.5 2.5 0 0 0 12 4.5v15a2.5 2.5 0 0 0 4.96.44 2.5 2.5 0 0 0 2.96-3.08 3 3 0 0 0 .34-5.58 2.5 2.5 0 0 0-1.32-4.24 2.5 2.5 0 0 0-1.98-3A2.5 2.5 0 0 0 14.5 2Z"/></svg>',
    clock: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 3"/></svg>',
    trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>',
    wrench: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="13" height="13"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>'
  };
  const icon = svgs[key] || svgs.wrench;
  const label = meta ? meta.label : name;
  return { icon: icon, label: label };
}
function createToolCard(name) {
  const card = document.createElement('div');
  card.className = 'tool-line';
  const meta = toolIcon(name);
  card.innerHTML = '<span class="tool-line-icon">' + meta.icon + '</span>' +
    '<span class="tool-line-label">' + escapeHtml(meta.label) + '</span>' +
    '<span class="tool-line-status running">◌ 执行中</span>' +
    '<div class="tool-card-body" style="display:none"></div>';
  card.addEventListener('click', () => { const b = card.querySelector('.tool-card-body'); if (b) b.style.display = b.style.display === 'none' ? 'block' : 'none'; });
  // 工具线插入消息卡内的当前进度位置，保证与文本段按发生顺序穿插；
  // 仅在无流式消息元素时退回消息容器末尾
  if (streamAiEl) {
    streamAiEl.insertBefore(card, streamAiEl.querySelector('.message-actions'));
  } else {
    const msgs = el('messages'); if (!msgs) return null;
    msgs.appendChild(card);
  }
  if (autoScroll) scrollToBottom();
  return card;
}

// ─── 流式分段渲染辅助 ───
// 取当前文本段元素；上一段已随工具调用收束时新建一个（排在最后一条工具线之后）
function ensureStreamTextEl() {
  if (!streamAiEl) return null;
  if (!streamTextEl || !streamTextEl.parentNode) {
    streamTextEl = document.createElement('div');
    streamTextEl.className = 'msg-card-body';
    streamAiEl.insertBefore(streamTextEl, streamAiEl.querySelector('.message-actions'));
  }
  return streamTextEl;
}
// 收束当前文本段：空段不留 DOM（避免消息卡顶部残留空块）
function finalizeStreamTextEl() {
  if (streamTextEl && streamTextEl.parentNode && !streamSegText) streamTextEl.remove();
  streamTextEl = null;
  streamSegText = '';
}
// 思维链/状态行的挂载锚点：消息顶部（header 之后）— 文本段可被工具调用收束删除，
// 不能再以 _body 为锚，否则空段删除后插入会崩溃
function streamTopAnchor() {
  if (!streamAiEl) return null;
  const header = streamAiEl.querySelector('.msg-card-header');
  return header ? header.nextSibling : (streamAiEl.firstChild || null);
}

function updateToolCard(card, success, output) {
  if (!card) return;
  const status = card.querySelector('.tool-line-status');
  // 状态三通道：图标形状 + 文字 + 主题状态色（不单独依赖颜色）
  if (status) {
    status.className = 'tool-line-status ' + (success ? 'ok' : 'err');
    status.textContent = (success ? '✓ 完成' : '✕ 失败');
  }
  const body = card.querySelector('.tool-card-body');
  if (body) body.innerHTML = `<div class="tool-card-body-section"><div class="tool-card-body-label">输出</div>${escapeHtml(output||'')}</div>`;
  if (autoScroll) scrollToBottom();
}

// ════════════════════════════════════════════
// SSE parsing
// ════════════════════════════════════════════
async function* parseSSEStream(reader) {
  const decoder = new TextDecoder(); let buffer = '';
  while (true) {
    const { value, done } = await reader.read(); if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const events = buffer.split('\n\n'); buffer = events.pop() || '';
    for (const eventText of events) {
      if (!eventText.trim() || eventText.startsWith(':')) continue;
      let data = '';
      for (const line of eventText.split('\n')) { const t = line.trim(); if (t.startsWith('data: ')) data += t.slice(6); else if (t.startsWith('data:')) data += t.slice(5).trim(); }
      if (!data) continue; if (data === '[DONE]') return; yield data;
    }
  }
}

// ════════════════════════════════════════════
// Chat management
// ════════════════════════════════════════════
function createNewChat() {
  currentChat = { id: Date.now(), title: '新对话', messages: [] };
  chats.unshift(currentChat); saveChats(); renderChatList();
  sessionPlan = [];   // 新对话不带旧计划的残留
  renderPlanList();
  updateComposerWsRow();  // 新对话 → 工作区行回归（初始化设置）
  const msgs = el('messages'); if (msgs) msgs.innerHTML = '';
  const wel = el('welcome'); if (wel) wel.style.display = 'flex';
  if (msgs) msgs.style.display = 'none';
  const ct = el('chat-title'); if (ct) ct.textContent = 'SKYLAND';
}

function renderChatList() {
  const cl = el('sb-chats'); if (!cl) return;
  syncSidebarSections();
  // 会话树与本列表共用"当前会话"高亮
  window.__skylandActiveConvId = currentChat ? currentChat.id : null;
  if (window.SkylandWsTree) { try { window.SkylandWsTree.render(window.__skylandActiveConvId); } catch (e) {} }
  // 按真实时间戳（chat.id = Date.now()）分组：今天 / 昨天 / 更早 — 不伪造任何数据
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfYesterday = startOfToday - 86400000;
  const groups = { today: [], yesterday: [], earlier: [] };
  chats.forEach(chat => {
    const t = Number(chat.id) || 0;
    if (t >= startOfToday) groups.today.push(chat);
    else if (t >= startOfYesterday) groups.yesterday.push(chat);
    else groups.earlier.push(chat);
  });
  const item = chat => {
    const isActive = currentChat && chat.id === currentChat.id;
    const isStreamingChat = streamChat && chat.id === streamChat.id;
    return `<div class="chat-item${isActive ? ' active' : ''}" data-id="${chat.id}">
      <span class="chat-title-text">${escapeHtml(chat.title)}</span>
      ${isStreamingChat ? '<span class="stream-indicator"></span>' : ''}
      <button class="chat-delete" data-del="${chat.id}" title="删除" aria-label="删除对话"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="13" height="13"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg></button>
    </div>`;
  };
  const section = (label, list) => list.length
    ? `<div class="sb-group"><div class="sb-group-label">${label}</div>${list.map(item).join('')}</div>`
    : '';
  cl.innerHTML =
    section('今天', groups.today) +
    section('昨天', groups.yesterday) +
    section('更早', groups.earlier);
}

function selectChat(id) {
  // 宽松 id 匹配：后端会话 id 可能为数字，ws-tree/队列等调用方传字符串
  // （严格 === 曾导致点击会话静默无响应）
  const chat = chats.find(c => String(c.id) === String(id));
  if (!chat) return false;
  // 切换到**另一条**会话时清空子智能体面板：agentNodes 属于当前会话的运行时数据，
  // 不跟着会话持久化。留在屏幕上会被误读成"新会话也派生了这些子代理"——
  // 属于张冠李戴的数据错误。仅在真正切换时清，避免重复点同一会话时误伤进行中的面板。
  if (currentChat && currentChat.id !== chat.id && typeof clearAgentTree === 'function') clearAgentTree();
  currentChat = chat;
  window.__skylandActiveConvId = chat.id;
  renderChatList();
  updateComposerWsRow();
  const msgs = el('messages'); if (!msgs) return;
  msgs.innerHTML = '';

  if (chat.messages.length === 0) {
    const wel = el('welcome'); if (wel) wel.style.display = 'flex';
    msgs.style.display = 'none';
    const ct = el('chat-title'); if (ct) ct.textContent = 'SKYLAND';
    return true;
  }

  const wel = el('welcome'); if (wel) wel.style.display = 'none';
  msgs.style.display = 'flex';
  const ct = el('chat-title'); if (ct) ct.textContent = chat.title;
  chat.messages.forEach(msg => addMessage(msg.role, msg.content, msg.images || msg.image || null, msg.attachments || null));
  renderMath(msgs);
  sessionOutputs = [];
  sessionSources = [];
  renderInfoOutputs();
  renderInfoSources();
  renderInfoAttachments();
  // 右栏状态恢复 — 轮次/工具数/时长/结果/上下文（随会话持久化，重开不再显示 0/—）
  sessionRound = chat._round || 0;
  sessionToolCount = chat._toolCount || 0;
  // 执行计划随会话恢复 — 切换/重开会话右栏不再从零重列（已完成状态保真）
  sessionPlan = Array.isArray(chat._plan) ? chat._plan : [];
  renderPlanList();
  const rrEl = el('info-round'); if (rrEl) rrEl.textContent = chat._round ? ('第 ' + chat._round + ' 轮') : '—';
  const ttEl = el('info-tool-count'); if (ttEl) ttEl.textContent = String(sessionToolCount);
  lastStreamDur = chat._dur || null;
  lastStreamOutcome = chat._outcome || null;
  syncRunState();
  if (chat._ctx && chat._ctx.total) {
    updateContextBar(chat._ctx.used, chat._ctx.total);
  } else if (chat.messages.length) {
    // 旧会话无持久化数据 → 后端按历史重估（estimate_content_tokens 同源逻辑）
    const hist = chat.messages.map(m => ({ role: m.role, content: m.content }));
    fetch('/api/chat/context-estimate', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ history: hist, model: selectedModel || undefined, context_length: getContextLengthOverride() || undefined }),
    }).then(r => r.json()).then(d => {
      if (d && d.status === 'ok' && currentChat === chat) updateContextBar(d.used, d.total);
    }).catch(() => {});
  } else {
    updateContextBar(0, getContextLengthOverride() || 128000);
  }
  setTimeout(refreshRail, 50);
  renderInfoAttachments();

  if (streamChat && streamChat.id === chat.id && streamFullResponse !== null) {
    const aiEls = msgs.querySelectorAll('.message-ai');
    streamAiEl = aiEls.length > 0 ? aiEls[aiEls.length - 1] : null;
    if (streamAiEl) {
      streamAiEl._body = streamAiEl.querySelector('.msg-card-body');
      // 重挂载后工具线已丢失，全文并入首个文本段；后续新文本续写在该段内保持一致
      streamTextEl = streamAiEl._body;
      streamSegText = streamFullResponse;
      if (streamFullResponse && streamAiEl._body) {
        streamAiEl._body.innerHTML = md(streamFullResponse);
        renderCodeBlocks(streamAiEl._body);
        streamAiEl.classList.add('streaming-cursor');
        if (autoScroll) scrollToBottom();
      }
    }
  }
  return true;
}

function deleteChat(id, e) {
  e.stopPropagation();
  // 宽松 id 匹配（同 selectChat）：后端会话 id 数字/字符串形态不确定
  const match = c => String(c.id) === String(id);
  const target = chats.find(match);
  if (!target) return;
  if (streamChat && String(streamChat.id) === String(id) && isStreaming) {
    if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('流式进行中，无法删除该会话', 'error');
    return;
  }
  chats = chats.filter(c => !match(c));
  // 工作区会话 → 同步删除 .skyland/conversations 中的对应文件
  if (wsActive()) {
    fetch(`/api/conversations/${encodeURIComponent(target.id)}?workdir=${encodeURIComponent(Workdir.value())}`, { method: 'DELETE' })
      .then(r => { if (!r.ok) console.error('工作区会话删除失败: HTTP', r.status); })
      .catch(() => {});
  }
  if (currentChat && match(currentChat)) {
    closeChatView();
  }
  saveChats(); renderChatList();
}

// ════════════════════════════════════════════
// Send message
// ════════════════════════════════════════════
async function sendMessage(fromStopButton = false) {
  const inp = el('input'); if (!inp) return;
  if (isStreaming) {
    // 点击发送按钮（流式期间显示为停止图标）→ 立即中止当前任务；
    // 输入框 Enter → 保持排队语义（内容进队列，流结束后依次发送）
    if (fromStopButton && abortController) {
      abortController.abort();
      return;
    }
    const text = inp.value.trim();
    if (text) { messageQueue.push(text); inp.value = ''; inp.style.height = 'auto'; updateQueueIndicator(); }
    return;
  }
  const text = inp.value.trim();
  if (!text) return;
  inp.value = ''; inp.style.height = 'auto';
  await doSend(text);
  while (messageQueue.length > 0) {
    updateQueueIndicator();
    const next = messageQueue.shift();
    updateQueueIndicator();
    await doSend(next);
  }
}

async function doSend(text) {
  if (!currentChat) createNewChat();
  streamChat = currentChat;
  streamFullResponse = '';
  streamTextEl = null;
  streamSegText = '';
  streamAiEl = null;
  streamStartedAt = Date.now();
  effortUnsupported = false;
  sessionRound = 0;
  sessionToolCount = 0;
  const r0 = el('info-round'); if (r0) r0.textContent = '—';
  const t0 = el('info-tool-count'); if (t0) t0.textContent = '0';
  streamReasoningStart = 0;
  lastStreamOutcome = null;
  lastStreamDur = null;
  syncRunState();
  if (workTimer) { clearInterval(workTimer); workTimer = null; }

  // 捕获发送内容（多图）后再清空
  const sentImages = pendingImages.slice();

  // 斜杠技能引入：消息以 /技能名 开头 → 剥离前缀，技能名走 body.skill 确定性注入
  let outgoingText = text;
  let slashSkill = null;
  const slashMatch = text.match(/^\/([a-z0-9_-]+)(?:\s+([\s\S]*))?$/i);
  if (slashMatch && Array.isArray(skillsCache) && skillsCache.some(s => s.name === slashMatch[1].toLowerCase())) {
    slashSkill = slashMatch[1].toLowerCase();
    outgoingText = (slashMatch[2] || '').trim() || '请严格按该技能的指令执行';
  }

  streamChat.messages.push({ role: 'user', content: text, images: sentImages.length ? sentImages : undefined, attachments: pendingAttachments.length ? pendingAttachments.map(x => ({ name: x.name })) : undefined });
  addMessage('user', text, sentImages, pendingAttachments.length ? pendingAttachments.map(x => ({ name: x.name })) : null);
  updateComposerWsRow();  // 首条消息已发出 → 工作区行隐藏（初始化设置已锁定为本会话上下文）

  if (streamChat.messages.filter(m => m.role === 'user').length === 1) {
    streamChat.title = text.slice(0, 30);
    const ct = el('chat-title'); if (ct) ct.textContent = streamChat.title;
    renderChatList();
  }
  saveChats();
  setStreaming(true);
  autoScroll = true;
  clearAgentTree();

  const workingEl = showWorking();
  const thinkingEl = null;
  const history = streamChat.messages.slice(0, -1).map(m => ({ role: m.role, content: m.content }));
  abortController = new AbortController();

  const body = {
    message: outgoingText, history: history,
    model: selectedModel || undefined,
    execution_mode: settings.executionMode,
    thinking_effort: settings.thinkingEffort,
    system_prompt: settings.systemPrompt || undefined,
    web_search_enabled: settings.webSearchEnabled,
    // 工作区路径 — 系统提示注入 + 工具层执行位置（单一来源：Workdir 状态）
    workspace: (typeof Workdir !== 'undefined' && Workdir.value()) || undefined,
    // 会话执行计划随请求回传续接 — 后端据此初始化本流计划，已完成项不再"复活"
    plan: sessionPlan.length ? sessionPlan : undefined,
  };
  if (slashSkill) body.skill = slashSkill;
  if (settings.roundsLimited) body.max_tool_rounds = settings.maxToolRounds;
  const ctxOverride = getContextLengthOverride();
  if (ctxOverride) body.context_length = ctxOverride; // 用户覆盖的上下文窗口（影响压缩阈值）
  // 上下文压缩专用模型（设置 → 性能与执行；空 = 当前对话模型）
  try { const cm = localStorage.getItem(K_COMPACT_MODEL); if (cm) body.compact_model = cm; } catch (e) {}
  if (typeof Workdir !== 'undefined' && Workdir.value()) {
    body.workdir = Workdir.value();
    body.conversation_id = streamChat.id;
  }

  if (pendingImages.length) { body.images = pendingImages; }
  if (pendingAttachments.length) body.attachments = pendingAttachments;

  try {
    const response = await fetch('/api/chat/stream', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body), signal: abortController.signal,
    });
    clearPendingImage();
    clearPendingAttachments();
    if (!response.ok) throw new Error('HTTP ' + response.status);

    if (isViewingStream() && workingEl && workingEl.parentNode) workingEl.remove();

    if (isViewingStream()) {
      streamAiEl = addMessage('ai', '');
      if (streamAiEl) { streamAiEl.classList.add('streaming-cursor'); streamAiEl._body = streamAiEl.querySelector('.msg-card-body'); streamTextEl = streamAiEl._body; }
    } else {
      streamChat.messages.push({ role: 'assistant', content: '' });
    }

    const reader = response.body.getReader();

    for await (const data of parseSSEStream(reader)) {
      try {
        const evt = JSON.parse(data);

        if (evt.type === 'text') {
          // 正文开始 → 思维链收束（ChatGPT 交互：思考结束后自动折叠，可手动展开）
          if (streamChat._reasoning && reasonTimer) {
            clearInterval(reasonTimer); reasonTimer = null;
            if (isViewingStream() && streamAiEl) {
              const box = streamAiEl.querySelector('.reasoning-box');
              if (box) {
                box.open = false;
                const s = box.querySelector('summary');
                if (s) s.innerHTML = '<span class="rb-dot done"></span>思考 · ' +
                  (streamReasoningStart ? '持续了 ' + fmtDuration(Date.now() - streamReasoningStart) : '结束');
              }
            }
          }
          streamFullResponse += evt.content;
          streamSegText += evt.content;
          const lastMsg = streamChat.messages[streamChat.messages.length - 1];
          if (lastMsg && lastMsg.role === 'assistant') lastMsg.content = streamFullResponse;

          // Periodic save every ~50 chunks to prevent data loss on tab close
          if (streamFullResponse.length % 2000 < evt.content.length) { saveChats(); }

          if (isViewingStream() && streamAiEl) {
            const target = ensureStreamTextEl();
            if (target) {
              target.innerHTML = md(streamSegText);
              renderCodeBlocks(target);
              streamAiEl.classList.add('streaming-cursor');
              if (autoScroll) scrollToBottom();
            }
          }
        } else if (evt.type === 'reasoning') {
          // 思维链实时流 — 可展开子缩进块（ChatGPT 式：流中实时计时，正文开始自动收起）
          // 计时起点每条消息独立：_reasoning 跨消息残留在会话对象上，不能再用它判定"首次"
          // （旧逻辑：第二条消息跳过起点初始化 → Date.now()-0 = 纪元差 → "持续了 29829114 分"）
          if (!streamChat._reasoning) { streamChat._reasoning = ''; }
          streamReasoningStart = streamReasoningStart || Date.now();
          streamChat._reasoning += evt.content;
          if (isViewingStream() && streamAiEl) {
            let box = streamAiEl.querySelector('.reasoning-box');
            if (!box) {
              box = document.createElement('details');
              box.className = 'reasoning-box';
              box.open = true;
              box.innerHTML = '<summary><span class="rb-dot"></span>思考中…</summary><div class="reasoning-body"></div>';
              streamAiEl.insertBefore(box, streamTopAnchor());
            }
            const body = box.querySelector('.reasoning-body');
            const sum = box.querySelector('summary');
            // 实时计时器：思维链事件间隔不定，用 interval 保证秒数持续走动
            if (!reasonTimer) {
              reasonTimer = setInterval(() => {
                if (!streamAiEl || !streamReasoningStart) return;
                const b = streamAiEl.querySelector('.reasoning-box');
                const s = b && b.querySelector('summary');
                if (s) s.innerHTML = '<span class="rb-dot"></span>思考 · ' + fmtDuration(Date.now() - streamReasoningStart);
              }, 1000);
            }
            if (sum && streamReasoningStart) {
              sum.innerHTML = '<span class="rb-dot"></span>思考 · ' + fmtDuration(Date.now() - streamReasoningStart);
            }
            if (body) {
              body.textContent = streamChat._reasoning.slice(-1500);
              if (autoScroll) scrollToBottom();
            }
          }
        } else if (evt.type === 'approval_request') {
          showApprovalModal(evt);
        } else if (evt.type === 'status') {
          // 智能体动作状态行（正在读取文件/生成文档/调用技能…）
          if (isViewingStream() && streamAiEl) {
            let line = streamAiEl.querySelector('.agent-status-line');
            if (!line) {
              line = document.createElement('div');
              line.className = 'agent-status-line';
              streamAiEl.insertBefore(line, streamTopAnchor());
            }
            line.innerHTML = '<span class="status-spinner"></span><span class="status-text">' +
              escapeHtml(evt.action) + (evt.detail ? '<span class="status-detail">' + escapeHtml(evt.detail) + '</span>' : '') + '</span>';
            if (autoScroll) scrollToBottom();
          }
        } else if (evt.type === 'tool_start') {
          sessionToolCount++;
          if (streamChat) streamChat._toolCount = sessionToolCount;
          const tEl = el('info-tool-count');
          if (tEl) tEl.textContent = sessionToolCount;
          if (isViewingStream() && streamAiEl) {
            // 收束当前文本段（内容已实时渲染在段元素内），工具线随即可插入其后
            finalizeStreamTextEl();
            const card = createToolCard(evt.name);
            if (card && evt.id) card.dataset.tid = evt.id;
            if (card && evt.args) {
              const cardBody = card.querySelector('.tool-card-body');
              if (cardBody) { cardBody.innerHTML = `<div class="tool-card-body-section"><div class="tool-card-body-label">参数</div>${escapeHtml(evt.args)}</div>`; }
            }
            if (autoScroll) scrollToBottom();
          }
          trackToolEvent(evt.name, 'start', evt.args);
        } else if (evt.type === 'tool_end') {
          if (isViewingStream() && streamAiEl) {
            // 优先按 tool_call id 精确匹配 — 并行只读工具的完成序与开始序不一致
            let card = null;
            if (evt.id) {
              const sel = '.tool-line[data-tid="' + (window.CSS && CSS.escape ? CSS.escape(evt.id) : evt.id) + '"]';
              card = streamAiEl.querySelector(sel);
            }
            if (!card) {
              const running = streamAiEl.querySelectorAll('.tool-line .tool-line-status.running');
              card = running.length > 0 ? running[running.length - 1].closest('.tool-line') : null;
            }
            if (card) updateToolCard(card, evt.success, evt.output || '');
          }
          trackToolEvent(evt.name, 'end', evt.success ? (evt.output || '') : '');
        } else if (evt.type === 'error') {
          lastStreamOutcome = 'error';
          syncRunState();
          if (isViewingStream() && streamAiEl) {
            const target = ensureStreamTextEl();
            if (target) {
              target.innerHTML = md(streamSegText) + `<div class="message-system" style="margin-top:8px;color:var(--error)">${escapeHtml(evt.message)}</div>`;
            }
            if (autoScroll) scrollToBottom();
          }
        } else if (evt.type === 'warning') {
          // 思考能力感知 — 模型不支持思考模式时右栏与状态行同时提示（不静默吞掉设置）
          effortUnsupported = true;
          updateInfoBar();
          if (isViewingStream() && streamAiEl) {
            let line = streamAiEl.querySelector('.agent-status-line');
            if (!line) {
              line = document.createElement('div');
              line.className = 'agent-status-line';
              streamAiEl.insertBefore(line, streamTopAnchor());
            }
            line.innerHTML = '<span class="status-warn-ico">' + SVG.warn + '</span><span class="status-text">' + escapeHtml(evt.message) + '</span>';
            if (autoScroll) scrollToBottom();
          }
        } else if (evt.type === 'round_start') {
          sessionRound = evt.round || 0;
          if (streamChat) streamChat._round = sessionRound;
          const rEl = el('info-round');
          if (rEl) rEl.textContent = '第 ' + sessionRound + ' 轮';
        } else if (evt.type === 'plan_update') {
          sessionPlan = Array.isArray(evt.todos) ? evt.todos : [];
          if (streamChat) streamChat._plan = sessionPlan;
          renderPlanList();
        } else if (evt.type === 'context_update') {
          updateContextBar(evt.used, evt.total);
          // 持久化到会话对象 — 切换/重开会话时恢复右栏上下文条
          if (streamChat) streamChat._ctx = { used: evt.used, total: evt.total };
        } else if (evt.type === 'context_compacted') {
          // 上下文压缩持久化：中段历史折叠为摘要，保留最近 2 条消息（约一轮对话）。
          // 服务端每次请求都从 history 重建，前端收缩后下一轮请求自然变小。
          if (streamChat && streamChat.messages.length > 3) {
            const tail = streamChat.messages.slice(-2);
            streamChat.messages = [{ role: 'user', content: '（此前对话历史已自动压缩为摘要）\n' + (evt.summary || '') }].concat(tail);
            saveChats();
          }
        } else if (evt.type === 'file_output') {
          // 智能体产出文件/图片 → 会话内即时卡片 + markdown 链接并入正文（重开会话仍可见）
          sessionOutputs.push({ name: evt.name, url: evt.url, kind: evt.kind, path: evt.path || '' });
          renderInfoOutputs();
          if (isViewingStream() && streamAiEl && evt.url) {
            const mdLink = evt.kind === 'image'
              ? `\n\n![${evt.name}](${evt.url})\n`
              : `\n\n[${evt.name}](${evt.url})\n`;
            streamFullResponse += mdLink;
            streamSegText += mdLink;
            const lastMsg = streamChat && streamChat.messages[streamChat.messages.length - 1];
            if (lastMsg && lastMsg.role === 'assistant') lastMsg.content = streamFullResponse;
            const target = ensureStreamTextEl();
            if (target) {
              target.innerHTML = md(streamSegText);
              renderCodeBlocks(target);
              if (autoScroll) scrollToBottom();
            }
          }
        } else if (evt.type === 'agent_spawn') {
          addAgentNode(evt.id, evt.name, evt.task, !!evt.writable);
        } else if (evt.type === 'agent_status') {
          // state = 机器状态，status = 展示文案，两者不可混用
          updateAgentNode(evt.id, evt.state, evt.status);
        } else if (evt.type === 'agent_end') {
          // 绝不 `evt.status || 'completed'`：状态缺失时默认成「完成」＝向用户
          // 谎报子任务成功，用户会据此决策。缺失/未知一律落到 failed。
          const result = evt.error_message || evt.result;
          updateAgentNode(evt.id, evt.state, '', result, evt.rounds, evt.elapsed_ms);
        }
      } catch(e) {
        streamFullResponse += data;
        streamSegText += data;
        const lastMsg = streamChat.messages[streamChat.messages.length - 1];
        if (lastMsg && lastMsg.role === 'assistant') lastMsg.content = streamFullResponse;
        if (isViewingStream() && streamAiEl) {
          const target = ensureStreamTextEl();
          if (target) {
            target.innerHTML = md(streamSegText);
            renderCodeBlocks(target);
            streamAiEl.classList.add('streaming-cursor');
            if (autoScroll) scrollToBottom();
          }
        }
      }
    }

    refreshRail();
    if (isViewingStream() && streamAiEl) {
      streamAiEl.classList.remove('streaming-cursor');
      const statusLine = streamAiEl.querySelector('.agent-status-line');
      if (statusLine) statusLine.remove();
      const rbox = streamAiEl.querySelector('.reasoning-box');
      if (rbox) {
        rbox.open = false;
        const s = rbox.querySelector('summary');
        if (s) s.innerHTML = '<span class="rb-dot done"></span>思考 · ' +
          (streamReasoningStart ? '持续了 ' + fmtDuration(Date.now() - streamReasoningStart) : '完成');
      }
      // 收尾：清掉消息末尾残留的空文本段
      if (streamTextEl && streamTextEl.parentNode && !streamSegText) streamTextEl.remove();
      streamTextEl = null;
      streamSegText = '';
      if (reasonTimer) { clearInterval(reasonTimer); reasonTimer = null; }
      if (workTimer) { clearInterval(workTimer); workTimer = null; }
      addMessageActions(streamAiEl, streamFullResponse);
      renderMath(streamAiEl);
    }

    const lastMsg = streamChat.messages[streamChat.messages.length - 1];
    if (!lastMsg || lastMsg.role !== 'assistant') {
      streamChat.messages.push({ role: 'assistant', content: streamFullResponse });
    } else {
      lastMsg.content = streamFullResponse;
    }
    lastStreamOutcome = 'done';
    if (streamStartedAt) lastStreamDur = fmtDuration(Date.now() - streamStartedAt);
    // 运行结果/时长随会话持久化 — 切换/重开会话右栏不再显示 —
    if (streamChat) { streamChat._outcome = 'done'; streamChat._dur = lastStreamDur; }
    syncRunState();
    saveChats();

    // 计划模式审批流（ExitPlanMode 简化版）：计划产出 → 用户批准 → 自动切 auto 执行
    if (settings.executionMode === 'plan' && streamFullResponse.trim() && streamAiEl && isViewingStream()) {
      appendPlanApprovalBar(streamAiEl, streamFullResponse);
    }

  } catch (error) {
    if (isViewingStream() && workingEl && workingEl.parentNode) workingEl.remove();
    if (error.name === 'AbortError') {
      // 中止后收束真实状态：运行中的工具卡 → 已取消（非伪造，是用户主动取消的事实）
      if (isViewingStream() && streamAiEl) {
        streamAiEl.querySelectorAll('.tool-line-status.running').forEach(st => {
          st.className = 'tool-line-status err';
          st.textContent = '✕ 已取消';
        });
        // 中止时收束思维链盒 — 否则「思考中…」摘要与计时残留显示（假运行态）
        const abox = streamAiEl.querySelector('.reasoning-box');
        if (abox) {
          abox.open = false;
          const asum = abox.querySelector('summary');
          if (asum) asum.innerHTML = '<span class="rb-dot done"></span>思考 · ' +
            (streamReasoningStart ? '持续了 ' + fmtDuration(Date.now() - streamReasoningStart) + '（已中止）' : '已中止');
        }
      }
      lastStreamOutcome = null; // 用户主动取消：不算失败也不算完成
      syncRunState();
      if (isViewingStream() && streamAiEl) {
        streamAiEl.classList.remove('streaming-cursor');
        if (streamFullResponse) { addMessageActions(streamAiEl, streamFullResponse); }
        else { streamAiEl.remove(); }
      }
      const lastMsg = streamChat.messages[streamChat.messages.length - 1];
      if (lastMsg && lastMsg.role === 'assistant') {
        if (streamFullResponse) { lastMsg.content = streamFullResponse; }
        else { streamChat.messages.pop(); }
      }
      saveChats();
    } else {
      lastStreamOutcome = 'error';
      if (streamStartedAt) lastStreamDur = fmtDuration(Date.now() - streamStartedAt);
      if (streamChat) { streamChat._outcome = 'error'; streamChat._dur = lastStreamDur; }
      syncRunState();
      if (isViewingStream()) addMessage('system', 'Error: ' + error.message);
    }
  }

  setStreaming(false);
  if (workTimer) { clearInterval(workTimer); workTimer = null; }
  if (reasonTimer) { clearInterval(reasonTimer); reasonTimer = null; }
  abortController = null;
  streamChat = null;
  streamAiEl = null;
  streamFullResponse = '';
  streamTextEl = null;
  streamSegText = '';
  renderChatList();
  const inp = el('input'); if (inp) inp.focus();
}

function setStreaming(on) {
  isStreaming = on;
  const sb = el('send-btn'); if (sb) sb.classList.toggle('stopping', on);
  const sd = el('status-dot'); if (sd) { sd.className = on ? 'status-dot thinking' : 'status-dot'; sd.title = on ? '生成中' : '空闲'; sd.setAttribute('aria-label', sd.title); }
  const as = el('agent-status'); if (as) as.textContent = on ? '工作中' : '空闲';
  syncRunState(); // 右栏运行状态随流式生命周期实时切换
  // 右栏「时长」实时跳动 — 旧实现仅在流结束时一次性赋值，生成期间一直显示 —
  if (on) {
    if (!runDurTimer) {
      runDurTimer = setInterval(() => {
        const d = el('info-run-dur');
        if (d && streamStartedAt) d.textContent = fmtDuration(Date.now() - streamStartedAt);
      }, 1000);
    }
  } else if (runDurTimer) {
    clearInterval(runDurTimer);
    runDurTimer = null;
  }
  // 流结束补排挂起的会话同步（见 scheduleWorkspaceSync 的 isStreaming 挂起注释）
  if (!on && wsPendingSync) {
    wsPendingSync = false;
    scheduleWorkspaceSync();
  }
}

// ─── 思考强度单选菜单 ───
const EFFORT_DESC = {
  low: '简洁回答，不深度推理',
  medium: '平衡速度与质量',
  high: '深度推理，逐步思考',
  max: '最深入分析每个步骤',
  ultra: '穷举方案与风险，慎重执行（慢）',
  off: '关闭思考，直接作答',
};
function openEffortMenu(anchor) {
  closeEffortMenu();
  closeSlashMenu();
  const menu = document.createElement('div');
  menu.className = 'effort-menu';
  menu.innerHTML = '<div class="effort-menu-title">思考强度</div>' +
    EFFORT_MODES.map(m => {
      const active = settings.thinkingEffort === m;
      return '<button class="effort-menu-item' + (active ? ' active' : '') + '" data-effort="' + m + '">' +
        '<span class="effort-check">' + (active ? '✓' : '') + '</span>' +
        '<span class="effort-name">' + (EFFORT_LABELS[m] || m) + '</span>' +
        '<span class="effort-desc">' + (EFFORT_DESC[m] || '') + '</span></button>';
    }).join('');
  document.body.appendChild(menu);
  // 视口感知定位：默认向下；下方放不下且上方空间更大时向上翻转，双轴钳制在窗口内
  const rect = anchor.getBoundingClientRect();
  const menuH = menu.offsetHeight || 220;
  const menuW = 240;
  const spaceBelow = window.innerHeight - rect.bottom;
  const spaceAbove = rect.top;
  let top;
  if (spaceBelow >= menuH + 12 || spaceBelow >= spaceAbove) {
    top = rect.bottom + 6;
  } else {
    top = rect.top - menuH - 6;
  }
  top = Math.max(8, Math.min(top, window.innerHeight - menuH - 8));
  menu.style.top = top + 'px';
  menu.style.left = Math.max(8, Math.min(rect.left, window.innerWidth - menuW - 8)) + 'px';
  const close = () => { menu.remove(); document.removeEventListener('click', onDoc); };
  const onDoc = (e) => { if (!menu.contains(e.target)) close(); };
  setTimeout(() => document.addEventListener('click', onDoc), 0);
  menu.querySelectorAll('.effort-menu-item').forEach(item => {
    item.addEventListener('click', () => {
      settings.thinkingEffort = item.dataset.effort;
      updateCtrlButtons(); saveSettings(); updateInfoBar(); updateComposerStatus();
      close();
    });
  });
}
function closeEffortMenu() {
  const m = document.querySelector('.effort-menu');
  if (m) m.remove();
}

// ─── 弹出菜单通用定位（视口感知，与 effort 菜单同规则）───
function positionAnchorMenu(menu, anchor) {
  const rect = anchor.getBoundingClientRect();
  const menuH = menu.offsetHeight || 220;
  const menuW = 280;
  const spaceBelow = window.innerHeight - rect.bottom;
  const spaceAbove = rect.top;
  let top;
  if (spaceBelow >= menuH + 12 || spaceBelow >= spaceAbove) {
    top = rect.bottom + 6;
  } else {
    top = rect.top - menuH - 6;
  }
  top = Math.max(8, Math.min(top, window.innerHeight - menuH - 8));
  menu.style.top = top + 'px';
  menu.style.left = Math.max(8, Math.min(rect.left, window.innerWidth - menuW - 8)) + 'px';
}
function bindAnchorMenuClose(menu) {
  const close = () => { menu.remove(); document.removeEventListener('click', onDoc); };
  const onDoc = (e) => { if (!menu.contains(e.target)) close(); };
  setTimeout(() => document.addEventListener('click', onDoc), 0);
  return close;
}

// ─── + 号菜单（图片/文件统一入口）───
function openPlusMenu(anchor) {
  closeEffortMenu();
  closeSlashMenu();
  const menu = document.createElement('div');
  menu.className = 'effort-menu';
  menu.innerHTML = '<div class="effort-menu-title">添加</div>' +
    '<button class="effort-menu-item" data-plus="image"><span class="effort-check"></span>' +
    '<span class="effort-name">图片</span><span class="effort-desc">选择或粘贴，最多 6 张</span></button>' +
    '<button class="effort-menu-item" data-plus="file"><span class="effort-check"></span>' +
    '<span class="effort-name">文件</span><span class="effort-desc">代码/文本，最多 10 个</span></button>';
  document.body.appendChild(menu);
  positionAnchorMenu(menu, anchor);
  const close = bindAnchorMenuClose(menu);
  menu.querySelectorAll('.effort-menu-item').forEach(item => {
    item.addEventListener('click', () => {
      const fi = el(item.dataset.plus === 'image' ? 'chat-file-input' : 'chat-attachment-input');
      if (fi) fi.click();
      close();
    });
  });
}

// ─── 访问模式单选菜单（计划/只读/编辑/自动）───
const EXEC_DESC = {
  auto: '自主执行 · 安全自评，不安全才弹窗询问',
  edit: '读写与命令，全部需你逐项授权',
  full: '全部权限放开，无审批（危险）',
  plan: '只读分析并输出计划，批准后执行',
  readonly: '仅读取分析，禁止任何修改',
};
function openExecMenu(anchor) {
  closeEffortMenu();
  closeSlashMenu();
  const menu = document.createElement('div');
  menu.className = 'effort-menu';
  menu.innerHTML = '<div class="effort-menu-title">访问模式</div>' +
    EXEC_MODES.map(m => {
      const active = settings.executionMode === m;
      return '<button class="effort-menu-item' + (active ? ' active' : '') + (m === 'full' ? ' danger' : '') + '" data-exec="' + m + '">' +
        '<span class="effort-check">' + (active ? '✓' : '') + '</span>' +
        '<span class="effort-name">' + (EXEC_LABELS[m] || m) + '</span>' +
        '<span class="effort-desc">' + (EXEC_DESC[m] || '') + '</span></button>';
    }).join('');
  document.body.appendChild(menu);
  positionAnchorMenu(menu, anchor);
  const close = bindAnchorMenuClose(menu);
  menu.querySelectorAll('.effort-menu-item').forEach(item => {
    item.addEventListener('click', () => {
      settings.executionMode = item.dataset.exec;
      updateCtrlButtons(); saveSettings(); updateInfoBar();
      close();
    });
  });
}

// ─── 斜杠技能菜单（"/" 触发，手动引入技能）───
let skillsCache = null; // [{name, title, description}]
async function ensureSkillsCache() {
  if (skillsCache !== null) return;
  try {
    const r = await fetch('/api/skills/list');
    const d = JSON.parse(await r.text());
    skillsCache = (d && Array.isArray(d.skills)) ? d.skills : [];
  } catch (e) { skillsCache = []; }
}
let slashMenuEl = null;
let slashMenuDocHandler = null; // 斜杠菜单的 document 关闭监听 — 重建时先移除防泄漏
let slashMenuIndex = 0;        // 当前键盘选中项索引（默认 0 = 匹配度最高项）
let slashMenuItems = [];       // 菜单内的 [data-skill] 按钮列表（键盘导航用）
function closeSlashMenu() {
  if (slashMenuEl) { slashMenuEl.remove(); slashMenuEl = null; }
  if (slashMenuDocHandler) { document.removeEventListener('click', slashMenuDocHandler); slashMenuDocHandler = null; }
  slashMenuIndex = 0;
  slashMenuItems = [];
}
function slashMenuHighlight() {
  slashMenuItems.forEach((item, i) => item.classList.toggle('slash-active', i === slashMenuIndex));
  const sel = slashMenuItems[slashMenuIndex];
  if (sel) sel.scrollIntoView({ block: 'nearest' });
}
function slashMenuAccept() {
  const item = slashMenuItems[slashMenuIndex];
  if (!item) { closeSlashMenu(); return; }
  const inp = el('input');
  if (inp) {
    inp.value = inp.value.replace(/^\/[a-z0-9_-]*$/i, '/' + item.dataset.skill + ' ');
    inp.focus();
    inp.dispatchEvent(new Event('input'));
  }
  closeSlashMenu();
}
function openSlashMenu(anchor, filter) {
  closeSlashMenu();
  const skills = (skillsCache || []).filter(s =>
    !filter || s.name.includes(filter) || (s.title || '').includes(filter));
  const menu = document.createElement('div');
  menu.className = 'effort-menu slash-menu';
  if (!skills.length) {
    menu.innerHTML = '<div class="effort-menu-title">技能</div><div class="slash-menu-empty">没有匹配的技能</div>';
  } else {
    menu.innerHTML = '<div class="effort-menu-title">引入技能 <span style="font-size:10px;color:var(--text-muted);font-weight:400;margin-left:auto">↑↓ 切换 · Tab 选中</span></div>' +
      skills.slice(0, 12).map(s =>
        '<button class="effort-menu-item" data-skill="' + escapeHtml(s.name) + '">' +
        '<span class="effort-check">/</span>' +
        '<span class="effort-name">' + escapeHtml(s.title || s.name) + '</span>' +
        '<span class="effort-desc">' + escapeHtml(s.name) + '</span></button>'
      ).join('');
  }
  document.body.appendChild(menu);
  positionAnchorMenu(menu, anchor);
  slashMenuEl = menu;
  slashMenuIndex = 0;
  slashMenuItems = Array.from(menu.querySelectorAll('[data-skill]'));
  slashMenuHighlight();
  const onDoc = (e) => { if (!menu.contains(e.target)) closeSlashMenu(); };
  slashMenuDocHandler = onDoc;
  setTimeout(() => document.addEventListener('click', onDoc), 0);
  menu.querySelectorAll('[data-skill]').forEach((item, idx) => {
    item.addEventListener('click', (e) => {
      e.stopPropagation();
      slashMenuIndex = idx;
      slashMenuAccept();
    });
    item.addEventListener('mouseenter', () => {
      slashMenuIndex = idx;
      slashMenuHighlight();
    });
  });
}
async function handleSlashInput() {
  const inp = el('input');
  if (!inp) return;
  const m = inp.value.match(/^\/([a-z0-9_-]*)$/i);
  if (m) {
    await ensureSkillsCache();
    closeSlashMenu(); // 过滤词变化 → 重建菜单
    openSlashMenu(inp, m[1].toLowerCase());
  } else {
    closeSlashMenu();
  }
}

function updateQueueIndicator() {
  const qi = el('queue-indicator'); if (!qi) return;
  if (messageQueue.length > 0) {
    qi.style.display = 'block';
    qi.innerHTML = messageQueue.map((text, i) =>
      '<div class="queue-row"><span class="queue-dot"></span><span class="queue-text">' + escapeHtml(text.slice(0, 60)) + '</span>' +
      '<button class="queue-send" data-qi="' + i + '" title="立即执行">立即</button>' +
      '<button class="queue-del" data-qd="' + i + '" title="移除">✕</button></div>'
    ).join('');
    qi.querySelectorAll('.queue-send').forEach(btn => {
      btn.addEventListener('click', () => {
        const idx = parseInt(btn.dataset.qi);
        const text = messageQueue.splice(idx, 1)[0];
        updateQueueIndicator();
        if (isStreaming) {
          // 指示器只在流式期间可见 —— 此时「立即」必然撞上流：回插而非吞掉
          // （曾发生：消息被移除却永不发送，零提示）
          messageQueue.splice(Math.min(idx, messageQueue.length), 0, text);
          updateQueueIndicator();
          if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('当前回复生成中，该消息仍在队列', 'error');
          return;
        }
        if (text) doSend(text);
      });
    });
    qi.querySelectorAll('.queue-del').forEach(btn => {
      btn.addEventListener('click', () => {
        messageQueue.splice(parseInt(btn.dataset.qd), 1);
        updateQueueIndicator();
      });
    });
  } else { qi.style.display = 'none'; qi.innerHTML = ''; }
}

// ════════════════════════════════════════════
// Agent Tree
// ════════════════════════════════════════════
let agentNodes = [];
// id = 后端事件唯一标识（tool_call_id），防同名子代理状态互相覆盖；name 仅做回退键（旧事件无 id）
// 子智能体节点：state 为机器可读状态，statusText 仅作展示。
// 后端 agent_status 的 status 是「第N轮 · read_file、grep」这类可变文案；
// 早期前端靠字符串猜状态，文案一改就会静默误判成「执行中」，故逻辑一律走 state。
const AGENT_STATE_LABEL = { running: '执行中', completed: '完成', failed: '失败', cancelled: '已取消' };

function addAgentNode(id, name, task, writable) {
  agentNodes.push({ id: id || name, name: name || '', task: task || '', writable: !!writable, state: 'running', statusText: '', result: '', rounds: 0, elapsed: 0 });
  renderAgentTree();
}
function updateAgentNode(id, state, statusText, result, rounds, elapsed) {
  if (id === undefined || id === null || id === '') return;
  let node = agentNodes.find(n => n.id === id);
  if (!node) {
    // 仅当同名节点**唯一**时才回退到按名字匹配：模型可能派生出两个同名子代理，
    // 此时按名字回退会改到别人的节点上（数据错乱且极难排查）。
    const same = agentNodes.filter(n => n.name === id);
    if (same.length === 1) node = same[0];
  }
  if (!node) return;
  const terminal = node.state === 'completed' || node.state === 'failed' || node.state === 'cancelled';
  // 终态不被迟到的 status 事件覆盖（并行派生时事件可能交错到达）
  if (terminal) { if (result !== undefined) node.result = result; return; }
  // 未知状态按 failed 处理，**绝不回退成「完成」** —— 假成功比报错危险得多：
  // 用户会以为子任务已完成并据此决策。
  node.state = AGENT_STATE_LABEL[state] ? state : 'failed';
  if (statusText) node.statusText = statusText;
  if (result !== undefined) node.result = result;
  if (rounds) node.rounds = rounds;
  if (elapsed) node.elapsed = elapsed;
  renderAgentTree();
}
function clearAgentTree() { agentNodes = []; renderAgentTree(); }
function renderAgentTree() {
  const container = el('task-list'); if (!container) return;
  if (agentNodes.length === 0) { container.innerHTML = '<div class="task-empty">暂无子智能体</div>'; return; }
  container.innerHTML = agentNodes.map(n => {
    // 一律以 state 判定，statusText 只作副标题展示
    const st = AGENT_STATE_LABEL[n.state] ? n.state : 'running';
    const sc = st === 'completed' ? 'done' : st;   // done 是历史 class 名，保留兼容既有样式
    const label = AGENT_STATE_LABEL[st] || '执行中';
    // 轮次/耗时：后端一直在发，此前被丢弃 —— 子代理是否真在推进，全看这两个数
    const meta = [];
    if (n.rounds) meta.push(n.rounds + ' 轮');
    if (n.elapsed) meta.push(n.elapsed >= 1000 ? (n.elapsed / 1000).toFixed(1) + 's' : n.elapsed + 'ms');
    const metaHtml = meta.length ? '<span class="agent-meta">' + meta.join(' · ') + '</span>' : '';
    const sub = (n.statusText && st === 'running') ? '<span class="agent-sub">' + escapeHtml(n.statusText) + '</span>' : '';
    const resultHtml = (n.result && st !== 'running') ? '<div class="agent-result" style="font-size:11px;color:var(--text-muted);margin-top:2px;padding-left:24px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap" title="' + escapeHtml(n.result) + '">' + escapeHtml(n.result.slice(0, 120)) + '</div>' : '';
    const writeTag = n.writable ? '<span class="agent-write-tag" title="该子代理可写入文件">可写</span>' : '';
    return '<div class="task-item ' + sc + '"><span class="task-checkbox"></span>' + writeTag + '<span class="task-text">' + escapeHtml(n.name) + ': ' + escapeHtml(n.task) + '</span>' + metaHtml + '<span class="agent-status">' + escapeHtml(label) + '</span>' + sub + resultHtml + '</div>';
  }).join('');
}

// ─── 执行计划（todo_write 工具 → plan_update 事件；区别于用户待办系统）───
// ─── 模型上下文窗口用户覆盖（设置面板「性能与执行」页）───
const K_CTX_OVERRIDE = SkylandStore.KEYS.ctxLength;
const K_COMPACT_MODEL = SkylandStore.KEYS.compactModel;
function getContextLengthOverride() {
  try {
    const v = parseInt(localStorage.getItem(K_CTX_OVERRIDE), 10);
    return (Number.isFinite(v) && v >= 4096) ? v : 0;
  } catch (e) { return 0; }
}
function setContextLengthOverride(v) {
  try {
    if (v && Number.isFinite(v) && v >= 4096) localStorage.setItem(K_CTX_OVERRIDE, String(v));
    else localStorage.removeItem(K_CTX_OVERRIDE);
  } catch (e) {}
  updateContextBar(0, v || 128000);
  updateComposerStatus();
}
function initCtxLengthSettings() {
  const presets = document.querySelectorAll('#ctx-length-presets [data-ctx]');
  if (!presets.length) return;
  const customWrap = el('ctx-custom-wrap');
  const customInput = el('ctx-custom-input');
  const current = getContextLengthOverride();
  // 初始状态：匹配预设高亮，非预设值自动切到"自定义"
  let matched = false;
  presets.forEach(btn => {
    const v = btn.dataset.ctx;
    if (v === 'custom') return;
    if (current === parseInt(v, 10)) { btn.classList.add('active'); matched = true; }
    else btn.classList.remove('active');
  });
  const customBtn = document.querySelector('#ctx-length-presets [data-ctx="custom"]');
  if (!matched && current > 0) {
    if (customBtn) customBtn.classList.add('active');
    if (customWrap) customWrap.style.display = '';
    if (customInput) customInput.value = current;
  } else if (!matched && !current) {
    // 未设置 → 默认 128K 高亮
    const def = document.querySelector('#ctx-length-presets [data-ctx="131072"]');
    if (def) def.classList.add('active');
  } else {
    if (customBtn) customBtn.classList.remove('active');
    if (customWrap) customWrap.style.display = 'none';
  }
  presets.forEach(btn => {
    btn.addEventListener('click', () => {
      presets.forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      const v = btn.dataset.ctx;
      if (v === 'custom') {
        if (customWrap) customWrap.style.display = '';
        if (customInput) { customInput.focus(); }
        // 不立即保存 — 等用户输入合法值后保存
        const existing = getContextLengthOverride();
        if (existing) setContextLengthOverride(existing); // 保持当前生效值直到输入新值
      } else {
        if (customWrap) customWrap.style.display = 'none';
        setContextLengthOverride(parseInt(v, 10));
      }
    });
  });
  if (customInput) {
    customInput.addEventListener('change', () => {
      const v = parseInt(customInput.value, 10);
      if (Number.isFinite(v) && v >= 4096 && v <= 16777216) {
        setContextLengthOverride(v);
      } else {
        customInput.value = '';
        customInput.placeholder = '请输入 4096 ~ 16777216 之间的 token 数';
      }
    });
  }
}

function renderPlanList() {
  const box = el('plan-list');
  if (!box) return;
  if (!sessionPlan.length) { box.innerHTML = '<div class="task-empty">暂无计划</div>'; return; }
  box.innerHTML = sessionPlan.map(t => {
    const isDone = t.status === 'completed';
    const sc = isDone ? 'done' : (t.status === 'in_progress' ? 'running' : '');
    const st = isDone ? '完成' : (t.status === 'in_progress' ? '进行中' : '待办');
    return `<div class="task-item ${sc}"><span class="task-checkbox"></span><span class="task-text">${escapeHtml(t.content || '')}</span><span class="agent-status">${escapeHtml(st)}</span></div>`;
  }).join('');
}

// ─── 计划模式审批条（ExitPlanMode 简化版）：批准 = 临时切 auto 并以计划为指令重发 ───
function appendPlanApprovalBar(aiEl, planText) {
  if (aiEl.querySelector('.plan-approval-bar')) return;
  const bar = document.createElement('div');
  bar.className = 'plan-approval-bar';
  bar.innerHTML =
    '<button class="plan-approve-btn">批准并执行</button>' +
    '<button class="plan-dismiss-btn">仅保留计划</button>';
  aiEl.appendChild(bar);
  bar.querySelector('.plan-approve-btn').addEventListener('click', async () => {
    bar.remove();
    const prevMode = settings.executionMode;
    settings.executionMode = 'auto';
    updateInfoBar(); updateCtrlButtons();
    try {
      await doSend('（用户已批准以下计划，请直接执行，并先用 todo_write 建立执行清单逐步跟踪进度）\n\n' + planText);
    } finally {
      settings.executionMode = prevMode;
      updateInfoBar(); updateCtrlButtons();
    }
  });
  bar.querySelector('.plan-dismiss-btn').addEventListener('click', () => bar.remove());
  if (autoScroll) scrollToBottom();
}

// ════════════════════════════════════════════
// Info bar updates
// ════════════════════════════════════════════
function updateContextBar(used, total) {
  const bar = el('context-bar'); if (!bar) return;
  if (!total || total === 0) return;
  const pct = Math.min(100, (used / total) * 100);
  bar.style.width = pct + '%';
  bar.className = 'context-bar' + (pct > 80 ? ' danger' : pct > 50 ? ' warn' : '');
  const cu = el('context-used'); if (cu) cu.textContent = used > 1000 ? (used / 1000).toFixed(1) + 'k' : used;
  const ct = el('context-total'); if (ct) ct.textContent = total > 1000 ? (total / 1000).toFixed(0) + 'k' : total;
  updateComposerStatus();
}

function updateInfoBar() {
  const im = el('info-mode'); if (im) im.textContent = EXEC_LABELS[settings.executionMode] || 'auto';
  const ie = el('info-effort');
  if (ie) {
    if (effortUnsupported) {
      ie.textContent = '不支持';
      ie.classList.add('effort-warn');
      ie.title = '当前模型不支持思考模式，思考强度设置未生效';
    } else {
      ie.textContent = EFFORT_LABELS[settings.thinkingEffort] || settings.thinkingEffort || 'medium';
      ie.classList.remove('effort-warn');
      ie.title = '';
    }
  }
  const imd = el('info-model'); if (imd) imd.textContent = selectedModel || '—';
  syncRunState();
}

// ─── 右栏运行状态 — 真实 isStreaming + 本次会话流结果（非伪造）───
let lastStreamOutcome = null; // null | 'done' | 'error'
function syncRunState() {
  const rs = el('info-run-state');
  if (!rs) return;
  let text = '空闲', cls = '';
  if (isStreaming) { text = '● 运行中'; cls = 'run-busy'; }
  else if (lastStreamOutcome === 'error') { text = '✕ 失败'; cls = 'run-err'; }
  else if (lastStreamOutcome === 'done') { text = '✓ 已完成'; cls = 'run-ok'; }
  rs.textContent = text;
  rs.className = 'info-value ' + cls;
  const dur = el('info-run-dur');
  if (dur) dur.textContent = lastStreamDur || '—';
}
let lastStreamDur = null;

// ─── 右栏输出内容/来源 — 从真实 tool_end 事件流收集 ───
let sessionOutputs = [];   // 本会话 AI 生成的文件（后端 file_output 事件统一登记，含完整路径）
let sessionSources = [];   // 本会话 web_search 查询
let pendingSearchQuery = '';
function trackToolEvent(name, phase, payload) {
  if (name === 'web_search') {
    if (phase === 'start') {
      try { pendingSearchQuery = (JSON.parse(payload || '{}').query || '').trim(); } catch (e) { pendingSearchQuery = ''; }
    }
  }
  // create_*/write_file/edit_file 的产出由后端 file_output 事件统一登记（含完整路径与下载链接）——
  // 旧前端正则分支与之重复推送（同一文件在面板出现两次），移除
  // bash 产物启发式识别 — 从命令输出提取绝对路径形态的文件（python/脚本生成的文件等，
  // 后端无法感知 bash 创建了什么，靠输出中的路径线索兜底）
  if (phase === 'end' && name === 'bash') {
    const re = /[A-Za-z]:\\[^\s:*?"'<>|]+?\.(?:txt|md|json|csv|py|js|ts|html|css|toml|ya?ml|log|docx|xlsx|pptx|pdf|png|jpe?g|gif|zip|7z|rar|exe|bat|ps1)/gi;
    const found = (payload || '').match(re) || [];
    let added = 0;
    for (const p of found) {
      if (added >= 3) break;
      if (sessionOutputs.some(o => (o.path || '').toLowerCase() === p.toLowerCase())) continue;
      const base = p.replace(/[\\\/]+$/, '').split(/[\\\/]/).pop();
      sessionOutputs.push({ name: base, path: p });
      added++;
    }
    if (added) renderInfoOutputs();
  }
  if (phase === 'end' && name === 'web_search' && (pendingSearchQuery || (payload || '').trim())) {
    sessionSources.push({ query: pendingSearchQuery || (payload || '').trim().slice(0, 40) });
    pendingSearchQuery = '';
    renderInfoSources();
  }
}
function renderInfoOutputs() {
  const box = el('info-outputs');
  if (!box) return;
  if (!sessionOutputs.length) { box.innerHTML = '<div class="task-empty">暂无输出</div>'; return; }
  box.innerHTML = sessionOutputs.slice(-10).map(o => {
    const val = escapeHtml(o.name || '');
    const style = 'text-align:left;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap';
    const inner = o.url
      ? '<a class="info-value" style="' + style + ';color:var(--accent)" href="' + escapeHtml(o.url) + '" target="_blank" rel="noopener">' + val + '</a>'
      : '<span class="info-value" style="' + style + '" title="' + escapeHtml(o.path || o.name || '') + '">' + val + '</span>';
    // 完整保存路径 — "文件在哪"是产出可见性的核心（只有文件名等于没说）
    const pathLine = o.path
      ? '<div class="info-value" style="text-align:left;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:10.5px;color:var(--text-muted);margin-top:1px" title="' + escapeHtml(o.path) + '">' + escapeHtml(o.path) + '</div>'
      : '';
    return '<div class="info-row"><span class="info-label">' + (o.kind === 'image' ? '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="12" height="12" style="vertical-align:-1px"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>' : '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="12" height="12" style="vertical-align:-1px"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>') + '</span><div style="flex:1;min-width:0">' + inner + pathLine + '</div></div>';
  }).join('');
}
function renderInfoSources() {
  const box = el('info-sources');
  if (!box) return;
  if (!sessionSources.length) { box.innerHTML = '<div class="task-empty">暂无来源</div>'; return; }
  box.innerHTML = sessionSources.slice(-10).map(s =>
    '<div class="info-row"><span class="info-label"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="12" height="12" style="vertical-align:-1px"><circle cx="11" cy="11" r="7"/><line x1="21" y1="21" x2="16.5" y2="16.5"/></svg></span>' +
    '<span class="info-value" style="text-align:left;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">' + escapeHtml(s.query || '网络搜索') + '</span></div>'
  ).join('');
}

// ─── Composer 状态行 — 模型/思考/上下文/附件的紧凑元信息 ───
function updateComposerStatus() {
  const cs = el('composer-status');
  if (!cs) return;
  const parts = [];
  if (settings.executionMode === 'full') parts.push('完全模式 · 无审批防护');
  if (selectedModel) parts.push(selectedModel);
  if (settings.thinkingEffort) parts.push('思考 ' + (EFFORT_LABELS[settings.thinkingEffort] || settings.thinkingEffort));
  const cu = el('context-used'); const ct = el('context-total');
  if (cu && ct) parts.push('上下文 ' + cu.textContent + '/' + ct.textContent);
  const att = pendingAttachments.length + pendingImages.length;
  if (att) parts.push('附件 ' + att);
  cs.textContent = parts.join(' · ');
}

// ─── 顶栏"更多操作"菜单（重命名/导出/删除当前对话 — 均复用已有数据与逻辑）───
function exportCurrentChat() {
  if (!currentChat) return;
  const blob = new Blob([JSON.stringify({ conversations: [currentChat] }, null, 2)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = 'skyland-chat-' + (currentChat.title || currentChat.id) + '.json';
  a.click();
  URL.revokeObjectURL(url);
}
function openChatMoreMenu(anchor) {
  closeEffortMenu();
  closeSlashMenu();
  const menu = document.createElement('div');
  menu.className = 'effort-menu';
  menu.innerHTML = '<div class="effort-menu-title">对话操作</div>' +
    '<button class="effort-menu-item" data-cm="rename"><span class="effort-check"></span><span class="effort-name">重命名对话</span><span class="effort-desc"></span></button>' +
    '<button class="effort-menu-item" data-cm="export"><span class="effort-check"></span><span class="effort-name">导出本对话</span><span class="effort-desc">JSON 文件</span></button>' +
    '<button class="effort-menu-item" data-cm="delete"><span class="effort-check"></span><span class="effort-name">删除对话</span><span class="effort-desc">不可恢复</span></button>';
  document.body.appendChild(menu);
  positionAnchorMenu(menu, anchor);
  const close = bindAnchorMenuClose(menu);
  menu.querySelectorAll('[data-cm]').forEach(item => {
    item.addEventListener('click', () => {
      const act = item.dataset.cm;
      close();
      if (act === 'rename' && currentChat) {
        const name = prompt('重命名对话', currentChat.title || '');
        if (name && name.trim()) {
          currentChat.title = name.trim().slice(0, 60);
          const ct2 = el('chat-title'); if (ct2) ct2.textContent = currentChat.title;
          renderChatList(); saveChats();
        }
      } else if (act === 'export') {
        exportCurrentChat();
      } else if (act === 'delete' && currentChat) {
        if (confirm('删除当前对话？此操作不可恢复。')) deleteChat(currentChat.id, { stopPropagation() {} });
      }
    });
  });
}

// 顶栏右栏开关与既有折叠按钮的状态联动
function syncInfoToggle() {
  const it = el('btn-info-toggle');
  const ib = el('info-bar');
  if (it && ib) it.classList.toggle('active', !ib.classList.contains('collapsed'));
}

// ─── 左栏工作区卡 — 真实工作目录（云工作显示友好文案；不显示会话计数——chats 无工作区归属，避免误导）───
// ─── 输入框工作区行（ChatGPT 式）───
// 仅初始对话可见（无消息时展示，发第一条后隐藏 — 工作区是"初始化设置"，锁定为本会话上下文）
// 交互闭环：点击行 → 打开工作区选择弹窗 → 选择后行内容实时更新 → 对话开始后行隐藏
function updateComposerWsRow() {
  const row = el('composer-ws-row');
  if (!row) return;
  // 仅初始对话（当前会话无消息）时显示
  const isInitial = !currentChat || !currentChat.messages || currentChat.messages.length === 0;
  row.style.display = isInitial ? 'flex' : 'none';
  if (!isInitial) return;
  const label = el('composer-ws-label');
  let wsName = '';
  let wsPath = '';
  try {
    if (typeof Workdir !== 'undefined' && Workdir.value()) {
      const g = Workdir.get();
      wsName = g.cloud ? '云工作' : Workdir.display();
      wsPath = g.cloud ? '@cloud' : g.path;
    }
  } catch (e) { /* Workdir 未初始化时忽略 */ }
  if (label) label.textContent = wsName ? wsName : '选择工作区…';
  row.classList.toggle('set', !!wsName);
  row.title = wsPath ? ('工作区：' + wsPath + '（点击可更换）') : '点击选择工作区 — 智能体将在此目录执行命令、读写文件';
}

// ─── 侧栏分区显隐 ───
// 有工作区 → 显示「工作区树」（内含各工作区的会话 + 最近会话）
// 无工作区 → 显示原本按今天/昨天/更早分组的扁平列表（行为不变，不做无谓改动）
function syncSidebarSections() {
  const tree = el('ws-tree'), treeLabel = el('sb-ws-tree-label');
  const list = el('sb-chats'), listLabel = el('sb-chats-label');
  const grouped = wsActive();
  if (tree) tree.style.display = grouped ? '' : 'none';
  if (treeLabel) treeLabel.style.display = grouped ? '' : 'none';
  if (list) list.style.display = grouped ? 'none' : '';
  if (listLabel) listLabel.style.display = grouped ? 'none' : '';
}

// ─── 清空主会话区（切工作区 / 删除当前会话 共用，此前散落三处重复代码）───
function closeChatView() {
  currentChat = null;
  window.__skylandActiveConvId = null;
  // 会话已被关闭（切换工作区/删除当前会话），子智能体面板同样失去归属，一并清空，
  // 否则会残留上一条会话的子代理，与主区空白状态自相矛盾。
  if (typeof clearAgentTree === 'function') clearAgentTree();
  const msgs = el('messages');
  if (msgs) { msgs.innerHTML = ''; msgs.style.display = 'none'; }
  const wel = el('welcome'); if (wel) wel.style.display = 'flex';
  const ct = el('chat-title'); if (ct) ct.textContent = 'SKYLAND';
}

// ─── 右栏"会话附件" — 从当前对话消息渲染已有附件数据 ───
function renderInfoAttachments() {
  const box = el('info-attachments');
  if (!box) return;
  const items = [];
  (currentChat && Array.isArray(currentChat.messages) ? currentChat.messages : []).forEach(m => {
    (m.images || (m.image ? [m.image] : [])).forEach((img, i) => items.push({ kind: 'image', name: '图片 ' + (i + 1) }));
    (m.attachments || []).forEach(a => items.push({ kind: 'file', name: a.name }));
  });
  if (!items.length) { box.innerHTML = '<div class="task-empty">暂无附件</div>'; return; }
  box.innerHTML = items.slice(0, 20).map(it2 =>
    '<div class="info-row"><span class="info-label">' + (it2.kind === 'image' ? '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="12" height="12" style="vertical-align:-1px"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>' : '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" width="12" height="12" style="vertical-align:-1px"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>') + '</span>' +
    '<span class="info-value" style="text-align:left;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">' + escapeHtml(it2.name) + '</span></div>'
  ).join('') + (items.length > 20 ? '<div class="task-empty">… 共 ' + items.length + ' 项</div>' : '');
}

// ════════════════════════════════════════════
// Image upload helpers
// ════════════════════════════════════════════
// ─── 多图收集与预览 ───
const MAX_IMAGES = 6;
function addPendingImages(list) {
  const next = pendingImages.concat(list).slice(0, MAX_IMAGES);
  pendingImages = next;
  renderImagePreview();
  checkVisionSupport();
  updateComposerStatus();
}
function removePendingImage(idx) {
  pendingImages.splice(idx, 1);
  renderImagePreview();
  if (!pendingImages.length) {
    const warning = el('chat-image-warning');
    if (warning) warning.style.display = 'none';
  }
  updateComposerStatus();
}
function renderImagePreview() {
  const preview = el('chat-image-preview');
  if (!preview) return;
  preview.innerHTML = pendingImages.map((src, i) =>
    '<div class="chat-image-item">' +
    '<img src="' + src + '" alt="待发送图片">' +
    '<button class="chat-image-clear" data-img-idx="' + i + '" title="移除">×</button>' +
    '</div>'
  ).join('');
  preview.style.display = pendingImages.length ? 'flex' : 'none';
  preview.querySelectorAll('.chat-image-clear').forEach(btn => {
    btn.addEventListener('click', () => removePendingImage(Number(btn.dataset.imgIdx)));
  });
}

function handleImageUploader(e) {
  const files = Array.from(e.target.files || []).filter(f => f.type.startsWith('image/'));
  if (!files.length) return;
  const oversized = files.some(f => f.size > 10 * 1024 * 1024);
  if (oversized) { alert('单张图片不能超过 10MB'); return; }
  if (pendingImages.length + files.length > MAX_IMAGES) { alert('一次最多发送 ' + MAX_IMAGES + ' 张图片'); return; }
  Promise.all(files.map(file => new Promise(resolve => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result || '';
      resolve(typeof result === 'string' && result.includes(',') ? result : 'data:image/png;base64,' + result);
    };
    reader.readAsDataURL(file);
  }))).then(list => { addPendingImages(list); });
  e.target.value = '';
}

function handleAttachmentUploader(e) {
  const files = Array.from(e.target.files || []);
  if (!files.length) return;
  const allowed = /\.(txt|md|markdown|json|yaml|yml|toml|rs|js|ts|tsx|jsx|css|html|xml|py|go|java|cpp|h|hpp|cs|sql|sh|bat|ps1)$/i;
  const selected = files.filter(file => allowed.test(file.name) && file.size <= 1024 * 1024).slice(0, 10);
  Promise.all(selected.map(file => file.text().then(content => ({ name: file.webkitRelativePath || file.name, content })))).then(items => {
    pendingAttachments = items;
    const label = el('chat-attachment-label');
    if (label) label.textContent = items.length ? `文件 ${items.length}` : '文件';
    const preview = el('chat-attachment-preview');
    if (preview) { preview.textContent = items.map(item => item.name).join('、'); preview.style.display = items.length ? 'block' : 'none'; }
    updateComposerStatus();
  }).catch(() => {});
  e.target.value = '';
}

function clearPendingAttachments() {
  pendingAttachments = [];
  const label = el('chat-attachment-label'); if (label) label.textContent = '文件';
  const preview = el('chat-attachment-preview'); if (preview) { preview.textContent = ''; preview.style.display = 'none'; }
  updateComposerStatus();
}

function handlePaste(e) {
  const items = e.clipboardData ? e.clipboardData.items : null;
  if (!items) return;
  // 收集剪贴板中的全部图片（多图粘贴），一次读入
  const imageFiles = [];
  for (const item of items) {
    if (item.type.startsWith('image/')) {
      const file = item.getAsFile();
      if (file) imageFiles.push(file);
    }
  }
  if (!imageFiles.length) return;
  if (pendingImages.length + imageFiles.length > MAX_IMAGES) { alert('一次最多发送 ' + MAX_IMAGES + ' 张图片'); return; }
  Promise.all(imageFiles.map(file => new Promise(resolve => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.readAsDataURL(file);
  }))).then(list => { addPendingImages(list); });
}

// Known vision-capable model keywords — if model name matches any, it supports image input
const VISION_KEYWORDS = ['gpt-4o', 'gpt-4-turbo', 'gpt-4-vision', 'gpt-5', 'claude-3', 'claude-4', 'gemini', 'vl', 'vision', 'llava', 'qwen-vl', 'multimodal', 'omni', 'deepseek-v4', 'deepseek-flash', 'glm-4v', 'glm-5', 'kimi-vl', 'kimi-k3'];
// Known text-only models that will silently ignore images
const TEXT_ONLY_KEYWORDS = ['gpt-3.5', 'text-only', 'deepseek-coder'];

function checkVisionSupport() {
  const model = (selectedModel || '').toLowerCase();
  if (!model) return;
  const isVision = VISION_KEYWORDS.some(k => model.includes(k));
  const isTextOnly = TEXT_ONLY_KEYWORDS.some(k => model.includes(k));
  const warning = el('chat-image-warning');
  if (!warning) return;
  // 每个模型只自动提示一次（用户可关闭）；已确认支持/关闭过的不再打扰
    const ackKey = SkylandStore.KEYS.visionAckPrefix + model;
  if (isTextOnly && !isVision && !localStorage.getItem(ackKey)) {
    warning.innerHTML = '<span class="vision-warn-text">当前模型 ' + escapeHtml(selectedModel) +
      ' 可能不支持图片识别，建议使用多模态模型（如 deepseek-v4-flash / gpt-4o）</span>' +
      '<button class="vision-warn-close" id="vision-warn-close" title="不再提示">✕</button>';
    warning.style.display = 'flex';
    const closeBtn = document.getElementById('vision-warn-close');
    if (closeBtn) closeBtn.onclick = () => {
      localStorage.setItem(ackKey, '1');
      warning.style.display = 'none';
    };
  } else {
    warning.style.display = 'none';
  }
}

function clearPendingImage() {
  pendingImages = [];
  const preview = el('chat-image-preview');
  if (preview) { preview.innerHTML = ''; preview.style.display = 'none'; }
  const warning = el('chat-image-warning');
  if (warning) warning.style.display = 'none';
  const fileInput = el('chat-file-input');
  if (fileInput) fileInput.value = '';
}

// ════════════════════════════════════════════
// Event registrations — wrapped in init
// ════════════════════════════════════════════
// ─── Provider presets + existing config loader ───
let presetsLoaded = false;
// 编辑状态：非 null = 正在修改该 toml key 对应的配置
let editingProviderKey = null;
// 最近一次拉取的已配置列表（「修改」按 key 从这里回填）
let providerListCache = [];

function providerStatus(msg) {
  const ps = el('provider-status');
  if (ps) ps.textContent = msg;
}

function loadPresets() {
  // 每次打开设置都刷新已配置列表（保存/删除/重启前状态可能已变）
  loadExistingProviders();
  if (presetsLoaded) return;
  // Load presets
  fetch('/api/providers/presets')
    .then(r => r.text())
    .then(text => { const data = JSON.parse(text);
      if (!data || !data.presets) return;
      const sel = el('provider-preset');
      if (!sel) return;
      sel.innerHTML = '<option value="">-- 选择预设厂商 --</option>' +
        data.presets.map(p => {
          const source = p.source === 'runtime' ? '实时发现' :
            p.source === 'directory' ? '目录预设' :
            p.source === 'manual' ? '自定义' : '未实时验证';
          return `<option value="${escapeHtml(p.base_url || '')}" data-model="${escapeHtml(p.default_model || '')}" data-source="${escapeHtml(p.source || '')}">${escapeHtml(p.name)} · ${source}</option>`;
        }).join('');
      presetsLoaded = true;
      if (settings.providerPreset) sel.value = settings.providerPreset;
    })
    .catch(() => {});
}

function loadExistingProviders() {
  fetch('/api/providers/config')
    .then(r => r.text())
    .then(text => { const data = JSON.parse(text);
      const container = el('existing-providers');
      if (!container) return;
      if (!data.providers || data.providers.length === 0) {
        providerListCache = [];
        container.innerHTML = '<div style="padding:8px 0">尚未配置任何模型</div>';
        return;
      }
      providerListCache = data.providers;
      const providerRows = data.providers.map(p => {
        const def = p.is_default ? ' <span style="color:var(--accent);font-weight:600">[默认]</span>' : '';
        const full = p.is_full_url ? ' · 完整URL' : '';
        return `<div style="padding:6px 8px;margin-bottom:4px;background:var(--bg-elevated);border-radius:var(--r-sm);border:1px solid var(--border-subtle)">
          <div style="display:flex;justify-content:space-between;align-items:center;gap:6px">
            <span style="font-weight:500;color:var(--text-secondary);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${escapeHtml(p.name)}${def}</span>
            <span style="display:flex;gap:6px;align-items:center;flex-shrink:0">
              <span style="font-size:11px;color:var(--text-muted)">${escapeHtml(p.api_key_masked)}</span>
              <button class="prov-act" data-act="edit" data-key="${escapeHtml(p.name)}" title="修改此配置" style="padding:2px 8px;font-size:11px;border:1px solid var(--border-subtle);border-radius:var(--r-sm);background:transparent;color:var(--text-secondary);cursor:pointer">修改</button>
              <button class="prov-act" data-act="del" data-key="${escapeHtml(p.name)}" title="删除此配置" style="padding:2px 8px;font-size:11px;border:1px solid var(--border-subtle);border-radius:var(--r-sm);background:transparent;color:var(--danger,#e55);cursor:pointer">删除</button>
            </span>
          </div>
          <div style="font-size:11px;color:var(--text-muted);margin-top:2px">
            ${escapeHtml(p.base_url || '(空)')} · ${escapeHtml(p.default_model || '(空)')}${full}
          </div>
        </div>`;
      }).join('');
      // 网络搜索状态行：与列表**一起**拼进同一次 innerHTML 赋值。
      // 原实现先赋 innerHTML → 绑定监听 → 再 `innerHTML +=` 追加搜索行；
      // 而 `innerHTML +=` 会重新解析整段 HTML，把刚绑定的节点连同监听器一起销毁，
      // 表现为「点【修改】毫无反应」——正是用户报的原始症状。
      const searchRow = data.search_configured
        ? `<div style="padding:6px 8px;margin-top:8px;background:var(--bg-elevated);border-radius:var(--r-sm);border:1px solid var(--border-subtle)">
          <div style="display:flex;justify-content:space-between;align-items:center">
            <span style="font-weight:500;color:var(--text-secondary)">网络搜索 (Tavily)</span>
            <span style="font-size:11px;color:var(--success)">已配置 ${escapeHtml(data.search_api_key_masked || '')}</span>
          </div>
        </div>`
        : '';
      container.innerHTML = providerRows + searchRow;
      container.querySelectorAll('.prov-act').forEach(btn => {
        btn.addEventListener('click', () => {
          const key = btn.dataset.key || '';
          if (btn.dataset.act === 'edit') startEditProvider(key);
          else deleteProvider(key);
        });
      });
    })
    .catch(() => {});
}

// ─── 已配置项的修改 / 删除 / 重启交互 ───
function startEditProvider(key) {
  const p = providerListCache.find(x => x.name === key);
  if (!p) { providerStatus('配置已不存在，列表已刷新，请重试'); loadExistingProviders(); return; }
  editingProviderKey = key;
  const listPage = el('provider-list-page');
  const editPage = el('provider-edit-page');
  if (listPage) listPage.style.display = 'none';
  if (editPage) editPage.style.display = '';
  const title = el('provider-edit-title');
  if (title) title.textContent = `修改模型配置：${p.name}`;
  const pn = el('provider-name'); if (pn) pn.value = p.name;
  const bu = el('provider-baseurl'); if (bu) bu.value = p.base_url || '';
  const mn = el('provider-model-name'); if (mn) mn.value = p.default_model || '';
  const cl = el('provider-context-length'); if (cl) cl.value = p.context_length || '';
  const pfu = el('provider-fullurl');
  if (pfu) { pfu.checked = !!p.is_full_url; settings.providerFullUrl = pfu.checked; saveSettings(); }
  const ak = el('provider-apikey');
  if (ak) { ak.value = ''; ak.placeholder = p.has_key ? `留空保持不变（当前 ${p.api_key_masked}）` : 'API Key'; }
  const saveBtn = el('btn-save-provider'); if (saveBtn) saveBtn.textContent = '保存修改';
  const cancelBtn = el('btn-cancel-edit'); if (cancelBtn) cancelBtn.style.display = 'block';
  const content = el('settings-content'); if (content) content.scrollTop = 0;
  providerStatus(`正在修改「${key}」的配置 — 保存后需重启生效`);
}

function cancelEditProvider() {
  editingProviderKey = null;
  const listPage = el('provider-list-page');
  const editPage = el('provider-edit-page');
  if (listPage) listPage.style.display = '';
  if (editPage) editPage.style.display = 'none';
  const pn = el('provider-name'); if (pn) pn.value = '';
  const ak = el('provider-apikey'); if (ak) { ak.value = ''; ak.placeholder = 'API Key'; }
  const bu = el('provider-baseurl'); if (bu) bu.value = '';
  const mn = el('provider-model-name'); if (mn) mn.value = '';
  const cl = el('provider-context-length'); if (cl) cl.value = '';
  const title = el('provider-edit-title'); if (title) title.textContent = '添加模型配置';
  const saveBtn = el('btn-save-provider'); if (saveBtn) saveBtn.textContent = '保存';
  const cancelBtn = el('btn-cancel-edit'); if (cancelBtn) cancelBtn.style.display = 'none';
}

function startAddProvider() {
  cancelEditProvider();
  const listPage = el('provider-list-page');
  const editPage = el('provider-edit-page');
  if (listPage) listPage.style.display = 'none';
  if (editPage) editPage.style.display = '';
  const cancelBtn = el('btn-cancel-edit');
  if (cancelBtn) cancelBtn.style.display = 'block';
  providerStatus('填写配置后点击保存');
  const pn = el('provider-name');
  if (pn) pn.focus();
}

function deleteProvider(key) {
  if (!key) return;
  if (!confirm(`确定删除配置「${key}」吗？\n删除后需重启应用才会生效。`)) return;
  providerStatus('删除中...');
  fetch('/api/providers/delete', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ name: key }) })
    .then(r => r.text().then(t => ({ ok: r.ok, text: t })))
    .then(({ ok, text }) => {
      let d;
      try { d = JSON.parse(text); } catch (e) { providerStatus('删除失败: ' + text.slice(0, 150)); return; }
      if (ok && d && d.status === 'ok') {
        if (editingProviderKey === key) cancelEditProvider();
        loadExistingProviders();
        showRestartRow();
        providerStatus(d.message || '已删除');
      } else {
        providerStatus((d && d.message) || '删除失败: HTTP ' + (ok ? '未知' : '错误'));
      }
    })
    .catch(e => providerStatus('删除失败: ' + e.message));
}

function showRestartRow() {
  const row = el('provider-restart-row');
  if (row) row.style.display = 'flex';
}

function init() {
  if (window._skylandChatInited) return;
  window._skylandChatInited = true;
  // CRITICAL: Load saved data FIRST, before any rendering or event binding
  loadSettings();
  if (window.SkylandSettings) {
    window.SkylandSettings.subscribe(() => {
      settings = { ...settings, ...window.SkylandSettings.snapshotFlat() };
      applyTheme();
    });
    window.SkylandSettings.load();
  }
  // 会话来源二选一：工作区激活 → .skyland/conversations（服务端）；否则 localStorage
  if (wsActive()) {
    loadWorkspaceChats().then(ok => { if (!ok) loadChats(); renderChatList(); }).catch(() => { loadChats(); });
  } else {
    loadChats();
  }
  applyTheme();
  if (typeof Workdir !== 'undefined') {
    Workdir.init();
    // 工作区切换 → 会话列表来源切换 + 视图重置（新工作区新会话集）
    Workdir.onChange(() => {
      sessionPlan = [];
      sessionOutputs = []; sessionSources = [];
      renderPlanList(); renderInfoOutputs(); renderInfoSources();
      closeChatView();
      if (wsActive()) {
        loadWorkspaceChats().then(ok => { if (!ok) loadChats(); renderChatList(); });
      } else { loadChats(); renderChatList(); }
      updateComposerWsRow();
    });
    // 工作区行点击 → 打开选择弹窗（交互闭环入口）
    const wsRow = el('composer-ws-row');
    if (wsRow) {
      wsRow.addEventListener('click', () => { if (typeof Workdir !== 'undefined') Workdir.showDialog(); });
      wsRow.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); if (typeof Workdir !== 'undefined') Workdir.showDialog(); } });
    }
    updateComposerWsRow();  // 初始渲染（视图首次进入时）
    // 工作区会话树 — 侧栏「工作区 → 会话」层级
    if (window.SkylandWsTree) window.SkylandWsTree.init();
  }

  // Model selector
  const ms = el('model-select');
  if (ms) ms.addEventListener('change', () => { selectedModel = ms.value; localStorage.setItem(K_MODEL, selectedModel); updateComposerStatus(); updateInfoBar(); });

  // Send（流式期间点击 = 停止按钮 → fromStopButton=true 走 abort 分支）
  const sb = el('send-btn');
  if (sb) sb.addEventListener('click', () => sendMessage(true));

  // + 号 — 图片/文件统一入口（弹出菜单选择）
  const plusBtn = el('btn-plus');
  if (plusBtn) plusBtn.addEventListener('click', (e) => { e.stopPropagation(); openPlusMenu(plusBtn); });
  const fileInput = el('chat-file-input');
  if (fileInput) fileInput.addEventListener('change', handleImageUploader);
  const attachmentInput = el('chat-attachment-input');
  if (attachmentInput) attachmentInput.addEventListener('change', handleAttachmentUploader);

  // Paste image support
  const inpPaste = el('input');
  if (inpPaste) inpPaste.addEventListener('paste', handlePaste);

  // Input
  const inp = el('input');
  if (inp) {
    inp.addEventListener('keydown', e => {
      // IME 组词中的回车是确认候选词，不是发送
      if (e.isComposing || e.keyCode === 229) return;
      // 斜杠菜单键盘导航：↑↓ 切换选中项 · Tab/Enter 选中 · Esc 关闭
      if (slashMenuEl && slashMenuItems.length > 0) {
        if (e.key === 'ArrowDown') {
          e.preventDefault();
          slashMenuIndex = (slashMenuIndex + 1) % slashMenuItems.length;
          slashMenuHighlight();
          return;
        }
        if (e.key === 'ArrowUp') {
          e.preventDefault();
          slashMenuIndex = (slashMenuIndex - 1 + slashMenuItems.length) % slashMenuItems.length;
          slashMenuHighlight();
          return;
        }
        if (e.key === 'Tab' || (e.key === 'Enter' && !e.ctrlKey)) {
          e.preventDefault();
          slashMenuAccept();
          return;
        }
        if (e.key === 'Escape') {
          e.preventDefault();
          closeSlashMenu();
          return;
        }
      }
      if (e.key === 'Enter' && !e.ctrlKey) { e.preventDefault(); sendMessage(); }
    });
    inp.addEventListener('input', () => {
      inp.style.height = 'auto'; inp.style.height = Math.min(inp.scrollHeight, 240) + 'px';
      handleSlashInput();
    });
  }

  // New chat
  const nb = el('btn-new-chat');
  if (nb) nb.addEventListener('click', () => { if (!isStreaming) { createNewChat(); const i = el('input'); if (i) i.focus(); } });

  // Menu
  const mb = el('btn-menu');
  if (mb) mb.addEventListener('click', () => { const s = el('sidebar'); if (s) s.classList.toggle('open'); const o = el('sidebar-overlay'); if (o) o.classList.toggle('active'); });

  const so = el('sidebar-overlay');
  if (so) so.addEventListener('click', () => { const s = el('sidebar'); if (s) s.classList.remove('open'); so.classList.remove('active'); });

  // 左栏图标收缩（桌面）— 状态持久化
  const railBtn = el('btn-sidebar-rail');
  const sbEl = el('sidebar');
  if (localStorage.getItem(SkylandStore.KEYS.sidebarRail) === '1' && sbEl) sbEl.classList.add('rail');
  if (railBtn) {
    const syncRailBtn = () => {
      const collapsed = sbEl && sbEl.classList.contains('rail');
      railBtn.title = collapsed ? '展开侧栏' : '收起侧栏';
      railBtn.setAttribute('aria-label', railBtn.title);
    };
    syncRailBtn();
    railBtn.addEventListener('click', () => {
      if (!sbEl) return;
      const collapsed = sbEl.classList.toggle('rail');
      localStorage.setItem(SkylandStore.KEYS.sidebarRail, collapsed ? '1' : '0');
      syncRailBtn();
    });
  }

  // Chat list
  const cl = el('sb-chats');
  if (cl) cl.addEventListener('click', e => {
    const delBtn = e.target.closest('[data-del]');
    if (delBtn) { deleteChat(delBtn.dataset.del, e); return; }
    const item = e.target.closest('[data-id]');
    if (item) { selectChat(Number(item.dataset.id)); const s = el('sidebar'); if (s) s.classList.remove('open'); const o = el('sidebar-overlay'); if (o) o.classList.remove('active'); }
  });

  // 创建工作区 — 与「新对话」同级的第二个主动作（用户要求：创建/工作区/最近 会话 三段式）
  const cws = el('btn-create-ws');
  if (cws) cws.addEventListener('click', () => {
    if (typeof Workdir !== 'undefined' && Workdir.showDialog) Workdir.showDialog();
  });

  // Settings
  const st = el('btn-settings');
  if (st) st.addEventListener('click', () => { const sp = el('settings-panel'); if (sp) sp.classList.add('open'); const s = el('settings-overlay'); if (s) s.classList.add('active'); loadPresets(); });
  const sc = el('btn-settings-close');
  if (sc) sc.addEventListener('click', () => { const sp = el('settings-panel'); if (sp) sp.classList.remove('open'); const s = el('settings-overlay'); if (s) s.classList.remove('active'); });
  const sv = el('settings-overlay');
  if (sv) sv.addEventListener('click', () => { const sp = el('settings-panel'); if (sp) sp.classList.remove('open'); sv.classList.remove('active'); });

  // Web search toggle
  const ws = el('btn-websearch');
  if (ws) ws.addEventListener('click', () => {
    settings.webSearchEnabled = !settings.webSearchEnabled;
    updateCtrlButtons();
    saveSettings();
  });

  // Thinking / Execution — 弹出单选菜单
  const tb = el('btn-thinking');
  if (tb) tb.addEventListener('click', (e) => {
    e.stopPropagation();
    openEffortMenu(tb);
  });
  const eb = el('btn-execution');
  if (eb) eb.addEventListener('click', (e) => {
    e.stopPropagation();
    openExecMenu(eb);
  });

  // Back to home
  const bh = el('btn-back-home');
  if (bh) bh.addEventListener('click', () => Router.navigate('/home'));
  // Agent 终端 → 主页（实验室抽屉叠加弹出）
  const cb = el('btn-chat-back');
  if (cb) cb.addEventListener('click', () => { if (window.SkylandOpenLab) window.SkylandOpenLab(); else Router.navigate('/home'); });

  // Export conversations
  const be = el('btn-export-chats');
  if (be) be.addEventListener('click', async () => {
    try {
      const r = await fetch('/api/conversations/export');
      const blob = await r.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'skyland-conversations-' + new Date().toISOString().slice(0,10) + '.json';
      a.click();
      URL.revokeObjectURL(url);
    } catch(e) { if (typeof showKBAlert === 'function') showKBAlert('导出失败: ' + e.message); else alert('导出失败: ' + e.message); }
  });

  // Import conversations
  const bi = el('btn-import-chats');
  const fi = el('import-file-input');
  if (bi) bi.addEventListener('click', () => { if (fi) fi.click(); });
  if (fi) fi.addEventListener('change', async () => {
    const file = fi.files[0];
    if (!file) return;
    try {
      const text = await file.text();
      const r = await fetch('/api/conversations/import', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: text,
      });
      const data = JSON.parse(await r.text());
      if (data.status === 'ok') {
        // Reload conversations from server — 必须逐条拉详情：
        // list 接口的 messages 是「条数」而非数组，直接映射会用空消息壳
        // 覆盖 chats 并经 saveChats 批量清空服务端/本地会话正文（数据丢失事故）
        try {
          const wd = wsActive() ? ('?workdir=' + encodeURIComponent(Workdir.value())) : '';
          const lr = await fetch('/api/conversations/list' + wd);
          const ldata = JSON.parse(await lr.text());
          if (ldata.conversations) {
            chats = await Promise.all(ldata.conversations.map(async (c) => {
              let msgs = [];
              try {
                const rr = await fetch(`/api/conversations/${c.id}${wd}`);
                const dd = await rr.json();
                if (dd && dd.status === 'ok' && Array.isArray(dd.data && dd.data.messages)) {
                  msgs = dd.data.messages.filter(m => m && m.role);
                }
              } catch (e3) {}
              return {
                id: c.id,
                title: c.title || '对话',
                messages: msgs,
              };
            }));
            saveChats();
            renderChatList();
          }
        } catch(e2) { renderChatList(); }
      } else {
        const msg = data.message || '未知错误';
        if (typeof showKBAlert === 'function') showKBAlert('导入失败: ' + msg);
        else alert('导入失败: ' + msg);
      }
    } catch(e) {
      if (typeof showKBAlert === 'function') showKBAlert('导入失败: ' + e.message);
      else alert('导入失败: ' + e.message);
    }
    fi.value = '';
  });

  // Info bar collapse
  const ic = el('btn-info-collapse');
  if (ic) ic.addEventListener('click', () => { const ib = el('info-bar'); if (ib) ib.classList.add('collapsed'); const xe = el('info-expand-btn'); if (xe) xe.classList.add('visible'); syncInfoToggle(); });
  const ie = el('info-expand-btn');
  if (ie) ie.addEventListener('click', () => { const ib = el('info-bar'); if (ib) ib.classList.remove('collapsed'); ie.classList.remove('visible'); syncInfoToggle(); });
  // 顶栏右栏开关 — 与右栏内折叠按钮同一状态源
  const it = el('btn-info-toggle');
  if (it) it.addEventListener('click', (e) => {
    e.stopPropagation();
    const ib = el('info-bar'); if (!ib) return;
    const collapsed = ib.classList.toggle('collapsed');
    const xe = el('info-expand-btn');
    if (xe) xe.classList.toggle('visible', collapsed);
    syncInfoToggle();
  });
  // 顶栏"更多操作"菜单
  const cmore = el('btn-chat-more');
  if (cmore) cmore.addEventListener('click', (e) => { e.stopPropagation(); openChatMoreMenu(cmore); });
  syncInfoToggle();

  // Tool rounds
  document.querySelectorAll('[data-rounds]').forEach(b => { b.addEventListener('click', () => { settings.roundsLimited = b.dataset.rounds === 'on'; updateRoundsUI(); saveSettings(); }); });
  const rs = el('rounds-slider');
  if (rs) rs.addEventListener('input', (e) => { settings.maxToolRounds = parseInt(e.target.value); const rv = el('rounds-value'); if (rv) rv.textContent = settings.maxToolRounds; saveSettings(); });

  // Font size
  document.querySelectorAll('.font-size-btn').forEach(b => { b.addEventListener('click', () => { document.querySelectorAll('.font-size-btn').forEach(x => x.classList.remove('active')); b.classList.add('active'); settings.fontSize = parseInt(b.dataset.size); applyTheme(); saveSettings(); }); });

  // System prompt
  const sp = el('system-prompt');
  if (sp) sp.addEventListener('input', (e) => { settings.systemPrompt = e.target.value; saveSettings(); });

  // Font selector
  const fs = el('font-select');
  if (fs) fs.addEventListener('change', (e) => { settings.font = e.target.value; applyTheme(); saveSettings(); });

  // Theme
  document.querySelectorAll('[data-theme]').forEach(b => { b.addEventListener('click', () => { document.querySelectorAll('[data-theme]').forEach(x => x.classList.remove('active')); b.classList.add('active'); settings.theme = b.dataset.theme; applyTheme(); saveSettings(); }); });

  // Color swatches
  document.querySelectorAll('.color-swatch').forEach(b => { b.addEventListener('click', () => { document.querySelectorAll('.color-swatch').forEach(x => x.classList.remove('active')); b.classList.add('active'); settings.accentColor = b.dataset.color; applyTheme(); saveSettings(); }); });

  // Search config
  const bss = el('btn-save-search');
  if (bss) bss.addEventListener('click', async () => {
    const key = (el('search-apikey')?.value || '').trim();
    if (!key) { const st = el('search-status'); if (st) st.textContent = '请输入 API Key'; return; }
    settings.searchApiKey = key; saveSettings();
    const sst = el('search-status'); if (sst) sst.textContent = '保存中...';
    try {
      const r = await fetch('/api/search/save', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ api_key: key }) });
      if (r.ok) { try { const d = JSON.parse(await r.text()); const sst = el('search-status'); if (sst) sst.textContent = (d && d.message) || '已保存'; } catch(e) { const sst = el('search-status'); if (sst) sst.textContent = '已保存'; } }
      else { const sst = el('search-status'); if (sst) sst.textContent = '保存失败: HTTP ' + r.status; }
    } catch(e) { const sst = el('search-status'); if (sst) sst.textContent = '保存失败: ' + e.message; }
  });
  const bts = el('btn-test-search');
  if (bts) bts.addEventListener('click', async () => {
    const key = (el('search-apikey')?.value || '').trim();
    if (!key) { const sst = el('search-status'); if (sst) sst.textContent = '请输入 API Key'; return; }
    const sst = el('search-status'); if (sst) sst.textContent = '测试中...';
    try {
      const r = await fetch('/api/search/test', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ api_key: key }) });
      if (r.ok) { const sst = el('search-status'); if (sst) sst.textContent = '连接成功'; }
      else { const sst = el('search-status'); if (sst) sst.textContent = '连接失败: HTTP ' + r.status; }
    } catch(e) { const sst = el('search-status'); if (sst) sst.textContent = '连接失败: ' + e.message; }
  });

  // Provider config
  const pp = el('provider-preset');
  if (pp) pp.addEventListener('change', (e) => {
    // 切换预设 = 开始一次全新配置 — 先退出编辑态，防止半编辑内容误存到旧 key
    if (editingProviderKey) cancelEditProvider();
    const opt = e.target.selectedOptions[0];
    if (opt) { const bu = el('provider-baseurl'); if (bu) { bu.value = opt.value || ''; settings.providerBaseUrl = bu.value; } const dm = opt.dataset.model; if (dm) { const ml = el('provider-model-list'); if (ml) ml.value = dm; } settings.providerPreset = opt.value || ''; saveSettings(); }
  });
  const pak2 = el('provider-apikey');
  if (pak2) pak2.addEventListener('input', () => { settings.providerApiKey = pak2.value; saveSettings(); });
  const pbu2 = el('provider-baseurl');
  if (pbu2) pbu2.addEventListener('input', () => { settings.providerBaseUrl = pbu2.value; saveSettings(); });
  const bfm = el('btn-fetch-models');
  if (bfm) bfm.addEventListener('click', async () => {
    const apiKey = (el('provider-apikey')?.value || '').trim();
    const baseUrl = (el('provider-baseurl')?.value || '').trim();
    if (!apiKey || !baseUrl) { const ps = el('provider-status'); if (ps) ps.textContent = '请输入 API Key 和 Base URL'; return; }
    const ps = el('provider-status'); if (ps) ps.textContent = '获取中...';
    try {
      const r = await fetch('/api/providers/models', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ api_key: apiKey, base_url: baseUrl, is_full_url: (el('provider-fullurl')?.checked || settings.providerFullUrl || false) }) });
      const text = await r.text();
      let d;
      try { d = JSON.parse(text); } catch(parseErr) {
        const ps = el('provider-status'); if (ps) ps.textContent = '获取失败: ' + text.slice(0, 150);
        return;
      }
      if (!r.ok) throw new Error(d.message || 'HTTP ' + r.status);
      if (d.models && d.models.length > 0) {
        const sel = el('provider-model-list'); if (sel) { sel.style.display = 'block'; sel.innerHTML = d.models.map(m => `<option value="${m}">${m}</option>`).join(''); }
        const ps = el('provider-status'); if (ps) ps.textContent = '找到 ' + d.models.length + ' 个模型';
      } else { const ps = el('provider-status'); if (ps) ps.textContent = '未找到模型'; }
    } catch(e) { const ps = el('provider-status'); if (ps) ps.textContent = '获取失败: ' + e.message; }
  });
  const btp = el('btn-test-provider');
  if (btp) btp.addEventListener('click', async () => {
    const apiKey = (el('provider-apikey')?.value || '').trim();
    const baseUrl = (el('provider-baseurl')?.value || '').trim();
    if (!apiKey || !baseUrl) { const ps = el('provider-status'); if (ps) ps.textContent = '请输入 API Key 和 Base URL'; return; }
    // Get model name from input or model list
    const manualModel = (el('provider-model-name')?.value || '').trim();
    const modelSel = el('provider-model-list');
    const presetSel = el('provider-preset');
    let modelName = manualModel;
    if (!modelName && modelSel && modelSel.selectedOptions.length > 0) { modelName = modelSel.selectedOptions[0].value; }
    if (!modelName) { modelName = presetSel?.selectedOptions[0]?.dataset.model || ''; }
    const ps = el('provider-status'); if (ps) ps.textContent = '测试中...';
    try {
      const r = await fetch('/api/providers/test', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ api_key: apiKey, base_url: baseUrl, is_full_url: (el('provider-fullurl')?.checked || settings.providerFullUrl || false), model: modelName }) });
      // Handle both JSON and non-JSON error responses
      const text = await r.text();
      try {
        const d = JSON.parse(text);
        const ps = el('provider-status'); if (ps) ps.textContent = d.message || (r.ok ? '连接成功' : '连接失败: HTTP ' + r.status);
      } catch(parseErr) {
        // Response is not JSON — display raw text
        const ps = el('provider-status'); if (ps) ps.textContent = r.ok ? '连接成功' : '连接失败: ' + text.slice(0, 200);
      }
    } catch(e) { const ps = el('provider-status'); if (ps) ps.textContent = '连接失败: ' + e.message; }
  });

  // Save provider config to toml（新建 / 编辑二合一：编辑态自动携带 original_key）
  const bsp = el('btn-save-provider');
  if (bsp) bsp.addEventListener('click', async () => {
    const apiKey = (el('provider-apikey')?.value || '').trim();
    const baseUrl = (el('provider-baseurl')?.value || '').trim();
    const nameInput = (el('provider-name')?.value || '').trim();
    const presetSel = el('provider-preset');
    const presetName = presetSel?.selectedOptions[0]?.textContent || 'custom';
    const name = nameInput || presetName;
    if (!baseUrl) { providerStatus('请输入 Base URL'); return; }
    // 编辑模式密钥可留空（后端保留原值）；新建必须填写
    if (!editingProviderKey && !apiKey) { providerStatus('请输入 API Key'); return; }
    // Get model name: manual input > model list selection > preset default
    const manualModel = (el('provider-model-name')?.value || '').trim();
    const modelSel = el('provider-model-list');
    let model = manualModel;
    if (!model && modelSel && modelSel.selectedOptions.length > 0) { model = modelSel.selectedOptions[0].value; }
    if (!model) { model = presetSel?.selectedOptions[0]?.dataset.model || 'gpt-4o'; }
    providerStatus('保存中...');
    try {
      const contextRaw = (el('provider-context-length')?.value || '').trim();
      const contextLength = contextRaw ? parseInt(contextRaw, 10) : null;
      const body = { name, api_key: apiKey, base_url: baseUrl, default_model: model,
        context_length: Number.isFinite(contextLength) && contextLength >= 4096 ? contextLength : null,
        is_full_url: (el('provider-fullurl')?.checked || settings.providerFullUrl || false) };
      if (editingProviderKey) body.original_key = editingProviderKey;
      const r = await fetch('/api/providers/save', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
      const text = await r.text();
      let d;
      try { d = JSON.parse(text); } catch (parseErr) { providerStatus('保存失败: ' + text.slice(0, 150)); return; }
      if (r.ok && d && d.status === 'ok') {
        if (apiKey) { settings.providerApiKey = apiKey; }
        settings.providerBaseUrl = baseUrl; saveSettings();
        cancelEditProvider(); // 清除编辑态（含新建场景的表单复位）
        loadExistingProviders();
        showRestartRow();
        providerStatus(d.message || '已保存，重启应用后生效');
      } else {
        providerStatus((d && d.message) || ('保存失败: HTTP ' + r.status));
      }
    } catch(e) { providerStatus('保存失败: ' + e.message); }
  });
  // 取消修改 — 表单复位
  const bce = el('btn-cancel-edit');
  if (bce) bce.addEventListener('click', () => { cancelEditProvider(); providerStatus('已取消修改'); });
  const bpb = el('btn-provider-edit-back');
  if (bpb) bpb.addEventListener('click', () => { cancelEditProvider(); providerStatus('已返回模型列表'); });
  const bpa = el('btn-provider-add');
  if (bpa) bpa.addEventListener('click', startAddProvider);
  // 重启应用生效 — Tauri 壳内直接重启；纯浏览器模式无法代管服务进程
  const bra = el('btn-restart-app');
  if (bra) bra.addEventListener('click', async () => {
    if (typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke) {
      providerStatus('正在重启应用...');
      try { await __TAURI__.core.invoke('restart_app'); }
      catch (e) { providerStatus('重启失败：' + ((e && e.message) || e) + ' — 请手动重启应用'); }
    } else {
      providerStatus('浏览器访问模式无法自动重启 — 请手动重启 SKYLAND 服务进程');
    }
  });

  // Welcome prompts（第 4 张为随机知识卡 — 预置知识库，进入时随机抽取）
  const KNOWLEDGE_POINTS = [
    { t: '一致性哈希', p: '给我讲清楚「一致性哈希」：它解决什么问题、核心思想是什么，再举一个真实场景的例子' },
    { t: 'CAP 定理', p: '给我讲清楚「CAP 定理」：为什么一致性、可用性、分区容错三者不可兼得，实际系统怎么取舍' },
    { t: 'B+ 树索引', p: '数据库索引为什么大多用 B+ 树而不是红黑树或哈希表？给我讲清楚原理和取舍' },
    { t: '乐观锁与悲观锁', p: '乐观锁和悲观锁分别适合什么场景？各举一个实际使用的例子' },
    { t: 'HTTP/3 与 QUIC', p: 'HTTP/3 为什么放弃 TCP 改用 QUIC？解决了 HTTP/2 的哪些实际痛点' },
    { t: '事件循环', p: '讲清楚 JS 的事件循环：宏任务、微任务的执行顺序，用一段代码例子说明' },
    { t: '背压 Backpressure', p: '什么是背压？流式系统里生产快于消费时有哪些应对策略' },
    { t: '缓存穿透/击穿/雪崩', p: '缓存穿透、击穿、雪崩分别是什么意思？各自的经典解法是什么' },
    { t: '零拷贝', p: '零拷贝技术解决了什么问题？sendfile、mmap 分别怎么工作' },
    { t: '协程与线程', p: '协程和线程的本质区别是什么？为什么高并发服务偏爱协程' },
    { t: 'TLS 握手', p: '讲清楚 TLS 1.3 的握手过程，它比 1.2 快在哪里' },
    { t: 'Raft 共识', p: '用通俗的方式讲清楚 Raft 共识算法：选举、日志复制、安全性' },
    { t: '三色标记 GC', p: '讲清楚垃圾回收的三色标记算法：三种颜色代表什么、写屏障解决什么' },
    { t: '布隆过滤器', p: '布隆过滤器的原理是什么？为什么它"说存在可能不存在，说不存在就一定不存在"' },
    { t: '虚拟内存', p: '讲清楚虚拟内存与缺页中断：进程看到的地址是怎么映射到物理内存的' },
    { t: '幂等性设计', p: '什么是幂等性？支付接口怎样设计才能保证重复请求不产生重复扣款' },
    { t: '消息队列削峰', p: '消息队列如何给系统削峰填谷？讲清楚异步、解耦、缓冲三个作用' },
    { t: '尾递归优化', p: '什么是尾递归？为什么尾递归可以优化成循环，举例说明' },
    { t: '动态规划 vs 贪心', p: '动态规划和贪心算法的边界在哪里？各举一个典型问题' },
    { t: 'A* 寻路', p: 'A* 算法和 Dijkstra 的区别是什么？启发函数起什么作用' },
    { t: '熵编码', p: '熵编码为什么能压缩数据？哈夫曼编码的原理讲一下' },
    { t: '傅里叶变换', p: '傅里叶变换到底在干什么？时域和频域的直觉理解，举一个实际应用' },
    { t: '分布式事务', p: '分布式事务为什么难？对比 2PC、TCC 和最终一致性的取舍' },
    { t: 'NAT 与端口映射', p: 'NAT 是怎么让多台设备共享一个公网 IP 的？端口映射原理讲一下' },
    { t: '指令流水线', p: 'CPU 指令流水线为什么能加速执行？分支预测失败会发生什么' },
    { t: '信号量 vs 互斥锁', p: '信号量和互斥锁的区别是什么？什么时候用信号量，什么时候用锁' },
    { t: '指针逃逸分析', p: 'Go 的逃逸分析是怎么回事？变量什么时候逃到堆上' },
    { t: '数据库连接池', p: '数据库连接池的原理是什么？连接数设多少合适，为什么' },
    { t: '读写分离', p: '数据库读写分离怎么实现？主从延迟带来什么问题、怎么缓解' },
    { t: '布谷鸟哈希', p: '布谷鸟哈希和普通哈希表的区别？它的查询为什么快' },
  ];
  const randomCard = el('prompt-card-random');
  if (randomCard && !randomCard.dataset.prompt) {
    const kp = KNOWLEDGE_POINTS[Math.floor(Math.random() * KNOWLEDGE_POINTS.length)];
    randomCard.dataset.prompt = kp.p;
    randomCard.textContent = kp.t;
  }
  document.querySelectorAll('.prompt-card').forEach(card => {
    card.addEventListener('click', () => { const inp = el('input'); if (inp) { inp.value = card.dataset.prompt; inp.dispatchEvent(new Event('input')); } sendMessage(); });
  });

  // ─── Restore settings UI state ───
  document.querySelectorAll('.color-swatch').forEach(s => s.classList.toggle('active', s.dataset.color === settings.accentColor));
  document.querySelectorAll('[data-theme]').forEach(b => b.classList.toggle('active', b.dataset.theme === settings.theme));
  document.querySelectorAll('[data-rounds]').forEach(b => b.classList.toggle('active', (b.dataset.rounds === 'on') === settings.roundsLimited));
  const rs2 = el('rounds-slider'); if (rs2) rs2.value = settings.maxToolRounds;
  const rv = el('rounds-value'); if (rv) rv.textContent = settings.maxToolRounds;
  const sw = el('rounds-slider-wrap'); if (sw) sw.style.display = settings.roundsLimited ? 'block' : 'none';
  const fs2 = el('font-select'); if (fs2) fs2.value = settings.font;
  const sp2 = el('system-prompt'); if (sp2) sp2.value = settings.systemPrompt || '';
  document.querySelectorAll('.font-size-btn').forEach(b => b.classList.toggle('active', parseInt(b.dataset.size) === settings.fontSize));

  // Search config — restore from settings and save on change
  const sak = el('search-apikey'); if (sak) { sak.value = settings.searchApiKey || ''; sak.addEventListener('input', () => { settings.searchApiKey = sak.value; saveSettings(); }); }
  // Also check if search is already configured server-side
  fetch('/api/search/status').then(r => r.text()).then(text => { const data = JSON.parse(text);
    if (data.configured && !settings.searchApiKey) {
      const sst = el('search-status'); if (sst) sst.textContent = '已配置 (服务端)';
    }
  }).catch(() => {});

  // Provider config — restore from settings
  const pak = el('provider-apikey'); if (pak) { pak.value = settings.providerApiKey || ''; pak.addEventListener('input', () => { settings.providerApiKey = pak.value; saveSettings(); }); }
  const pbu = el('provider-baseurl'); if (pbu) { pbu.value = settings.providerBaseUrl || ''; pbu.addEventListener('input', () => { settings.providerBaseUrl = pbu.value; saveSettings(); }); }
  const pp2 = el('provider-preset'); if (pp2) { pp2.value = settings.providerPreset || ''; }
  const pfu = el('provider-fullurl'); if (pfu) { pfu.checked = settings.providerFullUrl || false; pfu.addEventListener('change', () => { settings.providerFullUrl = pfu.checked; saveSettings(); }); }

  // Load models + search status
  fetch('/api/health')
    .then(r => r.text())
    .then(text => { const data = JSON.parse(text);
      const ms2 = el('model-select');
      const sd = el('status-dot');
      if (data.models && data.models.length > 0) {
        if (ms2) {
          ms2.innerHTML = '';
          data.models.forEach(m => { const opt = document.createElement('option'); opt.value = m; opt.textContent = m; ms2.appendChild(opt); });
          const saved = localStorage.getItem(K_MODEL);
          if (saved && data.models.includes(saved)) { ms2.value = saved; selectedModel = saved; }
          else { selectedModel = data.default_model || data.models[0]; ms2.value = selectedModel; }
        }
        if (sd) sd.className = 'status-dot';
        // 上下文压缩模型选择器（性能与执行页）— 留空 = 当前对话模型
        const cm2 = el('compact-model-select');
        if (cm2) {
          const savedCm = localStorage.getItem(K_COMPACT_MODEL) || '';
          cm2.innerHTML = '<option value="">默认（当前对话模型）</option>';
          data.models.forEach(m => { const o2 = document.createElement('option'); o2.value = m; o2.textContent = m; cm2.appendChild(o2); });
          cm2.value = (savedCm && data.models.includes(savedCm)) ? savedCm : '';
          cm2.addEventListener('change', () => {
            try {
              if (cm2.value) localStorage.setItem(K_COMPACT_MODEL, cm2.value);
              else localStorage.removeItem(K_COMPACT_MODEL);
            } catch (e) {}
          });
        }
      } else {
        if (ms2) ms2.innerHTML = '<option>离线</option>';
        if (sd) sd.className = 'status-dot offline';
      }
      // Search status — show web search button if configured
      if (data.search_enabled) {
        settings.searchConfigured = true;
      } else {
        settings.searchConfigured = false;
      }
      // Initialize context bar — user override takes priority over server value
      const userCtx = getContextLengthOverride();
      if (userCtx) {
        updateContextBar(0, userCtx);
      } else if (data.context_length) {
        updateContextBar(0, data.context_length);
      }
      updateCtrlButtons();
    })
    .catch(() => {
      const ms2 = el('model-select'); if (ms2) ms2.innerHTML = '<option>离线</option>';
      const sd = el('status-dot'); if (sd) sd.className = 'status-dot offline';
    });

  // Show chats from localStorage
  renderChatList();
  if (chats.length > 0) {
    selectChat(chats[0].id);
  } else {
    createNewChat();
  }
  updateComposerStatus();
  initCtxLengthSettings(); // 绑定上下文窗口设置（性能与执行页）

  const inp2 = el('input'); if (inp2) inp2.focus();
}

// Run init after DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', init);
} else {
  init();
}
