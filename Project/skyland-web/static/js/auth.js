// ═══════════════════════════════════════════════════════════════════
// SKYLAND Auth — 登录 / 运行模式引导 / 账号管理 / IP 白名单
// 云端模式 (server) 启用登录；一体模式 (standalone) 零打扰
// ═══════════════════════════════════════════════════════════════════

const Auth = (() => {
  let bootInfo = { mode: 'standalone', auth_required: false, first_run: false };
  let currentUser = null;

  function esc(s) {
    return (s || '').toString().replace(/&/g, '&amp;').replace(/</g, '&lt;')
      .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  function isTauri() {
    return typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke;
  }

  async function tauriInvoke(cmd, args) {
    return await __TAURI__.core.invoke(cmd, args);
  }

  // ═══════════ 框架级保障（防呆层）═══════════

  // ─── 全局 401 拦截 ───
  // 所有视图的 /api/ 请求统一处理会话失效：任何视图遗漏 401 处理都不会
  // 再导致静默失败或（历史上的）reload 循环。auth 端点除外——登录失败也是 401。
  const _origFetch = window.fetch.bind(window);
  window.fetch = function(input, init) {
    let path = '';
    try {
      if (typeof input === 'string') path = new URL(input, location.href).pathname;
      else if (input && typeof input.url === 'string') path = new URL(input.url, location.href).pathname;
    } catch (e) {}
    return _origFetch(input, init).then(resp => {
      if (resp && resp.status === 401 && path.startsWith('/api/') && !path.startsWith('/api/auth/')) {
        onSessionLost();
      }
      return resp;
    });
  };

  // ─── 遮罩守卫（中心化登记）───
  // 新增全屏遮罩层时只需在此登记，所有依赖方（键盘快捷键、会话失效等）自动生效
  const GUIDANCE_OVERLAYS = ['admin-setup-overlay', 'setup-overlay', 'setup-done-overlay'];
  const ALL_AUTH_OVERLAYS = ['login-overlay'].concat(GUIDANCE_OVERLAYS);
  function isOverlayActive() {
    for (const id of ALL_AUTH_OVERLAYS) {
      const el = document.getElementById(id);
      if (el && el.style.display !== 'none') return true;
    }
    return false;
  }

  // ─── 安全重载（断路器）───
  // 2 秒内连续重载视为疑似循环（历史 bug：错误路径 reload 造成无限横跳），拦截。
  // 正常用户操作（登录成功/退出/创建管理员后的刷新）间隔远超 2 秒，不受影响。
  function safeReload() {
    const key = SkylandStore.KEYS.lastReload;
    let last = 0;
    try { last = parseInt(sessionStorage.getItem(key) || '0', 10) || 0; } catch (e) {}
    const now = Date.now();
    if (now - last < 2000) {
      console.warn('safeReload: 2 秒内重复重载已拦截（疑似循环）');
      return;
    }
    try { sessionStorage.setItem(key, String(now)); } catch (e) {}
    location.reload();
  }

  // ─── 启动引导（router.js DOMContentLoaded 最先调用）───
  async function boot() {
    try {
      const r = await fetch('/api/server/info');
      if (r.ok) bootInfo = await r.json();
    } catch (e) {}

    if (bootInfo.first_run) {
      showFirstRunSetup();
      return;
    }

    if (bootInfo.auth_required) {
      try {
        const r = await fetch('/api/auth/me');
        const me = await r.json();
        if (me.authenticated) {
          currentUser = me;
          if (me.must_change_password) showMustChangeBanner();
          return;
        }
      } catch (e) {}
      // 初始管理员尚未创建 — 本机引导自建；远程提示到服务器本机操作
      if (bootInfo.users_initialized === false) {
        if (bootInfo.can_setup) {
          showAdminSetup();
        } else {
          showLogin(true);
        }
        return;
      }
      showLogin();
    }
  }

  function showMustChangeBanner() {
    const banner = document.createElement('div');
    banner.id = 'auth-must-change-banner';
    banner.innerHTML = `
      <span><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="13" height="13" style="vertical-align:-2px;margin-right:4px"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>当前使用初始密码，为保障账号安全请前往 <b>设置 → 账号</b> 修改密码</span>
      <button id="auth-must-change-close" title="关闭">✕</button>
    `;
    banner.querySelector('#auth-must-change-close').addEventListener('click', () => banner.remove());
    document.body.appendChild(banner);
  }

  // ─── 登录视图 ───
  function showLogin(pendingSetupHint) {
    let overlay = document.getElementById('login-overlay');
    if (!overlay) return;
    overlay.style.display = 'flex';

    const form = document.getElementById('login-form');
    const userInput = document.getElementById('login-username');
    const passInput = document.getElementById('login-password');
    const errBox = document.getElementById('login-error');
    const btn = document.getElementById('login-submit-btn');

    // 管理员尚未初始化的远程访客 — 指引到服务器本机完成初始设置
    if (pendingSetupHint && errBox) errBox.textContent = '初始管理员尚未创建 — 请在服务器本机完成创建后再登录';

    if (overlay.dataset.bound !== '1') {
      overlay.dataset.bound = '1';
      form.addEventListener('submit', async e => {
        e.preventDefault();
        if (btn.disabled) return;
        const username = userInput.value.trim();
        const password = passInput.value;
        if (!username || !password) return;
        btn.disabled = true;
        btn.textContent = '登录中...';
        errBox.textContent = '';

        try {
          const r = await fetch('/api/auth/login', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ username, password }),
          });
          const text = await r.text();
          let data;
          try { data = JSON.parse(text); } catch (e) { data = null; }
          if (r.ok && data && data.status === 'ok') {
            safeReload();
            return;
          }
          const msg = (data && data.message) || '登录失败';
          errBox.textContent = msg;
          passInput.value = '';
          passInput.focus();
        } catch (e) {
          errBox.textContent = '网络错误，无法连接服务器';
        } finally {
          btn.disabled = false;
          btn.textContent = '登 录';
        }
      });
    }
    setTimeout(() => userInput && userInput.focus(), 80);
  }

  // ─── 初始管理员创建（server 模式首启，本机）───
  function showAdminSetup() {
    const overlay = document.getElementById('admin-setup-overlay');
    if (!overlay) return;
    overlay.style.display = 'flex';

    const form = document.getElementById('admin-setup-form');
    const userInput = document.getElementById('admin-setup-username');
    const passInput = document.getElementById('admin-setup-password');
    const pass2Input = document.getElementById('admin-setup-password2');
    const errBox = document.getElementById('admin-setup-error');
    const btn = document.getElementById('admin-setup-btn');

    if (overlay.dataset.bound !== '1') {
      overlay.dataset.bound = '1';
      form.addEventListener('submit', async e => {
        e.preventDefault();
        if (btn.disabled) return;
        const username = userInput.value.trim();
        const password = passInput.value;
        if (!username || !password) return;
        if (password !== pass2Input.value) { errBox.textContent = '两次输入的密码不一致'; return; }
        if (password.length < 8) { errBox.textContent = '密码至少 8 位'; return; }
        if (username.length < 2) { errBox.textContent = '用户名至少 2 个字符'; return; }
        btn.disabled = true;
        btn.textContent = '创建中...';
        errBox.textContent = '';

        try {
          const r = await fetch('/api/auth/setup', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ username, password }),
          });
          const text = await r.text();
          let data;
          try { data = JSON.parse(text); } catch (e) { data = null; }
          if (r.ok && data && data.status === 'ok') {
            safeReload();
            return;
          }
          errBox.textContent = (data && data.message) || '创建失败';
        } catch (e) {
          errBox.textContent = '网络错误，无法连接服务器';
        } finally {
          btn.disabled = false;
          btn.textContent = '创建管理员账号';
        }
      });
    }
    setTimeout(() => userInput && userInput.focus(), 80);
  }

  // ─── 首次运行模式选择 ───
  function showFirstRunSetup() {
    const overlay = document.getElementById('setup-overlay');
    if (!overlay) return;
    overlay.style.display = 'flex';

    const bindInput = document.getElementById('setup-server-bind');
    const urlInput = document.getElementById('setup-server-url');
    const clientField = document.getElementById('setup-client-field');
    const serverField = document.getElementById('setup-server-field');
    const errBox = document.getElementById('setup-error');
    const confirmBtn = document.getElementById('setup-confirm-btn');
    let selectedMode = 'standalone';

    // 网页模式（非桌面端）下客户端模式不可用 — 禁用并警告，防止选中后重启退出
    if (!isTauri()) {
      const clientCard = overlay.querySelector('.setup-card[data-mode="client"]');
      if (clientCard && !clientCard.dataset.webWarned) {
        clientCard.dataset.webWarned = '1';
        clientCard.style.opacity = '0.5';
        clientCard.style.pointerEvents = 'none';
        clientCard.style.cursor = 'not-allowed';
        clientCard.removeAttribute('tabindex');
        clientCard.setAttribute('aria-disabled', 'true');
        const cdesc = clientCard.querySelector('.setup-card-desc');
        if (cdesc) cdesc.innerHTML += '<br><b style="color:var(--error,#e5484d)">网页模式不可用：客户端模式需要 SKYLAND 桌面应用</b>';
      }
    }

    function selectMode(mode) {
      selectedMode = mode;
      overlay.querySelectorAll('.setup-card').forEach(c => c.classList.remove('selected'));
      overlay.querySelector(`.setup-card[data-mode="${mode}"]`)?.classList.add('selected');
      if (clientField) clientField.style.display = mode === 'client' ? 'block' : 'none';
      if (serverField) serverField.style.display = mode === 'server' ? 'block' : 'none';
    }

    if (overlay.dataset.bound !== '1') {
      overlay.dataset.bound = '1';
      overlay.querySelectorAll('.setup-card').forEach(card => {
        card.addEventListener('click', () => selectMode(card.dataset.mode));
        // 键盘可达（role=button + tabindex 由 HTML 提供）
        card.addEventListener('keydown', e => {
          if (e.isComposing) return;
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            selectMode(card.dataset.mode);
          }
        });
      });

      confirmBtn.addEventListener('click', async () => {
        if (confirmBtn.disabled) return;
        const payload = { mode: selectedMode };
        if (selectedMode === 'client') {
          const url = (urlInput.value || '').trim();
          if (!/^https?:\/\/.+/.test(url)) {
            errBox.textContent = '请输入有效的服务器地址（以 http:// 或 https:// 开头）';
            return;
          }
          payload.server_url = url;
        }
        if (selectedMode === 'server') {
          const bind = (bindInput.value || '').trim();
          if (bind) payload.server_bind = bind;
        }
        confirmBtn.disabled = true;
        confirmBtn.textContent = '保存中...';
        errBox.textContent = '';
        try {
          const r = await fetch('/api/server/config', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
          });
          const data = await r.json();
          if (data.status === 'ok') {
            overlay.style.display = 'none';
            showSetupDone(data.mode);
          } else {
            errBox.textContent = data.message || '保存失败';
          }
        } catch (e) {
          errBox.textContent = '保存失败：' + e.message;
        } finally {
          confirmBtn.disabled = false;
          confirmBtn.textContent = '保存并应用';
        }
      });
    }
    selectMode('standalone');
  }

  function showSetupDone(mode) {
    const overlay = document.getElementById('setup-done-overlay');
    if (!overlay) return;
    const msg = document.getElementById('setup-done-msg');
    const canRestart = isTauri();
    const tips = {
      standalone: '已选择【一体模式】— 本机独立运行，开箱即用。',
      server: canRestart
        ? '已选择【云端服务端】— 应用即将重启，重启后将引导创建超级管理员账号。'
        : '已选择【云端服务端】— 请重启 SKYLAND 服务端程序，首次启动将引导创建超级管理员账号。',
      client: canRestart
        ? '已选择【客户端模式】— 应用即将重启并连接云端服务器，请使用服务端管理员分配的账号登录。'
        : '已选择【客户端模式】— 请重启应用以连接云端服务器。',
    };
    if (msg) msg.textContent = tips[mode] || tips.standalone;
    overlay.style.display = 'flex';
    const btn = document.getElementById('setup-done-ok');
    if (btn) btn.onclick = () => { overlay.style.display = 'none'; };
    // server/client 需重启生效 — 引导流程直接自动重启，用户无感切换；
    // 选择服务端模式重启后即进入超级管理员创建界面
    if (mode !== 'standalone' && canRestart) {
      setTimeout(async () => {
        try {
          await tauriInvoke('restart_app');
        } catch (e) {
          if (msg) msg.textContent = '自动重启失败 — 请手动退出应用（托盘右键 → 退出 SKYLAND）后重新启动。';
        }
      }, 1500);
    }
  }

  // ─── 设置面板：运行模式 + 账号 + 管理面板 ───
  async function renderSettingsContent() {
    const section = document.getElementById('auth-settings-section');
    if (!section) return;

    // 运行模式（桌面端可切换，网页端只读展示）
    let modeHtml = '';
    const modeNames = {
      standalone: '一体模式（本机独立运行）',
      server: '云端服务端（数据托管 + 登录验证）',
      client: '客户端（连接云端服务器）',
    };
    if (isTauri()) {
      let localCfg = null;
      try { localCfg = await tauriInvoke('get_app_config'); } catch (e) {}
      const cur = localCfg ? localCfg.mode : (bootInfo.mode || 'standalone');
      modeHtml = `
        <div class="setting-group">
          <label class="setting-label">运行模式</label>
          <select id="auth-mode-select" class="kb-modal-input" style="margin-bottom:6px" data-loaded-mode="${cur}">
            <option value="standalone" ${cur === 'standalone' ? 'selected' : ''}>一体模式 — 本机独立运行</option>
            <option value="server" ${cur === 'server' ? 'selected' : ''}>云端服务端 — 数据托管 + 登录验证</option>
            <option value="client" ${cur === 'client' ? 'selected' : ''}>客户端 — 连接云端服务器</option>
          </select>
          <div id="auth-client-url-wrap" style="display:${cur === 'client' ? 'block' : 'none'};margin-bottom:6px">
            <input type="text" id="auth-client-url" class="kb-modal-input" placeholder="服务器地址，如 http://192.168.1.100:3000" value="${localCfg ? esc(localCfg.server_url || '') : ''}">
            ${isTauri() ? '<button id="auth-lan-scan-btn" class="kb-modal-btn kb-modal-btn-ok" style="margin-top:6px;width:100%">搜索局域网服务端…</button><div id="auth-lan-results" class="auth-lan-results"></div>' : ''}
          </div>
          <p style="font-size:11px;color:var(--text-muted)">切换模式保存后需重启应用生效</p>
        </div>`;
    } else {
      modeHtml = `
        <div class="setting-group">
          <label class="setting-label">运行模式</label>
          <p style="font-size:12px;color:var(--text-secondary)">${esc(modeNames[bootInfo.mode] || bootInfo.mode)}（网页端模式由服务端配置决定）</p>
        </div>`;
    }

    // 账号区（仅认证模式显示）
    let accountHtml = '';
    if (bootInfo.auth_required && currentUser) {
      accountHtml = `
        <div class="setting-group">
          <label class="setting-label">账号</label>
          <div class="auth-account-row">
            <span class="auth-account-user"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="13" height="13" style="vertical-align:-2px;margin-right:4px"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>${esc(currentUser.username)} <span class="auth-role-badge ${currentUser.role}">${currentUser.role === 'admin' ? '管理员' : '成员'}</span></span>
            <button id="auth-logout-btn" class="kb-modal-btn kb-modal-btn-cancel auth-logout-btn">退出登录</button>
          </div>
          <details class="auth-cp-details">
            <summary>修改密码</summary>
            <input type="password" id="auth-cp-old" class="kb-modal-input" placeholder="原密码" style="margin:6px 0">
            <input type="password" id="auth-cp-new" class="kb-modal-input" placeholder="新密码（至少 8 位）" style="margin-bottom:6px">
            <input type="password" id="auth-cp-new2" class="kb-modal-input" placeholder="确认新密码" style="margin-bottom:6px">
            <div id="auth-cp-error" class="auth-error"></div>
            <button id="auth-cp-btn" class="kb-modal-btn kb-modal-btn-ok">确认修改</button>
          </details>
        </div>`;
    } else if (bootInfo.auth_required) {
      accountHtml = `
        <div class="setting-group">
          <label class="setting-label">账号</label>
          <p style="font-size:12px;color:var(--text-muted)">未登录</p>
        </div>`;
    }

    // 管理面板（仅 admin 可见）
    let adminHtml = '';
    if (bootInfo.auth_required && currentUser && currentUser.role === 'admin') {
      adminHtml = `
        <div class="setting-group">
          <label class="setting-label">用户管理（管理员）</label>
          <div id="auth-users-list" class="auth-users-list">加载中...</div>
          <div class="auth-user-create">
            <input type="text" id="auth-new-username" class="kb-modal-input" placeholder="用户名（2-32 位字母数字）">
            <input type="password" id="auth-new-password" class="kb-modal-input" placeholder="密码（至少 8 位）">
            <input type="password" id="auth-new-password2" class="kb-modal-input" placeholder="确认密码">
            <select id="auth-new-role" class="kb-modal-input">
              <option value="user">成员</option>
              <option value="admin">管理员</option>
            </select>
            <button id="auth-user-create-btn" class="kb-modal-btn kb-modal-btn-ok">创建用户</button>
          </div>
          <div id="auth-users-error" class="auth-error"></div>
        </div>
        <div class="setting-group">
          <label class="setting-label">访问白名单（IP，留空 = 不启用）</label>
          <div id="auth-whitelist-list" class="auth-whitelist-list"></div>
          <div class="auth-user-create">
            <input type="text" id="auth-wl-input" class="kb-modal-input" placeholder="如 192.168.1.100">
            <button id="auth-wl-add-btn" class="kb-modal-btn kb-modal-btn-ok">添加</button>
          </div>
          <div id="auth-wl-error" class="auth-error"></div>
        </div>`;
    }

    section.innerHTML = modeHtml + accountHtml + adminHtml;
    bindSettingsEvents();
    if (bootInfo.auth_required && currentUser && currentUser.role === 'admin') {
      loadUsers();
      loadWhitelist();
    }
  }

  function bindSettingsEvents() {
    const section = document.getElementById('auth-settings-section');
    if (!section) return;

    // 局域网服务端发现（Tauri 命令，UDP 广播）
    const lanScanBtn = section.querySelector('#auth-lan-scan-btn');
    if (lanScanBtn) {
      lanScanBtn.addEventListener('click', async () => {
        const results = section.querySelector('#auth-lan-results');
        if (!results) return;
        lanScanBtn.disabled = true;
        lanScanBtn.textContent = '扫描中（约 2 秒）…';
        results.innerHTML = '';
        try {
          const data = await tauriInvoke('discover_lan_servers');
          const servers = (data && data.servers) || [];
          if (servers.length === 0) {
            results.innerHTML = '<div class="setting-hint">未发现局域网内的 SKYLAND 服务端（需服务端处于云端模式）</div>';
          } else {
            results.innerHTML = servers.map(s =>
              `<div class="auth-lan-item" data-url="${esc(s.url)}"><span class="auth-lan-name">${esc(s.name)}</span><span class="auth-lan-url">${esc(s.url)}</span></div>`
            ).join('');
            results.querySelectorAll('.auth-lan-item').forEach(item => {
              item.addEventListener('click', () => {
                const urlEl = section.querySelector('#auth-client-url');
                if (urlEl) urlEl.value = item.dataset.url;
                const modeSel = section.querySelector('#auth-mode-select');
                if (modeSel) { modeSel.value = 'client'; }
              });
            });
          }
        } catch (e) {
          results.innerHTML = '<div class="setting-hint">扫描失败: ' + esc(String(e)) + '</div>';
        } finally {
          lanScanBtn.disabled = false;
          lanScanBtn.textContent = '搜索局域网服务端…';
        }
      });
    }

    // 模式切换保存（Tauri 原生命令，写本地 config/app.toml）
    const modeSaveWrap = section.querySelector('#auth-mode-select');
    if (modeSaveWrap && isTauri()) {
      const saveBtn = document.createElement('button');
      saveBtn.className = 'kb-modal-btn kb-modal-btn-ok';
      saveBtn.textContent = '保存运行模式';
      saveBtn.style.marginBottom = '6px';
      modeSaveWrap.parentElement.appendChild(saveBtn);
      saveBtn.addEventListener('click', async () => {
        const mode = modeSelectSafe();
        const urlEl = section.querySelector('#auth-client-url');
        saveBtn.disabled = true;
        try {
          await tauriInvoke('save_app_config', {
            mode,
            serverUrl: urlEl ? urlEl.value.trim() : null,
            serverBind: null,
          });
          if (mode !== modeSaveWrap.dataset.loadedMode) {
            // 模式变更需重启生效 — 自动重启，消除"保存后仍运行旧模式"的引导断层；
            // 切到服务端模式重启后即进入超级管理员创建界面（users 未初始化时）
            saveBtn.textContent = '✓ 已保存，正在重启…';
            setTimeout(() => { tauriInvoke('restart_app').catch(() => { saveBtn.textContent = '自动重启失败，请手动重启'; }); }, 1200);
          } else {
            saveBtn.textContent = '✓ 已保存';
            setTimeout(() => { saveBtn.textContent = '保存运行模式'; saveBtn.disabled = false; }, 2500);
          }
        } catch (e) {
          saveBtn.textContent = '保存失败';
          setTimeout(() => { saveBtn.textContent = '保存运行模式'; saveBtn.disabled = false; }, 2500);
        }
      });

      function modeSelectSafe() {
        return section.querySelector('#auth-mode-select').value;
      }
      modeSaveWrap.addEventListener('change', () => {
        const wrap = section.querySelector('#auth-client-url-wrap');
        if (wrap) wrap.style.display = modeSelectSafe() === 'client' ? 'block' : 'none';
      });
    }

    // 退出登录（删除凭证）
    const logoutBtn = section.querySelector('#auth-logout-btn');
    if (logoutBtn) {
      logoutBtn.addEventListener('click', async () => {
        try { await fetch('/api/auth/logout', { method: 'POST' }); } catch (e) {}
        safeReload();
      });
    }

    // 修改密码
    const cpBtn = section.querySelector('#auth-cp-btn');
    if (cpBtn) {
      cpBtn.addEventListener('click', async () => {
        if (cpBtn.disabled) return;
        const err = section.querySelector('#auth-cp-error');
        if (err) err.textContent = '';
        const oldPw = section.querySelector('#auth-cp-old').value;
        const newPw = section.querySelector('#auth-cp-new').value;
        const newPw2 = section.querySelector('#auth-cp-new2').value;
        if (newPw !== newPw2) { if (err) err.textContent = '两次输入的新密码不一致'; return; }
        if ((newPw || '').length < 8) { if (err) err.textContent = '新密码至少 8 位'; return; }
        cpBtn.disabled = true;
        try {
          const r = await fetch('/api/auth/change_password', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ old_password: oldPw, new_password: newPw }),
          });
          const data = await r.json();
          if (data.status === 'ok') {
            const banner = document.getElementById('auth-must-change-banner');
            if (banner) banner.remove();
            cpBtn.textContent = '✓ 已修改';
            setTimeout(() => { cpBtn.textContent = '确认修改'; }, 2000);
          } else {
            if (err) err.textContent = data.message || '修改失败';
          }
        } catch (e) {
          if (err) err.textContent = '网络错误';
        } finally {
          cpBtn.disabled = false;
        }
      });
    }

    // 创建用户
    const createBtn = section.querySelector('#auth-user-create-btn');
    if (createBtn) {
      createBtn.addEventListener('click', async () => {
        if (createBtn.disabled) return;
        const err = section.querySelector('#auth-users-error');
        if (err) err.textContent = '';
        const username = section.querySelector('#auth-new-username').value.trim();
        const password = section.querySelector('#auth-new-password').value;
        const password2 = section.querySelector('#auth-new-password2') ? section.querySelector('#auth-new-password2').value : password;
        if (password !== password2) { if (err) err.textContent = '两次输入的密码不一致'; return; }
        const role = section.querySelector('#auth-new-role').value;
        createBtn.disabled = true;
        try {
          const r = await fetch('/api/auth/users', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ username, password, role }),
          });
          const data = await r.json();
          if (data.status === 'ok') {
            section.querySelector('#auth-new-username').value = '';
            section.querySelector('#auth-new-password').value = '';
            const p2 = section.querySelector('#auth-new-password2');
            if (p2) p2.value = '';
            loadUsers();
          } else {
            if (err) err.textContent = data.message || '创建失败';
          }
        } catch (e) {
          if (err) err.textContent = '网络错误';
        } finally {
          createBtn.disabled = false;
        }
      });
    }

    // 白名单添加
    const wlBtn = section.querySelector('#auth-wl-add-btn');
    if (wlBtn) {
      wlBtn.addEventListener('click', async () => {
        const input = section.querySelector('#auth-wl-input');
        const err = section.querySelector('#auth-wl-error');
        if (err) err.textContent = '';
        const ip = (input.value || '').trim();
        if (!ip) return;
        const list = Array.from(section.querySelectorAll('.auth-wl-item span')).map(s => s.textContent);
        if (list.includes(ip)) { if (err) err.textContent = '该 IP 已在白名单中'; return; }
        list.push(ip);
        const savedOk = await saveWhitelist(list, err);
        // 防误锁确认被取消 / 保存失败 → 保留用户输入（曾发生：取消后输入框仍被清空）
        if (savedOk) input.value = '';
      });
    }
  }

  async function loadUsers() {
    const box = document.getElementById('auth-users-list');
    if (!box) return;
    try {
      const r = await fetch('/api/auth/users');
      const data = await r.json();
      if (data.status !== 'ok') { box.textContent = '加载失败'; return; }
      if (!data.users || data.users.length === 0) {
        box.innerHTML = '<p style="font-size:12px;color:var(--text-muted)">暂无用户</p>';
        return;
      }
      box.innerHTML = data.users.map(u => {
        const created = u.created ? new Date(u.created * 1000).toLocaleDateString() : '';
        return `
        <div class="auth-user-row">
          <span><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="13" height="13" style="vertical-align:-2px;margin-right:4px"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>${esc(u.username)} <span class="auth-role-badge ${u.role}">${u.role === 'admin' ? '管理员' : '成员'}</span>${u.must_change_password ? '<span class="auth-self-tag" title="正在使用初始密码，应尽快修改">需改密</span>' : ''}<span style="font-size:11px;color:var(--text-muted);margin-left:6px">${created}</span></span>
          ${currentUser && u.username === currentUser.username
            ? '<span class="auth-self-tag">当前</span>'
            : `<button class="auth-user-del" data-username="${esc(u.username)}" title="删除">✕</button>`}
        </div>
      `; }).join('');
      box.querySelectorAll('.auth-user-del').forEach(btn => {
        btn.addEventListener('click', async () => {
          if (!confirm(`确定删除用户 ${btn.dataset.username}？`)) return;
          try {
            const r = await fetch(`/api/auth/users/${encodeURIComponent(btn.dataset.username)}`, { method: 'DELETE' });
            const data = await r.json();
            if (data.status !== 'ok') {
              const err = document.getElementById('auth-users-error');
              if (err) err.textContent = data.message || '删除失败';
            }
            loadUsers();
          } catch (e) {}
        });
      });
    } catch (e) {
      box.textContent = '加载失败';
    }
  }

  async function loadWhitelist() {
    const box = document.getElementById('auth-whitelist-list');
    if (!box) return;
    try {
      const r = await fetch('/api/auth/whitelist');
      const data = await r.json();
      const ips = (data.status === 'ok' && Array.isArray(data.whitelist)) ? data.whitelist : [];
      if (ips.length === 0) {
        box.innerHTML = '<p style="font-size:12px;color:var(--text-muted)">白名单未启用（所有 IP 可访问）</p>';
        return;
      }
      box.innerHTML = ips.map(ip => `
        <div class="auth-wl-item">
          <span>${esc(ip)}</span>
          <button class="auth-wl-del" data-ip="${esc(ip)}" title="移除">✕</button>
        </div>
      `).join('');
      box.querySelectorAll('.auth-wl-del').forEach(btn => {
        btn.addEventListener('click', async () => {
          const list = Array.from(box.querySelectorAll('.auth-wl-item span'))
            .map(s => s.textContent).filter(x => x !== btn.dataset.ip);
          await saveWhitelist(list, document.getElementById('auth-wl-error'));
        });
      });
    } catch (e) {}
  }

  async function saveWhitelist(ips, errBox) {
    try {
      // 防误锁：启用白名单但当前设备 IP 不在其中 → 保存后本设备将被 403 拦截（含登录页）
      if (ips.length > 0 && bootInfo.client_ip && !ips.includes(bootInfo.client_ip)) {
        const ok = confirm(`当前设备 IP（${bootInfo.client_ip}）不在白名单中。\n\n保存后本设备将无法访问服务端（包括登录页）。确定继续？`);
        if (!ok) return false;
      }
      const r = await fetch('/api/auth/whitelist', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ips }),
      });
      const data = await r.json();
      if (data.status !== 'ok') {
        if (errBox) errBox.textContent = data.message || '保存失败';
        return false;
      }
      loadWhitelist();
      return true;
    } catch (e) {
      if (errBox) errBox.textContent = '网络错误';
      return false;
    }
  }

  // ─── 设置按钮钩子：打开设置时刷新账号/管理面板 ───
  function hookSettings() {
    const openBtn = document.getElementById('home-settings-btn');
    if (openBtn && openBtn.dataset.authHook !== '1') {
      openBtn.dataset.authHook = '1';
      openBtn.addEventListener('click', () => renderSettingsContent());
    }
  }

  // ─── 会话失效处理（视图收到 401 时调用）───
  // 直接显示登录遮罩而非 location.reload()：
  // 未登录时页面重载会重新初始化视图并再次触发 401 → 无限重载循环
  // （托盘菜单在未登录状态切换路由时的"反复横跳"即由此导致）
  function onSessionLost() {
    // 首启 / 模式选择 / 管理员创建进行中 — 不抢遮罩
    for (const id of GUIDANCE_OVERLAYS) {
      const el = document.getElementById(id);
      if (el && el.style.display !== 'none') return;
    }
    const wasLoggedIn = !!currentUser;
    currentUser = null;
    const overlay = document.getElementById('login-overlay');
    if (overlay && overlay.style.display !== 'flex') {
      overlay.style.display = 'flex';
      // 仅对"曾登录后会话过期"提示；从未登录则保持默认文案（boot 会接管）
      if (wasLoggedIn) {
        const err = document.getElementById('login-error');
        if (err) err.textContent = '会话已过期，请重新登录';
      }
    }
  }

  return { boot, hookSettings, onSessionLost, isOverlayActive };
})();
