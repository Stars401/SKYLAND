// ═══════════════════════════════════════════════════════════════
// SKYLAND 设置面板 v2 — 多级导航 + AI 能力（Skill/MCP）+ 性能开关
// 仅负责导航切换与新增分类；既有 id 的绑定逻辑在 app.js/router.js 不变
// ═══════════════════════════════════════════════════════════════

const Settings2 = (() => {
  const $ = id => document.getElementById(id);
  let mcpServers = [];
  let aiInited = false;

  function esc(s) { return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }

  function init() {
    if (window._skylandSettings2Inited) return;
    window._skylandSettings2Inited = true;
    // 导航切换
    const nav = $('settings-nav');
    if (nav) {
      nav.addEventListener('click', e => {
        const item = e.target.closest('.settings-nav-item');
        if (!item) return;
        switchPage(item.dataset.spage);
      });
    }
    // 打开面板时（app.js 打开）按需刷新 AI 能力页
    const st = $('btn-settings');
    if (st) st.addEventListener('click', () => { if (!aiInited) refreshAI(); });
    // 状态刷新
    const stRefresh = $('btn-status-refresh');
    if (stRefresh) stRefresh.addEventListener('click', renderStatus);

    // 记忆导入
    const memScan = $('btn-memory-scan');
    if (memScan) memScan.addEventListener('click', scanMemory);

    // MCP 按钮
    const addBtn = $('btn-mcp-add');
    if (addBtn) addBtn.addEventListener('click', addMcpRow);
    const saveBtn = $('btn-mcp-save');
    if (saveBtn) saveBtn.addEventListener('click', saveMcp);
    // 通知轮询开关
    syncPollToggle();
    const on = $('task-poll-on'), off = $('task-poll-off');
    if (on) on.addEventListener('click', () => setPoll(true));
    if (off) off.addEventListener('click', () => setPoll(false));
    // 待办云同步页
    initTodoSyncPage();
    initFontSelect();
    initDatadirButtons();
    // 全软件原生 select 统一美化（位置/样式可控的自绘弹层）
    SELECT_ENHANCE_LIST.forEach(sel => {
      const el = document.getElementById(sel);
      if (el) enhanceSelect(el);
    });
  }

  function switchPage(key) {
    document.querySelectorAll('.settings-nav-item').forEach(b =>
      b.classList.toggle('active', b.dataset.spage === key));
    document.querySelectorAll('.settings-page').forEach(p =>
      p.style.display = p.dataset.spage === key ? '' : 'none');
    if (key === 'ai') refreshAI();
    if (key === 'status') { renderStatus(); startStatusPoll(); }
    if (key === 'todo-sync') loadTodoSyncConfig();
  }

  function switchTo(key) { switchPage(key); }

  // ─── AI 能力页 ───
  async function refreshAI() {
    try {
      const r = await fetch('/api/skills/dir');
      const d = await r.json();
      const el = $('skills-path-display');
      if (el && d.path) el.value = d.path;
    } catch (e) {}
    try {
      const r = await fetch('/api/skills/list');
      const d = await r.json();
      const list = $('skills-list');
      if (list && d.status === 'ok') {
        list.innerHTML = (d.skills || []).map(s =>
          `<div class="skill-item"><span class="skill-name">${esc(s.title)}</span><span class="skill-name2">${esc(s.name)}</span>${s.builtin ? '<span class="skill-tag">内置</span>' : '<span class="skill-tag custom">自定义</span>'}<div class="skill-desc">${esc(s.description)}</div></div>`
        ).join('') || '<div class="setting-hint">容器中暂无技能</div>';
      }
    } catch (e) {}
  // 自定义子代理清单（Skyland 用户目录 agents/*.md）
    try {
      const r = await fetch('/api/agents');
      const d = await r.json();
      const dirEl = $('agents-path-display');
      if (dirEl && d.dir) dirEl.value = d.dir;
      const box = $('agents-list');
      if (box && d.status === 'ok') {
        const items = d.agents || [];
        box.innerHTML = items.map(a =>
          `<div class="skill-item"><span class="skill-name">${esc(a.name)}</span><span class="skill-tag custom">自定义代理</span><div class="skill-desc">${esc(a.description || '')} · 工具：${esc(a.tools || '全部只读')} · 轮次上限 ${a.rounds}</div></div>`
        ).join('') || '<div class="setting-hint">暂无自定义子代理 — 在上述目录放置 .md 文件即可扩展</div>';
      }
    } catch (e) {}
    await loadMcp();
    await scanMemory(true);
    aiInited = true;
  }

  // ─── 状态页（开发者面板）───
  let statusPollTimer = null;
  function fmtMB(mb) { return mb >= 1024 ? (mb / 1024).toFixed(2) + ' GB' : mb + ' MB'; }
  function fmtTime(secs) {
    if (!secs || secs <= 0) return '—';
    return new Date(secs * 1000).toLocaleString('zh-CN', { hour12: false });
  }
  function statusCard(title, rows) {
    return '<div class="setting-group"><label class="setting-label">' + title + '</label><div class="status-card">' +
      rows.filter(r => r[1] !== undefined && r[1] !== null && r[1] !== '').map(r =>
        '<div class="status-row"><span class="status-k">' + esc(r[0]) + '</span><span class="status-v">' + esc(String(r[1])) + '</span></div>'
      ).join('') + '</div></div>';
  }
  async function renderStatus() {
    const box = $('status-cards');
    if (!box) return;
    try {
      const r = await fetch('/api/system/status');
      if (r.status === 401 || r.status === 403) {
        box.innerHTML = '<div class="setting-hint">当前身份无权查看系统状态（server 模式仅管理员）</div>';
        return;
      }
      const d = await r.json();
      const s = d.server || {}, rt = d.runtime || {}, st = d.storage || {},
            api = d.api || {}, sy = d.system || {}, bd = d.build || {};
      const modeName = { standalone: '一体模式（本机）', server: '云端服务端', client: '云端客户端' }[s.mode] || s.mode;
      box.innerHTML =
        statusCard('服务器', [
          ['运行模式', modeName],
          ['监听端口', s.http_port ? ':' + s.http_port : '未启动'],
          ['客户端连接数（活跃会话）', s.mode === 'server' ? (s.client_connections + ' / 累计 ' + s.sessions_total) : '—（单机模式）'],
        ]) +
        statusCard('运行时', [
          ['运行内存（进程 RSS）', rt.memory_mb ? fmtMB(rt.memory_mb) : '—'],
          ['进程 PID', rt.pid],
        ]) +
        statusCard('存储占用', [
          ['知识库 + 对话（Skyland 用户数据）', fmtMB(st.user_data_mb) + '（' + (st.user_data_files || 0) + ' 文件）'],
          ['待办数据', fmtMB(st.todo_mb)],
          ['生图输出', fmtMB(st.image_mb)],
          ['云工作目录', fmtMB(st.cloud_mb)],
          ['技能 / 会话数', (st.skills || 0) + ' / ' + (st.conversations || 0)],
        ]) +
        statusCard('API', [
          ['已配置模型', api.models_configured ? api.models_configured + ' 个' : '未配置'],
          ['默认模型', api.default_model || '—'],
        ]) +
        statusCard('系统', [
          ['操作系统', (sy.os || '?') + ' · ' + (sy.arch || '?')],
          ['主机名', sy.hostname || '—'],
        ]) +
        statusCard('版本构建', [
          ['版本', 'v' + (bd.version || '?')],
          ['Git 分支', bd.git_branch || '—'],
          ['构建时间', bd.build_time === 'dev 构建' ? 'dev 构建（未注入时间戳）' : fmtTime(Number(bd.build_time))],
        ]);
    } catch (e) {
      box.innerHTML = '<div class="setting-hint">状态获取失败: ' + esc(String(e)) + '</div>';
    }
  }
  function startStatusPoll() {
    if (statusPollTimer) return;
    statusPollTimer = setInterval(() => {
      const page = document.querySelector('.settings-page[data-spage="status"]');
      if (page && page.style.display !== 'none' && document.getElementById('settings-panel')?.classList.contains('open')) {
        renderStatus();
      } else {
        clearInterval(statusPollTimer); statusPollTimer = null;
      }
    }, 5000);
  }

  // ─── 记忆导入（本机智能体配置 → 技能）───
  async function scanMemory(silent) {
    const list = $('memory-list');
    const status = $('memory-status');
    if (!list) return;
    if (!silent) { list.innerHTML = '<div class="setting-hint">扫描中…</div>'; }
    try {
      const r = await fetch('/api/memory/scan');
      const d = await r.json();
      const entries = (d.entries || []).filter(e => e.exists);
      if (entries.length === 0) {
        list.innerHTML = '<div class="setting-hint">未发现已知智能体配置（Claude Code / Codex / Cursor / Gemini CLI / AGENTS.md）</div>';
        return;
      }
      list.innerHTML = entries.map((e, i) => `
        <div class="mem-item">
          <div class="mem-info">
            <div class="mem-label">${esc(e.label)}</div>
            <div class="mem-path" title="${esc(e.path)}">${esc(e.path)} · ${(e.size/1024).toFixed(1)}KB</div>
            ${e.preview ? `<div class="mem-preview">${esc(e.preview.slice(0, 90))}…</div>` : ''}
          </div>
          <button class="kb-modal-btn kb-modal-btn-ok mem-import" data-i="${i}">导入为技能</button>
        </div>`).join('');
      list.querySelectorAll('.mem-import').forEach(btn => {
        btn.addEventListener('click', async () => {
          const entry = entries[parseInt(btn.dataset.i)];
          btn.disabled = true; btn.textContent = '导入中…';
          try {
            const r2 = await fetch('/api/memory/import', {
              method: 'POST', headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ path: entry.path }),
            });
            const d2 = await r2.json();
            if (status) status.textContent = d2.status === 'ok' ? d2.message : ('导入失败: ' + (d2.message || ''));
          } catch (e2) {
            if (status) status.textContent = '导入失败: ' + e2.message;
          }
          btn.disabled = false; btn.textContent = '导入为技能';
        });
      });
    } catch (e) {
      if (!silent && status) status.textContent = '扫描失败: ' + e.message;
    }
  }

  // ─── MCP 管理 ───
  async function loadMcp() {
    try {
      const r = await fetch('/api/mcp/config');
      const d = await r.json();
      mcpServers = d.servers || [];
      renderMcp();
    } catch (e) {}
  }

  function renderMcp() {
    const list = $('mcp-list');
    if (!list) return;
    if (mcpServers.length === 0) {
      list.innerHTML = '<div class="setting-hint">尚未登记 MCP 服务器</div>';
      return;
    }
    list.innerHTML = mcpServers.map((s, i) => `
      <div class="mcp-row" data-i="${i}">
        <input type="text" class="setting-input mcp-name" placeholder="名称" value="${esc(s.name)}" style="width:110px">
        <input type="text" class="setting-input mcp-url" placeholder="https://..." value="${esc(s.url)}" style="flex:1">
        <input type="text" class="setting-input mcp-desc" placeholder="能力描述" value="${esc(s.description)}" style="width:130px">
        <label class="mcp-enabled" title="启用则注入 AI 提示"><input type="checkbox" class="mcp-on-cb" ${s.enabled ? 'checked' : ''}></label>
        <button class="mcp-del" data-del="${i}" title="删除">✕</button>
      </div>`).join('');
    list.querySelectorAll('.mcp-del').forEach(btn => {
      btn.addEventListener('click', () => {
        mcpServers.splice(parseInt(btn.dataset.del), 1);
        renderMcp();
      });
    });
  }

  function addMcpRow() {
    mcpServers.push({ name: '', url: 'https://', description: '', enabled: true });
    renderMcp();
  }

  async function saveMcp() {
    const list = $('mcp-list');
    if (!list) return;
    const status = $('mcp-status');
    const rows = list.querySelectorAll('.mcp-row');
    const servers = [];
    rows.forEach(row => {
      servers.push({
        name: row.querySelector('.mcp-name').value.trim(),
        url: row.querySelector('.mcp-url').value.trim(),
        description: row.querySelector('.mcp-desc').value.trim(),
        enabled: row.querySelector('.mcp-on-cb').checked,
      });
    });
    try {
      const r = await fetch('/api/mcp/config', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ servers }),
      });
      const d = await r.json();
      if (status) status.textContent = d.status === 'ok' ? (d.message || '已保存') : ('保存失败: ' + (d.message || ''));
      if (typeof SkyToast === 'object' && SkyToast.show) {
        SkyToast.show(d.status === 'ok' ? 'MCP 配置已保存' : ('MCP 保存失败：' + (d.message || '')), d.status === 'ok' ? '' : 'error');
      }
      if (d.status === 'ok') loadMcp();
    } catch (e) {
      if (status) status.textContent = '保存失败: ' + e.message;
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('MCP 保存失败：' + e.message, 'error');
    }
  }

  // ─── 通知轮询开关（localStorage，router.js 轮询读取）───
  function pollEnabled() {
    if (window.SkylandSettings) return window.SkylandSettings.get('notifications.taskPoll') !== 'off';
    return localStorage.getItem(SkylandStore.KEYS.taskPoll) !== 'off';
  }
  function syncPollToggle() {
    const on = pollEnabled();
    const o = $('task-poll-on'), f = $('task-poll-off');
    if (o) o.classList.toggle('active', on);
    if (f) f.classList.toggle('active', !on);
  }
  function setPoll(enabled) {
    if (window.SkylandSettings) window.SkylandSettings.set('notifications.taskPoll', enabled ? 'on' : 'off');
    else localStorage.setItem(SkylandStore.KEYS.taskPoll, enabled ? 'on' : 'off');
    syncPollToggle();
  }

  return { init, switchTo, pollEnabled };
})();

// ═══════════════════════════════════════════════════════════════
// 待办云同步设置页（Memos 优先；凭据留空 = 保留原值）
// ═══════════════════════════════════════════════════════════════
let _syncPageInited = false;

function initTodoSyncPage() {
  if (_syncPageInited) return;
  _syncPageInited = true;
  const $ = id => document.getElementById(id);
  const authToken = $('todo-sync-auth-token'), authPassword = $('todo-sync-auth-password');
  const setAuthMode = mode => {
    authToken?.classList.toggle('active', mode === 'token');
    authPassword?.classList.toggle('active', mode === 'password');
    if ($('todo-sync-token-wrap')) $('todo-sync-token-wrap').style.display = mode === 'token' ? '' : 'none';
    if ($('todo-sync-password-wrap')) $('todo-sync-password-wrap').style.display = mode === 'password' ? '' : 'none';
  };
  authToken?.addEventListener('click', () => setAuthMode('token'));
  authPassword?.addEventListener('click', () => setAuthMode('password'));

  const setPoll = on => {
    $('todo-sync-poll-on')?.classList.toggle('active', on);
    $('todo-sync-poll-off')?.classList.toggle('active', !on);
  };
  $('todo-sync-poll-on')?.addEventListener('click', () => setPoll(true));
  $('todo-sync-poll-off')?.addEventListener('click', () => setPoll(false));

  const collect = () => ({
    provider: $('todo-sync-provider')?.value || 'memos',
    url: $('todo-sync-url')?.value.trim() || '',
    auth_mode: authToken?.classList.contains('active') ? 'token' : 'password',
    username: $('todo-sync-username')?.value.trim() || '',
    token: $('todo-sync-token')?.value || '',
    password: $('todo-sync-password')?.value || '',
    poll_enabled: $('todo-sync-poll-on')?.classList.contains('active') || false,
  });

  $('todo-sync-test')?.addEventListener('click', async () => {
    const st = $('todo-sync-status');
    if (st) st.textContent = '测试中…';
    try {
      const r = await fetch('/api/todo/sync/test', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(collect()) });
      const d = await r.json();
      if (st) st.textContent = (d.status === 'ok' ? '✓ ' : '✕ ') + (d.message || '');
      if (st) st.style.color = d.status === 'ok' ? 'var(--success)' : 'var(--error)';
    } catch (e) { if (st) { st.textContent = '✕ ' + e.message; st.style.color = 'var(--error)'; } }
  });

  $('todo-sync-save')?.addEventListener('click', async () => {
    const st = $('todo-sync-status');
    try {
      const r = await fetch('/api/todo/sync/config', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(collect()) });
      const d = await r.json();
      if (d.status === 'ok') {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('同步配置已保存');
        if (st) st.textContent = '';
        loadTodoSyncConfig();
      } else if (st) { st.textContent = '✕ ' + (d.message || '保存失败'); st.style.color = 'var(--error)'; }
    } catch (e) { if (st) { st.textContent = '✕ ' + e.message; st.style.color = 'var(--error)'; } }
  });

  $('todo-sync-now')?.addEventListener('click', () => runTodoSyncPull(true));
}

async function runTodoSyncPull(manual) {
  const st = $('todo-sync-status');
  try {
    const r = await fetch('/api/todo/sync/pull', { method: 'POST' });
    const d = await r.json();
    if (d.status === 'ok') {
      if (manual && st) { st.textContent = d.total ? `同步完成：新增 ${d.imported} / 已存在 ${d.skipped}（远端 ${d.total} 条）` : '远端暂无备忘'; st.style.color = ''; }
      if (d.imported > 0) {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show(`同步：拉取 ${d.imported} 条新待办`);
        if (typeof TodoApp !== 'undefined' && TodoApp.onShow) TodoApp.onShow();
      }
      return d;
    }
    if (manual && st) { st.textContent = '✕ ' + (d.message || '同步失败'); st.style.color = 'var(--error)'; }
  } catch (e) {
    if (manual && st) { st.textContent = '✕ ' + e.message; st.style.color = 'var(--error)'; }
  }
  return null;
}

async function loadTodoSyncConfig() {
  const $ = id => document.getElementById(id);
  if (!$('todo-sync-url')) return;
  try {
    const r = await fetch('/api/todo/sync/config');
    const d = await r.json();
    if (d.status !== 'ok') return;
    if ($('todo-sync-provider')) $('todo-sync-provider').value = d.provider || 'memos';
    if ($('todo-sync-url')) $('todo-sync-url').value = d.url || '';
    const authToken = $('todo-sync-auth-token'), authPassword = $('todo-sync-auth-password');
    const mode = d.auth_mode === 'password' ? 'password' : 'token';
    authToken?.classList.toggle('active', mode === 'token');
    authPassword?.classList.toggle('active', mode === 'password');
    if ($('todo-sync-token-wrap')) $('todo-sync-token-wrap').style.display = mode === 'token' ? '' : 'none';
    if ($('todo-sync-password-wrap')) $('todo-sync-password-wrap').style.display = mode === 'password' ? '' : 'none';
    if ($('todo-sync-token')) $('todo-sync-token').value = '';
    if ($('todo-sync-token')) $('todo-sync-token').placeholder = d.has_token ? '已保存（留空保持不变）' : 'Access Token（Memos 设置页生成）';
    if ($('todo-sync-username')) $('todo-sync-username').value = d.username || '';
    if ($('todo-sync-password')) { $('todo-sync-password').value = ''; $('todo-sync-password').placeholder = d.has_password ? '已保存（留空保持不变）' : '密码'; }
    $('todo-sync-poll-on')?.classList.toggle('active', !!d.poll_enabled);
    $('todo-sync-poll-off')?.classList.toggle('active', !d.poll_enabled);
    const ls = $('todo-sync-lastsync');
    if (ls) ls.textContent = d.last_sync > 0 ? '上次同步：' + new Date(d.last_sync * 1000).toLocaleString('zh-CN', { hour12: false }) : '尚未同步过';
  } catch (e) {}
}

// ═══════════════════════════════════════════════════════════════
// 系统字体选择器 — 检索 Windows 已安装字体（/api/system/fonts），
// 每项以该字体实时预览；选中写入 global-font-family（hidden），即时预览应用。
// ═══════════════════════════════════════════════════════════════
let _fontsCache = null;

// ═══════════════════════════════════════════════════════════════
// selectEnhance — 原生 select 的统一自绘替换（修复浏览器弹层位置异常/无样式）。
// 隐藏原生 select 保留语义；程序化赋值（select.value=x）经包装的 setter 刷新
// 按钮文案；动态填充 options（模型列表/预设厂商等）由 MutationObserver 感知。
// ═══════════════════════════════════════════════════════════════
const SELECT_ENHANCE_LIST = [
  'todo-sync-provider', 'global-font-weight', 'provider-preset', 'compact-model-select',
  'model-select', 'img-provider-select', 'img-size-select', 'img-quality-select',
  'img-style-select', 'kb-theme-select', 'kb-font-select',
];

function enhanceSelect(select) {
  if (!select || select.dataset.enhanced) return;
  select.dataset.enhanced = '1';
  const wrap = document.createElement('span');
  wrap.className = 'select-enh';
  select.parentNode.insertBefore(wrap, select);
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = (select.className || '') + ' select-enh-btn';
  if (select.id) btn.dataset.enhFor = select.id;
  const pop = document.createElement('div');
  pop.className = 'select-enh-pop';
  pop.style.display = 'none';
  wrap.appendChild(btn);
  wrap.appendChild(pop);
  select.style.display = 'none';
  // 布局接续：把原 select 的尺寸约束（flex/width）搬到包装层，
  // 否则 header 里的模型选择器等会被 flex:1 拉伸、固定宽的会塌缩
  ['flex', 'flexGrow', 'width', 'minWidth', 'maxWidth', 'marginLeft', 'marginRight'].forEach(p => {
    if (select.style[p]) wrap.style[p] = select.style[p];
  });

  const label = () => {
    const opt = select.options[select.selectedIndex];
    btn.textContent = opt ? opt.textContent : '请选择…';
  };
  const renderList = () => {
    pop.innerHTML = [...select.options].map((o, i) =>
      `<button type="button" class="select-enh-item${o.disabled ? ' disabled' : ''}${i === select.selectedIndex ? ' active' : ''}" data-i="${i}">${o.textContent}</button>`
    ).join('') || '<div class="font-select-empty">暂无可选项</div>';
    pop.querySelectorAll('.select-enh-item').forEach(b => {
      b.addEventListener('click', () => {
        const i = parseInt(b.dataset.i, 10);
        const opt = select.options[i];
        if (opt.disabled) return;
        select.value = opt.value;
        select.dispatchEvent(new Event('change', { bubbles: true }));
        label();
        pop.style.display = 'none';
      });
    });
  };
  const open = () => { renderList(); pop.style.display = 'block'; };
  btn.addEventListener('click', e => {
    e.stopPropagation();
    const isOpen = pop.style.display !== 'none';
    document.querySelectorAll('.select-enh-pop').forEach(p => { if (p !== pop) p.style.display = 'none'; });
    if (isOpen) { pop.style.display = 'none'; return; }
    open();
  });
  pop.addEventListener('click', e => e.stopPropagation());
  document.addEventListener('click', () => { pop.style.display = 'none'; });
  // 程序化赋值刷新按钮文案
  const desc = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value');
  Object.defineProperty(select, 'value', {
    get() { return desc.get.call(select); },
    set(v) { desc.set.call(select, v); label(); },
  });
  // 动态填充 options（模型列表/预设厂商）
  new MutationObserver(() => { label(); if (pop.style.display !== 'none') renderList(); })
    .observe(select, { childList: true });
  label();
}

// ═══════════════════════════════════════════════════════════════
// 数据目录页 — 每个路径输入框右侧【打开此目录位置】【浏览选择目录】
// ═══════════════════════════════════════════════════════════════
const DATADIR_FIELDS = [
  'global-conversations-path', 'global-config-path', 'global-userdata-path',
  'global-todo-path', 'global-img-path',
];
const isTauriLike = () => typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke;

function initDatadirButtons() {
  if (window._skylandDatadirBtns) return;
  window._skylandDatadirBtns = true;
  DATADIR_FIELDS.forEach(id => {
    const input = document.getElementById(id);
    if (!input || input.nextElementSibling?.classList?.contains('datadir-btns')) return;
    const box = document.createElement('span');
    box.className = 'datadir-btns';
    const openBtn = document.createElement('button');
    openBtn.type = 'button';
    openBtn.className = 'datadir-btn';
    openBtn.title = '打开此目录位置';
    openBtn.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" width="14" height="14"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>';
    const browseBtn = document.createElement('button');
    browseBtn.type = 'button';
    browseBtn.className = 'datadir-btn';
    browseBtn.title = '用资源管理器选择目录';
    browseBtn.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" width="14" height="14"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>';
    box.appendChild(openBtn);
    if (isTauriLike()) box.appendChild(browseBtn);
    input.parentNode.insertBefore(box, input.nextSibling);
    input.parentNode.style.display = 'flex';
    input.parentNode.style.gap = '6px';
    input.style.flex = '1';

    openBtn.addEventListener('click', async () => {
      const key = { 'global-conversations-path': 'conversations', 'global-config-path': 'config',
                    'global-userdata-path': 'userdata', 'global-todo-path': 'todo', 'global-img-path': 'images' }[id];
      try {
        const r = await fetch('/api/datadir/resolve?key=' + key);
        const d = await r.json();
        if (d.status !== 'ok') { if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show(d.message || '解析失败', 'error'); return; }
        if (isTauriLike() && __TAURI__.shell && __TAURI__.shell.open) {
          try { await __TAURI__.shell.open(d.abs); return; } catch (e) { console.error(e); }
        }
        try { await navigator.clipboard.writeText(d.abs); } catch (e) {}
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('目录已复制：' + d.abs);
      } catch (e) {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('解析失败：' + e.message, 'error');
      }
    });

    browseBtn.addEventListener('click', async () => {
      if (!isTauriLike()) return;
      try {
        const picked = await __TAURI__.core.invoke('pick_directory');
        if (picked) input.value = picked;
      } catch (e) { console.error('pick_directory failed:', e); }
    });
  });
}


function skyFontStack(name) {
  const system = "system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";
  if (!name || name === 'system') return system;
  return `'${name.replace(/'/g, '')}', ${system}`;
}

function initFontSelect() {
  const $ = id => document.getElementById(id);
  const btn = $('global-font-btn'), pop = $('global-font-pop');
  if (!btn || !pop) return;
  btn.addEventListener('click', async e => {
    e.stopPropagation();
    const open = pop.style.display !== 'none';
    if (open) { pop.style.display = 'none'; return; }
    pop.style.display = 'block';
    const search = $('global-font-search');
    if (search) { search.value = ''; renderFontList(''); search.focus(); }
    if (!_fontsCache) {
      try {
        const r = await fetch('/api/system/fonts');
        const d = await r.json();
        const list = (d.fonts || []).map(x => String(x));
        // 仅成功时缓存 —— 失败缓存空数组会让字体列表永久为空（直到刷新页面）
        if (list.length) { _fontsCache = list; renderFontList(search ? search.value : ''); }
        else { listFail('字体清单为空'); }
      } catch (e) { listFail('字体清单加载失败：' + e.message); }
    }
    function listFail(msg) {
      const list2 = $('global-font-list');
      if (list2) list2.innerHTML = '<div class="font-select-empty">' + msg + '，可关闭后重试</div>';
    }
  });
  const search = $('global-font-search');
  search?.addEventListener('input', () => renderFontList(search.value));
  search?.addEventListener('keydown', e => e.stopPropagation());
  document.addEventListener('click', e => {
    if (!pop.contains(e.target) && e.target !== btn && !btn.contains(e.target)) pop.style.display = 'none';
  });
}

function renderFontList(filter) {
  const list = $('global-font-list');
  if (!list) return;
  const current = $('global-font-family')?.value || 'system';
  const q = (filter || '').trim().toLowerCase();
  const mk = (value, label) => `
    <button type="button" class="font-select-item${value === current ? ' active' : ''}" data-font="${value.replace(/"/g, '')}" style="font-family:${skyFontStack(value).replace(/"/g, '&quot;')}">
      <span>${label}</span>
      ${value === current ? '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" width="13" height="13"><polyline points="20 6 9 17 4 12"/></svg>' : ''}
    </button>`;
  const sysItem = mk('system', '系统默认（推荐）');
  const fonts = (_fontsCache || []).filter(f => !q || f.toLowerCase().includes(q));
  const fontItems = fonts.map(f => mk(f, f)).join('');
  list.innerHTML = sysItem + (fontItems || '<div class="font-select-empty">未找到匹配的字体</div>');
  list.querySelectorAll('.font-select-item').forEach(b => {
    b.addEventListener('click', () => {
      const name = b.dataset.font;
      const hidden = $('global-font-family');
      if (hidden) hidden.value = name;
      const label = $('global-font-label');
      if (label) label.textContent = name === 'system' ? '系统默认（推荐）' : name;
      document.documentElement.style.setProperty('--font-body', skyFontStack(name));
      $('global-font-pop').style.display = 'none';
    });
  });
}
