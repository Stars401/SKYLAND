// ════════════════════════════════════════════
// SKYLAND AI Settings — 生图模型设置（统一配置面）
// LLM 模型配置归统一设置面板「模型与搜索」页（本模态曾设独立 LLM Tab，
// 与该页完全重复，已并入；本模态专职图片生成模型 + 失败自动重试）
// ════════════════════════════════════════════

const AISettings = (() => {
  const $ = id => document.getElementById(id);
  let _inited = false;
  let imageProviders = [];

  function init() {
    if (_inited) return;
    _inited = true;

    // 统一设置面板「模型与搜索」页 → 生图模型入口。
    // （agent-global-settings 曾被 router 与本模块双重绑定，一次点击同时弹出
    //  统一面板与本模态两个窗口 — 已收归 router 单一绑定统一面板）
    const panelBtn = $('btn-image-providers');
    if (panelBtn) panelBtn.addEventListener('click', open);

    const closeBtn = $('ai-settings-close');
    if (closeBtn) closeBtn.addEventListener('click', close);

    const overlay = $('ai-settings-overlay');
    if (overlay) overlay.addEventListener('click', e => {
      if (e.target === overlay) close();
    });

    const imgSave = $('ai-image-save');
    if (imgSave) imgSave.addEventListener('click', saveImageProviders);
  }

  function open() {
    const overlay = $('ai-settings-overlay');
    if (overlay) overlay.style.display = 'flex';
    loadImageProviders();
  }

  function close() {
    const overlay = $('ai-settings-overlay');
    if (overlay) overlay.style.display = 'none';
  }

  // ─── Image Provider Config ───

  function loadImageProviders() {
    fetch('/api/image/config')
      .then(r => r.json())
      .then(data => {
        imageProviders = data.providers || [];
        const retryInput = $('ai-img-retry');
        if (retryInput && Number.isFinite(data.retry_count)) retryInput.value = data.retry_count;
        renderImageProviders();
      })
      .catch(() => {});
  }

  function renderImageProviders() {
    const container = $('ai-image-providers');
    if (!container) return;
    if (imageProviders.length === 0) {
      container.innerHTML = '<div style="padding:8px 0;color:var(--text-muted)">暂无服务商配置</div>';
      return;
    }
    container.innerHTML = imageProviders.map((p, i) => `
      <div class="ai-settings-provider">
        <div class="ai-settings-provider-name">${p.name} <span class="ai-settings-type">(${p.provider_type})</span></div>
        <input type="hidden" class="ai-img-field" data-field="name" data-index="${i}" value="${p.name}">
        <label class="ai-settings-label">API Key <span style="font-size:11px;color:var(--text-muted)">(留空不修改)</span></label>
        <input type="password" class="ai-img-field" data-field="api_key" data-index="${i}" placeholder="${p.api_key_masked}">
        <label class="ai-settings-label">Base URL</label>
        <input type="text" class="ai-img-field" data-field="base_url" data-index="${i}" value="${p.base_url}">
        <label class="ai-settings-label">Model</label>
        <input type="text" class="ai-img-field" data-field="model" data-index="${i}" value="${p.model}">
        <label class="ai-settings-label">服务商类型</label>
        <select class="ai-img-field" data-field="provider_type" data-index="${i}">
          <option value="chatgpt" ${p.provider_type === 'chatgpt' ? 'selected' : ''}>ChatGPT (gpt-image-1)</option>
          <option value="nanobanana" ${p.provider_type === 'nanobanana' ? 'selected' : ''}>NanoBanana</option>
           <option value="openai_compatible" ${p.provider_type === 'openai_compatible' ? 'selected' : ''}>OpenAI 兼容图片接口</option>
           <option value="volcengine_seedream" ${p.provider_type === 'volcengine_seedream' ? 'selected' : ''}>火山引擎 Seedream</option>
           <option value="minimax" ${p.provider_type === 'minimax' ? 'selected' : ''}>MiniMax Image</option>
        </select>
        <label style="display:flex;align-items:center;gap:6px;margin-top:8px;cursor:pointer;font-size:12px;color:var(--text-muted)">
          <input type="checkbox" class="ai-img-field" data-field="is_full_url" data-index="${i}" ${p.is_full_url ? 'checked' : ''} style="cursor:pointer">
          <span>完整 URL（不自动添加 /v1 后缀）</span>
        </label>
      </div>
    `).join('');
  }

  function saveImageProviders() {
    const fields = document.querySelectorAll('.ai-img-field');
    const providers = {};
    fields.forEach(f => {
      const idx = f.dataset.index;
      const field = f.dataset.field;
      if (!providers[idx]) providers[idx] = {};
      if (field === 'is_full_url') {
        providers[idx][field] = f.checked;
      } else {
        providers[idx][field] = f.value;
      }
    });

    const promises = Object.values(providers).map(p => {
      return fetch('/api/image/config/save', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: p.name,
          api_key: p.api_key || '',
          base_url: p.base_url || '',
          model: p.model || '',
          provider_type: p.provider_type || 'chatgpt',
          is_full_url: p.is_full_url || false
        })
      });
    });

    // 全局生图设置 — 失败自动重试次数（0-10，默认 5），随本次保存一并提交
    const retryInput = $('ai-img-retry');
    if (retryInput) {
      const parsed = parseInt(retryInput.value, 10);
      const rc = Number.isFinite(parsed) ? Math.max(0, Math.min(10, parsed)) : 5;
      retryInput.value = rc;
      promises.push(fetch('/api/image/settings/save', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ retry_count: rc })
      }));
    }

    Promise.all(promises)
      .then(() => {
        loadImageProviders();
        // 数据互通：保存后让生图视图与节点流都拿到最新服务商/重试配置
        if (typeof ImageApp !== 'undefined' && ImageApp.onShow) ImageApp.onShow();
        if (typeof NodesApp !== 'undefined' && NodesApp.onShow) NodesApp.onShow();
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('图片生成配置已保存');
      })
      .catch(() => {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('图片生成配置保存失败', 'error');
        else alert('保存失败');
      });
  }

  return { init, open, close };
})();
