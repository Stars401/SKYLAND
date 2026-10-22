// ═════════════════════════════════════════════════════════════════
// SKYLAND Knowledge Base — WYSIWYG Editor & Manager (v2)
// Features: contentEditable WYSIWYG, KaTeX math, MD themes, custom modals
// ═════════════════════════════════════════════════════════════════

const KbApp = (() => {
  const $ = id => document.getElementById(id);

  // ─── State ───
  let currentKb = null;
  let currentDoc = null;
  let tree = [];
  let saveTimer = null;
  let isSaving = false;
  let savePending = false;
  let modified = false;
  let activeDropdown = null;
  let renderTimer = null;
  let isRendering = false;

  const settings = {
    font: 'system',
    fontSize: 15,
    lineHeight: 1.8,
    editorWidth: 860,
    sidebarWidth: 250,
    theme: 'dark',
    mdTheme: 'default',
  };

  const K_KB_SETTINGS = SkylandStore.LEGACY_SETTINGS_KEYS.kb;

  // ─── Colors ───
  const TEXT_COLORS = ['#1F2329', '#3370FF', '#F5222D', '#FA8C16', '#FAAD14', '#52C41A', '#13C2C2', '#722ED1', '#EB2F96', '#8F959E'];
  const HIGHLIGHT_COLORS = ['#E8F3FF', '#FFF1E8', '#FFFBDC', '#E6FBE5', '#E8FFFA', '#F0E8FF', '#FFE8F0', '#F0F0F0', '#FFF7D6', '#DEECFF'];

  const FONT_FAMILIES = {
    'system': "system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
    'mono': "'Fira Code', 'JetBrains Mono', 'Consolas', monospace",
  };

  // ─── MD Themes ───
  const MD_THEMES = [
    { id: 'default', name: '默认 (飞书蓝)' },
    { id: 'github', name: 'GitHub' },
    { id: 'newsprint', name: 'Newsprint' },
    { id: 'night', name: '暗夜' },
    { id: 'pixyll', name: 'Pixyll' },
  ];

  // ─── Document Templates ───
  const DOC_TEMPLATES = {
    blank: '',
    tech: `# 文档标题

## 概述

简要描述本文档的目的和范围。

## 背景

描述问题背景、技术选型、相关依赖等。

## 设计方案

### 架构图

\`\`\`
┌──────────┐     ┌──────────┐
│  模块 A  │ ──▶ │  模块 B  │
└──────────┘     └──────────┘
\`\`\`

### 关键流程

1. 第一步
2. 第二步
3. 第三步

## API 定义

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| id   | int  | 是   | 唯一标识 |
| name | str  | 否   | 名称 |

## 注意事项

- 安全性：输入校验、SQL 注入防护
- 性能：缓存策略、批量操作
- 兼容性：版本回退方案

## 变更记录

| 日期 | 版本 | 作者 | 变更 |
|------|------|------|------|
| ${new Date().toISOString().slice(0,10)} | v1.0 | - | 创建 |
`,
    meeting: `# 会议记录

**日期**: ${new Date().toISOString().slice(0,10)}
**参会人**: 
**记录人**: 

## 议题

1. 
2. 

## 讨论内容

### 议题一

**结论**:

**行动项**:
- [ ] 负责人 → 任务描述 → 截止日期

### 议题二

**结论**:

**行动项**:
- [ ] 负责人 → 任务描述 → 截止日期

## 下次会议

**时间**: 
**议题**: 
`,
    reading: `# 读书笔记

**书名**: 
**作者**: 
**阅读日期**: ${new Date().toISOString().slice(0,10)}

## 核心观点

> 

## 章节摘要

### 第一章

### 第二章

## 个人思考

1. 
2. 

## 金句摘录

> 

## 行动计划

- [ ] 
`,
  };

  // ═══════════════════════════════════════════════════════════════
  // Custom Modal System (replaces prompt/alert/confirm)
  // ═══════════════════════════════════════════════════════════════

  function showModal(opts) {
    return new Promise(resolve => {
      const overlay = document.createElement('div');
      overlay.className = 'kb-modal-overlay';

      const modal = document.createElement('div');
      modal.className = 'kb-modal';

      // Header
      const header = document.createElement('div');
      header.className = 'kb-modal-header';
      header.innerHTML = `<span class="kb-modal-title">${esc(opts.title || '提示')}</span>`;
      const closeBtn = document.createElement('button');
      closeBtn.className = 'kb-modal-close';
      closeBtn.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="16" height="16"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>';
      closeBtn.onclick = () => { overlay.remove(); resolve(null); };
      header.appendChild(closeBtn);
      modal.appendChild(header);

      // Body
      const body = document.createElement('div');
      body.className = 'kb-modal-body';

      if (opts.type === 'confirm') {
        const msg = document.createElement('p');
        msg.className = 'kb-modal-message';
        msg.textContent = opts.message || '确定执行此操作？';
        body.appendChild(msg);
      } else if (opts.type === 'prompt') {
        const msg = document.createElement('p');
        msg.className = 'kb-modal-message';
        msg.textContent = opts.message || '';
        body.appendChild(msg);
        const input = document.createElement('input');
        input.className = 'kb-modal-input';
        input.type = 'text';
        input.placeholder = opts.placeholder || '';
        input.value = opts.value || '';
        body.appendChild(input);
        setTimeout(() => input.focus(), 50);
        input.addEventListener('keydown', e => {
          if (e.key === 'Enter') { overlay.remove(); resolve(input.value); }
          if (e.key === 'Escape') { overlay.remove(); resolve(null); }
        });
      } else if (opts.type === 'template') {
        // Template selection grid
        const grid = document.createElement('div');
        grid.className = 'kb-template-grid';

        const nameInput = document.createElement('input');
        nameInput.className = 'kb-modal-input';
        nameInput.type = 'text';
        nameInput.placeholder = opts.placeholder || '输入名称...';
        nameInput.value = opts.value || '';
        body.appendChild(nameInput);

        // 名称必填的行内提示 — 空名称点模板曾静默关弹窗且不创建（用户视角"点了没反应"）
        const nameHint = document.createElement('p');
        nameHint.className = 'kb-form-hint';
        nameHint.style.display = 'none';
        nameHint.textContent = '请先输入名称，再选择模板';
        body.appendChild(nameHint);

        const requireName = () => {
          if (nameInput.value.trim()) return true;
          nameHint.style.display = '';
          nameInput.classList.remove('kb-input-shake');
          void nameInput.offsetWidth; // 重启动画
          nameInput.classList.add('kb-input-shake');
          nameInput.focus();
          return false;
        };
        nameInput.addEventListener('input', () => { nameHint.style.display = 'none'; });
        nameInput.addEventListener('animationend', () => nameInput.classList.remove('kb-input-shake'));

        const templates = opts.templates || [
          { id: 'blank', name: '空白', icon: 'doc', desc: '从零开始' },
          { id: 'tech', name: '技术文档', icon: 'wrench', desc: 'API设计与架构' },
          { id: 'meeting', name: '会议记录', icon: 'note', desc: '会议纪要模板' },
          { id: 'reading', name: '读书笔记', icon: 'book', desc: '阅读摘录' },
        ];

        templates.forEach(tpl => {
          const card = document.createElement('div');
          card.className = 'kb-template-card';
          card.innerHTML = `
            <div class="kb-template-icon">${TPL_ICONS[tpl.icon] || TPL_ICONS.doc}</div>
            <div class="kb-template-name">${esc(tpl.name)}</div>
            <div class="kb-template-desc">${esc(tpl.desc)}</div>
          `;
          card.onclick = () => {
            if (!requireName()) return;
            overlay.remove(); resolve({ name: nameInput.value.trim(), template: tpl.id });
          };
          grid.appendChild(card);
        });
        body.appendChild(grid);
        setTimeout(() => nameInput.focus(), 50);
        nameInput.addEventListener('keydown', e => {
          if (e.key === 'Enter') {
            if (!requireName()) return;
            overlay.remove(); resolve({ name: nameInput.value.trim(), template: 'blank' });
          }
          if (e.key === 'Escape') { overlay.remove(); resolve(null); }
        });
      } else if (opts.type === 'alert') {
        const msg = document.createElement('p');
        msg.className = 'kb-modal-message';
        msg.textContent = opts.message || '';
        body.appendChild(msg);
      }

      modal.appendChild(body);

      // Footer
      const footer = document.createElement('div');
      footer.className = 'kb-modal-footer';

      if (opts.type === 'confirm') {
        const cancelBtn = document.createElement('button');
        cancelBtn.className = 'kb-modal-btn kb-modal-btn-cancel';
        cancelBtn.textContent = '取消';
        cancelBtn.onclick = () => { overlay.remove(); resolve(false); };
        footer.appendChild(cancelBtn);

        const okBtn = document.createElement('button');
        okBtn.className = 'kb-modal-btn kb-modal-btn-ok';
        okBtn.textContent = '确定';
        okBtn.onclick = () => { overlay.remove(); resolve(true); };
        footer.appendChild(okBtn);
      } else if (opts.type === 'prompt') {
        const cancelBtn = document.createElement('button');
        cancelBtn.className = 'kb-modal-btn kb-modal-btn-cancel';
        cancelBtn.textContent = '取消';
        cancelBtn.onclick = () => { overlay.remove(); resolve(null); };
        footer.appendChild(cancelBtn);

        const okBtn = document.createElement('button');
        okBtn.className = 'kb-modal-btn kb-modal-btn-ok';
        okBtn.textContent = '确定';
        okBtn.onclick = () => {
          const input = body.querySelector('.kb-modal-input');
          overlay.remove(); resolve(input ? input.value : null);
        };
        footer.appendChild(okBtn);
      } else if (opts.type === 'alert') {
        const okBtn = document.createElement('button');
        okBtn.className = 'kb-modal-btn kb-modal-btn-ok';
        okBtn.textContent = '知道了';
        okBtn.onclick = () => { overlay.remove(); resolve(true); };
        footer.appendChild(okBtn);
      } else if (opts.type === 'template') {
        const cancelBtn = document.createElement('button');
        cancelBtn.className = 'kb-modal-btn kb-modal-btn-cancel';
        cancelBtn.textContent = '取消';
        cancelBtn.onclick = () => { overlay.remove(); resolve(null); };
        footer.appendChild(cancelBtn);

        const createBtn = document.createElement('button');
        createBtn.className = 'kb-modal-btn kb-modal-btn-ok';
        createBtn.textContent = '创建';
        createBtn.onclick = () => {
          const input = body.querySelector('.kb-modal-input');
          const hint = body.querySelector('.kb-form-hint');
          if (!input || !input.value.trim()) {
            if (hint) hint.style.display = '';
            if (input) { input.classList.remove('kb-input-shake'); void input.offsetWidth; input.classList.add('kb-input-shake'); input.focus(); }
            return;
          }
          overlay.remove(); resolve({ name: input.value.trim(), template: 'blank' });
        };
        footer.appendChild(createBtn);
      }

      modal.appendChild(footer);
      overlay.appendChild(modal);
      overlay.addEventListener('click', e => { if (e.target === overlay) { overlay.remove(); resolve(null); } });

      // Escape key closes all modal types
      const onKey = e => {
        if (e.key === 'Escape') { overlay.remove(); resolve(null); document.removeEventListener('keydown', onKey); }
      };
      document.addEventListener('keydown', onKey);

      document.body.appendChild(overlay);
    });
  }

  // ═══════════════════════════════════════════════════════════════
  // Init
  // ═══════════════════════════════════════════════════════════════
  function init() {
    loadSettings();
    bindEvents();
    buildColorGrids();
    buildThemeSelector();
    applySettings();
    loadKbList();
    loadSkills();
    const skillSave = $('kb-skill-save');
    if (skillSave) skillSave.addEventListener('click', saveSkill);
    const skillContent = $('kb-skill-content');
    if (skillContent) skillContent.addEventListener('input', () => {
      const st = $('kb-skill-status');
      if (st && st.textContent === '已保存') st.textContent = '未保存';
    });
  }

  function onShow() {
    if (currentKb) loadTree(currentKb);
  }

  // ═══════════════════════════════════════════════════════════════
  // Settings
  // ═══════════════════════════════════════════════════════════════
  function loadSettings() {
    if (window.SkylandSettings) {
      const stored = window.SkylandSettings.get('editor') || {};
      if (stored && typeof stored === 'object') Object.assign(settings, stored);
      const theme = window.SkylandSettings.get('appearance.theme');
      if (theme) settings.theme = theme;
    }
    try {
      const s = JSON.parse(localStorage.getItem(K_KB_SETTINGS));
      if (s && !window.SkylandSettings) Object.assign(settings, s);
    } catch (e) {}
    try {
      const gs = JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings));
      if (gs && !window.SkylandSettings) settings.theme = gs.theme || 'dark';
    } catch(e) {}
  }

  function saveSettings() {
    if (window.SkylandSettings) {
      window.SkylandSettings.set('editor', settings);
      window.SkylandSettings.save().catch(() => {});
      return;
    }
    try { localStorage.setItem(K_KB_SETTINGS, JSON.stringify(settings)); } catch(e) {}
  }

  function applySettings() {
    const root = document.documentElement.style;
    const fontBody = FONT_FAMILIES[settings.font] || FONT_FAMILIES['system'];
    root.setProperty('--kb-font-body', fontBody);
    root.setProperty('--kb-font-size', settings.fontSize + 'px');
    root.setProperty('--kb-line-height', String(settings.lineHeight));

    const editor = $('kb-editor-area');
    if (editor) {
      editor.style.fontFamily = fontBody;
      editor.style.fontSize = settings.fontSize + 'px';
      editor.style.lineHeight = String(settings.lineHeight);
      editor.style.maxWidth = settings.editorWidth + 'px';
    }

    const sidebar = $('kb-sidebar');
    if (sidebar) sidebar.style.width = settings.sidebarWidth + 'px';

    // Apply MD theme
    const editorWrap = $('kb-editor');
    if (editorWrap) {
      editorWrap.classList.remove('md-theme-default', 'md-theme-github', 'md-theme-newsprint', 'md-theme-night', 'md-theme-pixyll');
      editorWrap.classList.add('md-theme-' + settings.mdTheme);
    }

    // Sync UI
    const wv = $('kb-width-value'); if (wv) wv.textContent = settings.editorWidth;
    const sv = $('kb-sidebar-value'); if (sv) sv.textContent = settings.sidebarWidth;
    const ws = $('kb-width-slider'); if (ws) ws.value = settings.editorWidth;
    const ss = $('kb-sidebar-slider'); if (ss) ss.value = settings.sidebarWidth;
    const fs = $('kb-font-select'); if (fs) fs.value = settings.font;
    const ts = $('kb-theme-select'); if (ts) ts.value = settings.mdTheme;

    document.querySelectorAll('.kb-fontsize-btn').forEach(b => b.classList.toggle('active', parseInt(b.dataset.size) === settings.fontSize));
    document.querySelectorAll('.kb-lineheight-btn').forEach(b => b.classList.toggle('active', parseFloat(b.dataset.lh) === settings.lineHeight));

    // Restore outline collapse state
    if (settings.outlineCollapsed) {
      const panel = $('kb-outline');
      const expandBtn = $('kb-outline-expand');
      if (panel) { panel.classList.add('collapsed'); panel.style.width = '0'; }
      if (expandBtn) expandBtn.style.display = 'flex';
    }
  }

  // ═══════════════════════════════════════════════════════════════
  // Theme Selector
  // ═══════════════════════════════════════════════════════════════
  function buildThemeSelector() {
    const sel = $('kb-theme-select');
    if (!sel) return;
    sel.innerHTML = MD_THEMES.map(t => `<option value="${t.id}">${t.name}</option>`).join('');
    sel.value = settings.mdTheme;
  }

  // ═══════════════════════════════════════════════════════════════
  // Event Bindings
  // ═══════════════════════════════════════════════════════════════
  function bindEvents() {
    const bh = $('kb-back-home'); if (bh) bh.addEventListener('click', () => Router.navigate('/home'));
    const tt = $('kb-theme-toggle'); if (tt) tt.addEventListener('click', toggleDarkTheme);
    const nk = $('kb-new-kb'); if (nk) nk.addEventListener('click', createKb);
    const nd = $('kb-new-doc'); if (nd) nd.addEventListener('click', createDoc);

    // Mobile menu toggle
    const mb = $('kb-menu-btn');
    if (mb) mb.addEventListener('click', () => {
      const sb = $('kb-sidebar'); if (sb) sb.classList.toggle('kb-sidebar-open');
    });

    // KB search
    const searchInput = $('kb-search-input');
    if (searchInput) {
      let searchTimer = null;
      searchInput.addEventListener('input', () => {
        if (searchTimer) clearTimeout(searchTimer);
        const q = searchInput.value.trim();
        const resultsEl = $('kb-search-results');
        if (!q) { if (resultsEl) resultsEl.style.display = 'none'; return; }
        searchTimer = setTimeout(() => performSearch(q), 300);
      });
    }

    // Settings panel
    const st = $('kb-settings-toggle'); if (st) st.addEventListener('click', () => {
      $('kb-settings-panel').classList.add('open');
      $('kb-settings-overlay').classList.add('active');
    });
    const sc = $('kb-settings-close'); if (sc) sc.addEventListener('click', closeSettings);
    const so = $('kb-settings-overlay'); if (so) so.addEventListener('click', closeSettings);

    // Settings controls
    const fsl = $('kb-font-select'); if (fsl) fsl.addEventListener('change', e => { settings.font = e.target.value; applySettings(); saveSettings(); });
    const tsl = $('kb-theme-select'); if (tsl) tsl.addEventListener('change', e => { settings.mdTheme = e.target.value; applySettings(); saveSettings(); });
    const wsl = $('kb-width-slider'); if (wsl) wsl.addEventListener('input', e => {
      settings.editorWidth = parseInt(e.target.value);
      $('kb-width-value').textContent = settings.editorWidth;
      applySettings(); saveSettings();
    });
    const ssl = $('kb-sidebar-slider'); if (ssl) ssl.addEventListener('input', e => {
      settings.sidebarWidth = parseInt(e.target.value);
      $('kb-sidebar-value').textContent = settings.sidebarWidth;
      applySettings(); saveSettings();
    });
    document.querySelectorAll('.kb-fontsize-btn').forEach(b => b.addEventListener('click', () => {
      settings.fontSize = parseInt(b.dataset.size); applySettings(); saveSettings();
    }));
    document.querySelectorAll('.kb-lineheight-btn').forEach(b => b.addEventListener('click', () => {
      settings.lineHeight = parseFloat(b.dataset.lh); applySettings(); saveSettings();
    }));

    // View mode toggle (edit / split / source)
    document.querySelectorAll('.kb-view-btn').forEach(b => b.addEventListener('click', () => {
      document.querySelectorAll('.kb-view-btn').forEach(x => x.classList.remove('active'));
      b.classList.add('active');
      switchViewMode(b.dataset.mode);
    }));

    // Toolbar
    const tb = $('kb-toolbar'); if (tb) tb.addEventListener('click', e => {
      const btn = e.target.closest('[data-action]');
      if (btn) { e.preventDefault(); handleToolbar(btn.dataset.action, btn); }
    });

    // WYSIWYG editor — contentEditable
    const editor = $('kb-editor-area');
    if (editor) {
      editor.addEventListener('input', onEditorInput);
      editor.addEventListener('keydown', handleEditorKeydown);
      editor.addEventListener('paste', handlePaste);
      editor.addEventListener('mouseup', updateToolbarState);
      editor.addEventListener('keyup', updateToolbarState);
    }

    // Doc title
    const dt = $('kb-doc-title');
    if (dt) dt.addEventListener('change', () => scheduleSave());

    // Resizer
    bindResizer();

    // Close dropdowns
    document.addEventListener('click', e => {
      if (activeDropdown && !e.target.closest('.kb-color-dropdown') && !e.target.closest('[data-action]')) {
        hideDropdowns();
      }
    });

    // Outline collapse/expand
    const oc = $('kb-outline-collapse');
    if (oc) oc.addEventListener('click', () => {
      const panel = $('kb-outline');
      const expandBtn = $('kb-outline-expand');
      if (panel) { panel.classList.add('collapsed'); panel.style.width = '0'; }
      if (expandBtn) expandBtn.style.display = 'flex';
      settings.outlineCollapsed = true;
      saveSettings();
    });
    const oe = $('kb-outline-expand');
    if (oe) oe.addEventListener('click', () => {
      const panel = $('kb-outline');
      if (panel) { panel.classList.remove('collapsed'); panel.style.width = ''; }
      oe.style.display = 'none';
      settings.outlineCollapsed = false;
      saveSettings();
    });

    // Outline click → scroll to heading
    const ol = $('kb-outline-list');
    if (ol) ol.addEventListener('click', e => {
      const item = e.target.closest('.kb-outline-item');
      if (item) {
        const idx = parseInt(item.dataset.idx);
        scrollToHeading(idx);
      }
    });

    // Scroll tracking for outline active state
    const es = $('kb-edit-pane');
    if (es) es.addEventListener('scroll', () => {
      updateOutlineActive();
    });
  }

  function closeSettings() {
    $('kb-settings-panel').classList.remove('open');
    $('kb-settings-overlay').classList.remove('active');
  }

  function toggleDarkTheme() {
    settings.theme = settings.theme === 'dark' ? 'light' : 'dark';
    document.documentElement.classList.toggle('dark', settings.theme === 'dark');
    if (window.SkylandSettings) {
      window.SkylandSettings.set('appearance.theme', settings.theme);
      window.SkylandSettings.save().catch(() => {});
    }
    try {
      const gs = JSON.parse(localStorage.getItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings)) || {};
      gs.theme = settings.theme;
      if (!window.SkylandSettings) localStorage.setItem(SkylandStore.LEGACY_SETTINGS_KEYS.settings, JSON.stringify(gs));
    } catch(e) {}
    saveSettings();
  }

  function bindResizer() {
    const rz = $('kb-resizer');
    if (!rz) return;
    let resizing = false;
    rz.addEventListener('mousedown', e => { resizing = true; document.body.style.cursor = 'col-resize'; e.preventDefault(); });
    document.addEventListener('mousemove', e => {
      if (!resizing) return;
      const w = Math.max(180, Math.min(400, e.clientX));
      const sb = $('kb-sidebar'); if (sb) sb.style.width = w + 'px';
      settings.sidebarWidth = w;
      $('kb-sidebar-value').textContent = w;
    });
    document.addEventListener('mouseup', () => { if (resizing) { resizing = false; document.body.style.cursor = ''; saveSettings(); } });
  }

  // ═══════════════════════════════════════════════════════════════
  // WYSIWYG Editor — Markdown→HTML render + HTML→Markdown save
  // ═══════════════════════════════════════════════════════════════

  // Markdown → HTML, then render into the contentEditable
  function renderMarkdown(md) {
    if (!md) {
      return '<p class="kb-placeholder-line" data-empty="true"><br></p>';
    }
    // Pre-process: protect $$...$$ block math from marked.js paragraph splitting
    const mathBlocks = [];
    let processed = md.replace(/\$\$([\s\S]*?)\$\$/g, (match, latex) => {
      const idx = mathBlocks.length;
      mathBlocks.push(latex.trim());
      return `\n\n@@MATHBLOCK_${idx}@@\n\n`;
    });
    // Also protect inline $...$ from marked.js emphasis parsing
    const mathInlines = [];
    processed = processed.replace(/\$([^\$\n]+)\$/g, (match, latex) => {
      const idx = mathInlines.length;
      mathInlines.push(latex);
      return `@@MATHINLINE_${idx}@@`;
    });

    // Pre-process: fix image URLs BEFORE marked.js so it generates correct <img> tags
    // R1.3: collect per-image width from KB-managed markdown so renderMarkdown can wrap them.
    kbEditorImgWidths = {};
    processed = processed.replace(/!\[([^\]]*)\]\(([^)\s]+)\)\{w=(\d+)\}/g, (m, alt, url, w) => {
      kbEditorImgWidths[url] = parseInt(w, 10);
      return '![' + alt + '](' + url + ')';
    });
    // Also recognize the legacy form: ![alt](url<!--w=NN-->) which the editor wrote before.
    processed = processed.replace(/!\[([^\]]*)\]\(([^)\s]+)<!--w=(\d+)-->\)/g, (m, alt, url, w) => {
      kbEditorImgWidths[url] = parseInt(w, 10);
      return '![' + alt + '](' + url + ')';
    });
    if (currentKb) {
      const kbBase = `/api/kb/${encodeURIComponent(currentKb)}/image/`;
      processed = processed.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (match, alt, url) => {
        // Skip absolute URLs (http, https, /api, data)
        if (/^(https?:|\/api\/|data:)/i.test(url)) return match;
        // Extract just the filename from path (e.g., "图片和附件/image%203.png" → "image%203.png")
        let filename = url.split('/').pop() || url;
        // URL-decode then re-encode for clean URL
        try { filename = decodeURIComponent(filename); } catch(e) {}
        const encoded = encodeURIComponent(filename);
        const newSrc = `${kbBase}${encoded}`;
        if (kbEditorImgWidths[url]) kbEditorImgWidths[newSrc] = kbEditorImgWidths[url];
        return `![${alt}](${newSrc})`;
      });
    }

    // Pre-process: clean up ** wrapping ``` delimiters (from previous buggy saves)
    // Handles: **```, **```hlsl**, ```**, ```hlsl**
    processed = processed.replace(/\*\*```/g, '```');
    processed = processed.replace(/```(\w*)\*\*/g, '```$1');

    // Pre-process: protect code blocks from marked.js parsing
    const codeBlocks = [];
    processed = processed.replace(/```(\w*)\n?([\s\S]*?)```/g, (match, lang, code) => {
      const idx = codeBlocks.length;
      // Clean up: strip ** wrappers from each code line (from previous buggy saves)
      let cleanCode = code.replace(/\n$/, '');
      cleanCode = cleanCode.split('\n').map(line => {
        // Only strip ** if they wrap the ENTIRE line (both start and end)
        const m = line.match(/^(\s*)\*\*([\s\S]+)\*\*(\s*)$/);
        if (m) return m[1] + m[2] + m[3];
        return line;
      }).join('\n');
      codeBlocks.push({ lang, code: cleanCode });
      return `\n\n@@CODEBLOCK_${idx}@@\n\n`;
    });

    let html;
    if (typeof marked !== 'undefined' && marked.parse) {
      marked.setOptions({ breaks: true, gfm: true });
      html = marked.parse(processed);
      // XSS 防护: 移除危险标签 (script, onerror, onclick 等)
      html = html.replace(/<script[\s\S]*?<\/script>/gi, '')
                 .replace(/<iframe[\s\S]*?<\/iframe>/gi, '')
                 .replace(/\son\w+\s*=\s*"[^"]*"/gi, '')
                 .replace(/\son\w+\s*=\s*'[^']*'/gi, '')
                 .replace(/\son\w+\s*=\s*[^\s>]+/gi, '')
                 .replace(/javascript:/gi, '');
    } else {
      html = '<p>' + esc(processed) + '</p>';
    }

    // Restore code blocks
    html = html.replace(/@@CODEBLOCK_(\d+)@@/g, (match, idx) => {
      const block = codeBlocks[parseInt(idx)];
      const lang = block.lang || '';
      const code = block.code || '';
      return '<pre><code class="language-' + lang + '">' + esc(code) + '</code></pre>';
    });

    // Restore math blocks: render with KaTeX
    html = html.replace(/@@MATHBLOCK_(\d+)@@/g, (match, idx) => {
      const latex = mathBlocks[parseInt(idx)];
      if (typeof katex !== 'undefined') {
        try {
          return '<div class="kb-math-rendered">' + katex.renderToString(latex, { displayMode: true, throwOnError: false }) + '</div>';
        } catch(e) { return '<div class="kb-math-rendered">$$' + esc(latex) + '$$</div>'; }
      }
      return '<div class="kb-math-rendered">$$' + esc(latex) + '$$</div>';
    });
    // Restore inline math
    html = html.replace(/@@MATHINLINE_(\d+)@@/g, (match, idx) => {
      const latex = mathInlines[parseInt(idx)];
      if (typeof katex !== 'undefined') {
        try {
          return katex.renderToString(latex, { displayMode: false, throwOnError: false });
        } catch(e) { return '$' + esc(latex) + '$'; }
      }
      return '$' + esc(latex) + '$';
    });

    return html;
  }
  // R1.3: widths map keyed by final <img src> so wrapInlineImages can attach data-width.
  let kbEditorImgWidths = {};

  // Render math formulas in an element using KaTeX auto-render
  // NOTE: Must NOT be named renderMathInElement — that shadows KaTeX's global function
  function renderMathInEl(el) {
    if (!el) return;
    // Check if KaTeX auto-render is available (global function)
    if (typeof window.renderMathInElement !== 'function') return;
    try {
      window.renderMathInElement(el, {
        delimiters: [
          { left: '$$', right: '$$', display: true },
          { left: '$', right: '$', display: false },
        ],
        throwOnError: false,
      });
    } catch(e) {}
  }

  // Load markdown into the WYSIWYG editor
  function loadIntoEditor(md) {
    const editor = $('kb-editor-area');
    if (!editor) return;
    editor.innerHTML = renderMarkdown(md);
    // Highlight code blocks
    editor.querySelectorAll('pre code').forEach(code => {
      if (typeof hljs !== 'undefined') { try { hljs.highlightElement(code); } catch(e) {} }
    });
    // Render math
    renderMathInEl(editor);
    // Build document outline
    updateOutline();
    // Update placeholder state
    updatePlaceholder();
    // R1.3: wrap inline images so users can resize them by dragging the SE handle.
    wrapInlineImages(editor);
    initImageResize();
  }

  // R1.3: wrap every direct <img> in a <span class="kb-img-wrap"> with a SE resize handle.
  function wrapInlineImages(editor) {
    if (!editor) return;
    const imgs = Array.from(editor.querySelectorAll('img'));
    for (const img of imgs) {
      if (img.parentElement && img.parentElement.classList && img.parentElement.classList.contains('kb-img-wrap')) {
        continue;
      }
      const wrap = document.createElement('span');
      wrap.className = 'kb-img-wrap';
      const knownW = kbEditorImgWidths[img.getAttribute('src') || ''] || (img.naturalWidth || img.width || 0);
      if (knownW > 0) { wrap.dataset.width = String(knownW); img.style.width = knownW + 'px'; }
      img.parentNode.insertBefore(wrap, img);
      wrap.appendChild(img);
      const handle = document.createElement('span');
      handle.className = 'kb-img-handle se';
      handle.title = '拖动调整宽度';
      wrap.appendChild(handle);
      img.addEventListener('click', (ev) => {
        ev.stopPropagation();
        editor.querySelectorAll('.kb-img-wrap.kb-img-active').forEach(w => w.classList.remove('kb-img-active'));
        wrap.classList.add('kb-img-active');
      });
    }
  }

  // R1.3: single delegated mousedown/move/up on the editor so we can resize any kb-img-wrap.
  let kbImgResizeBound = false;
  function initImageResize() {
    const editor = $('kb-editor-area');
    if (!editor || kbImgResizeBound) return;
    kbImgResizeBound = true;
    let dragState = null;
    editor.addEventListener('mousedown', (e) => {
      const handle = e.target.closest && e.target.closest('.kb-img-handle');
      if (!handle) return;
      const wrap = handle.closest('.kb-img-wrap');
      if (!wrap) return;
      e.preventDefault();
      const img = wrap.querySelector('img');
      const startW = img.getBoundingClientRect().width;
      const startX = e.clientX;
      wrap.classList.add('kb-img-resizing');
      dragState = { wrap, img, startW, startX };
    });
    document.addEventListener('mousemove', (e) => {
      if (!dragState) return;
      const dx = e.clientX - dragState.startX;
      const next = Math.max(40, Math.round(dragState.startW + dx));
      dragState.wrap.dataset.width = String(next);
      dragState.img.style.width = next + 'px';
    });
    document.addEventListener('mouseup', () => {
      if (!dragState) return;
      dragState.wrap.classList.remove('kb-img-resizing');
      dragState = null;
      scheduleSave();
    });
    editor.addEventListener('blur', () => {
      editor.querySelectorAll('.kb-img-wrap.kb-img-active').forEach(w => w.classList.remove('kb-img-active'));
    }, true);
  }

  // R1.4: KB breadcrumb rendered into #kb-breadcrumb — clicking a segment navigates back to it.
  function renderBreadcrumb() {
    const el = $('kb-breadcrumb');
    if (!el) return;
    el.innerHTML = '';
    if (!currentKb) {
      el.innerHTML = '<span class="kb-breadcrumb-empty">未选择文档</span>';
      return;
    }
    const parts = [];
    parts.push({ name: currentKb, path: '', kind: 'kb' });
    if (currentDoc) {
      const segs = currentDoc.split('/').filter(Boolean);
      let acc = '';
      for (let i = 0; i < segs.length; i++) {
        acc = acc ? acc + '/' + segs[i] : segs[i];
        parts.push({ name: segs[i].replace(/\.md$/, ''), path: acc, kind: i === segs.length - 1 ? 'doc' : 'dir' });
      }
    }
    parts.forEach((p, i) => {
      const span = document.createElement('span');
      span.className = 'kb-breadcrumb-item' + (i === parts.length - 1 ? ' current' : '');
      span.textContent = p.name;
      span.title = p.path || p.name;
      if (i < parts.length - 1) {
        span.addEventListener('click', () => {
          if (p.kind === 'kb') {
            selectKb(currentKb);
          } else if (p.path.endsWith('.md')) {
            selectDoc(p.path);
          } else {
            const t = $('kb-doc-title'); if (t) t.value = '';
            currentDoc = null;
            showEmpty();
          }
        });
      }
      el.appendChild(span);
      if (i < parts.length - 1) {
        const sep = document.createElement('span');
        sep.className = 'kb-breadcrumb-sep';
        sep.textContent = '›';
        el.appendChild(sep);
      }
    });
  }

  // R1.7: KB overview cards (documents / folders / chars / current kb).
  function computeTreeStats(nodes, acc) {
    if (!Array.isArray(nodes)) return acc;
    for (const n of nodes) {
      if (n.is_dir) { acc.dirs++; if (n.children) computeTreeStats(n.children, acc); }
      else acc.docs++;
    }
    return acc;
  }
  function renderKbOverview() {
    const el = $('kb-overview');
    if (!el) return;
    if (!currentKb) { el.innerHTML = ''; return; }
    const stats = { docs: 0, dirs: 0 };
    computeTreeStats(tree, stats);
    const chars = (getEditorMarkdown() || '').length;
    el.innerHTML = `
      <div class="kb-overview-cell" title="文档数量"><span class="kb-overview-num">${stats.docs}</span><span class="kb-overview-label">文档</span></div>
      <div class="kb-overview-cell" title="目录数量"><span class="kb-overview-num">${stats.dirs}</span><span class="kb-overview-label">目录</span></div>
      <div class="kb-overview-cell" title="当前文档字数"><span class="kb-overview-num">${chars}</span><span class="kb-overview-label">当前字数</span></div>
      <div class="kb-overview-cell" title="当前知识库"><span class="kb-overview-num" style="font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(currentKb)}</span><span class="kb-overview-label">知识库</span></div>
    `;
  }

  // HTML → Markdown converter (recursive DOM walker)
  function htmlToMarkdown(root) {
    let md = '';
    for (const node of root.childNodes) {
      md += nodeToMarkdown(node);
    }
    return md.replace(/\n{3,}/g, '\n\n').trim();
  }

  function nodeToMarkdown(node) {
    if (node.nodeType === Node.TEXT_NODE) {
      return node.textContent;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return '';

    const tag = node.tagName.toLowerCase();
    const inner = () => {
      let s = '';
      for (const c of node.childNodes) s += nodeToMarkdown(c);
      return s;
    };

    switch (tag) {
      case 'h1': return '\n# ' + inner().trim() + '\n\n';
      case 'h2': return '\n## ' + inner().trim() + '\n\n';
      case 'h3': return '\n### ' + inner().trim() + '\n\n';
      case 'h4': return '\n#### ' + inner().trim() + '\n\n';
      case 'h5': return '\n##### ' + inner().trim() + '\n\n';
      case 'h6': return '\n###### ' + inner().trim() + '\n\n';
      case 'p': {
        // Math block inside a <p>
        if (node.classList && node.classList.contains('kb-math-block')) {
          const editEl = node.querySelector('.kb-math-edit');
          const latex = editEl ? editEl.textContent.trim() : '';
          return latex ? '\n$$' + latex + '$$\n\n' : '';
        }
        const text = inner().trim();
        return text ? text + '\n\n' : '';
      }
      case 'div': {
        // Math block (interactive, with edit box)
        if (node.classList && node.classList.contains('kb-math-block')) {
          const editEl = node.querySelector('.kb-math-edit');
          const latex = editEl ? editEl.textContent.trim() : '';
          return latex ? '\n$$' + latex + '$$\n\n' : '';
        }
        // Math block (rendered, read-only)
        if (node.classList && node.classList.contains('kb-math-rendered')) {
          // Try to extract original LaTeX from KaTeX annotation
          const annotation = node.querySelector('.katex annotation');
          if (annotation) {
            return '\n$$' + annotation.textContent + '$$\n\n';
          }
          // Fallback: use text content
          const text = node.textContent.trim();
          return text ? '\n$$' + text + '$$\n\n' : '';
        }
        return inner() + '\n';
      }
      case 'br': return '\n';
      case 'strong': case 'b': return '**' + inner() + '**';
      case 'em': case 'i': return '*' + inner() + '*';
      case 's': case 'del': case 'strike': return '~~' + inner() + '~~';
      case 'code': {
        // Inline code (not inside pre)
        if (node.parentElement && node.parentElement.tagName === 'PRE') {
          return node.textContent;
        }
        return '`' + inner() + '`';
      }
      case 'pre': {
        const code = node.querySelector('code');
        let lang = '';
        let text;
        if (code) {
          const m = code.className.match(/language-(\w+)/);
          if (m) lang = m[1];
          // Always use textContent for code blocks — prevents markdown formatting inside code
          text = code.textContent;
        } else {
          text = node.textContent;
        }
        return '\n```' + lang + '\n' + text.trim() + '\n```\n\n';
      }
      case 'blockquote': {
        const content = inner().trim();
        return '\n' + content.split('\n').map(l => l.startsWith('> ') ? l : '> ' + l).join('\n') + '\n\n';
      }
      case 'ul': {
        let items = '';
        for (const li of node.children) {
          if (li.tagName === 'LI') {
            items += '- ' + (() => { let s=''; for(const c of li.childNodes) s+=nodeToMarkdown(c); return s.trim(); })() + '\n';
          }
        }
        return '\n' + items + '\n';
      }
      case 'ol': {
        let items = '';
        let idx = 1;
        for (const li of node.children) {
          if (li.tagName === 'LI') {
            items += idx + '. ' + (() => { let s=''; for(const c of li.childNodes) s+=nodeToMarkdown(c); return s.trim(); })() + '\n';
            idx++;
          }
        }
        return '\n' + items + '\n';
      }
      case 'li': return inner();
      case 'a': {
        const href = node.getAttribute('href') || '';
        return '[' + inner() + '](' + href + ')';
      }
      case 'img': {
        const src = node.getAttribute('src') || '';
        const alt = node.getAttribute('alt') || '';
        const wrap = node.parentElement && node.parentElement.classList && node.parentElement.classList.contains('kb-img-wrap') ? node.parentElement : null;
        const w = wrap && wrap.dataset && wrap.dataset.width ? parseInt(wrap.dataset.width, 10) : 0;
        if (w > 0) return '![' + alt + '](' + src + '<!--w=' + w + '-->)';
        return '![' + alt + '](' + src + ')';
      }
      case 'hr': return '\n---\n\n';
      case 'table': return tableToMarkdown(node);
      case 'span': {
        // Preserve inline styles (color, background, font-size)
        const style = node.getAttribute('style') || '';
        if (style) {
          return '<span style="' + esc(style) + '">' + inner() + '</span>';
        }
        return inner();
      }
      case 'section': return inner() + '\n';
      case 'input': {
        if (node.type === 'checkbox') {
          return node.checked ? '- [x] ' : '- [ ] ';
        }
        return '';
      }
      default: return inner();
    }
  }

  function tableToMarkdown(table) {
    const rows = table.querySelectorAll('tr');
    if (rows.length === 0) return '';
    let md = '\n';
    let cellCount = 0;
    rows.forEach((row, ri) => {
      const cells = row.querySelectorAll('th, td');
      if (ri === 0) cellCount = cells.length;
      md += '| ' + Array.from(cells).map(c => c.textContent.trim()).join(' | ') + ' |\n';
      if (ri === 0) {
        md += '|' + Array.from({length: cellCount}).map(() => '---').join('|') + '|\n';
      }
    });
    return md + '\n';
  }

  // Get current markdown from the editor
  function getEditorMarkdown() {
    const editor = $('kb-editor-area');
    if (!editor) return '';
    return htmlToMarkdown(editor);
  }

  // ═══════════════════════════════════════════════════════════════
  // Editor Input Handler (debounced live re-render)
  // ═══════════════════════════════════════════════════════════════

  let viewMode = 'edit';

  function switchViewMode(mode) {
    viewMode = mode;
    const body = $('kb-editor-body');
    const editPane = $('kb-edit-pane');
    const sourcePane = $('kb-source-pane');
    const sourceCode = $('kb-source-code');
    const editor = $('kb-editor-area');
    if (!body || !editPane || !sourcePane) return;

    body.classList.remove('kb-mode-split', 'kb-mode-source');
    editPane.style.display = '';
    sourcePane.style.display = 'none';

    if (mode === 'split') {
      body.classList.add('kb-mode-split');
      sourcePane.style.display = '';
      if (sourceCode) sourceCode.textContent = getEditorMarkdown();
    } else if (mode === 'source') {
      body.classList.add('kb-mode-source');
      // Show markdown source in the editor area itself
      if (editor) {
        const md = getEditorMarkdown();
        editor.textContent = md;
      }
    } else {
      // edit mode — if coming from source, re-render
      if (editor && editor.textContent && !editor.querySelector('h1, h2, h3, p, div, pre, ul, ol, blockquote')) {
        // Editor contains raw text (source mode) — re-render
        const md = editor.textContent;
        loadIntoEditor(md);
      }
    }
  }

  function updateSourcePane() {
    if (viewMode !== 'split') return;
    const sourceCode = $('kb-source-code');
    if (sourceCode) sourceCode.textContent = getEditorMarkdown();
  }

  function onEditorInput() {
    modified = true;
    updateWordCount();
    scheduleSave();
    updatePlaceholder();
    updateSourcePane();

    // Debounced re-render for markdown shortcuts + outline update
    if (renderTimer) clearTimeout(renderTimer);
    renderTimer = setTimeout(() => {
      handleMarkdownShortcuts();
      updateOutline();
    }, 400);
  }

  // Show/hide placeholder when editor is empty
  function updatePlaceholder() {
    const editor = $('kb-editor-area');
    if (!editor) return;
    const isEmpty = editor.textContent.trim() === '' && editor.querySelectorAll('img, hr, table, pre, blockquote').length === 0;
    editor.classList.toggle('kb-editor-empty', isEmpty);
  }

  // Check and apply markdown shortcuts (e.g. user typed "# " → heading)
  function handleMarkdownShortcuts() {
    const editor = $('kb-editor-area');
    if (!editor) return;
    const sel = window.getSelection();
    if (!sel.rangeCount) return;

    // Get the current block element
    let block = sel.anchorNode;
    if (block.nodeType === Node.TEXT_NODE) block = block.parentElement;
    if (!block) return;

    // Walk up to find the block-level element
    while (block && block !== editor) {
      const tag = block.tagName?.toLowerCase();
      if (['p', 'h1', 'h2', 'h3', 'h4', 'li', 'blockquote'].includes(tag)) break;
      block = block.parentElement;
    }
    if (!block || block === editor) return;

    const text = block.textContent;
    const tag = block.tagName.toLowerCase();

    // Check for markdown prefix patterns
    if (tag === 'p') {
      // Heading: # , ## , ### 
      const headingMatch = text.match(/^(#{1,6})\s+(.*)/);
      if (headingMatch) {
        const level = headingMatch[1].length;
        const content = headingMatch[2];
        block.outerHTML = `<h${level}>${esc(content)}</h${level}>`;
        // Place cursor at end of new heading
        const newEl = editor.querySelector(`h${level}:last-of-type`);
        if (newEl) placeCaretAtEnd(newEl);
        return;
      }
      // Blockquote: > 
      if (/^>\s+/.test(text)) {
        const content = text.replace(/^>\s+/, '');
        block.outerHTML = `<blockquote><p>${esc(content)}</p></blockquote>`;
        const newBlock = editor.querySelector('blockquote:last-of-type p');
        if (newBlock) placeCaretAtEnd(newBlock);
        return;
      }
      // Unordered list: - , * 
      if (/^[-*]\s+/.test(text)) {
        const content = text.replace(/^[-*]\s+/, '');
        block.outerHTML = `<ul><li>${esc(content)}</li></ul>`;
        const li = editor.querySelector('ul:last-of-type li:last-of-type');
        if (li) placeCaretAtEnd(li);
        return;
      }
      // Ordered list: 1. 
      if (/^\d+\.\s+/.test(text)) {
        const content = text.replace(/^\d+\.\s+/, '');
        block.outerHTML = `<ol><li>${esc(content)}</li></ol>`;
        const li = editor.querySelector('ol:last-of-type li:last-of-type');
        if (li) placeCaretAtEnd(li);
        return;
      }
      // Code block: ```
      if (/^```/.test(text)) {
        const codeText = text.replace(/^```/, '').trim();
        block.outerHTML = `<pre><code>${esc(codeText)}</code></pre>`;
        const code = editor.querySelector('pre:last-of-type code');
        if (code) placeCaretAtEnd(code);
        return;
      }
      // Math block: $$
      if (/^\$\$/.test(text)) {
        const latex = text.replace(/^\$\$/, '').trim();
        const mathHtml = `<div class="kb-math-block" contenteditable="false">
          <span class="kb-math-edit" contenteditable="true" data-placeholder="输入 LaTeX 公式...">${esc(latex || 'E=mc^2')}</span>
          <span class="kb-math-render"></span>
        </div>`;
        block.outerHTML = mathHtml;
        const mathBlock = editor.querySelector('.kb-math-block:last-of-type');
        if (mathBlock) {
          const editEl = mathBlock.querySelector('.kb-math-edit');
          const renderEl = mathBlock.querySelector('.kb-math-render');
          if (editEl && renderEl) {
            // Render initial math with KaTeX
            const initialLatex = latex || 'E=mc^2';
            renderMath(initialLatex, renderEl);
            // On input: re-render
            editEl.addEventListener('input', () => {
              const l = editEl.textContent.trim();
              renderMath(l, renderEl);
              onEditorInput();
            });
            editEl.focus();
            const range = document.createRange();
            range.selectNodeContents(editEl);
            const selection = window.getSelection();
            selection.removeAllRanges();
            selection.addRange(range);
          }
        }
        renderMathInEl(editor);
        return;
      }
      // Horizontal rule: --- or ***
      if (/^(-{3,}|\*{3,})$/.test(text.trim())) {
        block.outerHTML = '<hr>';
        return;
      }
    }
  }

  function placeCaretAtEnd(el) {
    const range = document.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  // ═══════════════════════════════════════════════════════════════
  // Editor Keyboard Handler
  // ═══════════════════════════════════════════════════════════════

  function handleEditorKeydown(e) {
    // Ctrl+B / Ctrl+I / Ctrl+S
    if (e.ctrlKey || e.metaKey) {
      switch (e.key.toLowerCase()) {
        case 'b': e.preventDefault(); document.execCommand('bold'); onEditorInput(); break;
        case 'i': e.preventDefault(); document.execCommand('italic'); onEditorInput(); break;
        case 's': e.preventDefault(); saveDoc(); break;
      }
    }

    // Enter key — handle block creation
    if (e.key === 'Enter' && !e.shiftKey) {
      // Let the browser handle it, then check for markdown shortcuts
      setTimeout(() => { handleMarkdownShortcuts(); }, 10);
    }

    // Tab key — indent in lists, otherwise insert spaces
    if (e.key === 'Tab') {
      e.preventDefault();
      document.execCommand('insertText', false, '  ');
    }

    // Space key — check for markdown shortcuts after space
    if (e.key === ' ') {
      setTimeout(() => { handleMarkdownShortcuts(); }, 10);
    }
  }

  // ═══════════════════════════════════════════════════════════════
  // Toolbar State
  // ═══════════════════════════════════════════════════════════════

  function updateToolbarState() {
    const btns = document.querySelectorAll('.kb-tb-btn[data-action]');
    btns.forEach(btn => {
      const action = btn.dataset.action;
      let active = false;
      try {
        switch (action) {
          case 'bold': active = document.queryCommandState('bold'); break;
          case 'italic': active = document.queryCommandState('italic'); break;
          case 'strike': active = document.queryCommandState('strikeThrough'); break;
          case 'code': active = document.queryCommandState('code'); break;
        }
      } catch(e) {}
      btn.classList.toggle('active', active);
    });
  }

  // ═══════════════════════════════════════════════════════════════
  // Toolbar Actions (WYSIWYG execCommand)
  // ═══════════════════════════════════════════════════════════════

  function handleToolbar(action, btn) {
    const editor = $('kb-editor-area');
    if (!editor) return;
    editor.focus();

    switch (action) {
      case 'h1': document.execCommand('formatBlock', false, 'H1'); break;
      case 'h2': document.execCommand('formatBlock', false, 'H2'); break;
      case 'h3': document.execCommand('formatBlock', false, 'H3'); break;
      case 'bold': document.execCommand('bold'); break;
      case 'italic': document.execCommand('italic'); break;
      case 'strike': document.execCommand('strikeThrough'); break;
      case 'quote': document.execCommand('formatBlock', false, 'BLOCKQUOTE'); break;
      case 'ul': document.execCommand('insertUnorderedList'); break;
      case 'ol': document.execCommand('insertOrderedList'); break;
      case 'code': document.execCommand('formatBlock', false, 'PRE'); break;
      case 'codeblock': {
        // Feishu-style: wrap selection in code block
        const sel = window.getSelection();
        if (sel.rangeCount > 0 && !sel.isCollapsed) {
          const range = sel.getRangeAt(0);
          const pre = document.createElement('pre');
          const code = document.createElement('code');
          code.textContent = range.toString();
          pre.appendChild(code);
          range.deleteContents();
          range.insertNode(pre);
        } else {
          const pre = document.createElement('pre');
          const code = document.createElement('code');
          code.textContent = '// code';
          pre.appendChild(code);
          document.execCommand('insertHTML', false, pre.outerHTML);
        }
        break;
      }
      case 'hr': document.execCommand('insertHTML', false, '<hr><p><br></p>'); break;
      case 'table': {
        const tableHtml = '<table><thead><tr><th>列1</th><th>列2</th><th>列3</th></tr></thead><tbody><tr><td>内容</td><td>内容</td><td>内容</td></tr></tbody></table><p><br></p>';
        document.execCommand('insertHTML', false, tableHtml);
        break;
      }
      case 'link': {
        const sel = window.getSelection();
        const selectedText = sel.rangeCount > 0 ? sel.toString() : '';
        showModal({
          title: '插入链接',
          type: 'prompt',
          message: '输入链接 URL：',
          value: 'https://',
          placeholder: 'https://example.com',
        }).then(url => {
          if (url) {
            const text = selectedText || '链接';
            document.execCommand('insertHTML', false, `<a href="${esc(url)}" target="_blank">${esc(text)}</a>`);
            onEditorInput();
          }
        });
        break;
      }
      case 'image': {
        showModal({
          title: '插入图片',
          type: 'prompt',
          message: '输入图片 URL（或粘贴图片到编辑器）：',
          placeholder: 'https://example.com/image.png',
        }).then(url => {
          if (url) {
            document.execCommand('insertHTML', false, `<img src="${esc(url)}" alt="图片" /><p><br></p>`);
            onEditorInput();
          }
        });
        break;
      }
      case 'math': {
        // Insert a math block with editing box
        insertMathBlock();
        break;
      }
      case 'source': {
        // Cycle: edit → source → edit
        if (viewMode === 'source') {
          document.querySelectorAll('.kb-view-btn').forEach(x => x.classList.remove('active'));
          document.querySelector('.kb-view-btn[data-mode="edit"]')?.classList.add('active');
          switchViewMode('edit');
        } else {
          document.querySelectorAll('.kb-view-btn').forEach(x => x.classList.remove('active'));
          document.querySelector('.kb-view-btn[data-mode="source"]')?.classList.add('active');
          switchViewMode('source');
        }
        break;
      }
      case 'color': showColorDropdown('color', btn); break;
      case 'highlight': showColorDropdown('highlight', btn); break;
      case 'fontsize': showFontsizeDropdown(btn); break;
    }

    onEditorInput();
  }

  // ═══════════════════════════════════════════════════════════════
  // Math Formula Block
  // ═══════════════════════════════════════════════════════════════

  function insertMathBlock() {
    const editor = $('kb-editor-area');
    if (!editor) return;
    editor.focus();

    // Insert a block math placeholder
    const html = '<p class="kb-math-block" contenteditable="false"><span class="kb-math-edit" contenteditable="true" data-placeholder="输入 LaTeX 公式...">E=mc^2</span><span class="kb-math-render">$$E=mc^2$$</span></p><p><br></p>';
    document.execCommand('insertHTML', false, html);

    // Find the inserted math block and set up editing
    const mathBlocks = editor.querySelectorAll('.kb-math-block');
    const lastBlock = mathBlocks[mathBlocks.length - 1];
    if (lastBlock) {
      const editEl = lastBlock.querySelector('.kb-math-edit');
      const renderEl = lastBlock.querySelector('.kb-math-render');
      if (editEl && renderEl) {
        // Render initial math
        renderMath(editEl.textContent, renderEl);

        // On input: re-render
        editEl.addEventListener('input', () => {
          const latex = editEl.textContent.trim();
          renderMath(latex, renderEl);
          onEditorInput();
        });
        // Focus the edit area
        editEl.focus();
        // Select all text
        const range = document.createRange();
        range.selectNodeContents(editEl);
        const sel = window.getSelection();
        sel.removeAllRanges();
        sel.addRange(range);
      }
    }
  }

  function renderMath(latex, container) {
    if (typeof katex !== 'undefined' && latex) {
      try {
        katex.render(latex, container, { displayMode: true, throwOnError: false });
      } catch(e) {
        container.textContent = '$$' + latex + '$$';
      }
    } else {
      container.textContent = '$$' + latex + '$$';
    }
  }

  // ═══════════════════════════════════════════════════════════════
  // Color & Font Size Dropdowns
  // ═══════════════════════════════════════════════════════════════

  function buildColorGrids() {
    const colorGrid = $('kb-color-grid');
    if (colorGrid) {
      colorGrid.innerHTML = TEXT_COLORS.map(c =>
        `<button class="kb-color-cell" data-color="${c}" style="background:${c}"></button>`
      ).join('');
      colorGrid.querySelectorAll('.kb-color-cell').forEach(cell => {
        cell.addEventListener('click', () => {
          document.execCommand('foreColor', false, cell.dataset.color);
          hideDropdowns();
          onEditorInput();
        });
      });
    }

    const hlGrid = $('kb-highlight-grid');
    if (hlGrid) {
      hlGrid.innerHTML = HIGHLIGHT_COLORS.map(c =>
        `<button class="kb-color-cell" data-color="${c}" style="background:${c}"></button>`
      ).join('');
      hlGrid.querySelectorAll('.kb-color-cell').forEach(cell => {
        cell.addEventListener('click', () => {
          document.execCommand('hiliteColor', false, cell.dataset.color);
          hideDropdowns();
          onEditorInput();
        });
      });
    }

    const fsList = $('kb-fontsize-list');
    if (fsList) {
      fsList.querySelectorAll('button').forEach(btn => {
        btn.addEventListener('click', () => {
          // Use span wrapping for precise font size
          const sel = window.getSelection();
          if (sel.rangeCount > 0 && !sel.isCollapsed) {
            const range = sel.getRangeAt(0);
            const span = document.createElement('span');
            span.style.fontSize = btn.dataset.size;
            span.appendChild(range.extractContents());
            range.insertNode(span);
          } else {
            document.execCommand('fontSize', false, '7');
            const fonts = document.querySelectorAll('font[size="7"]');
            fonts.forEach(f => {
              f.style.fontSize = btn.dataset.size;
              f.removeAttribute('size');
            });
          }
          hideDropdowns();
          onEditorInput();
        });
      });
    }
  }

  function showColorDropdown(type, btn) {
    hideDropdowns();
    const id = type === 'color' ? 'kb-color-dropdown' : type === 'highlight' ? 'kb-highlight-dropdown' : 'kb-fontsize-dropdown';
    const dropdown = $(id);
    if (!dropdown) return;
    const rect = btn.getBoundingClientRect();
    dropdown.style.display = 'block';
    dropdown.style.left = rect.left + 'px';
    dropdown.style.top = (rect.bottom + 4) + 'px';
    activeDropdown = dropdown;
  }

  function hideDropdowns() {
    ['kb-color-dropdown', 'kb-highlight-dropdown', 'kb-fontsize-dropdown'].forEach(id => {
      const el = $(id); if (el) el.style.display = 'none';
    });
    activeDropdown = null;
  }

  // ═══════════════════════════════════════════════════════════════
  // Image Paste
  // ═══════════════════════════════════════════════════════════════

  function handlePaste(e) {
    if (!currentKb) return;
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of items) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        const blob = item.getAsFile();
        if (blob) uploadImage(blob);
        return;
      }
    }
    // For text/HTML paste: force plain text to avoid style pollution
    e.preventDefault();
    const text = e.clipboardData.getData('text/plain');
    if (text) {
      document.execCommand('insertText', false, text);
    }
  }

  async function uploadImage(blob) {
    if (!currentKb) {
      showModal({ title: '提示', type: 'alert', message: '请先选择知识库' });
      return;
    }
    const ext = blob.type.split('/')[1] || 'png';
    const filename = `image_${Date.now()}.${ext}`;
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/image`, {
        method: 'POST',
        headers: { 'X-Filename': filename, 'Content-Type': blob.type },
        body: blob,
      });
      const data = await safeJson(r);
      if (data && data.status === 'ok') {
        document.execCommand('insertHTML', false, `<img src="${data.url}" alt="图片" /><p><br></p>`);
        onEditorInput();
      } else {
        showModal({ title: '上传失败', type: 'alert', message: (data && data.message) || '图片上传失败' });
      }
    } catch (e) {
      showModal({ title: '上传失败', type: 'alert', message: '图片上传失败: ' + e.message });
    }
  }

  // ═══════════════════════════════════════════════════════════════
  // KB List
  // ═══════════════════════════════════════════════════════════════

  async function performSearch(query) {
    if (!currentKb) return;
    const resultsEl = $('kb-search-results');
    if (!resultsEl) return;
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/search?path=${encodeURIComponent(query)}`);
      const data = await safeJson(r);
      const results = (data && data.results) || [];
      if (results.length === 0) {
        resultsEl.innerHTML = '<div class="kb-search-empty">未找到匹配文档</div>';
      } else {
        // R1.2: highlight via <mark> when the backend splits around the match.
        resultsEl.innerHTML = results.map(r => `
          <div class="kb-search-item" data-path="${r.path}">
            <div class="kb-search-item-name">${esc(r.name)} <span class="kb-search-item-count">${r.matches}</span></div>
            <div class="kb-search-item-snippet">${formatSearchSnippet(r)}</div>
          </div>
        `).join('');
        resultsEl.querySelectorAll('.kb-search-item').forEach(item => {
          item.addEventListener('click', () => {
            selectDoc(item.dataset.path);
            resultsEl.style.display = 'none';
            const si = $('kb-search-input'); // searchInput 是 bindEvents 局部变量，此处不可达（曾抛 ReferenceError）
            if (si) si.value = '';
          });
        });
      }
      resultsEl.style.display = 'block';
    } catch(e) { resultsEl.innerHTML = '<div class="kb-search-empty">搜索失败</div>'; resultsEl.style.display = 'block'; }
  }

  // R1.2: render before/<mark>match</mark>/after when backend provided the split, otherwise fall back.
  function formatSearchSnippet(r) {
    if (r && typeof r.before === 'string' && typeof r.match_text === 'string') {
      const head = r.before.length > 60 ? '…' + r.before.slice(-60) : r.before;
      const tail = r.after && r.after.length > 60 ? r.after.slice(0, 60) + '…' : (r.after || '');
      return esc(head) + '<mark class="kb-search-mark">' + esc(r.match_text) + '</mark>' + esc(tail);
    }
    return esc((r && r.snippet) || '');
  }
  async function loadKbList() {
    try {
      const r = await fetch('/api/kb/list');
      const data = await safeJson(r);
      renderKbList(data.kbs || []);
    } catch (e) { console.error('loadKbList', e); }
  }

  function renderKbList(kbs) {
    const el = $('kb-list');
    if (!el) return;
    if (kbs.length === 0) {
      el.innerHTML = '<div class="kb-list-empty">暂无知识库<br><span class="kb-list-hint">点击 + 创建</span></div>';
      return;
    }
    el.innerHTML = kbs.map(kb => `
      <div class="kb-item${kb.name === currentKb ? ' active' : ''}" data-name="${esc(kb.name)}">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/></svg>
        <span class="kb-item-name">${esc(kb.name)}</span>
        <span class="kb-item-count">${kb.doc_count}</span>
        <button class="kb-item-del" data-del="${esc(kb.name)}" title="删除"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="12" height="12"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg></button>
      </div>
    `).join('');

    el.querySelectorAll('.kb-item').forEach(item => {
      item.addEventListener('click', e => {
        if (e.target.closest('[data-del]')) return;
        selectKb(item.dataset.name);
      });
    });
    el.querySelectorAll('[data-del]').forEach(btn => {
      btn.addEventListener('click', e => {
        e.stopPropagation();
        showModal({
          title: '删除知识库',
          type: 'confirm',
          message: `确定删除知识库 "${btn.dataset.del}"？此操作不可恢复。`,
        }).then(ok => { if (ok) deleteKb(btn.dataset.del); });
      });
    });
  }

  async function createKb() {
    const result = await showModal({
      title: '新建知识库',
      type: 'template',
      placeholder: '输入知识库名称...',
      templates: [
        { id: 'blank', name: '空白知识库', icon: 'folder', desc: '从零开始创建' },
        { id: 'tech', name: '技术文档库', icon: 'wrench', desc: 'API设计与架构文档' },
        { id: 'meeting', name: '会议记录库', icon: 'note', desc: '会议纪要与决策' },
        { id: 'reading', name: '读书笔记库', icon: 'book', desc: '阅读摘录与思考' },
      ],
    });
    if (!result || !result.name) return;

    try {
      const r = await fetch('/api/kb/create', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: result.name }),
      });
      const data = await safeJson(r);
      if (data && data.status === 'ok') {
        // If template selected, create a template doc
        if (result.template && result.template !== 'blank') {
          currentKb = result.name;
          const tplContent = DOC_TEMPLATES[result.template] || '';
          const docName = result.template === 'tech' ? '技术文档模板' : result.template === 'meeting' ? '会议记录模板' : '读书笔记模板';
          await fetch(`/api/kb/${encodeURIComponent(result.name)}/doc`, {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ path: docName + '.md', content: tplContent }),
          });
        }
        loadKbList();
        selectKb(result.name);
      } else {
        showModal({ title: '创建失败', type: 'alert', message: (data && data.message) || '创建失败' });
      }
    } catch (e) {
      showModal({ title: '创建失败', type: 'alert', message: '创建失败: ' + e.message });
    }
  }

  async function deleteKb(name) {
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(name)}`, { method: 'DELETE' });
      const data = await safeJson(r);
      if (data && data.status === 'ok') {
        if (currentKb === name) {
          currentKb = null; currentDoc = null;
          $('kb-tree-section').style.display = 'none';
          showEmpty();
        }
        loadKbList();
      } else {
        showModal({ title: '删除失败', type: 'alert', message: (data && data.message) || '删除失败' });
      }
    } catch (e) {
      showModal({ title: '删除失败', type: 'alert', message: '删除失败: ' + e.message });
    }
  }

  async function selectKb(name) {
    await flushPendingSave();
    currentKb = name; currentDoc = null;
    $('kb-tree-section').style.display = '';
    $('kb-current-name').textContent = name;
    loadKbList();
    loadTree(name);
    showEmpty();
    // R1.4 / R1.7: refresh breadcrumb and overview when switching KBs.
    renderBreadcrumb();
    renderKbOverview();
  }

  // ═══════════════════════════════════════════════════════════════
  // Document Tree
  // ═══════════════════════════════════════════════════════════════

  async function loadTree(name) {
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(name)}/tree`);
      const data = await safeJson(r);
      tree = data.tree || [];
      renderTree(tree);
      // R1.7: overview depends on the tree totals.
      renderKbOverview();
    } catch (e) { console.error('loadTree', e); }
  }

  function renderTree(nodes) {
    const container = $('kb-tree');
    if (!container) return;
    if (nodes.length === 0) {
      container.innerHTML = '<div class="kb-tree-empty">暂无文档<br><span class="kb-list-hint">点击 + 创建</span></div>';
      return;
    }
    container.innerHTML = '';
    renderTreeNodes(container, nodes);
  }

  function renderTreeNodes(container, nodes) {
    nodes.forEach(node => {
      container.appendChild(createTreeItem(node));
      if (node.is_dir && node.children) {
        renderTreeNodes(container, node.children);
      }
    });
  }

  function formatDate(ts) {
    if (!ts) return '';
    const d = new Date(ts * 1000);
    const now = new Date();
    const diff = (now - d) / 1000;
    if (diff < 60) return '刚刚';
    if (diff < 3600) return Math.floor(diff / 60) + '分钟前';
    if (diff < 86400) return Math.floor(diff / 3600) + '小时前';
    if (diff < 2592000) return Math.floor(diff / 86400) + '天前';
    return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`;
  }

  function createTreeItem(node) {
    const item = document.createElement('div');
    item.className = 'kb-tree-item' + (node.is_dir ? ' is-dir collapsed' : '') + (node.path === currentDoc ? ' active' : '');
    const depth = node.path.split('/').length - 1;
    item.style.paddingLeft = (10 + depth * 16) + 'px';
    item.dataset.path = node.path;
    item.dataset.isDir = node.is_dir ? '1' : '0';
    item.draggable = !node.is_dir;

    const iconStr = node.icon || '';
    const dateStr = formatDate(node.modified);
    const iconSvg = KB_DOC_ICONS[iconStr] || '';
    const iconHtml = iconSvg ? `<span class="kb-tree-doc-icon">${iconSvg}</span>` : '';

    if (node.is_dir) {
      item.innerHTML = `
        <svg class="kb-tree-arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="12" height="12"><polyline points="9 18 15 12 9 6"/></svg>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
        <span>${esc(node.name)}</span>
        <span class="kb-tree-date">${dateStr}</span>
      `;
      item.querySelector('.kb-tree-arrow').addEventListener('click', e => {
        e.stopPropagation();
        item.classList.toggle('collapsed');
      });
    } else {
      item.innerHTML = `
        ${iconHtml}
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14" style="${iconStr ? 'display:none' : ''}"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
        <span class="kb-tree-doc-name">${esc(node.name)}</span>
        <span class="kb-tree-date">${dateStr}</span>
        <button class="kb-tree-action" data-action="history" data-path="${node.path}" title="历史版本"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="11" height="11"><path d="M3 3v5h5"/><path d="M3.05 13A9 9 0 1 0 6 5.3L3 8"/><path d="M12 7v5l4 2"/></svg></button>
        <button class="kb-tree-action" data-action="icon" data-path="${node.path}" title="设置图标"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="11" height="11"><circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" y1="9" x2="9.01" y2="9"/><line x1="15" y1="9" x2="15.01" y2="9"/></svg></button>
        <button class="kb-tree-action" data-action="delete" data-path="${node.path}" title="删除"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="11" height="11"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg></button>
      `;
      item.addEventListener('click', () => selectDoc(node.path));
    }

    // Action buttons
    item.querySelectorAll('.kb-tree-action').forEach(btn => {
      btn.addEventListener('click', e => {
        e.stopPropagation();
        const action = btn.dataset.action;
        const path = btn.dataset.path;
        if (action === 'delete') {
          showModal({ title: '删除文档', type: 'confirm', message: '确定删除此文档？此操作不可恢复。' })
            .then(ok => { if (ok) deleteDoc(path); });
        } else if (action === 'history') {
          showHistory(path);
        } else if (action === 'icon') {
          showIconPicker(path);
        }
      });
    });

    // Drag-drop
    if (!node.is_dir) {
      item.addEventListener('dragstart', e => {
        e.dataTransfer.setData('text/plain', node.path);
        e.dataTransfer.effectAllowed = 'move';
        item.classList.add('kb-tree-dragging');
      });
      item.addEventListener('dragend', () => {
        item.classList.remove('kb-tree-dragging');
      });
    }
    if (node.is_dir) {
      item.addEventListener('dragover', e => {
        e.preventDefault();
        e.dataTransfer.dropEffect = 'move';
        item.classList.add('kb-tree-dragover');
      });
      item.addEventListener('dragleave', () => {
        item.classList.remove('kb-tree-dragover');
      });
      item.addEventListener('drop', e => {
        e.preventDefault();
        item.classList.remove('kb-tree-dragover');
        const srcPath = e.dataTransfer.getData('text/plain');
        if (srcPath) moveDoc(srcPath, node.path);
      });
    }

    return item;
  }

  async function moveDoc(path, targetDir) {
    if (!currentKb || path === targetDir) return;
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/move`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path, target_dir: targetDir }),
      });
      const text = await r.text();
      let data;
      try { data = JSON.parse(text); } catch(e) {
        showModal({ title: '移动失败', type: 'alert', message: '服务器返回非 JSON' });
        return;
      }
      if (data && data.status === 'ok') {
        if (currentDoc === path) { currentDoc = data.new_path; }
        loadTree(currentKb);
      } else {
        showModal({ title: '移动失败', type: 'alert', message: (data && data.message) || '移动失败' });
      }
    } catch (e) {
      showModal({ title: '移动失败', type: 'alert', message: '移动失败: ' + e.message });
    }
  }

  async function showHistory(path) {
    if (!currentKb) return;
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/history?path=${encodeURIComponent(path)}`);
      const text = await r.text();
      let data;
      try { data = JSON.parse(text); } catch(e) {
        showModal({ title: '加载历史', type: 'alert', message: '服务器返回非 JSON: ' + text.slice(0, 100) });
        return;
      }
      const history = data.history || [];

      const overlay = document.createElement('div');
      overlay.className = 'kb-modal-overlay';
      const modal = document.createElement('div');
      modal.className = 'kb-modal';
      modal.innerHTML = `
        <div class="kb-modal-header">
          <span class="kb-modal-title">版本历史 — ${esc(path.replace(/\.md$/, ''))}</span>
        </div>
        <div class="kb-modal-body">
          ${history.length === 0 ? '<p class="kb-modal-message">暂无历史版本</p>' :
            history.map(h => `
              <div class="kb-history-item" data-filename="${esc(h.filename)}">
                <span class="kb-history-date">${formatDate(h.timestamp)}</span>
                <span class="kb-history-size">${Math.round(h.size / 1024 * 10) / 10}KB</span>
                <button class="kb-modal-btn kb-modal-btn-ok kb-history-restore" data-filename="${esc(h.filename)}">恢复</button>
              </div>
            `).join('')
          }
        </div>
        <div class="kb-modal-footer">
          <button class="kb-modal-btn kb-modal-btn-cancel">关闭</button>
        </div>
      `;
      overlay.appendChild(modal);
      overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
      modal.querySelector('.kb-modal-btn-cancel').onclick = () => overlay.remove();

      modal.querySelectorAll('.kb-history-restore').forEach(btn => {
        btn.addEventListener('click', async () => {
          const filename = btn.dataset.filename;
          overlay.remove();
          await restoreVersion(path, filename);
        });
      });

      document.body.appendChild(overlay);
    } catch (e) {
      showModal({ title: '加载历史', type: 'alert', message: '加载失败: ' + e.message });
    }
  }

  async function restoreVersion(path, filename) {
    if (!currentKb) return;
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/history/restore`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path, filename }),
      });
      const text = await r.text();
      let data;
      try { data = JSON.parse(text); } catch(e) {
        showModal({ title: '恢复失败', type: 'alert', message: '服务器返回非 JSON: ' + text.slice(0, 100) });
        return;
      }
      if (data && data.status === 'ok') {
        loadIntoEditor(data.content);
        modified = true;
        scheduleSave();
        showModal({ title: '恢复成功', type: 'alert', message: '已恢复到历史版本，当前版本已自动备份。' });
      } else {
        showModal({ title: '恢复失败', type: 'alert', message: (data && data.message) || '恢复失败' });
      }
    } catch (e) {
      showModal({ title: '恢复失败', type: 'alert', message: '恢复失败: ' + e.message });
    }
  }

  function showIconPicker(path) {
    const iconEntries = Object.entries(KB_DOC_ICONS);
    const overlay = document.createElement('div');
    overlay.className = 'kb-modal-overlay';
    const modal = document.createElement('div');
    modal.className = 'kb-modal';
    modal.innerHTML = `
      <div class="kb-modal-header">
        <span class="kb-modal-title">设置文档图标</span>
      </div>
      <div class="kb-modal-body">
        <div class="kb-icon-grid">
          ${iconEntries.map(([key, svg]) => `<button class="kb-icon-cell" data-icon="${key}" title="${key}">${svg}</button>`).join('')}
          <button class="kb-icon-cell kb-icon-none" data-icon="" title="无图标">无</button>
        </div>
      </div>
      <div class="kb-modal-footer">
        <button class="kb-modal-btn kb-modal-btn-cancel">取消</button>
      </div>
    `;
    overlay.appendChild(modal);
    overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
    modal.querySelector('.kb-modal-btn-cancel').onclick = () => overlay.remove();

    modal.querySelectorAll('.kb-icon-cell').forEach(btn => {
      btn.addEventListener('click', async () => {
        const icon = btn.dataset.icon;
        overlay.remove();
        await setDocIcon(path, icon);
      });
    });

    document.body.appendChild(overlay);
  }

  async function setDocIcon(path, icon) {
    if (!currentKb) return;
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/icon`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path, icon }),
      });
      const text = await r.text();
      let data;
      try { data = JSON.parse(text); } catch(e) {
        showModal({ title: '设置图标', type: 'alert', message: '服务器返回非 JSON: ' + text.slice(0, 100) });
        return;
      }
      if (data && data.status === 'ok') {
        loadTree(currentKb);
      } else {
        showModal({ title: '设置图标', type: 'alert', message: (data && data.message) || '设置失败' });
      }
    } catch (e) {
      showModal({ title: '设置图标', type: 'alert', message: '设置失败: ' + e.message });
    }
  }

  async function createDoc() {
    if (!currentKb) return;
    const result = await showModal({
      title: '新建文档',
      type: 'template',
      placeholder: '输入文档名称...',
      templates: [
        { id: 'blank', name: '空白文档', icon: 'doc', desc: '从零开始编写' },
        { id: 'tech', name: '技术文档', icon: 'wrench', desc: 'API设计与架构模板' },
        { id: 'meeting', name: '会议记录', icon: 'note', desc: '会议纪要模板' },
        { id: 'reading', name: '读书笔记', icon: 'book', desc: '阅读摘录模板' },
      ],
    });
    if (!result || !result.name) return;

    const name = result.name;
    const path = name.endsWith('.md') ? name : name + '.md';
    const content = DOC_TEMPLATES[result.template] || '';

    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/doc`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path, content: content || '# ' + name + '\n\n' }),
      });
      const data = await safeJson(r);
      if (data && data.status === 'ok') {
        loadTree(currentKb);
        selectDoc(path);
      } else {
        showModal({ title: '创建失败', type: 'alert', message: (data && data.message) || '创建失败' });
      }
    } catch (e) {
      showModal({ title: '创建失败', type: 'alert', message: '创建失败: ' + e.message });
    }
  }

  async function deleteDoc(path) {
    if (!currentKb) return;
    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/doc?path=${encodeURIComponent(path)}`, { method: 'DELETE' });
      const data = await safeJson(r);
      if (data && data.status === 'ok') {
        if (currentDoc === path) { currentDoc = null; showEmpty(); }
        loadTree(currentKb);
      } else {
        showModal({ title: '删除失败', type: 'alert', message: (data && data.message) || '删除失败' });
      }
    } catch (e) {
      showModal({ title: '删除失败', type: 'alert', message: '删除失败: ' + e.message });
    }
  }

  async function selectDoc(path) {
    await flushPendingSave();

    currentDoc = path;
    $('kb-tree').querySelectorAll('.kb-tree-item').forEach(item => {
      const p = item.querySelector('[data-path]');
      item.classList.toggle('active', p && p.dataset.path === path);
    });

    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(currentKb)}/doc?path=${encodeURIComponent(path)}`);
      const text = await r.text();
      let data;
      try { data = JSON.parse(text); } catch(e) {
        showModal({ title: '加载失败', type: 'alert', message: '服务器返回非 JSON: ' + text.slice(0, 100) });
        return;
      }
      if (data && data.status === 'ok') {
        $('kb-empty').style.display = 'none';
        $('kb-editor').style.display = '';
        const kse = $('kb-skill-editor'); if (kse) kse.style.display = 'none';
        currentSkill = null;
        $('kb-doc-title').value = path.replace(/\.md$/, '').split('/').pop();
        loadIntoEditor(data.content);
        modified = false;
        updateWordCount();
        $('kb-save-status').textContent = '已保存';
        // R1.4 / R1.7: surface the breadcrumb and overview card for the new doc.
        renderBreadcrumb();
        renderKbOverview();
      } else {
        showModal({ title: '加载失败', type: 'alert', message: (data && data.message) || '加载失败' });
      }
    } catch (e) {
      showModal({ title: '加载失败', type: 'alert', message: '加载失败: ' + e.message });
    }
  }

  function showEmpty() {
    $('kb-empty').style.display = 'flex';
    $('kb-editor').style.display = 'none';
    const kse = $('kb-skill-editor'); if (kse) kse.style.display = 'none';
    currentSkill = null;
  }

  // ═══════════════════════════════════════════════════════════════
  // 技能库（Skill）— 内置技能在知识库内可视化查看/编辑
  // 数据面：/api/skills/list + /api/skills/doc（GET 原文 / POST 保存）
  // 编辑语义：SKILL.md 原文（front matter + 正文），保存即对 use_skill 生效
  // ═══════════════════════════════════════════════════════════════
  let currentSkill = null;
  let skillsCache = [];

  async function loadSkills() {
    const list = $('kb-skill-list');
    if (!list) return;
    try {
      const r = await fetch('/api/skills/list');
      const d = await safeJson(r);
      skillsCache = (d && d.skills) || [];
      const cnt = $('kb-skill-count');
      if (cnt) cnt.textContent = skillsCache.length ? skillsCache.length + ' 项' : '';
      if (!skillsCache.length) {
        list.innerHTML = '<div class="kb-skill-empty">容器中暂无技能</div>';
        return;
      }
      list.innerHTML = skillsCache.map(sk => `
        <div class="kb-skill-item${currentSkill === sk.name ? ' active' : ''}" data-skill="${esc(sk.name)}" role="button" tabindex="0" title="${esc(sk.description || sk.title)}">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="13" height="13"><path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><polyline points="13 2 13 9 20 9"/></svg>
          <span class="kb-skill-item-name">${esc(sk.title || sk.name)}</span>
          <span class="kb-skill-item-tag${sk.builtin ? '' : ' custom'}">${sk.builtin ? '内置' : '自定义'}</span>
        </div>`).join('');
      // 一次性赋值后再绑监听（innerHTML 重建会销毁监听器 — 项目约定）
      list.querySelectorAll('.kb-skill-item').forEach(el => {
        el.addEventListener('click', () => selectSkill(el.dataset.skill));
        el.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); selectSkill(el.dataset.skill); } });
      });
    } catch (e) {
      list.innerHTML = '<div class="kb-skill-empty">技能清单加载失败</div>';
    }
  }

  async function selectSkill(name) {
    const meta = skillsCache.find(x => x.name === name);
    try {
      const r = await fetch('/api/skills/doc?name=' + encodeURIComponent(name));
      const d = await safeJson(r);
      if (!d || d.status !== 'ok') throw new Error((d && d.message) || '读取失败');
      currentSkill = name; // 保持知识库选择，仅切换主区视图
      $('kb-empty').style.display = 'none';
      $('kb-editor').style.display = 'none';
      $('kb-skill-editor').style.display = 'flex';
      $('kb-skill-title').textContent = (meta && meta.title) || name;
      $('kb-skill-builtin').textContent = meta && meta.builtin ? '内置' : '自定义';
      $('kb-skill-builtin').className = 'kb-skill-tag' + (meta && meta.builtin ? '' : ' custom');
      $('kb-skill-content').value = d.content;
      $('kb-skill-status').textContent = '';
    } catch (e) {
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('技能读取失败：' + e.message, 'error');
      else showModal({ title: '读取失败', type: 'alert', message: String(e.message || e) });
    }
    loadSkills();
  }

  async function saveSkill() {
    if (!currentSkill) return;
    const content = $('kb-skill-content').value;
    const btn = $('kb-skill-save');
    btn.disabled = true; btn.textContent = '保存中…';
    try {
      const r = await fetch('/api/skills/doc', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: currentSkill, content }),
      });
      const d = await safeJson(r);
      if (d && d.status === 'ok') {
        const msg = d.warning ? ('已保存（注意：' + d.warning + '）') : '技能已保存';
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show(msg, d.warning ? 'error' : '');
        $('kb-skill-status').textContent = d.warning ? '注意：缺 front matter' : '已保存';
      } else {
        throw new Error((d && d.message) || '保存失败');
      }
    } catch (e) {
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('技能保存失败：' + e.message, 'error');
    }
    btn.disabled = false; btn.textContent = '保存';
    loadSkills();
  }

  // ═══════════════════════════════════════════════════════════════
  // Save
  // ═══════════════════════════════════════════════════════════════

  function scheduleSave() {
    if (saveTimer) clearTimeout(saveTimer);
    $('kb-save-status').textContent = '编辑中…（自动保存）';
    saveTimer = setTimeout(() => { saveTimer = null; doSave(); }, 1500);
  }

  // 切换文档/知识库前：把挂起的自动保存立即落盘（保存的是「当时的」currentDoc）
  // 并取消在途定时器 —— 否则定时器在切换后触发，A 的内容被写进 B 的文件
  //（跨文档覆盖），或 currentDoc 已置空导致静默丢稿 + 状态栏永久卡「保存中」
  async function flushPendingSave() {
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = null; }
    if (modified && currentKb && currentDoc && !isSaving) {
      await doSave();
    }
  }

  async function doSave() {
    if (isSaving) { savePending = true; return; }
    await saveDoc();
    if (savePending) {
      savePending = false;
      setTimeout(() => doSave(), 50);
    }
  }

  async function saveDoc() {
    if (!currentKb || !currentDoc) return;
    isSaving = true;
    // 快照落盘目标 —— await 期间用户可能切换文档，写回时必须写「开始保存时」的
    // 那份，否则 A 的内容会被写进 B 的文件
    const saveKb = currentKb;
    let path = currentDoc;
    const content = getEditorMarkdown();

    // Title rename
    const titleInput = $('kb-doc-title');
    if (titleInput && titleInput.value.trim()) {
      const newName = titleInput.value.trim();
      const dir = path.includes('/') ? path.substring(0, path.lastIndexOf('/') + 1) : '';
      const newPath = dir + newName + '.md';
      if (newPath !== path) {
        try {
          const r = await fetch(`/api/kb/${encodeURIComponent(saveKb)}/rename`, {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ old_path: path, new_path: newPath }),
          });
          const data = await safeJson(r);
          if (data && data.status === 'ok') { currentDoc = newPath; path = newPath; loadTree(saveKb); }
          else {
            // 改重名是日常操作 —— 失败必须可见（曾静默：状态栏照显「已保存」，树上仍是旧名）
            $('kb-save-status').textContent = '重命名失败：' + ((data && data.message) || '未知错误');
          }
        } catch(e) {
          $('kb-save-status').textContent = '重命名失败：网络异常';
        }
      }
    }

    try {
      const r = await fetch(`/api/kb/${encodeURIComponent(saveKb)}/doc`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path, content }),
      });
      // 401：会话过期 —— 不引导的话用户会继续编辑、持续丢稿
      if (r.status === 401) {
        $('kb-save-status').textContent = '登录已过期，内容保留在编辑器';
        if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost();
        isSaving = false;
        return;
      }
      const data = await safeJson(r);
      if (data && data.status === 'ok') {
        // 仅当仍停留在同一份文档时才清 modified —— await 期间切换了文档
        // 则新文档的未保存修改不能被这里误标为已保存
        if (currentKb === saveKb && currentDoc === path) modified = false;
        if (currentDoc === path) $('kb-save-status').textContent = '已保存';
      } else {
        $('kb-save-status').textContent = '保存失败：' + ((data && data.message) || '未知错误');
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('知识库保存失败：' + ((data && data.message) || '未知错误'), 'error');
      }
    } catch (e) {
      $('kb-save-status').textContent = '保存失败：网络异常';
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('知识库保存失败：网络异常', 'error');
    }
    isSaving = false;
  }

  // ═══════════════════════════════════════════════════════════════
  // Word Count
  // ═══════════════════════════════════════════════════════════════

  function updateWordCount() {
    const editor = $('kb-editor-area');
    if (!editor) return;
    const text = editor.innerText || '';
    const count = text.replace(/\s/g, '').length;
    $('kb-word-count').textContent = count + ' 字';
  }

  // ═══════════════════════════════════════════════════════════════
  // Document Outline (TOC)
  // ═══════════════════════════════════════════════════════════════

  let outlineHeadings = [];

  function updateOutline() {
    const editor = $('kb-editor-area');
    const list = $('kb-outline-list');
    if (!editor || !list) return;

    const headings = editor.querySelectorAll('h1, h2, h3, h4, h5, h6');
    outlineHeadings = [];

    if (headings.length === 0) {
      list.innerHTML = '<div class="kb-outline-empty">暂无标题<br><span style="font-size:11px;opacity:0.7">输入 # 添加标题</span></div>';
      return;
    }

    list.innerHTML = '';
    headings.forEach((h, i) => {
      const level = parseInt(h.tagName[1]);
      const text = h.textContent.trim() || '(无标题)';
      outlineHeadings.push({ el: h, level, text, idx: i });

      const item = document.createElement('div');
      item.className = 'kb-outline-item';
      item.dataset.idx = String(i);
      item.style.paddingLeft = (8 + (level - 1) * 12) + 'px';
      item.textContent = text.length > 30 ? text.slice(0, 30) + '…' : text;
      list.appendChild(item);
    });

    updateOutlineActive();
  }

  function updateOutlineActive() {
    if (outlineHeadings.length === 0) return;
    const scrollEl = $('kb-edit-pane');
    if (!scrollEl) return;

    const scrollTop = scrollEl.scrollTop;
    let activeIdx = 0;

    for (let i = 0; i < outlineHeadings.length; i++) {
      const el = outlineHeadings[i].el;
      const rect = el.getBoundingClientRect();
      const scrollRect = scrollEl.getBoundingClientRect();
      const offsetTop = rect.top - scrollRect.top;
      if (offsetTop <= 60) activeIdx = i;
      else break;
    }

    const items = document.querySelectorAll('.kb-outline-item');
    items.forEach((item, i) => item.classList.toggle('active', i === activeIdx));
  }

  function scrollToHeading(idx) {
    if (idx < 0 || idx >= outlineHeadings.length) return;
    const heading = outlineHeadings[idx].el;
    const scrollEl = $('kb-edit-pane');
    if (!scrollEl || !heading) return;

    const scrollRect = scrollEl.getBoundingClientRect();
    const headingRect = heading.getBoundingClientRect();
    const offset = headingRect.top - scrollRect.top;
    scrollEl.scrollTo({ top: scrollEl.scrollTop + offset - 20, behavior: 'smooth' });
  }

  // ═══════════════════════════════════════════════════════════════
  // Helpers
  // ═══════════════════════════════════════════════════════════════

  function esc(s) {
    return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  // 文档图标（关键词→SVG）— 选择器与树渲染共用；历史数据里的 emoji 字面量
  // 不在表内 → 渲染为无图标（优雅降级，不做数据迁移）
  const KB_DOC_ICONS = {
    doc: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="9" y1="13" x2="15" y2="13"/><line x1="9" y1="17" x2="13" y2="17"/></svg>',
    note: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>',
    wrench: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>',
    book: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/></svg>',
    bulb: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M9 18h6"/><path d="M10 22h4"/><path d="M12 2a7 7 0 0 0-4 12.7c.6.5 1 1.4 1 2.3h6c0-.9.4-1.8 1-2.3A7 7 0 0 0 12 2z"/></svg>',
    target: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="2"/></svg>',
    chart: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><line x1="18" y1="20" x2="18" y2="10"/><line x1="12" y1="20" x2="12" y2="4"/><line x1="6" y1="20" x2="6" y2="14"/></svg>',
    lock: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>',
    code: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>',
    globe: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>',
    folder: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>',
    flag: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14"><path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15"/></svg>',
  };
  // 模板弹窗图标（内联 SVG — 低饱和线性风格，替代 emoji）
  const TPL_ICONS = {
    doc: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="22" height="22"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="9" y1="13" x2="15" y2="13"/><line x1="9" y1="17" x2="13" y2="17"/></svg>',
    wrench: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="22" height="22"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>',
    note: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="22" height="22"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>',
    book: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="22" height="22"><path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/></svg>',
    folder: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="22" height="22"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>',
  };

  // Safe JSON parse from fetch Response — handles empty/non-JSON responses
  async function safeJson(r) {
    const text = await r.text();
    if (!text || text.trim() === '') return null;
    try { return JSON.parse(text); } catch(e) { return null; }
  }

  // ═══════════════════════════════════════════════════════════════
  // Public API
  // ═══════════════════════════════════════════════════════════════

  return { init, onShow };
})();
