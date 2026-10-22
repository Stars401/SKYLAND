// ═══════════════════════════════════════════════════════════════
// SKYLAND 工作区会话树 — 侧栏「工作区 → 会话」层级
// ═══════════════════════════════════════════════════════════════
// 要解决的问题：切换工作区后，原工作区的历史对话"整片消失"。
//
// 病根不是数据丢了，而是**信息架构**错了：
//   chats 是一个**跟随当前工作区的全局数组**。切换工作区 → 整个数组被
//   另一个工作区的数据整体替换 → 侧栏只渲染这一个数组 → 旧对话看不见。
//   数据其实还在各自工作区的 .skyland/conversations/ 里，只是没有入口。
//
// 本模块的职责边界（刻意不碰消息体加载）：
//   · 维护「工作区注册表」——稳定 id、路径、显示名
//   · 为每个工作区拉取**会话元信息**（id/title/modified），不做 N+1 详情请求
//   · 渲染 工作区→会话 树，所有工作区**同时可见**，切换只改"当前"标记
//   · 点击别的工作区下的会话 → 先切工作区再打开
//
// 关键设计：只拉 list（元信息）不拉 get（消息体）。
// 侧栏要的是"有哪些会话"，不是"每条会话的全部内容"。

(function () {
  'use strict';

  if (window.SkylandWsTree) return;

  const K_TREE = 'skyland_ws_tree_index';   // wsId -> {identity, name, convs, mtime, err}
  const K_ORDER = 'skyland_ws_tree_order';  // 工作区展示顺序（最近在前）
  const K_EXPANDED = 'skyland_ws_tree_expanded';  // 用户手动展开的工作区
  const UNGROUPED = '__ungrouped__';        // 无工作区的本地会话归到这一组

  // 展开态与"当前工作区"是两件事：
  //   · 当前 = 智能体实际工作的位置（点名称切换）
  //   · 展开 = 是否想看这个工作区的会话（点箭头切换）
  // 混成一件的话，就没法在不切换工作区的前提下浏览别的目录的历史会话 ——
  // 而"浏览"恰恰是本次要恢复的能力。当前工作区始终强制展开。
  function expandedSet() {
    const v = readJSON(K_EXPANDED, []);
    return new Set(Array.isArray(v) ? v : []);
  }

  function toggleExpand(id) {
    const s = expandedSet();
    if (s.has(id)) s.delete(id); else s.add(id);
    writeJSON(K_EXPANDED, Array.from(s));
  }

  // ── 稳定 id ────────────────────────────────────────────────
  // 必须是**确定性**的：同一路径重新选中要落回同一分组。
  // 若用随机 uuid，每次切换工作区都会新建一个空分组，历史对话就"失联"了。
  function normalizeIdentity(path, cloud) {
    if (cloud) return '@cloud';
    let p = String(path || '').trim();
    if (!p) return '';
    p = p.replace(/\\/g, '/').replace(/\/+$/, '');
    // Windows 盘符大小写不敏感：C:\A 与 c:\a 是同一个目录，必须归一
    p = p.replace(/^([A-Za-z]):/, function (_, d) { return d.toUpperCase() + ':'; });
    return p;
  }

  // FNV-1a 32bit。短、稳定、无依赖。冲突概率对"十几个工作区"可忽略，
  // 且下方仍以 identity 原文做二次校验，冲突时追加序号自愈。
  function hash32(str) {
    let h = 0x811c9dc5;
    for (let i = 0; i < str.length; i++) {
      h ^= str.charCodeAt(i);
      h = (h + ((h << 1) + (h << 4) + (h << 7) + (h << 8) + (h << 24))) >>> 0;
    }
    return h.toString(16).padStart(8, '0');
  }

  function makeId(identity) {
    return 'ws_' + hash32(identity);
  }

  function displayName(identity) {
    if (identity === '@cloud') return '云工作';
    if (!identity) return '未分组';
    const parts = identity.replace(/\/+$/, '').split('/');
    return parts[parts.length - 1] || identity;
  }

  function readJSON(key, fallback) {
    try {
      const v = JSON.parse(localStorage.getItem(key));
      return v === null || v === undefined ? fallback : v;
    } catch (e) { return fallback; }
  }

  function writeJSON(key, value) {
    try { localStorage.setItem(key, JSON.stringify(value)); } catch (e) {}
  }

  function loadIndex() {
    const raw = readJSON(K_TREE, {});
    return (raw && typeof raw === 'object' && !Array.isArray(raw)) ? raw : {};
  }

  function saveIndex(idx) { writeJSON(K_TREE, idx); }

  function loadOrder() {
    const o = readJSON(K_ORDER, []);
    return Array.isArray(o) ? o : [];
  }

  function saveOrder(o) { writeJSON(K_ORDER, o); }

  // ── 外部依赖的取值（Workdir 可能在别的脚本里，且未必已初始化）──
  function currentIdentity() {
    try {
      if (typeof Workdir === 'undefined' || !Workdir.get) return '';
      const g = Workdir.get();
      return normalizeIdentity(g && g.path, g && g.cloud);
    } catch (e) { return ''; }
  }

  /**
   * 登记一个工作区。已存在则原样返回 id（保证 id 稳定）。
   * 这是"切换工作区不丢历史"的地基：每个工作区在索引里有独立条目。
   */
  function ensure(identity) {
    if (!identity) return UNGROUPED;
    const idx = loadIndex();
    let id = makeId(identity);
    // 极小概率哈希碰撞：identity 不同却撞同一个 id，追加序号自愈而非静默覆盖
    if (idx[id] && idx[id].identity !== identity) {
      let n = 2;
      while (idx[id + '_' + n] && idx[id + '_' + n].identity !== identity) n++;
      id = id + '_' + n;
    }
    if (!idx[id]) {
      idx[id] = { identity: identity, name: displayName(identity), convs: [], mtime: 0, err: '' };
      saveIndex(idx);
    }
    // 置顶：最近用过的排前面（同时保持其余相对顺序）
    const order = loadOrder().filter(function (x) { return x !== id; });
    order.unshift(id);
    saveOrder(order);
    return id;
  }

  /**
   * 拉取单个工作区的会话元信息。
   * 只调 list（返回 id/title/messages数/modified），**不**逐条调 get 拉消息体 ——
   * 否则 N 个工作区 × M 条会话 = N+1 次请求，侧栏会卡住。
   */
  async function refreshOne(id) {
    const idx = loadIndex();
    const rec = idx[id];
    if (!rec) return null;
    if (id === UNGROUPED) {
      // 无工作区：会话在 localStorage，没有服务端列表可言
      rec.convs = [];
      rec.mtime = Date.now();
      saveIndex(idx);
      return rec;
    }
    try {
      const wd = encodeURIComponent(rec.identity);
      const r = await fetch('/api/conversations/list?workdir=' + wd);
      if (!r.ok) throw new Error('HTTP ' + r.status);
      const d = await r.json();
      const list = (d && Array.isArray(d.conversations)) ? d.conversations : [];
      rec.convs = list
        .filter(function (c) { return c && c.id; })
        .map(function (c) {
          return {
            id: c.id,
            title: c.title || '对话',
            modified: Number(c.modified) || 0,
            count: Number(c.messages) || 0,
          };
        })
        .sort(function (a, b) { return b.modified - a.modified; });
      rec.err = '';
    } catch (e) {
      // 单个工作区失败不能拖垮整棵树：标记错误、保留上次数据
      rec.err = (e && e.message) || '读取失败';
    }
    rec.mtime = Date.now();
    saveIndex(idx);
    return rec;
  }

  async function refreshAll() {
    // 始终包含当前工作区 —— 用户刚切换过来时，它可能还没进索引
    const curId = ensure(currentIdentity());
    // 自动登记最近工作区：否则树只能在工作区切换后才出现，
    // 首次进入 Agent 终端时侧栏一片空白（用户须先点输入框弹窗才能"唤醒"树）
    if (typeof Workdir !== 'undefined' && Workdir.recentList) {
      try {
        Workdir.recentList().forEach(function (r) {
          if (r.cloud) ensure('@cloud');
          else if (r.path) ensure(normalizeIdentity(r.path));
        });
      } catch (e) {}
    }
    // 注意：ensure 会写入索引 —— 必须在登记**之后**重读，否则新条目被
    // 旧快照过滤掉、永远轮不到 refreshOne（mtime 停在 0、会话列恒空）
    const idx = loadIndex();
    const ids = Array.from(new Set(loadOrder().filter(function (id) { return idx[id]; }).concat([curId])));
    // 串行刷新 —— refreshOne 是「读索引→改本工作区→写回」，并行会互相覆盖
    // （后写者用旧快照抹掉先写者的会话列表，表现为部分工作区会话时有时无）
    for (const id of ids) {
      try { await refreshOne(id); } catch (e) {}
    }
    return ids;
  }

  function esc(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  }

  function fmtWhen(ts) {
    if (!ts) return '';
    const d = new Date(ts);
    if (isNaN(d.getTime())) return '';
    const now = Date.now();
    const sameDay = new Date(now).toDateString() === d.toDateString();
    const hm = String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
    if (sameDay) return hm;
    return (d.getMonth() + 1) + '/' + d.getDate();
  }

  function readUngrouped() {
    try {
      const raw = JSON.parse(localStorage.getItem(SkylandStore.KEYS.chats) || '[]');
      if (!Array.isArray(raw)) return [];
      return raw
        .filter(function (c) { return c && c.id != null; })
        .map(function (c) {
          return {
            id: c.id,
            title: c.title || '对话',
            modified: Number(c.id) || 0,
            count: (c.messages || []).length,
          };
        })
        .sort(function (a, b) { return b.modified - a.modified; });
    } catch (e) { return []; }
  }

  /**
   * 渲染整棵树。所有已登记工作区**同时**出现 —— 这是本次修复的核心：
   * 切走的工作区不再从侧栏消失，它只是不再带"当前"标记。
   */
  function render(activeConvId) {
    const host = document.getElementById('ws-tree');
    if (!host) return;
    const idx = loadIndex();
    const curIdentity = currentIdentity();
    const curId = curIdentity ? makeId(curIdentity) : UNGROUPED;

    const order = loadOrder().filter(function (id) { return idx[id]; });
    // 当前工作区置顶
    order.sort(function (a, b) {
      if (a === curId) return -1;
      if (b === curId) return 1;
      return 0;
    });

    const expanded = expandedSet();
    // 箭头单独可点：只改展开态，不切换工作区（切换 = 点名称）
    const caretHtml = function (id, open) {
      return '<span class="ws-tree-caret' + (open ? ' open' : '') + '" data-toggle="' + esc(id) +
        '" role="button" tabindex="0" title="' + (open ? '收起' : '展开') + '">' +
        (open
          ? '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="11" height="11"><polyline points="6 9 12 15 18 9"/></svg>'
          : '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="11" height="11"><polyline points="9 18 15 12 9 6"/></svg>') +
        '</span>';
    };
    const folder = '<svg class="ws-tree-folder" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="13" height="13"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>';

    function wsBlock(id, rec) {
      const isActive = id === curId;
      const open = isActive || expanded.has(id);
      const convs = rec.convs || [];
      const children = convs.length
        ? convs.map(function (c) {
            const on = String(activeConvId) === String(c.id) && isActive;
            return '<div class="ws-tree-conv' + (on ? ' active' : '') + '" data-ws="' + esc(id) + '" data-conv="' + esc(c.id) +
              '" role="button" tabindex="0" title="' + esc(c.title) + '">' +
              '<span class="ws-tree-conv-title">' + esc(c.title) + '</span>' +
              '<span class="ws-tree-conv-time">' + esc(fmtWhen(c.modified)) + '</span>' +
              '<button class="chat-delete ws-tree-del" data-ws="' + esc(id) + '" data-conv="' + esc(c.id) +
              '" title="删除" aria-label="删除对话">' +
              '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="12" height="12"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>' +
              '</button></div>';
          }).join('')
        : '<div class="ws-tree-empty">' + (rec.err ? esc(rec.err) : '暂无会话') + '</div>';
      return '<div class="ws-tree-ws' + (isActive ? ' active' : '') + (open ? ' open' : '') + '">' +
        '<button class="ws-tree-ws-head" data-ws="' + esc(id) + '" title="' + esc(rec.identity) + '">' +
          caretHtml(id, open) + folder +
          '<span class="ws-tree-ws-name">' + esc(rec.name) + '</span>' +
          (isActive ? '<span class="ws-tree-badge">当前</span>' : '') +
        '</button>' +
        '<div class="ws-tree-children">' + children + '</div>' +
      '</div>';
    }

    const wsHtml = order.length
      ? order.map(function (id) { return wsBlock(id, idx[id]); }).join('')
      : '<div class="ws-tree-empty">还没有工作区，点上方「创建工作区」开始</div>';

    // 最近会话：跨工作区聚合。这是"切走的工作区还能一眼看到"的兜底
    const recent = [];
    order.forEach(function (id) {
      (idx[id].convs || []).forEach(function (c) {
        recent.push({ id: c.id, title: c.title, modified: c.modified, wsId: id, wsName: idx[id].name });
      });
    });
    if (!curIdentity) {
      readUngrouped().forEach(function (c) {
        recent.push({ id: c.id, title: c.title, modified: c.modified, wsId: UNGROUPED, wsName: '未分组' });
      });
    }
    recent.sort(function (a, b) { return b.modified - a.modified; });
    const recentTop = recent.slice(0, 12);

    const recentHtml = recentTop.length
      ? '<div class="sb-group"><div class="sb-group-label">最近会话</div>' + recentTop.map(function (c) {
          const on = String(activeConvId) === String(c.id) && c.wsId === curId;
          return '<div class="chat-item ws-tree-recent' + (on ? ' active' : '') + '" data-ws="' + esc(c.wsId) +
            '" data-conv="' + esc(c.id) + '" role="button" tabindex="0" title="' + esc(c.wsName + ' / ' + c.title) + '">' +
            '<span class="ws-tree-recent-ws">' + esc(c.wsName) + '</span>' +
            '<span class="chat-title-text">' + esc(c.title) + '</span>' +
            '<span class="ws-tree-conv-time">' + esc(fmtWhen(c.modified)) + '</span></div>';
        }).join('') + '</div>'
      : '';

    host.innerHTML = wsHtml + recentHtml;
    bind(host);
  }

  function openConversation(wsId, convId) {
    // 目标不在当前工作区 → 先切过去（触发 onChange 异步重载会话列表），
    // 再**轮询重试** selectChat 直到列表加载完成 —— 固定 300ms 曾撞上
    // 加载未完成（chats 里还没有该 id）→ selectChat 静默失败 = 点击无响应
    const curIdentity = currentIdentity();
    const wantId = curIdentity ? makeId(curIdentity) : UNGROUPED;
    if (wsId !== wantId) {
      const rec = loadIndex()[wsId];
      if (rec && typeof Workdir !== 'undefined' && Workdir.set) {
        if (rec.identity === '@cloud') Workdir.set('', true);
        else if (rec.identity) Workdir.set(rec.identity, false);
        pollSelect(convId, 0);
        return;
      }
    }
    // 同工作区同样走轮询：进入页面后 loadWorkspaceChats 异步未完成时
    // 单次 selectChat 会静默失败（事故 3 的残留变体）
    pollSelect(convId, 0);
  }

  // selectChat 返回 true=已切换；未就绪返回 false → 继续等。
  // 世代护栏：各工作区会话 id 从 1 独立编号，若在**旧列表**上轮询会误开
  // 旧工作区的同 id 会话 —— 必须等 loadWorkspaceChats 完成（世代 > 切换时刻）
  function pollSelect(convId, attempt) {
    if (attempt > 40) {
      // ~5s 上限：列表加载失败属网络层问题 —— 必须给用户可见反馈而非静默
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('会话打开失败：工作区会话列表加载超时', 'error');
      return;
    }
    const genReady = (typeof Workdir === 'undefined') ||
      (Workdir.lastLoadedAt || 0) > (Workdir.lastSwitchAt || 0);
    let ok = false;
    if (genReady) {
      try { ok = (typeof selectChat === 'function') && selectChat(convId) === true; } catch (e) {}
    }
    if (!ok) setTimeout(function () { pollSelect(convId, attempt + 1); }, 130);
  }

  let boundHost = null;

  /**
   * 删除某工作区下的一条会话。
   * 树里能直接看到**非当前**工作区的会话，所以这里不能复用 app.js 的 deleteChat ——
   * 那个函数只作用于当前 `chats` 数组 + 当前工作区。跨工作区删除必须自己带 workdir。
   */
  async function removeConversation(wsId, convId) {
    const rec = loadIndex()[wsId];
    if (!rec) return;
    const target = (rec.convs || []).find(function (c) { return String(c.id) === String(convId); });
    if (!confirm('删除对话「' + ((target && target.title) || '对话') + '」？此操作不可撤销。')) return;
    if (wsId !== UNGROUPED) {
      try {
        const url = '/api/conversations/' + encodeURIComponent(convId) + '?workdir=' + encodeURIComponent(rec.identity);
        await fetch(url, { method: 'DELETE' });
      } catch (e) { /* 网络失败仍从本地索引移除，避免"删不掉"卡住用户 */ }
    }
    const idx = loadIndex();
    if (idx[wsId]) {
      idx[wsId].convs = (idx[wsId].convs || []).filter(function (c) { return String(c.id) !== String(convId); });
      saveIndex(idx);
    }
    // 删掉的正是当前打开的会话 → 清空主区（否则会停在一个已不存在的会话上）
    if (String(window.__skylandActiveConvId) === String(convId)) {
      window.__skylandActiveConvId = null;
      try { if (typeof closeChatView === 'function') closeChatView(); } catch (e) {}
    }
    render(window.__skylandActiveConvId);
    try { if (typeof renderChatList === 'function') renderChatList(); } catch (e) {}
  }

  // ── 工作区右键菜单 ──────────────────────────────────────────
  // 重命名（改显示名，不影响身份/历史）；更换目录（改 identity，历史保留在
  // 原目录）；打开位置（Tauri）；从列表移除（只摘登记，不动磁盘）。
  let menuEl = null;

  function closeWsMenu() {
    if (menuEl) { menuEl.remove(); menuEl = null; }
    document.removeEventListener('click', onDocClickClose, true);
    document.removeEventListener('keydown', onEscClose, true);
  }
  function onDocClickClose(e) {
    if (menuEl && !menuEl.contains(e.target)) closeWsMenu();
  }
  function onEscClose(e) { if (e.key === 'Escape') closeWsMenu(); }

  function openWsMenu(x, y, wsId, rec) {
    closeWsMenu();
    menuEl = document.createElement('div');
    menuEl.className = 'ws-ctx-menu';
    const isCurrent = wsId === (currentIdentity() ? makeId(currentIdentity()) : UNGROUPED);
    const tauri = typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke;
    menuEl.innerHTML =
      '<button data-act="rename">重命名</button>' +
      '<button data-act="retarget">更换目标文件夹…</button>' +
      (tauri ? '<button data-act="open">在资源管理器中打开</button>' : '') +
      '<span class="ws-ctx-sep"></span>' +
      (isCurrent ? '<button data-act="info" class="ws-ctx-dim">当前工作区</button>'
                 : '<button data-act="switch">切换到此工作区</button>') +
      '<button data-act="remove" class="ws-ctx-danger">从列表移除</button>';
    document.body.appendChild(menuEl);
    // 视口感知定位：越界翻转（底部/右侧空间不足时向内收）
    const r = menuEl.getBoundingClientRect();
    menuEl.style.left = Math.max(8, Math.min(x, window.innerWidth - r.width - 8)) + 'px';
    menuEl.style.top = Math.max(8, Math.min(y, window.innerHeight - r.height - 8)) + 'px';
    menuEl.addEventListener('click', function (e) {
      const btn = e.target.closest('button[data-act]');
      if (!btn) return;
      const act = btn.dataset.act;
      closeWsMenu();
      if (act === 'rename') renameWorkspace(wsId, rec);
      else if (act === 'retarget') retargetWorkspace(wsId, rec);
      else if (act === 'open') openInExplorer(rec.identity);
      else if (act === 'switch') { if (rec.identity && typeof Workdir !== 'undefined' && Workdir.set) Workdir.set(rec.identity === '@cloud' ? '' : rec.identity, rec.identity === '@cloud'); }
      else if (act === 'remove') removeWorkspace(wsId, rec);
    });
    setTimeout(function () {
      document.addEventListener('click', onDocClickClose, true);
      document.addEventListener('keydown', onEscClose, true);
    }, 0);
  }

  function wsModal(title, bodyHtml, onOk, okLabel) {
    const overlay = document.createElement('div');
    overlay.className = 'kb-modal-overlay';
    const modal = document.createElement('div');
    modal.className = 'kb-modal';
    modal.style.width = 'min(440px, 92vw)';
    modal.innerHTML = '<div class="kb-modal-header"><span class="kb-modal-title"></span></div>' +
      '<div class="kb-modal-body"></div>' +
      '<div class="kb-modal-footer">' +
      '<button class="kb-modal-btn kb-modal-btn-cancel">取消</button>' +
      '<button class="kb-modal-btn kb-modal-btn-ok"></button></div>';
    modal.querySelector('.kb-modal-title').textContent = title;
    modal.querySelector('.kb-modal-body').innerHTML = bodyHtml;
    modal.querySelector('.kb-modal-btn-ok').textContent = okLabel || '确定';
    overlay.appendChild(modal);
    overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
    modal.querySelector('.kb-modal-btn-cancel').onclick = () => overlay.remove();
    modal.querySelector('.kb-modal-btn-ok').onclick = () => { if (onOk(modal) !== false) overlay.remove(); };
    document.body.appendChild(overlay);
    setTimeout(() => { const inp = modal.querySelector('input'); if (inp) { inp.focus(); inp.select(); } }, 50);
    return modal;
  }

  function renameWorkspace(wsId, rec) {
    wsModal('重命名工作区',
      '<input type="text" class="kb-modal-input" value="' + esc(rec.name) + '" maxlength="40">' +
      '<p style="font-size:11px;color:var(--text-muted);margin-top:8px">仅修改侧栏显示名；工作目录与会话历史不受影响。</p>',
      modal => {
        const v = modal.querySelector('input').value.trim();
        if (!v) return false;
        const idx = loadIndex();
        if (idx[wsId]) { idx[wsId].name = v; idx[wsId].customName = true; saveIndex(idx); render(window.__skylandActiveConvId); }
      }, '保存');
  }

  // 目录选择器可用性：桌面壳用原生 pick_directory；纯浏览器用
  // Chromium 的 showDirectoryPicker（WebView2/Edge/Chrome 均支持，localhost 下可用）。
  function canPickDir() {
    if (typeof __TAURI__ !== 'undefined' && __TAURI__.core && __TAURI__.core.invoke) return 'tauri';
    if (typeof window.showDirectoryPicker === 'function') return 'fs';
    return null;
  }
  async function pickDir() {
    const how = canPickDir();
    try {
      if (how === 'tauri') return await __TAURI__.core.invoke('pick_directory');
      if (how === 'fs') {
        const handle = await window.showDirectoryPicker();
        // 安全限制：浏览器只暴露文件夹名（非绝对路径）— 作为相对工作区名使用
        return handle ? handle.name : null;
      }
    } catch (e) {
      if (e && e.name === 'AbortError') return null; // 用户取消
      console.error('pick directory failed:', e);
    }
    return null;
  }

  function retargetWorkspace(wsId, rec) {
    const how = canPickDir();
    const browseHtml = how
      ? '<button class="kb-modal-btn" id="ws-retarget-browse" title="在资源管理器中选择目录" style="white-space:nowrap;padding:8px 12px;flex:none;display:flex;align-items:center;gap:4px">' +
        '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>浏览…</button>'
      : '';
    const modal = wsModal('更换目标文件夹',
      '<div style="display:flex;gap:8px;align-items:center">' +
      '<input type="text" class="kb-modal-input" value="' + esc(rec.identity) + '" placeholder="目录路径" style="flex:1;margin:0">' + browseHtml + '</div>' +
      '<p style="font-size:11px;color:var(--text-muted);margin-top:8px;line-height:1.7">注意：会话历史仍保留在<b>原目录</b>的 .skyland/ 中；新目录首次使用时会初始化自己的 .skyland。</p>',
      m => {
        const v = m.querySelector('input').value.trim();
        if (!v || v === rec.identity) return false;
        applyRetarget(wsId, rec, v);
      }, '更换');
    const browse = modal.querySelector('#ws-retarget-browse');
    if (browse) browse.addEventListener('click', async () => {
      const picked = await pickDir();
      if (picked) modal.querySelector('input').value = picked;
    });
  }

  function applyRetarget(wsId, rec, newPath) {
    const identity = normalizeIdentity(newPath);
    if (!identity) return;
    const idx = loadIndex();
    if (!idx[wsId]) return;
    idx[wsId].identity = identity;
    // 未手动改过名 → 跟随新目录名；改过名 → 保留用户命名
    if (!idx[wsId].customName) idx[wsId].name = displayName(identity);
    idx[wsId].convs = [];
    idx[wsId].err = '';
    saveIndex(idx);
    fetch('/api/workspace/prepare', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ workdir: identity }),
    }).catch(() => {});
    if (typeof Workdir !== 'undefined' && Workdir.set) {
      const cur = currentIdentity();
      if (cur === rec.identity) Workdir.set(identity, false);
    }
    scheduleRefresh(80);
  }

  function openInExplorer(path) {
    try {
      if (typeof __TAURI__ !== 'undefined' && __TAURI__.shell && __TAURI__.shell.open) {
        __TAURI__.shell.open(path);
      } else {
        try { navigator.clipboard.writeText(path); } catch (e) {}
        alert('工作区路径已复制到剪贴板：\n' + path);
      }
    } catch (e) { console.error(e); }
  }

  function removeWorkspace(wsId, rec) {
    if (!confirm('从列表移除工作区「' + rec.name + '」？\n仅移除侧栏登记，目录与会话历史不会删除。')) return;
    const idx = loadIndex();
    delete idx[wsId];
    saveIndex(idx);
    saveOrder(loadOrder().filter(x => x !== wsId));
    render(window.__skylandActiveConvId);
  }

  function bind(host) {
    if (boundHost === host) return;
    boundHost = host;
    host.addEventListener('contextmenu', function (e) {
      const head = e.target.closest('.ws-tree-ws-head');
      if (!head) return;
      e.preventDefault();
      const rec = loadIndex()[head.dataset.ws];
      if (!rec || rec.identity === '@cloud') return;   // 云工作/未分组不提供目录级操作
      openWsMenu(e.clientX, e.clientY, head.dataset.ws, rec);
    });
    host.addEventListener('click', function (e) {
      const delBtn = e.target.closest('.ws-tree-del');
      if (delBtn) {
        e.preventDefault();
        e.stopPropagation();
        removeConversation(delBtn.dataset.ws, delBtn.dataset.conv);
        return;
      }
      const conv = e.target.closest('.ws-tree-conv, .ws-tree-recent');
      if (conv) {
        e.preventDefault();
        openConversation(conv.dataset.ws, conv.dataset.conv);
        return;
      }
      const head = e.target.closest('.ws-tree-ws-head');
      if (head) {
        e.preventDefault();
        // 点箭头 = 只改展开态（不切换工作区）；点名称 = 切换工作区（**不隐藏**其它工作区）
        const caretEl = e.target.closest('.ws-tree-caret');
        if (caretEl) {
          toggleExpand(caretEl.dataset.toggle);
          render(window.__skylandActiveConvId);
          return;
        }
        const rec = loadIndex()[head.dataset.ws];
        if (!rec || typeof Workdir === 'undefined' || !Workdir.set) return;
        if (rec.identity === '@cloud') Workdir.set('', true);
        else if (rec.identity) Workdir.set(rec.identity, false);
      }
    });
    host.addEventListener('keydown', function (e) {
      if (e.key !== 'Enter' && e.key !== ' ') return;
      const caretEl = e.target.closest('.ws-tree-caret');
      if (caretEl) {
        e.preventDefault();
        toggleExpand(caretEl.dataset.toggle);
        render(window.__skylandActiveConvId);
        return;
      }
      const conv = e.target.closest('.ws-tree-conv, .ws-tree-recent');
      if (!conv) return;
      e.preventDefault();
      openConversation(conv.dataset.ws, conv.dataset.conv);
    });
  }

  // ── 对外 API ───────────────────────────────────────────────
  let timer = null;

  function scheduleRefresh(delay) {
    if (timer) clearTimeout(timer);
    timer = setTimeout(function () {
      timer = null;
      refreshAll()
        .then(function () { render(window.__skylandActiveConvId); })
        .catch(function () {});
    }, delay || 200);
  }

  function init() {
    if (window._skylandWsTreeInited) return;
    window._skylandWsTreeInited = true;
    const cur = currentIdentity();
    if (cur) ensure(cur);
    scheduleRefresh(60);
    if (typeof Workdir !== 'undefined' && Workdir.onChange) {
      Workdir.onChange(function () { ensure(currentIdentity()); scheduleRefresh(80); });
    }
  }

  window.SkylandWsTree = {
    init: init,
    render: render,
    refresh: function () { return refreshAll().then(function () { render(window.__skylandActiveConvId); }); },
    scheduleRefresh: scheduleRefresh,
    openConversation: openConversation,
    removeConversation: removeConversation,
    ensure: ensure,
    normalizeIdentity: normalizeIdentity,
    makeId: makeId,
    currentIdentity: currentIdentity,
    loadIndex: loadIndex,
    UNGROUPED: UNGROUPED,
  };
  window.WsTree = window.SkylandWsTree;
})();
