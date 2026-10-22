// ════════════════════════════════════════════
// SKYLAND AI Image Generation — 图片生成模块
// Supports: prompt tags, style templates, drawing canvas,
// reference image upload, provider settings, history gallery
// ════════════════════════════════════════════

const ImageApp = (() => {
  const $ = id => document.getElementById(id);

  // ─── State ───
  let _inited = false;
  let selectedTags = new Set();
  let referenceImage = null;       // base64 string (no data: prefix)
  let providers = [];
  let isGenerating = false;
  let currentImageFilename = null;
  let retryCount = 5; // 后端失败自动重试次数（/api/image/config.retry_count）— 用于前端硬超时扩展

  // Canvas state
  let canvasCtx = null;
  let canvasDrawing = false;
  let canvasTool = 'pen';
  let canvasLastX = 0;
  let canvasLastY = 0;

  // ─── Constants ───
  const PROMPT_TAGS = [
    '超高质量', '电影感', '8K', '超精细',
    '柔和光线', '深景深', '自然光', '赛博朋克',
    '水彩', '油画', '像素艺术', '概念艺术'
  ];

  const STYLE_TEMPLATES = {
    'photorealistic': 'photorealistic, ultra detailed, ',
    'concept-art':    'concept art, detailed, ',
    'anime':          'anime style, ',
    'oil-painting':   'oil painting, ',
    'watercolor':     'watercolor, ',
    '3d-render':      '3D render, octane, ',
    'cyberpunk':      'cyberpunk, neon, ',
    'minimalist':      'minimalist, ',
  };

  // localStorage keys — 分辨率预设与自定义尺寸记忆
  const K_IMG_SIZE = SkylandStore.KEYS.imgSize;
  const K_IMG_CUSTOM = SkylandStore.KEYS.imgCustom;

  // ─── Utilities ───

  function esc(s) {
    return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;')
      .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  async function safeJson(r) {
    const text = await r.text();
    try {
      return JSON.parse(text);
    } catch (e) {
      throw new Error('服务器返回非 JSON: ' + text.substring(0, 200));
    }
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
        <div class="kb-modal-footer">
          <button class="kb-modal-btn kb-modal-btn-ok" id="img-alert-ok">知道了</button>
        </div>`;
      overlay.appendChild(modal);
      overlay.addEventListener('click', e => {
        if (e.target === overlay) { overlay.remove(); resolve(); }
      });
      modal.querySelector('#img-alert-ok').onclick = () => { overlay.remove(); resolve(); };
      document.body.appendChild(overlay);
    });
  }

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
          <button class="kb-modal-btn kb-modal-btn-cancel" id="img-confirm-cancel">取消</button>
          <button class="kb-modal-btn kb-modal-btn-ok" id="img-confirm-ok">确定</button>
        </div>`;
      overlay.appendChild(modal);
      overlay.addEventListener('click', e => {
        if (e.target === overlay) { overlay.remove(); resolve(false); }
      });
      modal.querySelector('#img-confirm-cancel').onclick = () => { overlay.remove(); resolve(false); };
      modal.querySelector('#img-confirm-ok').onclick = () => { overlay.remove(); resolve(true); };
      document.body.appendChild(overlay);
    });
  }

  // ─── Init ───

  async function init() {
    if (_inited) return;
    _inited = true;

    bindEvents();
    renderTagGrid();
    await loadConfig();
    await loadHistory();
  }

  function onShow() {
    loadHistory();
    // 生图设置可能在 AISettings 中保存过 — 回到本视图即刷新服务商与重试配置（数据互通）
    loadConfig();
  }

  // ─── Event Binding ───

  function bindEvents() {
    // Back button → agent sub-nav
    const backBtn = $('img-back-agent');
    if (backBtn) backBtn.addEventListener('click', () => { if (window.SkylandOpenLab) window.SkylandOpenLab(); else Router.navigate('/home'); });

    // Generate
    const genBtn = $('img-btn-generate');
    if (genBtn) genBtn.addEventListener('click', generate);

    // 切换服务商 → 自定义尺寸即时按新服务商约束校验（不等点生成才发现不合法）
    const providerSel = $('img-provider-select');
    if (providerSel) providerSel.addEventListener('change', () => {
      if (($('img-size-select') || {}).value !== 'custom') return;
      const res = resolveSize(providerSel.value);
      const hint = $('img-custom-hint');
      if (hint) {
        if (res.err) {
          hint.textContent = '注意：' + res.err;
          hint.style.color = '#E6A23C';
        } else {
          hint.style.color = '';
          updateCustomHint();
        }
      }
    });

    // 分辨率预设 / 自定义尺寸（切换 + 记忆 + 实时提示）
    const sizeSelect = $('img-size-select');
    if (sizeSelect) sizeSelect.addEventListener('change', toggleCustomSize);
    const customW = $('img-custom-w');
    const customH = $('img-custom-h');
    if (customW) customW.addEventListener('input', () => { updateCustomHint(); saveCustomSize(); });
    if (customH) customH.addEventListener('input', () => { updateCustomHint(); saveCustomSize(); });
    restoreSizeSelection();

    // Drawing canvas modal
    const canvasBtn = $('img-btn-canvas');
    if (canvasBtn) canvasBtn.addEventListener('click', openCanvas);

    // Upload reference image
    const uploadBtn = $('img-btn-upload');
    const refInput = $('img-ref-input');
    if (uploadBtn && refInput) {
      uploadBtn.addEventListener('click', () => refInput.click());
      refInput.addEventListener('change', handleFileUpload);
    }

    // Clear reference image
    const refClear = $('img-ref-clear');
    if (refClear) refClear.addEventListener('click', clearReferenceImage);

    // Settings — 打开统一生图设置（AISettings，含服务商与失败重试）
    const settingsBtn = $('img-btn-settings');
    if (settingsBtn) settingsBtn.addEventListener('click', openSettings);

    // Canvas modal events
    bindCanvasEvents();

    // Ctrl+Enter on prompt textarea → generate
    const promptInput = $('img-prompt-input');
    if (promptInput) {
      promptInput.addEventListener('keydown', e => {
        if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
          e.preventDefault();
          generate();
        }
      });
    }
  }

  // ─── Config ───

  async function loadConfig() {
    try {
      const r = await fetch('/api/image/config');
      const data = await safeJson(r);
      if (data && Number.isFinite(data.retry_count)) retryCount = data.retry_count;
      if (data && Array.isArray(data.providers)) {
        providers = data.providers;
        populateProviders();
      }
    } catch (e) {
      console.error('ImageApp loadConfig error:', e);
    }
  }

  function populateProviders() {
    const select = $('img-provider-select');
    if (!select) return;
    select.innerHTML = providers.map(p =>
      `<option value="${esc(p.name)}">${esc(p.name)} (${esc(p.provider_type)})</option>`
    ).join('');
  }

  // ─── History ───

  async function loadHistory() {
    try {
      const r = await fetch('/api/image/history');
      const data = await safeJson(r);
      if (data && Array.isArray(data.images)) {
        renderHistory(data.images);
      }
    } catch (e) {
      console.error('ImageApp loadHistory error:', e);
    }
  }

  function renderHistory(images) {
    const container = $('img-history');
    if (!container) return;

    if (!images || images.length === 0) {
      container.innerHTML = '<div class="img-history-empty">暂无生成记录</div>';
      return;
    }

    container.innerHTML = images.map(img => `
      <div class="img-history-item" data-filename="${esc(img.filename)}">
        <img src="${esc(img.url)}" alt="${esc(img.prompt || '')}" title="${esc(img.prompt || '无提示词')}" loading="lazy">
        <button class="img-history-delete" data-filename="${esc(img.filename)}" title="删除">×</button>
      </div>
    `).join('');

    // Click thumbnail → show in preview
    container.querySelectorAll('.img-history-item').forEach(item => {
      item.addEventListener('click', e => {
        if (e.target.classList.contains('img-history-delete')) return;
        const filename = item.dataset.filename;
        const img = images.find(i => i.filename === filename);
        if (img) showPreviewImage(img.url, img.prompt, img.filename);
      });
    });

    // Delete buttons
    container.querySelectorAll('.img-history-delete').forEach(btn => {
      btn.addEventListener('click', e => {
        e.stopPropagation();
        deleteImage(btn.dataset.filename);
      });
    });
  }

  // ─── Prompt Tags ───

  function renderTagGrid() {
    const grid = $('img-tag-grid');
    if (!grid) return;
    grid.innerHTML = PROMPT_TAGS.map(tag =>
      `<button class="img-tag-btn${selectedTags.has(tag) ? ' active' : ''}" data-tag="${esc(tag)}">${esc(tag)}</button>`
    ).join('');

    grid.querySelectorAll('.img-tag-btn').forEach(btn => {
      btn.addEventListener('click', () => toggleTag(btn.dataset.tag));
    });
  }

  function toggleTag(tag) {
    if (selectedTags.has(tag)) {
      selectedTags.delete(tag);
    } else {
      selectedTags.add(tag);
    }
    renderTagGrid();
    renderPromptTags();
  }

  function renderPromptTags() {
    const container = $('img-prompt-tags');
    if (!container) return;
    if (selectedTags.size === 0) {
      container.innerHTML = '';
      return;
    }
    container.innerHTML = [...selectedTags].map(tag =>
      `<span class="img-prompt-tag">${esc(tag)}<button class="img-prompt-tag-remove" data-tag="${esc(tag)}">×</button></span>`
    ).join('');

    container.querySelectorAll('.img-prompt-tag-remove').forEach(btn => {
      btn.addEventListener('click', () => toggleTag(btn.dataset.tag));
    });
  }

  // ─── Style Templates ───

  function getStylePrefix() {
    const select = $('img-style-select');
    if (!select) return '';
    return STYLE_TEMPLATES[select.value] || '';
  }

  // ─── Reference Image ───

  function setReferenceImage(base64) {
    referenceImage = base64;
    const preview = $('img-ref-preview');
    const thumb = $('img-ref-thumb');
    if (preview && thumb) {
      thumb.src = 'data:image/png;base64,' + base64;
      preview.style.display = 'flex';
    }
  }

  function clearReferenceImage() {
    referenceImage = null;
    const preview = $('img-ref-preview');
    const thumb = $('img-ref-thumb');
    if (preview) preview.style.display = 'none';
    if (thumb) thumb.src = '';
    const refInput = $('img-ref-input');
    if (refInput) refInput.value = '';
  }

  function handleFileUpload(e) {
    const file = e.target.files[0];
    if (!file) return;
    if (!file.type.startsWith('image/')) {
      showAlert('格式错误', '请选择图片文件');
      return;
    }
    // Limit to 10MB
    if (file.size > 10 * 1024 * 1024) {
      showAlert('文件过大', '参考图不能超过 10MB');
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result || '';
      const base64 = result.includes(',') ? result.split(',')[1] : result;
      setReferenceImage(base64);
    };
    reader.onerror = () => {
      showAlert('上传失败', '无法读取文件');
    };
    reader.readAsDataURL(file);
  }

  // ─── Drawing Canvas ───

  function bindCanvasEvents() {
    const closeBtn = $('img-canvas-close');
    const useBtn = $('img-canvas-use');
    const modal = $('img-canvas-modal');
    const canvas = $('img-draw-canvas');

    if (closeBtn) closeBtn.addEventListener('click', closeCanvas);
    if (useBtn) useBtn.addEventListener('click', exportCanvas);

    if (modal) {
      modal.addEventListener('click', e => {
        if (e.target === modal) closeCanvas();
      });
    }

    // Tool buttons (pen / eraser / clear)
    document.querySelectorAll('.img-canvas-tool').forEach(btn => {
      btn.addEventListener('click', () => {
        const tool = btn.dataset.tool;
        if (tool === 'clear') {
          clearCanvas();
          return;
        }
        document.querySelectorAll('.img-canvas-tool').forEach(b => {
          if (b.dataset.tool !== 'clear') b.classList.remove('active');
        });
        btn.classList.add('active');
        canvasTool = tool;
      });
    });

    if (canvas) {
      canvasCtx = canvas.getContext('2d');

      // Mouse events
      canvas.addEventListener('mousedown', startDraw);
      canvas.addEventListener('mousemove', draw);
      canvas.addEventListener('mouseup', stopDraw);
      canvas.addEventListener('mouseleave', stopDraw);

      // Touch events
      canvas.addEventListener('touchstart', e => {
        e.preventDefault();
        const t = e.touches[0];
        startDraw({ clientX: t.clientX, clientY: t.clientY });
      }, { passive: false });

      canvas.addEventListener('touchmove', e => {
        e.preventDefault();
        const t = e.touches[0];
        draw({ clientX: t.clientX, clientY: t.clientY });
      }, { passive: false });

      canvas.addEventListener('touchend', stopDraw);
    }
  }

  function openCanvas() {
    const modal = $('img-canvas-modal');
    if (!modal) return;
    modal.style.display = 'flex';
    // Ensure canvas is initialized with white background
    clearCanvas();
  }

  function closeCanvas() {
    const modal = $('img-canvas-modal');
    if (modal) modal.style.display = 'none';
  }

  function getCanvasCoords(e) {
    const canvas = $('img-draw-canvas');
    if (!canvas) return { x: 0, y: 0 };
    const rect = canvas.getBoundingClientRect();
    const scaleX = canvas.width / rect.width;
    const scaleY = canvas.height / rect.height;
    return {
      x: (e.clientX - rect.left) * scaleX,
      y: (e.clientY - rect.top) * scaleY
    };
  }

  function getBrushParams() {
    const sizeSlider = $('img-canvas-size');
    const colorPicker = $('img-canvas-color');
    const size = sizeSlider ? parseInt(sizeSlider.value, 10) || 4 : 4;
    const color = canvasTool === 'eraser'
      ? '#FFFFFF'
      : (colorPicker ? colorPicker.value : '#3370FF');
    return { size, color };
  }

  function startDraw(e) {
    canvasDrawing = true;
    const coords = getCanvasCoords(e);
    canvasLastX = coords.x;
    canvasLastY = coords.y;

    // Draw a dot so single clicks are visible
    if (canvasCtx) {
      const { size, color } = getBrushParams();
      canvasCtx.beginPath();
      canvasCtx.arc(coords.x, coords.y, size / 2, 0, Math.PI * 2);
      canvasCtx.fillStyle = color;
      canvasCtx.fill();
    }
  }

  function draw(e) {
    if (!canvasDrawing || !canvasCtx) return;
    const coords = getCanvasCoords(e);
    const { size, color } = getBrushParams();

    canvasCtx.beginPath();
    canvasCtx.moveTo(canvasLastX, canvasLastY);
    canvasCtx.lineTo(coords.x, coords.y);
    canvasCtx.strokeStyle = color;
    canvasCtx.lineWidth = size;
    canvasCtx.lineCap = 'round';
    canvasCtx.lineJoin = 'round';
    canvasCtx.stroke();

    canvasLastX = coords.x;
    canvasLastY = coords.y;
  }

  function stopDraw() {
    canvasDrawing = false;
  }

  function clearCanvas() {
    const canvas = $('img-draw-canvas');
    if (!canvas || !canvasCtx) return;
    canvasCtx.fillStyle = '#FFFFFF';
    canvasCtx.fillRect(0, 0, canvas.width, canvas.height);
  }

  function exportCanvas() {
    const canvas = $('img-draw-canvas');
    if (!canvas) return;
    const dataUrl = canvas.toDataURL('image/png');
    const base64 = dataUrl.includes(',') ? dataUrl.split(',')[1] : dataUrl;
    setReferenceImage(base64);
    closeCanvas();
  }

  // ─── Generate ───

  // ─── 分辨率：预设 + 自定义（2048 2K 预设；自定义暴露宽高并按服务商约束校验）───
  function restoreSizeSelection() {
    const sizeSelect = $('img-size-select');
    if (!sizeSelect) return;
    let saved = null;
    if (window.SkylandSettings) saved = window.SkylandSettings.get('paths.imageSize');
    else try { saved = localStorage.getItem(K_IMG_SIZE); } catch (e) {}
    if (saved && [...sizeSelect.options].some(o => o.value === saved)) sizeSelect.value = saved;
    try {
      const c = window.SkylandSettings
        ? window.SkylandSettings.get('paths.imageCustom')
        : JSON.parse(localStorage.getItem(K_IMG_CUSTOM) || 'null');
      if (c && c.w && c.h) {
        const cw = $('img-custom-w');
        const ch = $('img-custom-h');
        if (cw) cw.value = c.w;
        if (ch) ch.value = c.h;
      }
    } catch (e) {}
    toggleCustomSize();
  }
  function toggleCustomSize() {
    const sizeSelect = $('img-size-select');
    const box = $('img-custom-size');
    if (!sizeSelect || !box) return;
    box.style.display = sizeSelect.value === 'custom' ? '' : 'none';
    if (window.SkylandSettings) {
      window.SkylandSettings.set('paths.imageSize', sizeSelect.value);
      window.SkylandSettings.save().catch(() => {});
    } else try { localStorage.setItem(K_IMG_SIZE, sizeSelect.value); } catch (e) {}
    updateCustomHint();
  }
  function saveCustomSize() {
    const cw = $('img-custom-w');
    const ch = $('img-custom-h');
    if (!cw || !ch) return;
    if (window.SkylandSettings) {
      window.SkylandSettings.set('paths.imageCustom', { w: cw.value, h: ch.value });
      window.SkylandSettings.save().catch(() => {});
    } else try { localStorage.setItem(K_IMG_CUSTOM, JSON.stringify({ w: cw.value, h: ch.value })); } catch (e) {}
  }
  function updateCustomHint() {
    const hint = $('img-custom-hint');
    if (!hint) return;
    hint.style.color = '';
    const sizeSelect = $('img-size-select');
    if (!sizeSelect || sizeSelect.value !== 'custom') { hint.textContent = ''; return; }
    const w = parseInt(($('img-custom-w') || {}).value, 10) || 0;
    const h = parseInt(($('img-custom-h') || {}).value, 10) || 0;
    if (!w || !h) { hint.textContent = ''; return; }
    const total = w * h;
    const ratio = (Math.max(w, h) / Math.min(w, h)).toFixed(2);
    hint.textContent = `共 ${(total / 1e6).toFixed(2)}M 像素 · 比例 ${ratio}:1` +
      ((w % 16 === 0 && h % 16 === 0) ? ' · 16 倍数 ✓' : ' · 非 16 倍数（gpt-image-2 会拒绝）');
  }
  /// 解析当前尺寸：预设直接用；自定义取宽高输入并按服务商约束提前校验（与后端同规，
  /// 避免 5 次重试后才收到 400）
  function resolveSize(providerName) {
    const sizeSelect = $('img-size-select');
    let size = sizeSelect ? sizeSelect.value : '1024x1024';
    if (size === 'custom') {
      const w = parseInt(($('img-custom-w') || {}).value, 10) || 0;
      const h = parseInt(($('img-custom-h') || {}).value, 10) || 0;
      if (!w || !h || w < 64 || h < 64 || w > 8192 || h > 8192) {
        return { err: '自定义宽高须为 64~8192 的整数' };
      }
      size = w + 'x' + h;
    }
    if (size.toLowerCase() !== 'auto') {
      const parts = size.split('x');
      const w = parseInt(parts[0], 10);
      const h = parseInt(parts[1], 10);
      const p = providers.find(x => x.name === providerName);
      const isGptImage2 = p && (p.provider_type === 'chatgpt' || p.provider_type === 'openai_compatible') && /gpt-image-2/i.test(p.model || '');
      if (isGptImage2) {
        // gpt-image-2 官方四约束（与后端 validate_gpt_image2_size 一致）
        if (w % 16 !== 0 || h % 16 !== 0) return { err: `gpt-image-2 要求宽高均为 16 的倍数，${w}x${h} 不满足（如 2048x2048、2048x1152）` };
        if (Math.max(w, h) > 3840) return { err: `gpt-image-2 最长边不能超过 3840px（当前 ${w}x${h}）` };
        if (Math.max(w, h) > Math.min(w, h) * 3) return { err: `gpt-image-2 长短边比不能超过 3:1（当前 ${w}x${h}）` };
        const total = w * h;
        if (total < 655360 || total > 8294400) return { err: `gpt-image-2 总像素须在 655,360~8,294,400（当前 ${total}）` };
      }
    }
    return { size };
  }

  async function generate() {
    if (isGenerating) return;

    const promptInput = $('img-prompt-input');
    const promptText = promptInput ? promptInput.value.trim() : '';

    if (!promptText && selectedTags.size === 0) {
      showAlert('提示', '请输入提示词或选择标签');
      return;
    }

    const providerSelect = $('img-provider-select');
    const provider = providerSelect ? providerSelect.value : '';
    if (!provider) {
      showAlert('提示', '请选择服务商');
      return;
    }

    const sizeRes = resolveSize(provider);
    if (sizeRes.err) {
      showAlert('分辨率无效', sizeRes.err);
      return;
    }
    const size = sizeRes.size;
    const qualitySelect = $('img-quality-select');
    const styleSelect = $('img-style-select');
    const quality = qualitySelect ? qualitySelect.value : 'standard';
    const style = styleSelect ? styleSelect.value : '';

    // Build final prompt: style prefix + user text + selected tags
    const stylePrefix = getStylePrefix();
    const tagsText = selectedTags.size > 0 ? [...selectedTags].join(', ') : '';
    let finalPrompt = stylePrefix + promptText;
    if (tagsText) {
      finalPrompt += (promptText ? ', ' : '') + tagsText;
    }

    // Loading state
    isGenerating = true;
    const genBtn = $('img-btn-generate');
    if (genBtn) {
      genBtn.disabled = true;
      genBtn.textContent = '生成中...';
    }
    showPreviewLoading();

    // Timer + AbortController for timeout handling
    const abortController = new AbortController();
    let genTimer = null;
    let genTimeout = null;
    let manuallyAborted = false;
    let timedOut = false;
    const startTime = Date.now();
    const preview = $('img-preview');

    // Update timer every second
    genTimer = setInterval(() => {
      const elapsed = Math.floor((Date.now() - startTime) / 1000);
      const timerEl = preview?.querySelector('.gen-timer');
      if (timerEl) timerEl.textContent = elapsed + 's';
      // After 60s, show abort button
      if (elapsed >= 60) {
        const abortBtn = preview?.querySelector('.gen-abort');
        if (abortBtn) abortBtn.style.display = 'inline-flex';
      }
    }, 1000);

    // Add abort button handler
    const abortBtnEl = preview?.querySelector('.gen-abort');
    if (abortBtnEl) abortBtnEl.addEventListener('click', () => {
      manuallyAborted = true;
      abortController.abort();
    });

    // Keep a hard upper bound even when the provider never sends a response.
    // 硬超时随重试次数扩展：后端会重试 retryCount 次（每次最长 120s），前端不能提前掐断
    genTimeout = setTimeout(() => {
      timedOut = true;
      abortController.abort();
    }, (1 + (Number.isFinite(retryCount) ? retryCount : 5)) * 120000 + 30000);

    try {
      const body = {
        provider,
        prompt: finalPrompt,
        size,
        quality,
      };
      if (referenceImage) {
        body.reference_image = referenceImage;
      }

      const r = await fetch('/api/image/generate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
        signal: abortController.signal,
      });

      if (!r.ok) {
        const errText = await r.text().catch(() => '');
        let errMsg = errText;
        try { const ej = JSON.parse(errText); errMsg = ej.message || ej.error || errText; } catch(_) {}
        throw new Error(errMsg || ('服务器错误 (' + r.status + ')'));
      }

      const data = await safeJson(r);

      if (data && data.status === 'ok' && data.url) {
        currentImageFilename = data.filename || null;
        showPreviewImage(data.url, finalPrompt, data.filename);
        // 服务商如实反馈（如 MiniMax 自定义尺寸已按最近支持比例生成）
        if (data.message) {
          const preview = $('img-preview');
          const note = document.createElement('div');
          note.style.cssText = 'padding:6px 10px;font-size:12.6px;color:var(--text-muted);text-align:center';
          note.textContent = data.message;
          if (preview) preview.appendChild(note);
        }
        loadHistory();  // refresh sidebar thumbnails
      } else {
        throw new Error((data && data.message) ? data.message : '生成失败');
      }
    } catch (e) {
      if (e.name === 'AbortError') {
        showPreviewError(manuallyAborted
          ? '已手动中止生成'
          : (timedOut ? '生成请求超过 120 秒，已自动中止' : '生成请求已中止'));
      } else {
        showPreviewError(e.message || '生成失败');
      }
    } finally {
      if (genTimer) clearInterval(genTimer);
      if (genTimeout) clearTimeout(genTimeout);
      isGenerating = false;
      if (genBtn) {
        genBtn.disabled = false;
        genBtn.textContent = '生成';
      }
    }
  }

  // ─── Preview States ───

  const EMPTY_HTML = `
      <div class="img-empty" id="img-empty">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" width="48" height="48"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="M21 15l-5-5L5 21"/></svg>
        <p>输入提示词后点击生成</p>
      </div>`;

  function showPreviewLoading() {
    const preview = $('img-preview');
    if (!preview) return;
    preview.innerHTML = `
      <div class="img-preview-loading">
        <div class="img-spinner"></div>
        <p>正在生成图片...</p>
        <div class="gen-timer-wrap">
          <span class="gen-timer">0s</span>
          <button class="gen-abort" style="display:none">中止生成</button>
        </div>
      </div>`;
  }

  function showPreviewImage(url, prompt, filename) {
    const preview = $('img-preview');
    if (!preview) return;
    currentImageFilename = filename || null;
    preview.innerHTML = `
      <div class="img-result">
        <img src="${esc(url)}" alt="生成结果" class="img-result-img">
        <div class="img-result-info">
          ${prompt ? `<div class="img-result-prompt">${esc(prompt)}</div>` : ''}
          <div class="img-result-actions">
            <a href="${esc(url)}" download class="img-btn">下载</a>
            ${filename ? '<button class="img-btn" id="img-result-delete">删除</button>' : ''}
          </div>
        </div>
      </div>`;

    const delBtn = $('img-result-delete');
    if (delBtn && filename) {
      delBtn.addEventListener('click', () => deleteImage(filename));
    }
  }

  function showPreviewError(message) {
    const preview = $('img-preview');
    if (!preview) return;
    preview.innerHTML = `
      <div class="img-preview-error">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" width="48" height="48">
          <circle cx="12" cy="12" r="10"/>
          <line x1="12" y1="8" x2="12" y2="12"/>
          <line x1="12" y1="16" x2="12.01" y2="16"/>
        </svg>
        <p>生成失败</p>
        <p class="img-error-detail">${esc(message)}</p>
      </div>`;
  }

  function showPreviewEmpty() {
    const preview = $('img-preview');
    if (!preview) return;
    currentImageFilename = null;
    preview.innerHTML = EMPTY_HTML;
  }

  // ─── Delete Image ───

  async function deleteImage(filename) {
    if (!filename) return;
    const confirmed = await showConfirm('删除图片', '确定删除此图片？此操作不可恢复。');
    if (!confirmed) return;

    try {
      const r = await fetch('/api/image/file/' + encodeURIComponent(filename), {
        method: 'DELETE',
      });
      const data = await safeJson(r);

      if (data && data.status === 'ok') {
        if (currentImageFilename === filename) {
          showPreviewEmpty();
        }
        loadHistory();
      } else {
        showAlert('删除失败', (data && data.message) ? data.message : '未知错误');
      }
    } catch (e) {
      showAlert('删除失败', e.message);
    }
  }

  // ─── Settings — 统一生图设置入口 ───
  // 独立 img-settings-modal 已退役（与 AISettings 生图页完全重复）；此处委托全局
  // AISettings，并保持 nodes.js 的 ImageApp.openSettings() 调用兼容（曾未导出而静默失效）
  function openSettings() {
    if (typeof AISettings !== 'undefined' && AISettings.open) { AISettings.open(); return; }
    showAlert('提示', '生图设置面板加载失败，请刷新页面重试');
  }

  return { init, onShow, openSettings };
})();
