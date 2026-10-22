// ═══════════════════════════════════════════════════════════════
// SKYLAND 工作区 — 智能体执行工作的位置（对齐 Claude Code 项目目录）
// standalone: 任意本地目录（相对/绝对/浏览选择）| server/client: 相对子路径 或 云工作(@cloud)
// 选定后：会话数据落 {工作区}/.skyland/conversations，bash/文件工具以工作区为 CWD
// ═══════════════════════════════════════════════════════════════

const Workdir = (() => {
  let serverMode = 'standalone';
  const changeCbs = [];
  const K_STATE = SkylandStore.KEYS.workdir;     // {path, cloud}
  const K_RECENT = SkylandStore.KEYS.wsRecent;   // [{path, cloud}] 最近工作区

  function esc(s) { return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }

  function get() {
    try {
      const g = JSON.parse(localStorage.getItem(K_STATE)) || {};
      return { path: g.path || '', cloud: !!g.cloud };
    } catch (e) { return { path: '', cloud: false }; }
  }

  function recentList() {
    try {
      const a = JSON.parse(localStorage.getItem(K_RECENT)) || [];
      return Array.isArray(a) ? a.filter(x => x && (x.path || x.cloud)) : [];
    } catch (e) { return []; }
  }

  // 服务端同步（数据目录持久化 — 跟随数据目录而非浏览器）：写入即时推送，启动时拉取合并
  function syncRecentToServer() {
    fetch('/api/workspace/list', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(recentList()),
    }).catch(() => {});
  }
  function pullRecentFromServer() {
    fetch('/api/workspace/list')
      .then(r => r.json())
      .then(d => {
        if (d && d.status === 'ok' && Array.isArray(d.workspaces) && d.workspaces.length) {
          // 服务端为真源 — 覆盖 localStorage（浏览器缓存只是会话内加速）
          try { localStorage.setItem(K_RECENT, JSON.stringify(d.workspaces)); } catch (e) {}
          updateLabel(); // 最近列表变化可能影响弹窗展示 — 刷新标签
        }
      })
      .catch(() => {});
  }

  function pushRecent(path, cloud) {
    const list = recentList().filter(x => !(x.path === path && !!x.cloud === !!cloud));
    list.unshift({ path, cloud: !!cloud });
    try { localStorage.setItem(K_RECENT, JSON.stringify(list.slice(0, 8))); } catch (e) {}
    syncRecentToServer();
  }

  function removeRecent(path, cloud) {
    const list = recentList().filter(x => !(x.path === path && !!x.cloud === !!cloud));
    try { localStorage.setItem(K_RECENT, JSON.stringify(list)); } catch (e) {}
    syncRecentToServer();
  }

  function fireChange() {
    changeCbs.forEach(cb => { try { cb(); } catch (e) {} });
    updateLabel();
  }

  let lastSwitchAt = 0;
  /// 选定/切换工作区：登记最近 + 初始化 .skyland 结构 + 通知（会话列表刷新等）
  function set(path, cloud) {
    lastSwitchAt = Date.now(); // 世代标记：跨工作区选会话的轮询以此为「旧列表已失效」判据
    localStorage.setItem(K_STATE, JSON.stringify({ path, cloud: !!cloud }));
    if (path || cloud) {
      pushRecent(path, cloud);
      // 初始化 .skyland/{conversations,cache} — 失败不阻断（首条会话保存时会再建）
      fetch('/api/workspace/prepare', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ workdir: cloud ? '@cloud' : path }),
      }).catch(() => {});
    }
    fireChange();
  }

  function clear() {
    localStorage.removeItem(K_STATE);
    fireChange();
  }

  function label() {
    const { path, cloud } = get();
    if (cloud) return '云工作';
    if (path) return display();
    return '目录';
  }

  /// 侧栏/提示用的展示名：目录 basename（绝对路径取末段，相对路径原样）
  function display() {
    const { path, cloud } = get();
    if (cloud) return '云工作';
    if (!path) return '';
    const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/);
    return parts[parts.length - 1] || path;
  }

  function value() {
    const { path, cloud } = get();
    if (cloud && serverMode !== 'standalone') return '@cloud';
    return path || undefined;
  }

  function updateLabel() {
    const el = document.getElementById('workdir-label');
    if (el) el.textContent = display() || '目录';
    const btn = document.getElementById('btn-workdir');
    if (btn) {
      const active = !!get().path || get().cloud;
      btn.classList.toggle('mode-edit', active);
      btn.title = active ? '工作区：' + (get().cloud ? '云工作' : get().path) : '选择智能体工作区';
    }
  }

  function isTauriLike() {
    return typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke;
  }
  // 纯浏览器目录选择兜底（Chromium showDirectoryPicker — 仅暴露文件夹名，
  // 作为相对工作区名使用；桌面壳仍走原生 pick_directory 拿绝对路径）
  function canBrowse() {
    return isTauriLike() || typeof window.showDirectoryPicker === 'function';
  }
  async function browsePick(inputEl, btnEl) {
    btnEl.disabled = true;
    try {
      if (isTauriLike()) {
        const picked = await __TAURI__.core.invoke('pick_directory');
        if (picked) inputEl.value = picked;
      } else if (typeof window.showDirectoryPicker === 'function') {
        const handle = await window.showDirectoryPicker();
        if (handle) inputEl.value = handle.name;
      }
    } catch (e) {
      if (!(e && e.name === 'AbortError')) console.error('pick_directory failed:', e);
    }
    btnEl.disabled = false;
  }

  async function openLocation(path) {
    try {
      if (typeof __TAURI__ !== 'undefined' && __TAURI__.shell && __TAURI__.shell.open) {
        await __TAURI__.shell.open(path);
      } else {
        // 浏览器模式无法打开资源管理器 — 提示路径让用户自行打开
        try { navigator.clipboard.writeText(path); } catch (e) {}
        alert('工作区路径已复制到剪贴板：\n' + path);
      }
    } catch (e) { console.error('open workspace failed:', e); }
  }

  function init() {
    if (window._skylandWorkdirInited) return;
    window._skylandWorkdirInited = true;
    fetch('/api/server/info').then(r => r.json()).then(d => { serverMode = d.mode || 'standalone'; updateLabel(); }).catch(() => {});
    pullRecentFromServer();  // 从数据目录拉取最近工作区（服务端为真源，localStorage 为缓存）
    const btn = document.getElementById('btn-workdir');
    if (btn) btn.addEventListener('click', showDialog);
    const card = document.getElementById('sb-workspace');
    if (card) card.addEventListener('click', showDialog);
    updateLabel();
  }

  function showDialog() {
    const cur = get();
    const overlay = document.createElement('div');
    overlay.className = 'kb-modal-overlay';
    const modal = document.createElement('div');
    modal.className = 'kb-modal';
    const recents = recentList().slice(0, 6);
    const recentsHtml = recents.length ? `
      <label class="setting-label" style="margin-top:4px">最近工作区</label>
      <div style="display:flex;flex-direction:column;gap:4px;margin-bottom:10px">
        ${recents.map((r, i) => `
          <div style="display:flex;align-items:center;gap:6px">
            <button class="kb-modal-btn ws-recent-btn" data-i="${i}" style="flex:1;text-align:left;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12.5px">${esc(r.cloud ? '云工作' : (r.path || ''))}</button>
            <button class="kb-modal-btn ws-rec-del" data-i="${i}" title="从最近列表移除" style="padding:6px 10px;color:var(--text-muted)">✕</button>
          </div>`).join('')}
      </div>` : '';
    modal.innerHTML = `
      <div class="kb-modal-header"><span class="kb-modal-title">工作区</span></div>
      <div class="kb-modal-body">
        <div style="font-size:11.5px;color:var(--text-muted);margin-bottom:10px;line-height:1.6">
          工作区 = 智能体执行工作的位置：文件生成/读写、命令执行都发生在这里，会话数据保存在工作区的 <b>.skyland/</b> 文件夹（对齐 Claude Code 的 .claude）。
        </div>
        ${cur.path || cur.cloud ? `
        <div style="display:flex;align-items:center;gap:8px;margin-bottom:12px;padding:8px 10px;background:var(--bg-hover);border-radius:8px">
          <span style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12.5px">当前：${esc(cur.cloud ? '云工作' : cur.path)}</span>
          ${cur.cloud ? '' : '<button class="kb-modal-btn" id="workdir-open" style="white-space:nowrap;padding:6px 10px">打开位置</button>'}
          <button class="kb-modal-btn" id="workdir-clear" style="white-space:nowrap;padding:6px 10px;color:var(--error)">清除</button>
        </div>` : ''}
        ${recentsHtml}
        <label class="setting-label">${serverMode === 'standalone' ? '本地目录（相对路径，或点"浏览"选任意目录）' : '本地子目录（相对路径）'}</label>
        <div style="display:flex;gap:6px;margin-bottom:8px">
          <input type="text" class="kb-modal-input" id="workdir-input" placeholder="${serverMode === 'standalone' ? '如 Project/报告，或浏览选择任意目录（留空 = 根工作目录）' : '如 Project/报告（留空 = 根工作目录）'}" value="${esc(cur.path)}" style="flex:1">
          ${canBrowse() ? '<button class="kb-modal-btn kb-modal-btn-ok" id="workdir-browse" title="用资源管理器选择目录" style="white-space:nowrap;padding:8px 12px">浏览…</button>' : ''}
        </div>
        <label style="display:flex;align-items:center;gap:8px;font-size:13px;cursor:pointer ${serverMode === 'standalone' ? ';opacity:.4;pointer-events:none' : ''}">
          <input type="checkbox" id="workdir-cloud" ${cur.cloud ? 'checked' : ''}>
          <span>云工作模式${serverMode === 'standalone' ? '（仅 server / client 模式可用）' : ' — 文件生成在服务端 CloudWork/sessions/' + esc(currentConvHint())}</span>
        </label>
        ${serverMode !== 'standalone' ? '<p style="font-size:11px;color:var(--text-muted);margin-top:6px">server / client 模式仅支持相对子路径或云工作（多用户安全）；浏览绝对路径仅在一体模式可用。</p>' : ''}
      </div>
      <div class="kb-modal-footer">
        <button class="kb-modal-btn kb-modal-btn-cancel" id="workdir-cancel">取消</button>
        <button class="kb-modal-btn kb-modal-btn-ok" id="workdir-save">确定</button>
      </div>`;
    overlay.appendChild(modal);
    overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
    modal.querySelector('#workdir-cancel').onclick = () => overlay.remove();
    modal.querySelector('#workdir-save').onclick = () => {
      const path = modal.querySelector('#workdir-input').value.trim();
      const cloud = modal.querySelector('#workdir-cloud').checked;
      if (cloud && serverMode === 'standalone') { overlay.remove(); return; }
      set(path, cloud);
      overlay.remove();
    };
    const openBtn = modal.querySelector('#workdir-open');
    if (openBtn) openBtn.onclick = () => openLocation(cur.path);
    const clearBtn = modal.querySelector('#workdir-clear');
    if (clearBtn) clearBtn.onclick = () => { clear(); overlay.remove(); };
    // 最近列表快捷选择
    modal.querySelectorAll('.ws-recent-btn').forEach(b => b.onclick = () => {
      const r = recents[Number(b.dataset.i)];
      if (r) set(r.path || '', !!r.cloud);
      overlay.remove();
    });
    modal.querySelectorAll('.ws-rec-del').forEach(b => b.onclick = (e) => {
      e.stopPropagation();
      const r = recents[Number(b.dataset.i)];
      if (r) { removeRecent(r.path || '', !!r.cloud); showDialog(); }
    });
    // 浏览按钮 — 原生资源管理器选目录（Tauri 环境）
    const browseBtn = modal.querySelector('#workdir-browse');
    if (browseBtn) browseBtn.addEventListener('click', () => browsePick(modal.querySelector('#workdir-input'), browseBtn));
    document.body.appendChild(overlay);
    setTimeout(() => modal.querySelector('#workdir-input')?.focus(), 50);
  }

  function currentConvHint() { return '{会话id}'; }

  return {
    init, get, set, clear, value, label, display, showDialog, recentList,
    get lastSwitchAt() { return lastSwitchAt; },
    lastLoadedAt: 0, // loadWorkspaceChats 完成时更新（app.js 写入）
    onChange: cb => changeCbs.push(cb),
  };
})();
