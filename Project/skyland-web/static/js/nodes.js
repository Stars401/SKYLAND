// ════════════════════════════════════════════
// SKYLAND Node Flow — 节点流图像生成模块
// Vanilla JS IIFE, no build step
// Node-based visual workflow editor for AI image generation
// ════════════════════════════════════════════

const NodesApp = (() => {
  const $ = id => document.getElementById(id);

  // ─── State ───
  let _inited = false;
  let canvas, ctx, canvasWrap;
  let dpr = window.devicePixelRatio || 1;
  let viewW = 0, viewH = 0;

  // View transform: screen = (world + pan) * zoom
  let panX = 0, panY = 0, zoom = 1.0;

  // Panning
  let isPanning = false;
  let panStartX = 0, panStartY = 0;
  let panStartPanX = 0, panStartPanY = 0;
  let spaceHeld = false;
  let viewHistoryTimer = null;

  // Nodes & connections
  let nodes = [];
  let connections = [];
  let nextNodeId = 1;
  let nextConnId = 1;
  let selectedNodeId = null;
  let selectedNodeIds = new Set(); // Multi-select (box select + shift-click)
  let selectedConnId = null;       // Selected connection line

  // Interaction state
  let dragNode = null;
  let dragOffsetX = 0, dragOffsetY = 0;
  let connectingFrom = null; // { nodeId, portIndex, isOutput }
  let mouseSX = 0, mouseSY = 0;
  let mouseWX = 0, mouseWY = 0;

  // Box selection state
  let isBoxSelecting = false;
  let boxStartX = 0, boxStartY = 0; // Screen coords
  let boxEndX = 0, boxEndY = 0;

  // Multi-drag state
  let dragStartPositions = []; // [{id, x, y}] snapshot before drag

  // Rendering
  let rafId = null;
  let resultImages = {}; // nodeId -> Image object

  // Search panel
  let searchOpen = false;
  let searchIdx = 0;

  // Property panel
  let propNodeId = null;

  // Execution
  let isExecuting = false;
  let workflowAbortController = null;
  let workflowCancelRequested = false;
  let workflowStartedAt = 0;
  let outputTimerHandle = null;

  // Graph history. A transaction records one coherent user action, such as a
  // drag, connection, deletion, or property edit, instead of every mousemove.
  const MAX_HISTORY = 100;
  let undoStack = [];
  let redoStack = [];
  let historyTransaction = null;
  let restoringHistory = false;

  // Providers (loaded from API — image generation providers)
  let providers = [];
  let imageRetryCount = 5; // 后端生图重试次数（/api/image/config.retry_count）— 用于节点请求超时扩展
  // LLM models (loaded from /api/health — for llm-call node)
  let llmModels = [];
  let llmDefaultModel = '';

  // Short-lived feedback for rejected graph actions. A non-modal notice keeps
  // a failed connection drop understandable without interrupting editing.
  let graphNoticeTimer = null;

  // CSS vars cache
  let cv = {};
  let bodyFont = 'sans-serif';

  // ─── Node Definitions ───
  // Dynamic-input nodes grow automatically when all ports are connected
  const DYNAMIC_BASE_INPUTS = 2; // merge, text-template, save-to-folder start with 2

  const NODE_DEFINITIONS = {
    'prompt': {
      title: '提示词',
      inputs: [],
      outputs: [{ name: 'text', type: 'text' }],
      properties: { prompt: '' },
      width: 200,
      height: 90
    },
    'negative-prompt': {
      title: '负面提示词',
      inputs: [],
      outputs: [{ name: 'text', type: 'text' }],
      properties: { prompt: '' },
      width: 200,
      height: 90
    },
    'image-input': {
      title: '图片输入',
      inputs: [],
      outputs: [{ name: 'image', type: 'image' }],
      properties: { imageData: '' },
      width: 200,
      height: 90
    },
    'style-preset': {
      title: '风格预设',
      inputs: [],
      outputs: [{ name: 'text', type: 'text' }],
      properties: { style: 'photorealistic' },
      width: 200,
      height: 90
    },
    'resolution': {
      title: '分辨率',
      inputs: [],
      outputs: [{ name: 'size', type: 'size' }],
      properties: { size: '1024x1024', customWidth: 1024, customHeight: 1024 },
      width: 200,
      height: 90
    },
    'quality': {
      title: '质量',
      inputs: [],
      outputs: [{ name: 'quality', type: 'quality' }],
      properties: { quality: 'standard' },
      width: 200,
      height: 90
    },
    'seed': {
      title: '随机种子',
      inputs: [],
      outputs: [{ name: 'seed', type: 'seed' }],
      properties: { seed: -1 },
      width: 200,
      height: 90
    },
    'canvas': {
      title: '画板',
      inputs: [],
      outputs: [{ name: 'image', type: 'image' }],
      properties: { imageData: '' },
      width: 200,
      height: 90
    },
    'text-template': {
      title: '文本模板',
      inputs: [
        { name: 'var1', type: 'text' },
        { name: 'var2', type: 'text' }
      ],
      outputs: [{ name: 'text', type: 'text' }],
      properties: { template: '' },
      width: 200,
      height: 124,
      dynamic: true
    },
    'merge': {
      title: '合并',
      inputs: [
        { name: 'text1', type: 'text' },
        { name: 'text2', type: 'text' }
      ],
      outputs: [{ name: 'text', type: 'text' }],
      properties: { separator: ', ' },
      width: 200,
      height: 124,
      dynamic: true  // inputs grow automatically
    },
    'image-scale': {
      title: '图片缩放',
      inputs: [{ name: 'image', type: 'image' }],
      outputs: [{ name: 'image', type: 'image' }],
      properties: { scale: '1.0' },
      width: 200,
      height: 110
    },
    'texture-asset': {
      title: '美术贴图资产',
      inputs: [
        { name: 'image', type: 'image' },
        { name: 'text', type: 'text' },
        { name: 'size', type: 'size' },
        { name: 'quality', type: 'quality' }
      ],
      outputs: [{ name: 'image', type: 'image' }],
      properties: {
        provider: '',
        quality: 'standard',
        textureType: 'diffuse_color',
        normalDir: 'opengl',
        colorSpace: 'sRGB',
        seamless: true
      },
      width: 220,
      height: 180
    },
    'texture-pack': {
      title: 'ORM 通道打包',
      inputs: [
        { name: 'roughness', type: 'image' },
        { name: 'metallic', type: 'image' },
        { name: 'ao', type: 'image' }
      ],
      outputs: [{ name: 'image', type: 'image' }],
      properties: { channelOrder: 'Roughness / Metallic / AO', lastOutput: '' },
      width: 220,
      height: 150
    },
    'generate': {
      title: '生成',
      inputs: [
        { name: 'text', type: 'text' },
        { name: 'image', type: 'image' },
        { name: 'size', type: 'size' },
        { name: 'quality', type: 'quality' },
        { name: 'seed', type: 'seed' }
      ],
      outputs: [{ name: 'image', type: 'image' }],
      properties: { provider: '', quality: 'standard' },
      width: 220,
      height: 196
    },
    'output': {
      title: '输出',
      inputs: [{ name: 'image', type: 'image' }],
      outputs: [{ name: 'save', type: 'save' }],
      properties: {
        resultUrl: '', status: '', errorMsg: '', executionMessage: '', elapsedMs: 0
      },
      width: 200,
      height: 120
    },
    'llm-call': {
      title: 'LLM 推理',
      inputs: [
        { name: 'prompt', type: 'text' },
        { name: 'context', type: 'text' },
        { name: 'image', type: 'image' }
      ],
      outputs: [{ name: 'text', type: 'text' }],
      properties: {
        model: '',
        systemPrompt: '',
        temperature: 0.7,
        optimizedPrompt: '',
        lastOutput: '',
        lastInput: '',
        lastContext: '',
        lastImage: '',
        lastSystemPrompt: '',
        lastRunAt: 0
      },
      width: 220,
      height: 170
    },
    'save-to-folder': {
      title: '保存到文件夹',
      inputs: [
        { name: 'image1', type: 'image' },
        { name: 'image2', type: 'image' }
      ],
      outputs: [],
      properties: { folderPath: '' },
      width: 220,
      height: 130,
      dynamic: true
    },
    'comment': {
      title: '注释',
      inputs: [],
      outputs: [],
      properties: { text: '双击编辑注释' },
      width: 280,
      height: 100,
      isComment: true
    }
  };

  // ─── Node Colors ───
  // Full-body background colors per node category (low-saturation)
  // Format: { accent: top strip, bg: body fill, header: title bar, border: border }
  const NODE_COLORS = {
    'prompt':           { accent: '#8B8B3A', bg: 'rgba(180,180,80,0.08)', header: 'rgba(180,180,80,0.12)', border: 'rgba(180,180,80,0.3)' },
    'negative-prompt':  { accent: '#8B3A3A', bg: 'rgba(200,80,80,0.08)',  header: 'rgba(200,80,80,0.12)',  border: 'rgba(200,80,80,0.3)' },
    'image-input':      { accent: '#3A6B8B', bg: 'rgba(80,140,200,0.08)', header: 'rgba(80,140,200,0.12)', border: 'rgba(80,140,200,0.3)' },
    'style-preset':     { accent: '#8B8B3A', bg: 'rgba(180,180,80,0.08)', header: 'rgba(180,180,80,0.12)', border: 'rgba(180,180,80,0.3)' },
    'resolution':       { accent: '#6B6B6B', bg: 'rgba(120,120,120,0.06)',header: 'rgba(120,120,120,0.10)',border: 'rgba(120,120,120,0.25)' },
    'quality':          { accent: '#6B6B6B', bg: 'rgba(120,120,120,0.06)',header: 'rgba(120,120,120,0.10)',border: 'rgba(120,120,120,0.25)' },
    'seed':             { accent: '#6B6B6B', bg: 'rgba(120,120,120,0.06)',header: 'rgba(120,120,120,0.10)',border: 'rgba(120,120,120,0.25)' },
    'canvas':           { accent: '#3A6B8B', bg: 'rgba(80,140,200,0.08)', header: 'rgba(80,140,200,0.12)', border: 'rgba(80,140,200,0.3)' },
    'text-template':    { accent: '#3A8B5A', bg: 'rgba(80,180,120,0.08)', header: 'rgba(80,180,120,0.12)', border: 'rgba(80,180,120,0.3)' },
    'merge':            { accent: '#3A8B5A', bg: 'rgba(80,180,120,0.08)', header: 'rgba(80,180,120,0.12)', border: 'rgba(80,180,120,0.3)' },
    'image-scale':      { accent: '#3A6B8B', bg: 'rgba(80,140,200,0.08)', header: 'rgba(80,140,200,0.12)', border: 'rgba(80,140,200,0.3)' },
    'llm-call':         { accent: '#5A3A8B', bg: 'rgba(130,80,200,0.10)', header: 'rgba(130,80,200,0.14)', border: 'rgba(130,80,200,0.35)' },
    'texture-asset':    { accent: '#3A5B8B', bg: 'rgba(60,100,180,0.10)', header: 'rgba(60,100,180,0.14)', border: 'rgba(60,100,180,0.35)' },
    'texture-pack':     { accent: '#5B6B8B', bg: 'rgba(90,110,160,0.10)', header: 'rgba(90,110,160,0.14)', border: 'rgba(90,110,160,0.35)' },
    'generate':         { accent: '#3A5B8B', bg: 'rgba(60,100,180,0.10)', header: 'rgba(60,100,180,0.14)', border: 'rgba(60,100,180,0.35)' },
    'output':           { accent: '#B85B1A', bg: 'rgba(230,130,50,0.12)', header: 'rgba(230,130,50,0.16)', border: 'rgba(230,130,50,0.4)' },
    'save-to-folder':   { accent: '#3A8B5A', bg: 'rgba(80,180,120,0.08)', header: 'rgba(80,180,120,0.12)', border: 'rgba(80,180,120,0.3)' },
    'comment':          { accent: '#8B8B3A', bg: 'rgba(255,235,120,0.12)', header: 'rgba(255,235,120,0.16)', border: 'rgba(200,180,60,0.4)' },
  };

  // ─── Constants ───

  const SEARCH_TYPES = [
    { type: 'prompt',           name: '提示词',       desc: '输入文本提示词' },
    { type: 'negative-prompt',  name: '负面提示词',   desc: '排除不需要的元素' },
    { type: 'image-input',      name: '图片输入',     desc: '上传参考图片' },
    { type: 'style-preset',     name: '风格预设',     desc: '选择艺术风格模板' },
    { type: 'resolution',       name: '分辨率',       desc: '设置输出尺寸' },
    { type: 'quality',          name: '质量',         desc: '标准或高清' },
    { type: 'seed',             name: '随机种子',     desc: '控制生成可复现性' },
    { type: 'canvas',           name: '画板',         desc: '手绘参考图' },
    { type: 'text-template',    name: '文本模板',     desc: '变量替换模板（动态端口）' },
    { type: 'merge',            name: '合并',         desc: '合并多个文本输入（动态端口）' },
    { type: 'image-scale',      name: '图片缩放',     desc: '缩放参考图' },
    { type: 'texture-asset',    name: '美术贴图资产', desc: 'Diffuse/法线/高光/白模贴图生成' },
    { type: 'texture-pack',     name: 'ORM 通道打包', desc: '将 Roughness、Metallic、AO 合成为 RGB 贴图' },
    { type: 'llm-call',         name: 'LLM 推理',   desc: '调用AI模型进行文本推理' },
    { type: 'generate',         name: '生成',         desc: 'AI 图像生成节点' },
    { type: 'output',           name: '输出',         desc: '显示生成结果' },
    { type: 'save-to-folder',   name: '保存到文件夹', desc: '批量保存图片到文件夹（动态端口）' },
    { type: 'comment',          name: '注释',         desc: '添加注释说明' }
  ];

  const STYLE_PRESETS = [
    { value: 'photorealistic', label: '真实照片' },
    { value: 'concept-art',    label: '概念艺术' },
    { value: 'anime',          label: '动漫' },
    { value: 'oil-painting',   label: '油画' },
    { value: 'watercolor',     label: '水彩' },
    { value: '3d-render',      label: '3D渲染' },
    { value: 'cyberpunk',      label: '赛博朋克' },
    { value: 'minimalist',     label: '极简' }
  ];

  const STYLE_PREFIXES = {
    'photorealistic': 'photorealistic, ultra detailed, ',
    'concept-art':    'concept art, detailed, ',
    'anime':          'anime style, ',
    'oil-painting':   'oil painting, ',
    'watercolor':     'watercolor, ',
    '3d-render':      '3D render, octane, ',
    'cyberpunk':      'cyberpunk, neon, ',
    'minimalist':     'minimalist, '
  };

  const RESOLUTIONS = [
    { value: '1024x1024',  label: '1024 × 1024 (方形)' },
    { value: '1792x1024',  label: '1792 × 1024 (横版)' },
    { value: '1024x1792',  label: '1024 × 1792 (竖版)' },
    { value: '512x512',    label: '512 × 512 (快速)' },
    { value: '2048x2048',  label: '2048 × 2048 (高清)' },
    { value: 'custom',     label: '自定义' }
  ];

  const QUALITY_OPTIONS = [
    { value: 'standard', label: '标准' },
    { value: 'hd',       label: '高清' }
  ];

  const TEXTURE_TYPE_OPTIONS = [
    { value: 'diffuse_color',      label: 'Diffuse Color' },
    { value: 'diff_to_normal',     label: 'Diff to Normal' },
    { value: 'spec_mask',          label: 'Spec Mask' },
    { value: 'roughness',          label: 'Roughness / 粗糙度' },
    { value: 'smoothness',         label: 'Smoothness / 光滑度' },
    { value: 'metallic',           label: 'Metallic / 金属度' },
    { value: 'ao',                 label: 'AO / 环境遮蔽' },
    { value: 'opacity',            label: 'Opacity / Cutout' },
    { value: 'curvature',          label: 'Curvature / 曲率' },
    { value: 'cavity',             label: 'Cavity / 缝隙' },
    { value: 'height',              label: 'Height / Displacement' },
    { value: 'emissive',            label: 'Emissive / 自发光' },
    { value: 'sss_thickness',       label: 'SSS / Thickness' },
    { value: 'white_model_diffuse', label: 'White Model → Diffuse' }
  ];

  // Port layout constants (in world coordinates)
  const TITLE_H     = 28;
  const PORT_R      = 8;
  const PORT_GAP    = 24;
  const PORT_START  = TITLE_H + 18;
  const GRID_SIZE   = 24;

  // Generate types — only these can feed into output node
  const GENERATE_TYPES = ['generate', 'texture-asset'];

  // Terminal node types — the workflow needs at least one of these to be
  // executable; both consume upstream images and mark the end of a branch.
  const TERMINAL_TYPES = ['output', 'save-to-folder'];

  // ─── Utilities ───

  function esc(s) {
    return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;')
      .replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  async function safeJson(r) {
    const text = await r.text();
    try { return JSON.parse(text); }
    catch (e) { throw new Error('服务器返回非 JSON: ' + text.substring(0, 200)); }
  }

  function showGraphNotice(message, kind) {
    if (!message) return;
    let notice = $('nodes-graph-notice');
    if (!notice) {
      notice = document.createElement('div');
      notice.id = 'nodes-graph-notice';
      notice.className = 'nodes-graph-notice';
      (canvasWrap || $('nodes-canvas-wrap') || document.body).appendChild(notice);
    }
    notice.textContent = message;
    notice.dataset.kind = kind || 'warning';
    notice.classList.add('visible');
    if (graphNoticeTimer) clearTimeout(graphNoticeTimer);
    graphNoticeTimer = setTimeout(() => {
      notice.classList.remove('visible');
      graphNoticeTimer = null;
    }, 2600);
  }

  function formatElapsed(ms) {
    const totalSeconds = Math.max(0, Math.floor(Number(ms || 0) / 1000));
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return minutes > 0
      ? minutes + '分' + String(seconds).padStart(2, '0') + '秒'
      : String(seconds).padStart(2, '0') + '秒';
  }

  function updateOutputElapsed() {
    if (!workflowStartedAt) return;
    const elapsedMs = Math.max(0, performance.now() - workflowStartedAt);
    for (const node of nodes) {
      if (!TERMINAL_TYPES.includes(node.type) || node.properties.status !== 'loading') continue;
      node.properties.elapsedMs = elapsedMs;
    }
    if (propNodeId !== null) {
      const selected = nodes.find(node => node.id === propNodeId);
      if (selected && TERMINAL_TYPES.includes(selected.type) && selected.properties.status === 'loading') {
        renderPropBody(selected);
      }
    }
  }

  function startOutputElapsed() {
    workflowStartedAt = performance.now();
    if (outputTimerHandle !== null) clearInterval(outputTimerHandle);
    outputTimerHandle = setInterval(updateOutputElapsed, 250);
    updateOutputElapsed();
  }

  function stopOutputElapsed() {
    updateOutputElapsed();
    if (outputTimerHandle !== null) {
      clearInterval(outputTimerHandle);
      outputTimerHandle = null;
    }
    workflowStartedAt = 0;
  }

  // Keep the workflow abort signal alive until the response body has been
  // consumed. Some providers send headers quickly and then stall while
  // streaming the JSON body, so aborting only fetch() is not sufficient.
  async function fetchTextWithWorkflowAbort(url, options, timeoutMs) {
    const controller = new AbortController();
    const workflowSignal = workflowAbortController?.signal;
    let timedOut = false;
    const timeoutId = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);
    const abortFromWorkflow = () => controller.abort();

    if (workflowSignal) {
      if (workflowSignal.aborted) controller.abort();
      else workflowSignal.addEventListener('abort', abortFromWorkflow, { once: true });
    }

    try {
      const response = await fetch(url, { ...(options || {}), signal: controller.signal });
      const text = await response.text();
      return { response, text };
    } catch (error) {
      if (timedOut && error && error.name === 'AbortError') {
        const timeoutError = new Error('请求超时（' + Math.round(timeoutMs / 1000) + '秒）');
        timeoutError.name = 'TimeoutError';
        throw timeoutError;
      }
      throw error;
    } finally {
      clearTimeout(timeoutId);
      if (workflowSignal) workflowSignal.removeEventListener('abort', abortFromWorkflow);
    }
  }

  function refreshCssVars() {
    const cs = getComputedStyle(document.documentElement);
    cv = {
      bgCard:      cs.getPropertyValue('--bg-card').trim()      || '#2A2A2A',
      bgSidebar:   cs.getPropertyValue('--bg-sidebar').trim()   || '#1D1D1D',
      bgWindow:    cs.getPropertyValue('--bg-window').trim()    || '#171717',
      bgHover:     cs.getPropertyValue('--bg-hover').trim()     || '#333333',
      border:      cs.getPropertyValue('--border').trim()       || '#343434',
      borderLight: cs.getPropertyValue('--border-light').trim() || '#404040',
      accent:      cs.getPropertyValue('--accent').trim()       || '#3370FF',
      accentText:  cs.getPropertyValue('--accent-text').trim()  || '#ffffff',
      accentLight: cs.getPropertyValue('--accent-light').trim() || 'rgba(51,112,255,0.10)',
      textPrimary: cs.getPropertyValue('--text-primary').trim() || '#E8E8E8',
      textSecond:  cs.getPropertyValue('--text-secondary').trim()|| '#A0A0A0',
      textMuted:   cs.getPropertyValue('--text-muted').trim()   || '#6B6B6B',
      success:     cs.getPropertyValue('--success').trim()      || '#4ADE80',
      warning:     cs.getPropertyValue('--warning').trim()      || '#FBBF24',
      error:       cs.getPropertyValue('--error').trim()        || '#F87171'
    };
    bodyFont = getComputedStyle(document.body).fontFamily || 'sans-serif';
  }

  // ─── Coordinate Transforms ───

  function screenToWorld(sx, sy) {
    return { x: sx / zoom - panX, y: sy / zoom - panY };
  }

  function worldToScreen(wx, wy) {
    return { x: (wx + panX) * zoom, y: (wy + panY) * zoom };
  }

  function getMousePos(e) {
    const rect = canvas.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  // ─── Port Position Helpers ───

  function getPortPos(node, portIndex, isOutput) {
    const x = isOutput ? node.x + node.width : node.x;
    const y = node.y + PORT_START + portIndex * PORT_GAP;
    return { x, y };
  }

  // ─── Node Management ───

  function createNode(type, x, y) {
    const def = NODE_DEFINITIONS[type];
    if (!def) return null;
    beginHistory();
    const node = {
      id: nextNodeId++,
      type,
      x: x || 0,
      y: y || 0,
      width: def.width,
      height: def.height,
      title: def.title,
      inputs: def.inputs.map(p => ({ ...p })),
      outputs: def.outputs.map(p => ({ ...p })),
      properties: JSON.parse(JSON.stringify(def.properties))
    };
    if ((type === 'generate' || type === 'texture-asset') && providers.length > 0) {
      if (!node.properties.provider) {
        node.properties.provider = providers[0].name;
      }
    }
    if (type === 'llm-call' && llmModels.length > 0) {
      if (!node.properties.model) node.properties.model = llmDefaultModel || llmModels[0];
    }
    nodes.push(node);
    commitHistory();
    return node;
  }

  function graphSnapshot() {
    return JSON.stringify({
      nodes: nodes.map(node => ({
        ...node,
        inputs: node.inputs.map(port => ({ ...port })),
        outputs: node.outputs.map(port => ({ ...port })),
        properties: JSON.parse(JSON.stringify(node.properties))
      })),
      connections: connections.map(connection => ({
        ...connection,
        toPortIdx: connectionPortIndex(connection)
      })),
      nextNodeId,
      nextConnId,
      panX,
      panY,
      zoom
    });
  }

  function beginHistory() {
    if (restoringHistory || historyTransaction !== null) return;
    historyTransaction = graphSnapshot();
  }

  function commitHistory() {
    if (historyTransaction === null || restoringHistory) return;
    const before = historyTransaction;
    historyTransaction = null;
    if (before === graphSnapshot()) return;
    undoStack.push(before);
    if (undoStack.length > MAX_HISTORY) undoStack.shift();
    redoStack = [];
  }

  function restoreGraphSnapshot(serialized) {
    let snapshot;
    try {
      snapshot = typeof serialized === 'string' ? JSON.parse(serialized) : serialized;
    } catch (e) {
      console.error('无法恢复工作流历史:', e);
      return false;
    }
    if (!snapshot || !Array.isArray(snapshot.nodes) || !Array.isArray(snapshot.connections)) {
      return false;
    }

    restoringHistory = true;
    try {
      const restoredNodes = snapshot.nodes
        .map(raw => {
          const def = NODE_DEFINITIONS[raw.type];
          if (!def || !Number.isInteger(Number(raw.id))) return null;
          return {
            id: Number(raw.id),
            type: raw.type,
            x: Number.isFinite(Number(raw.x)) ? Number(raw.x) : 0,
            y: Number.isFinite(Number(raw.y)) ? Number(raw.y) : 0,
            width: Number(raw.width) > 0 ? Number(raw.width) : def.width,
            height: Number(raw.height) > 0 ? Number(raw.height) : def.height,
            title: typeof raw.title === 'string' ? raw.title : def.title,
            inputs: Array.isArray(raw.inputs) ? raw.inputs.map(port => ({ ...port })) : def.inputs.map(port => ({ ...port })),
            outputs: Array.isArray(raw.outputs) ? raw.outputs.map(port => ({ ...port })) : def.outputs.map(port => ({ ...port })),
            properties: { ...JSON.parse(JSON.stringify(def.properties)), ...(raw.properties || {}) }
          };
        })
        .filter(Boolean);
      const nodeIds = new Set(restoredNodes.map(node => node.id));
      const restoredConnections = snapshot.connections
        .map(raw => ({
          id: Number(raw.id),
          fromNodeId: Number(raw.fromNodeId),
          fromPortIdx: Number(raw.fromPortIdx),
          toNodeId: Number(raw.toNodeId),
          toPortIdx: connectionPortIndex(raw)
        }))
        .filter(connection => Number.isInteger(connection.id) &&
          nodeIds.has(connection.fromNodeId) && nodeIds.has(connection.toNodeId));

      nodes = restoredNodes;
      connections = restoredConnections;
      panX = Number.isFinite(Number(snapshot.panX)) ? Number(snapshot.panX) : panX;
      panY = Number.isFinite(Number(snapshot.panY)) ? Number(snapshot.panY) : panY;
      zoom = Number.isFinite(Number(snapshot.zoom)) ? Number(snapshot.zoom) : zoom;
      nextNodeId = Number.isInteger(Number(snapshot.nextNodeId)) ? Number(snapshot.nextNodeId) : 1;
      nextConnId = Number.isInteger(Number(snapshot.nextConnId)) ? Number(snapshot.nextConnId) : 1;
      normalizeGraph();
      resultImages = {};
      nodeExecStatus = {};
      nodeOutputs = {};
      selectedNodeId = null;
      selectedNodeIds.clear();
      selectedConnId = null;
      propNodeId = null;
      closePropPanel();
    } finally {
      restoringHistory = false;
    }
    return true;
  }

  function undo() {
    if (isExecuting || undoStack.length === 0) return;
    if (historyTransaction !== null) commitHistory();
    const previous = undoStack.pop();
    if (!previous) return;
    redoStack.push(graphSnapshot());
    restoreGraphSnapshot(previous);
  }

  function redo() {
    if (isExecuting || redoStack.length === 0) return;
    if (historyTransaction !== null) commitHistory();
    const next = redoStack.pop();
    if (!next) return;
    undoStack.push(graphSnapshot());
    restoreGraphSnapshot(next);
  }

  function resetHistory() {
    if (viewHistoryTimer) {
      clearTimeout(viewHistoryTimer);
      viewHistoryTimer = null;
    }
    historyTransaction = null;
    undoStack = [];
    redoStack = [];
  }

  function flushViewHistory() {
    if (!viewHistoryTimer) return;
    clearTimeout(viewHistoryTimer);
    viewHistoryTimer = null;
    commitHistory();
  }

  function dynamicPortName(node, index) {
    const prefix = node.type === 'merge' ? 'text' :
      node.type === 'text-template' ? 'var' :
      node.type === 'save-to-folder' ? 'image' : 'input';
    return prefix + (index + 1);
  }

  function updateDynamicNodeLayout(node) {
    const def = NODE_DEFINITIONS[node.type];
    if (!def || !def.dynamic) return;
    node.height = def.height + Math.max(0, node.inputs.length - DYNAMIC_BASE_INPUTS) * PORT_GAP + 20;
  }

  // Dynamic nodes use one connection per visible port. The extra port is a
  // capacity affordance, not permission to fan multiple wires into one port.
  function syncDynamicPorts(node) {
    const def = NODE_DEFINITIONS[node.type];
    if (!def || !def.dynamic) return;
    while (node.inputs.length < DYNAMIC_BASE_INPUTS) {
      const index = node.inputs.length;
      node.inputs.push({ name: dynamicPortName(node, index), type: node.type === 'save-to-folder' ? 'image' : 'text' });
    }
    const used = index => connections.some(c =>
      c.toNodeId === node.id && connectionPortIndex(c) === index
    );
    while (node.inputs.length > DYNAMIC_BASE_INPUTS && !used(node.inputs.length - 1)) {
      node.inputs.pop();
    }
    while (node.inputs.some((_, index) => used(index)) &&
           node.inputs.every((_, index) => used(index))) {
      const index = node.inputs.length;
      node.inputs.push({ name: dynamicPortName(node, index), type: node.type === 'save-to-folder' ? 'image' : 'text' });
    }
    node.inputs.forEach((port, index) => {
      port.name = dynamicPortName(node, index);
      port.type = node.type === 'save-to-folder' ? 'image' : 'text';
    });
    updateDynamicNodeLayout(node);
  }

  function normalizeGraph() {
    const validNodes = nodes.filter(node => NODE_DEFINITIONS[node.type] && Number.isInteger(Number(node.id)));
    nodes = validNodes;
    const nodeById = new Map(nodes.map(node => [Number(node.id), node]));
    const normalized = [];

    for (const node of nodes) {
      const def = NODE_DEFINITIONS[node.type];
      if (!def) continue;
      node.id = Number(node.id);
      node.inputs = Array.isArray(node.inputs) ? node.inputs : def.inputs.map(port => ({ ...port }));
      node.outputs = Array.isArray(node.outputs) ? node.outputs : def.outputs.map(port => ({ ...port }));
      node.properties = { ...JSON.parse(JSON.stringify(def.properties)), ...(node.properties || {}) };
      if (!def.dynamic) {
        node.inputs = def.inputs.map((port, index) => ({
          ...port,
          ...(node.inputs[index] || {})
        }));
      }
      node.outputs = def.outputs.map((port, index) => ({
        ...port,
        ...(node.outputs[index] || {})
      }));
      if (def.dynamic) {
        node.inputs = node.inputs.map((port, index) => ({
          ...port,
          name: dynamicPortName(node, index),
          type: node.type === 'save-to-folder' ? 'image' : 'text'
        }));
      }
      updateDynamicNodeLayout(node);
    }

    // Keep every valid raw connection until dynamic ports have been repaired.
    // Older workflow files could contain two wires aimed at one dynamic slot;
    // deduplicating by target first would silently lose the extra wire before
    // it had a chance to move to a newly allocated slot.
    const validConnections = [];
    const usedConnectionIds = new Set();
    let repairConnectionId = Math.max(
      nextConnId,
      connections.reduce((max, raw) => Math.max(max, Number(raw.id) + 1 || 1), 1)
    );
    for (const raw of connections) {
      let id = Number(raw.id);
      const connection = {
        id,
        fromNodeId: Number(raw.fromNodeId),
        fromPortIdx: Number(raw.fromPortIdx),
        toNodeId: Number(raw.toNodeId),
        toPortIdx: connectionPortIndex(raw)
      };
      const fromNode = nodeById.get(connection.fromNodeId);
      const toNode = nodeById.get(connection.toNodeId);
      if (!Number.isInteger(id) || !fromNode || !toNode ||
          !Number.isInteger(connection.fromPortIdx) || !Number.isInteger(connection.toPortIdx) ||
          connection.toPortIdx < 0 || !fromNode.outputs[connection.fromPortIdx]) continue;
      if (usedConnectionIds.has(id)) {
        while (usedConnectionIds.has(repairConnectionId)) repairConnectionId++;
        id = repairConnectionId++;
        connection.id = id;
      }
      usedConnectionIds.add(id);
      validConnections.push(connection);
    }

    const dynamicGroups = new Map();
    const usedStaticTargets = new Set();
    for (const connection of validConnections) {
      const fromNode = nodeById.get(connection.fromNodeId);
      const toNode = nodeById.get(connection.toNodeId);
      const def = NODE_DEFINITIONS[toNode.type];
      if (def.dynamic) {
        if (!dynamicGroups.has(toNode.id)) dynamicGroups.set(toNode.id, []);
        dynamicGroups.get(toNode.id).push(connection);
      } else {
        const key = connection.toNodeId + ':' + connection.toPortIdx;
        if (usedStaticTargets.has(key) || !toNode.inputs[connection.toPortIdx] ||
            !isCompatibleConnection(fromNode, connection.fromPortIdx, toNode, connection.toPortIdx)) continue;
        usedStaticTargets.add(key);
        normalized.push(connection);
      }
    }

    for (const [nodeId, group] of dynamicGroups) {
      const node = nodeById.get(nodeId);
      group.sort((a, b) => a.toPortIdx - b.toPortIdx || a.id - b.id);
      const requestedPorts = new Set(group.map(connection => connection.toPortIdx));
      const usedPorts = new Set();
      const portType = node.type === 'save-to-folder' ? 'image' : 'text';

      const ensurePort = index => {
        while (node.inputs.length <= index) {
          const nextIndex = node.inputs.length;
          node.inputs.push({ name: dynamicPortName(node, nextIndex), type: portType });
        }
      };

      for (const connection of group) {
        const fromNode = nodeById.get(connection.fromNodeId);
        if (!fromNode) continue;
        let targetIndex = connection.toPortIdx;
        ensurePort(targetIndex);
        if (usedPorts.has(targetIndex)) {
          // Preserve distinct historical slot numbers. A duplicate is moved
          // to the next genuinely free slot, skipping slots requested by a
          // different connection so that the original slot remains stable.
          targetIndex++;
          while (usedPorts.has(targetIndex) || requestedPorts.has(targetIndex)) targetIndex++;
          ensurePort(targetIndex);
        }
        if (!isCompatibleConnection(fromNode, connection.fromPortIdx, node, targetIndex)) continue;
        usedPorts.add(targetIndex);
        connection.toPortIdx = targetIndex;
        normalized.push(connection);
      }
      while (node.inputs.length < DYNAMIC_BASE_INPUTS) {
        const index = node.inputs.length;
        node.inputs.push({ name: dynamicPortName(node, index), type: portType });
      }
    }

    connections = normalized;
    for (const node of nodes) {
      if (NODE_DEFINITIONS[node.type]?.dynamic) syncDynamicPorts(node);
    }
    nextNodeId = Math.max(nextNodeId, nodes.reduce((max, node) => Math.max(max, node.id + 1), 1));
    nextConnId = Math.max(nextConnId, connections.reduce((max, connection) => Math.max(max, connection.id + 1), 1));
  }

  function deleteNode(nodeId) {
    nodes = nodes.filter(n => n.id !== nodeId);
    connections = connections.filter(c => c.fromNodeId !== nodeId && c.toNodeId !== nodeId);
    for (const node of nodes) {
      if (NODE_DEFINITIONS[node.type]?.dynamic) syncDynamicPorts(node);
    }
    if (selectedNodeId === nodeId) selectedNodeId = null;
    selectedNodeIds.delete(nodeId);
    if (propNodeId === nodeId) closePropPanel();
    delete resultImages[nodeId];
  }

  // ─── Connection Management ───

  function connectionPortIndex(connection) {
    const value = connection && (connection.toPortIdx !== undefined ? connection.toPortIdx : connection.toPortIndex);
    return Number.isInteger(value) ? value : parseInt(value, 10);
  }

  function inputPortIndex(node, portName) {
    return node && Array.isArray(node.inputs)
      ? node.inputs.findIndex(port => port.name === portName)
      : -1;
  }

  function inputConnection(node, portIdx) {
    if (!node || portIdx < 0) return null;
    return connections.find(connection =>
      connection.toNodeId === node.id && connectionPortIndex(connection) === portIdx
    ) || null;
  }

  // A connected quality node is the single source of truth for generate nodes.
  // Keeping this lookup in one place prevents the canvas and property panel
  // from displaying a stale local fallback value.
  function qualityOverride(node) {
    if (!node || (node.type !== 'generate' && node.type !== 'texture-asset')) return null;
    const portIdx = inputPortIndex(node, 'quality');
    const connection = inputConnection(node, portIdx);
    if (!connection) return null;
    const source = nodes.find(candidate => candidate.id === connection.fromNodeId);
    if (!source || source.type !== 'quality') return null;
    return {
      connection,
      source,
      value: source.properties.quality === 'hd' ? 'hd' : 'standard'
    };
  }

  function effectiveQuality(node) {
    const override = qualityOverride(node);
    return override ? override.value : (node.properties.quality === 'hd' ? 'hd' : 'standard');
  }

  function refreshQualityDependents(sourceNode) {
    if (!sourceNode || sourceNode.type !== 'quality') return;
    for (const connection of connections) {
      if (connection.fromNodeId !== sourceNode.id) continue;
      const target = nodes.find(node => node.id === connection.toNodeId);
      const targetPortIdx = connectionPortIndex(connection);
      if (target && (target.type === 'generate' || target.type === 'texture-asset') && target.inputs[targetPortIdx]?.name === 'quality') {
        refreshPropPanel(target);
      }
    }
  }

  function isCompatibleConnection(fromNode, fromPortIdx, toNode, toPortIdx) {
    const fromPort = fromNode && fromNode.outputs && fromNode.outputs[fromPortIdx];
    const toPort = toNode && toNode.inputs && toNode.inputs[toPortIdx];
    if (!fromPort || !toPort) return false;
    return fromPort.type === toPort.type || (fromPort.type === 'save' && toPort.type === 'image');
  }

  function connectionWouldCycle(fromNodeId, toNodeId) {
    if (fromNodeId === toNodeId) return true;
    const outgoing = new Map();
    for (const connection of connections) {
      if (!outgoing.has(connection.fromNodeId)) outgoing.set(connection.fromNodeId, []);
      outgoing.get(connection.fromNodeId).push(connection.toNodeId);
    }
    const pending = [toNodeId];
    const visited = new Set();
    while (pending.length > 0) {
      const current = pending.pop();
      if (current === fromNodeId) return true;
      if (visited.has(current)) continue;
      visited.add(current);
      for (const next of outgoing.get(current) || []) pending.push(next);
    }
    return false;
  }

  function connectionRejectionReason(fromNode, fromPortIdx, toNode, toPortIdx) {
    if (!fromNode || !toNode) return '连接目标不存在';
    if (fromNode.id === toNode.id) return '不能把节点连接到自身';
    const fromPort = fromNode.outputs[fromPortIdx];
    const toPort = toNode.inputs[toPortIdx];
    if (!fromPort || !toPort) return '请选择有效的输出端口和输入端口';
    if (!isCompatibleConnection(fromNode, fromPortIdx, toNode, toPortIdx)) {
      return '端口类型不兼容：' + fromPort.type + ' 无法连接到 ' + toPort.type;
    }
    if (connections.some(c => c.toNodeId === toNode.id && connectionPortIndex(c) === toPortIdx)) {
      return '输入端口「' + toPort.name + '」已连接，请选择空的输入端口';
    }
    if (connectionWouldCycle(fromNode.id, toNode.id)) {
      return '连接会形成循环依赖，已拒绝';
    }
    return '';
  }

  function canConnect(fromNode, fromPortIdx, toNode, toPortIdx) {
    return !connectionRejectionReason(fromNode, fromPortIdx, toNode, toPortIdx);
  }

  function createConnection(fromNodeId, fromPortIdx, toNodeId, toPortIdx) {
    const fromNode = nodes.find(n => n.id === fromNodeId);
    const toNode = nodes.find(n => n.id === toNodeId);
    const reason = connectionRejectionReason(fromNode, fromPortIdx, toNode, toPortIdx);
    if (reason) {
      showGraphNotice(reason, 'warning');
      return false;
    }

    beginHistory();

    const connection = {
      id: nextConnId++,
      fromNodeId, fromPortIdx,
      toNodeId, toPortIdx
    };
    connections.push(connection);

    syncDynamicPorts(toNode);
    commitHistory();
    refreshPropPanel(toNode);
    const targetPortIdx = connectionPortIndex(connection);
    const targetPort = toNode.inputs[targetPortIdx];
    showGraphNotice('已连接：' + fromNode.outputs[fromPortIdx].name + ' → ' +
      (targetPort ? targetPort.name : '输入端口'), 'success');
    return true;
  }

  function removeConnection(connectionId) {
    const connection = connections.find(c => c.id === connectionId);
    if (!connection) return false;
    beginHistory();
    connections = connections.filter(c => c.id !== connectionId);
    const target = nodes.find(n => n.id === connection.toNodeId);
    if (target && NODE_DEFINITIONS[target.type]?.dynamic) syncDynamicPorts(target);
    commitHistory();
    refreshPropPanel(target);
    return true;
  }

  // ─── Hit Testing ───

  function hitTestNode(wx, wy) {
    for (let i = nodes.length - 1; i >= 0; i--) {
      const n = nodes[i];
      if (wx >= n.x && wx <= n.x + n.width &&
          wy >= n.y && wy <= n.y + n.height) {
        return n;
      }
    }
    return null;
  }

  function hitTestPort(wx, wy) {
    const r = PORT_R + 8 / zoom;  // Constant 8px screen-space padding regardless of zoom
    for (let i = nodes.length - 1; i >= 0; i--) {
      const n = nodes[i];
      for (let j = 0; j < n.outputs.length; j++) {
        const p = getPortPos(n, j, true);
        const dx = wx - p.x, dy = wy - p.y;
        if (dx * dx + dy * dy <= r * r)
          return { node: n, portIndex: j, isOutput: true };
      }
      for (let j = 0; j < n.inputs.length; j++) {
        const p = getPortPos(n, j, false);
        const dx = wx - p.x, dy = wy - p.y;
        const nDef = NODE_DEFINITIONS[n.type];
        const isDyn = nDef && nDef.dynamic;
        if (isDyn) {
          // Dynamic ports are vertical rectangles — wider hit area
          const hitW = (PORT_R + 4) / zoom;
          // Keep adjacent dynamic slots independently clickable at every zoom.
          const hitH = Math.min(PORT_GAP * 0.45, PORT_R + 6) / zoom;
          if (Math.abs(dx) <= hitW && Math.abs(dy) <= hitH)
            return { node: n, portIndex: j, isOutput: false };
        } else {
          if (dx * dx + dy * dy <= r * r)
            return { node: n, portIndex: j, isOutput: false };
        }
      }
    }
    return null;
  }

  // Hit test connection lines — checks distance from point to bezier curve
  // Uses screen-space coordinates to match drawBezier's curve shape exactly
  function hitTestConnection(wx, wy) {
    const threshold = 12;  // 12px screen-space threshold (was 8, increased for usability)
    const testPt = worldToScreen(wx, wy);
    const sx = testPt.x;
    const sy = testPt.y;
    for (const conn of connections) {
      const fromNode = nodes.find(n => n.id === conn.fromNodeId);
      const toNode = nodes.find(n => n.id === conn.toNodeId);
      if (!fromNode || !toNode) continue;
      const fp = getPortPos(fromNode, conn.fromPortIdx, true);
      const tp = getPortPos(toNode, connectionPortIndex(conn), false);
      const fs = worldToScreen(fp.x, fp.y);
      const ts = worldToScreen(tp.x, tp.y);
      // Use same control point offset as drawBezier (screen-space)
      for (let i = 0; i <= 20; i++) {
        const t = i / 20;
        const off = Math.max(40, Math.abs(ts.x - fs.x) * 0.5);
        const x = bezierAt(fs.x, fs.x + off, ts.x - off, ts.x, t);
        const y = bezierAt(fs.y, fs.y, ts.y, ts.y, t);
        const dx = sx - x, dy = sy - y;
        if (dx * dx + dy * dy <= threshold * threshold) return conn;
      }
    }
    return null;
  }

  function bezierAt(p0, p1, p2, p3, t) {
    const mt = 1 - t;
    return mt*mt*mt*p0 + 3*mt*mt*t*p1 + 3*mt*t*t*p2 + t*t*t*p3;
  }

  // ─── Canvas Sizing ───

  function resizeCanvas() {
    if (!canvas || !canvasWrap) return;
    const rect = canvasWrap.getBoundingClientRect();
    viewW = Math.max(rect.width, 100);
    viewH = Math.max(rect.height, 100);
    dpr = window.devicePixelRatio || 1;
    canvas.width = Math.floor(viewW * dpr);
    canvas.height = Math.floor(viewH * dpr);
    canvas.style.width = viewW + 'px';
    canvas.style.height = viewH + 'px';
  }

  // ─── Rendering ───

  function render() {
    rafId = requestAnimationFrame(render);
    if (!ctx) return;

    // Skip heavy work if view is not active
    const view = $('view-nodes');
    if (!view || view.style.display === 'none') return;

    ctx.save();
    ctx.scale(dpr, dpr);

    // Clear
    ctx.fillStyle = cv.bgWindow;
    ctx.fillRect(0, 0, viewW, viewH);

    drawGrid();
    drawConnections();
    if (connectingFrom) drawConnectingLine();
    for (const n of nodes) drawNode(n);
    if (isBoxSelecting) drawSelectionBox();

    ctx.restore();
  }

  function drawSelectionBox() {
    const x = Math.min(boxStartX, boxEndX);
    const y = Math.min(boxStartY, boxEndY);
    const w = Math.abs(boxEndX - boxStartX);
    const h = Math.abs(boxEndY - boxStartY);
    ctx.fillStyle = cv.accentLight || 'rgba(51,112,255,0.1)';
    ctx.fillRect(x, y, w, h);
    ctx.strokeStyle = cv.accent || '#3370FF';
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    ctx.strokeRect(x, y, w, h);
    ctx.setLineDash([]);
  }

  function drawGrid() {
    const tl = screenToWorld(0, 0);
    const br = screenToWorld(viewW, viewH);
    const startX = Math.floor(tl.x / GRID_SIZE) * GRID_SIZE;
    const endX = Math.ceil(br.x / GRID_SIZE) * GRID_SIZE;
    const startY = Math.floor(tl.y / GRID_SIZE) * GRID_SIZE;
    const endY = Math.ceil(br.y / GRID_SIZE) * GRID_SIZE;

    // Fix #3: use fixed color instead of CSS variable for grid visibility
    const isDark = document.documentElement.classList.contains('dark');
    ctx.fillStyle = isDark ? 'rgba(100,100,100,0.3)' : 'rgba(150,150,150,0.4)';
    for (let x = startX; x <= endX; x += GRID_SIZE) {
      for (let y = startY; y <= endY; y += GRID_SIZE) {
        const s = worldToScreen(x, y);
        // Draw 2px dots
        ctx.fillRect(s.x - 1, s.y - 1, 2, 2);
      }
    }
  }

  function drawNode(node) {
    const s = worldToScreen(node.x, node.y);
    const w = node.width * zoom;
    const h = node.height * zoom;
    const r = 8 * zoom;
    const isSelected = selectedNodeId === node.id || selectedNodeIds.has(node.id);

    // Comment nodes: ComfyUI-style sticky note (yellowish, no title bar, no ports)
    if (node.type === 'comment') {
      const cnc = NODE_COLORS['comment'];
      ctx.save();
      ctx.shadowColor = 'rgba(0,0,0,0.15)';
      ctx.shadowBlur = 6 * zoom;
      ctx.shadowOffsetY = 1 * zoom;
      const isDark = document.documentElement.classList.contains('dark');
      ctx.fillStyle = isDark ? (cnc ? cnc.bg : 'rgba(255,235,120,0.12)') : 'rgba(255,235,120,0.5)';
      roundRect(ctx, s.x, s.y, w, h, 4 * zoom);
      ctx.fill();
      ctx.restore();
      // Dashed border
      ctx.strokeStyle = isSelected ? cv.accent : (cnc ? cnc.border : 'rgba(200,180,60,0.4)');
      ctx.lineWidth = isSelected ? 2 : 1;
      ctx.setLineDash([4, 3]);
      roundRect(ctx, s.x, s.y, w, h, 4 * zoom);
      ctx.stroke();
      ctx.setLineDash([]);
      // Comment text (wrapping, centered area)
      ctx.fillStyle = isDark ? 'rgba(255,235,120,0.9)' : 'rgba(80,60,0,0.9)';
      ctx.font = (12 * zoom) + 'px ' + bodyFont;
      ctx.textBaseline = 'top';
      ctx.textAlign = 'left';
      const text = node.properties.text || '双击编辑注释';
      const maxWidth = (node.width - 16) * zoom;
      let line = '';
      let lineY = s.y + 8 * zoom;
      for (let i = 0; i < text.length; i++) {
        const testLine = line + text[i];
        if (ctx.measureText(testLine).width > maxWidth && line) {
          ctx.fillText(line, s.x + 8 * zoom, lineY);
          line = text[i];
          lineY += 16 * zoom;
          if (lineY > s.y + (node.height - 8) * zoom) break;
        } else {
          line = testLine;
        }
      }
      if (line && lineY <= s.y + (node.height - 8) * zoom) {
        ctx.fillText(line, s.x + 8 * zoom, lineY);
      }
      return; // Skip all normal node drawing
    }

    // Shadow + background (category-colored body)
    const nc = NODE_COLORS[node.type];
    ctx.save();
    ctx.shadowColor = 'rgba(0,0,0,0.3)';
    ctx.shadowBlur = 8 * zoom;
    ctx.shadowOffsetY = 2 * zoom;
    // Full-body category color (low-saturation overlay on card background)
    ctx.fillStyle = cv.bgCard;
    roundRect(ctx, s.x, s.y, w, h, r);
    ctx.fill();
    if (nc && nc.bg) {
      ctx.fillStyle = nc.bg;
      roundRect(ctx, s.x, s.y, w, h, r);
      ctx.fill();
    }
    ctx.restore();

    // Colored top border (3px accent strip)
    if (nc && nc.accent) {
      ctx.save();
      roundRect(ctx, s.x, s.y, w, h, r);
      ctx.clip();
      ctx.fillStyle = nc.accent;
      ctx.fillRect(s.x, s.y, w, 3 * zoom);
      ctx.restore();
    }

    // Title bar background (category-tinted)
    ctx.save();
    roundRect(ctx, s.x, s.y, w, TITLE_H * zoom, r);
    ctx.clip();
    ctx.fillStyle = (nc && nc.header) ? nc.header : cv.bgSidebar;
    ctx.fillRect(s.x, s.y + 3 * zoom, w, (TITLE_H - 3) * zoom);
    ctx.restore();

    // Border (accent if selected, category color otherwise)
    ctx.strokeStyle = isSelected ? cv.accent : (nc ? nc.border : cv.border);
    ctx.lineWidth = isSelected ? 2 : 1;
    roundRect(ctx, s.x, s.y, w, h, r);
    ctx.stroke();

    // Title bar separator
    ctx.strokeStyle = cv.border;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(s.x, s.y + TITLE_H * zoom);
    ctx.lineTo(s.x + w, s.y + TITLE_H * zoom);
    ctx.stroke();

    // Title text
    ctx.fillStyle = cv.textPrimary;
    ctx.font = (12 * zoom) + 'px ' + bodyFont;
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'left';
    ctx.fillText(ellipsize(node.title, w - 20 * zoom), s.x + 10 * zoom, s.y + (TITLE_H / 2) * zoom);

    drawPorts(node);
    drawNodeContent(node);
  }

  function drawPorts(node) {
    const portR = PORT_R * zoom;
    const nodeDef = NODE_DEFINITIONS[node.type];
    const isDynamic = nodeDef && nodeDef.dynamic;

    // Input ports (left side)
    for (let i = 0; i < node.inputs.length; i++) {
      const pos = getPortPos(node, i, false);
      const s = worldToScreen(pos.x, pos.y);

      if (isDynamic) {
        // Dynamic nodes: draw vertical rectangle (multi-input indicator)
        const rectW = 6 * zoom;
        const rectH = 16 * zoom;
        ctx.fillStyle = cv.bgCard;
        ctx.fillRect(s.x - rectW/2 - 1, s.y - rectH/2 - 1, rectW + 2, rectH + 2);
        ctx.fillStyle = cv.accent;
        ctx.fillRect(s.x - rectW/2, s.y - rectH/2, rectW, rectH);
        ctx.strokeStyle = cv.border;
        ctx.lineWidth = 1;
        ctx.strokeRect(s.x - rectW/2, s.y - rectH/2, rectW, rectH);
      } else {
        // Non-dynamic nodes: draw circle (single-input only)
        ctx.fillStyle = cv.bgCard;
        ctx.beginPath();
        ctx.arc(s.x, s.y, portR + 1.5, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = cv.accent;
        ctx.beginPath();
        ctx.arc(s.x, s.y, portR, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = cv.border;
        ctx.lineWidth = 1;
        ctx.stroke();
      }
      // Label
      ctx.fillStyle = cv.textSecond;
      ctx.font = (10 * zoom) + 'px ' + bodyFont;
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'left';
      ctx.fillText(ellipsize(node.inputs[i].name, Math.max(24 * zoom, node.width * zoom - 34 * zoom)), s.x + portR + 4 * zoom, s.y);
    }

    // Output ports (right side) — use node.outputs
    for (let i = 0; i < node.outputs.length; i++) {
      const pos = getPortPos(node, i, true);
      const s = worldToScreen(pos.x, pos.y);
      ctx.fillStyle = cv.bgCard;
      ctx.beginPath();
      ctx.arc(s.x, s.y, portR + 1.5, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = cv.accent;
      ctx.beginPath();
      ctx.arc(s.x, s.y, portR, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = cv.border;
      ctx.lineWidth = 1;
      ctx.stroke();
      // Label
      ctx.fillStyle = cv.textSecond;
      ctx.font = (10 * zoom) + 'px ' + bodyFont;
      ctx.textBaseline = 'middle';
      ctx.textAlign = 'right';
      ctx.fillText(ellipsize(node.outputs[i].name, Math.max(24 * zoom, node.width * zoom - 34 * zoom)), s.x - portR - 4 * zoom, s.y);
    }
  }

  function drawNodeContent(node) {
    const s = worldToScreen(node.x, node.y);
    const w = node.width * zoom;
    const lastPortY = node.inputs.length > 0
      ? PORT_START + (node.inputs.length - 1) * PORT_GAP
      : TITLE_H;
    const cy = s.y + Math.max(TITLE_H + 6, lastPortY + 10) * zoom;

    ctx.fillStyle = cv.textMuted;
    ctx.font = (10 * zoom) + 'px ' + bodyFont;
    ctx.textBaseline = 'top';
    ctx.textAlign = 'left';
    const contentMaxWidth = Math.max(24 * zoom, w - 20 * zoom);
    const drawContentText = (value, x, y, maxWidth = contentMaxWidth) => {
      ctx.fillText(ellipsize(value, maxWidth), x, y);
    };

    switch (node.type) {
      case 'prompt':
      case 'negative-prompt': {
        const t = node.properties.prompt || '';
        drawContentText(t || '(空)', s.x + 10 * zoom, cy);
        break;
      }
      case 'image-input': {
        drawContentText(node.properties.imageData ? '已上传图片' : '(无图片)', s.x + 10 * zoom, cy);
        break;
      }
      case 'style-preset': {
        const st = STYLE_PRESETS.find(p => p.value === node.properties.style);
        drawContentText(st ? st.label : node.properties.style, s.x + 10 * zoom, cy);
        break;
      }
      case 'resolution': {
        if (node.properties.size === 'custom') {
          drawContentText((node.properties.customWidth || 1024) + ' × ' + (node.properties.customHeight || 1024), s.x + 10 * zoom, cy);
        } else {
          drawContentText(node.properties.size, s.x + 10 * zoom, cy);
        }
        break;
      }
      case 'quality': {
        drawContentText(node.properties.quality === 'hd' ? '高清' : '标准', s.x + 10 * zoom, cy);
        break;
      }
      case 'seed': {
        const seed = node.properties.seed;
        drawContentText(seed === -1 ? '随机' : String(seed), s.x + 10 * zoom, cy);
        break;
      }
      case 'canvas': {
        drawContentText(node.properties.imageData ? '已绘制' : '(空白)', s.x + 10 * zoom, cy);
        break;
      }
      case 'text-template': {
        const t = node.properties.template || '';
        const preview = t.length > 28 ? t.substring(0, 28) + '...' : (t || '(空模板)');
        const infoY = s.y + (node.height - 16) * zoom;
        drawContentText(preview, s.x + 10 * zoom, infoY);
        break;
      }
      case 'merge': {
        const infoY = s.y + (node.height - 16) * zoom;
        drawContentText('输入: ' + node.inputs.length + ' · 分隔: ' + (node.properties.separator || ', '), s.x + 10 * zoom, infoY);
        break;
      }
      case 'image-scale': {
        drawContentText('缩放: ' + (node.properties.scale || '1.0') + 'x', s.x + 10 * zoom, cy);
        break;
      }
      case 'texture-asset': {
        // Fix #5: show textureType label + normalDir if applicable
        const typeLabels = {
          'diffuse_color': 'Diffuse Color',
          'diff_to_normal': 'Diff → Normal',
          'spec_mask': 'Spec Mask',
          'roughness': 'Roughness',
          'metallic': 'Metallic',
          'ao': 'AO',
          'height': 'Height',
          'emissive': 'Emissive',
          'sss_thickness': 'SSS / Thickness',
          'white_model_diffuse': '白模 → Diffuse'
        };
        const typeLabel = typeLabels[node.properties.textureType] || 'Diffuse Color';
        drawContentText(typeLabel, s.x + 10 * zoom, cy);
        if (node.properties.textureType === 'diff_to_normal') {
          const dir = node.properties.normalDir === 'dx' ? 'DirectX' : 'OpenGL';
          drawContentText('方向: ' + dir, s.x + 10 * zoom, cy + 14 * zoom);
        }
        // Provider info + effective quality at bottom — mirrors generate node
        const prov = node.properties.provider || '(未选择)';
        const qual = effectiveQuality(node) === 'hd' ? '高清' : '标准';
        const infoY = s.y + (node.height - 20) * zoom;
        ctx.fillStyle = cv.textMuted;
        ctx.font = (9 * zoom) + 'px ' + bodyFont;
        drawContentText(prov + ' · ' + qual, s.x + 8 * zoom, infoY, Math.max(24 * zoom, w - 16 * zoom));
        break;
      }
      case 'texture-pack': {
        drawContentText('R / G / B = 粗糙度 / 金属度 / AO', s.x + 10 * zoom, cy, Math.max(24 * zoom, w - 16 * zoom));
        drawContentText(node.properties.lastOutput ? '✓ 已打包' : '等待三张灰度贴图', s.x + 10 * zoom, cy + 16 * zoom);
        break;
      }
      case 'generate': {
        // Draw provider info at bottom of node, below ports — avoids overlap
        const prov = node.properties.provider || '(未选择)';
        const qual = effectiveQuality(node) === 'hd' ? '高清' : '标准';
        const infoY = s.y + (node.height - 20) * zoom;
        ctx.fillStyle = cv.textMuted;
        ctx.font = (9 * zoom) + 'px ' + bodyFont;
        drawContentText(prov + ' · ' + qual, s.x + 8 * zoom, infoY, Math.max(24 * zoom, w - 16 * zoom));
        break;
      }
      case 'llm-call': {
        const model = node.properties.model || '(未选择)';
        const sysPreview = (node.properties.systemPrompt || '').substring(0, 20);
        drawContentText(model, s.x + 10 * zoom, cy);
        if (sysPreview) drawContentText(sysPreview, s.x + 10 * zoom, cy + 14 * zoom);
        if (nodeExecStatus[node.id] === 'running') {
          ctx.fillStyle = cv.warning;
          drawContentText('推理中...', s.x + 10 * zoom, cy + 28 * zoom);
        } else if (nodeExecStatus[node.id] === 'ok' && nodeOutputs[node.id]) {
          ctx.fillStyle = cv.success;
          const out = (nodeOutputs[node.id].text || '').substring(0, 25);
          drawContentText('✓ ' + out, s.x + 10 * zoom, cy + 28 * zoom);
        } else if (node.properties.lastOutput) {
          ctx.fillStyle = cv.textMuted;
          drawContentText('最近输出: ' + node.properties.lastOutput, s.x + 10 * zoom, cy + 28 * zoom);
        }
        break;
      }
      case 'output': {
        if (node.properties.status === 'loading') {
          ctx.fillStyle = cv.warning;
          drawContentText('执行中 · ' + formatElapsed(node.properties.elapsedMs), s.x + 10 * zoom, cy);
          ctx.fillStyle = cv.textMuted;
          drawContentText(node.properties.executionMessage || '等待上游图片', s.x + 10 * zoom, cy + 14 * zoom);
        } else if (node.properties.status === 'error') {
          ctx.fillStyle = cv.error;
          // Show short error on canvas, full error in property panel
          const fullMsg = node.properties.errorMsg || '生成失败';
          const shortMsg = fullMsg.length > 25 ? fullMsg.substring(0, 25) + '...' : fullMsg;
          drawContentText(shortMsg, s.x + 10 * zoom, cy);
          // Second line if message is long
          if (fullMsg.length > 25) {
            ctx.font = (8 * zoom) + 'px ' + bodyFont;
            ctx.fillStyle = cv.textMuted;
            drawContentText('双击查看详情', s.x + 10 * zoom, cy + 14 * zoom);
          }
        } else if (node.properties.resultUrl) {
          ctx.fillStyle = cv.success;
          drawContentText('✓ 已生成 · ' + formatElapsed(node.properties.elapsedMs), s.x + 10 * zoom, cy);
          const img = resultImages[node.id];
          if (img && img.complete && img.naturalWidth > 0) {
            const availW = w - 20 * zoom;
            const availH = (node.height - TITLE_H - 42) * zoom;
            const ratio = img.naturalWidth / img.naturalHeight;
            let iw = availW, ih = iw / ratio;
            if (ih > availH) { ih = availH; iw = ih * ratio; }
            ctx.drawImage(img, s.x + (w - iw) / 2, cy + 18 * zoom, iw, ih);
          }
        } else {
          drawContentText('等待执行', s.x + 10 * zoom, cy);
        }
        break;
      }
      case 'save-to-folder': {
        if (nodeExecStatus[node.id] === 'running') {
          ctx.fillStyle = cv.warning;
          drawContentText('执行中 · ' + formatElapsed(node.properties.elapsedMs), s.x + 10 * zoom, cy);
        } else if (node.properties.status === 'error') {
          ctx.fillStyle = cv.error;
          const fullMsg = node.properties.errorMsg || '保存失败';
          drawContentText(fullMsg.length > 25 ? fullMsg.substring(0, 25) + '...' : fullMsg, s.x + 10 * zoom, cy);
        } else if (node.properties.status === 'ok') {
          ctx.fillStyle = cv.success;
          drawContentText('✓ 已保存 · ' + formatElapsed(node.properties.elapsedMs), s.x + 10 * zoom, cy);
        }
        const infoY = s.y + (node.height - 30) * zoom;
        drawContentText('保存到: ' + (node.properties.folderPath || node.properties.selectedDirectory || '默认生图目录'), s.x + 10 * zoom, infoY, Math.max(24 * zoom, w - 16 * zoom));
        ctx.fillStyle = cv.textMuted;
        ctx.font = (9 * zoom) + 'px ' + bodyFont;
        drawContentText('输入: ' + node.inputs.length + ' 张图片', s.x + 10 * zoom, infoY + 14 * zoom, Math.max(24 * zoom, w - 16 * zoom));
        break;
      }
      case 'comment': {
        // Comment node is fully rendered in drawNode() — skip here
        break;
      }
    }
  }

  function ellipsize(value, maxWidth) {
    const text = String(value ?? '');
    if (!text || maxWidth <= 0) return '';
    if (ctx.measureText(text).width <= maxWidth) return text;
    let result = text;
    while (result.length > 1 && ctx.measureText(result + '...').width > maxWidth) {
      result = result.slice(0, -1);
    }
    return result + '...';
  }

  function drawConnections() {
    for (const conn of connections) {
      const fromNode = nodes.find(n => n.id === conn.fromNodeId);
      const toNode = nodes.find(n => n.id === conn.toNodeId);
      if (!fromNode || !toNode) continue;
      const fp = getPortPos(fromNode, conn.fromPortIdx, true);
      const tp = getPortPos(toNode, connectionPortIndex(conn), false);
      const fs = worldToScreen(fp.x, fp.y);
      const ts = worldToScreen(tp.x, tp.y);
      drawBezier(fs.x, fs.y, ts.x, ts.y, conn.id === selectedConnId);
    }
  }

  function drawBezier(x1, y1, x2, y2, highlight) {
    const dx = Math.abs(x2 - x1);
    const off = Math.max(40, dx * 0.5);
    ctx.strokeStyle = highlight ? cv.error || '#F87171' : cv.accent;
    ctx.lineWidth = highlight ? 3 : 2;
    ctx.setLineDash(highlight ? [6, 4] : []);
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.bezierCurveTo(x1 + off, y1, x2 - off, y2, x2, y2);
    ctx.stroke();
    ctx.setLineDash([]);
    // Endpoint dots
    ctx.fillStyle = cv.accent;
    ctx.beginPath(); ctx.arc(x1, y1, 3, 0, Math.PI * 2); ctx.fill();
    ctx.beginPath(); ctx.arc(x2, y2, 3, 0, Math.PI * 2); ctx.fill();
  }

  function drawConnectingLine() {
    const node = nodes.find(n => n.id === connectingFrom.nodeId);
    if (!node) return;
    const pos = getPortPos(node, connectingFrom.portIndex, connectingFrom.isOutput);
    const fs = worldToScreen(pos.x, pos.y);
    const off = Math.max(40, Math.abs(mouseSX - fs.x) * 0.5);

    ctx.strokeStyle = cv.accent;
    ctx.lineWidth = 2;
    ctx.setLineDash([6, 4]);
    ctx.beginPath();
    if (connectingFrom.isOutput) {
      ctx.moveTo(fs.x, fs.y);
      ctx.bezierCurveTo(fs.x + off, fs.y, mouseSX - off, mouseSY, mouseSX, mouseSY);
    } else {
      ctx.moveTo(fs.x, fs.y);
      ctx.bezierCurveTo(fs.x - off, fs.y, mouseSX + off, mouseSY, mouseSX, mouseSY);
    }
    ctx.stroke();
    ctx.setLineDash([]);
  }

  function roundRect(c, x, y, w, h, r) {
    if (w < 2 * r) r = w / 2;
    if (h < 2 * r) r = h / 2;
    c.beginPath();
    c.moveTo(x + r, y);
    c.arcTo(x + w, y, x + w, y + h, r);
    c.arcTo(x + w, y + h, x, y + h, r);
    c.arcTo(x, y + h, x, y, r);
    c.arcTo(x, y, x + w, y, r);
    c.closePath();
  }

  // ─── Init ───

  async function init() {
    if (_inited) return;

    canvas = $('nodes-canvas');
    canvasWrap = $('nodes-canvas-wrap');
    if (!canvas || !canvasWrap) return;
    // Only latch the guard after the DOM is reachable — an early bail must
    // stay retryable, or a transient missing element would kill this view
    // for the whole session (all toolbar buttons permanently dead).
    _inited = true;

    ctx = canvas.getContext('2d');
    refreshCssVars();
    resizeCanvas();

    // Center initial view
    panX = viewW / 2 / zoom;
    panY = 80 / zoom;

    await loadProviders();
    await loadLLMModels();
    bindEvents();

    // Start render loop
    render();
  }

  function onShow() {
    refreshCssVars();
    resizeCanvas();
  }

  // ─── Load Providers ───

  async function loadLLMModels() {
    try {
      const r = await fetch('/api/health');
      const data = await safeJson(r);
      if (data && Array.isArray(data.models)) {
        llmModels = data.models;
        llmDefaultModel = data.default_model || (llmModels.length > 0 ? llmModels[0] : '');
        // Assign default model to llm-call nodes
        for (const n of nodes) {
          if (n.type === 'llm-call' && (!n.properties.model || !llmModels.includes(n.properties.model))) {
            n.properties.model = llmDefaultModel;
          }
        }
        if (propNodeId !== null) {
          const node = nodes.find(n => n.id === propNodeId);
          if (node && node.type === 'llm-call') renderPropBody(node);
        }
      }
    } catch (e) {
      console.error('NodesApp loadLLMModels error:', e);
    }
  }

  async function loadProviders() {
    try {
      const r = await fetch('/api/image/config');
      const data = await safeJson(r);
      if (data && Number.isFinite(data.retry_count)) imageRetryCount = data.retry_count;
      if (data && Array.isArray(data.providers)) {
        providers = data.providers;
        // Assign default provider to existing generate/texture-asset nodes
        for (const n of nodes) {
          if (n.type === 'generate' || n.type === 'texture-asset') {
            if (!n.properties.provider ||
                !providers.find(p => p.name === n.properties.provider)) {
              n.properties.provider = providers.length > 0 ? providers[0].name : '';
            }
          }
        }
        // Refresh prop panel if showing a generate/texture-asset node
        if (propNodeId !== null) {
          const node = nodes.find(n => n.id === propNodeId);
          if (node && (node.type === 'generate' || node.type === 'texture-asset')) renderPropBody(node);
        }
      }
    } catch (e) {
      console.error('NodesApp loadProviders error:', e);
    }
  }

  // ─── Event Binding ───

  function bindEvents() {
    // Back button → 主页（实验室抽屉叠加弹出）
    const backBtn = $('nodes-back-agent');
    if (backBtn) backBtn.addEventListener('click', () => {
      if (window.SkylandOpenLab) window.SkylandOpenLab();
      else if (typeof Router !== 'undefined') Router.navigate('/home');
    });

    // Run button
    const runBtn = $('nodes-btn-run');
    if (runBtn) runBtn.addEventListener('click', executeWorkflow);

    // Save workflow
    const saveBtn = $('nodes-btn-save');
    if (saveBtn) saveBtn.addEventListener('click', saveWorkflow);
    // Load workflow
    const loadBtn = $('nodes-btn-load');
    if (loadBtn) loadBtn.addEventListener('click', openWorkflow);

    // Canvas mouse events
    if (canvas) {
      canvas.addEventListener('mousedown', onMouseDown);
      canvas.addEventListener('mousemove', onMouseMove);
      canvas.addEventListener('wheel', onWheel, { passive: false });
      canvas.addEventListener('dblclick', onDoubleClick);

      // Alt+right-click scissors: single contextmenu handler
      // contextmenu is the reliable event for right-click across all browsers
      canvas.addEventListener('contextmenu', e => {
        e.preventDefault();
        if (e.altKey) {
          const sp = getMousePos(e);
          const wp = screenToWorld(sp.x, sp.y);
          const conn = hitTestConnection(wp.x, wp.y);
          if (conn) {
            removeConnection(conn.id);
            selectedConnId = null;
          }
          return;
        }
        if (searchOpen) closeSearchPanel();
      });
    }

    // Window-level mouseup (handles release outside canvas)
    window.addEventListener('mouseup', onMouseUp);

    // Keyboard
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('keyup', onKeyUp);

    // Resize
    window.addEventListener('resize', () => {
      if (isNodesViewActive()) resizeCanvas();
    });

    // Search panel
    const searchInput = $('nodes-search-input');
    if (searchInput) {
      searchInput.addEventListener('input', onSearchInput);
      searchInput.addEventListener('keydown', onSearchKeyDown);
    }

    // Property edits are one undoable action per focused control.
    const propBody = $('nodes-prop-body');
    if (propBody) {
      propBody.addEventListener('focusin', () => {
        if (propNodeId !== null) beginHistory();
      });
      propBody.addEventListener('focusout', () => {
        if (historyTransaction !== null) commitHistory();
      });
    }

    // Property panel close button
    const propClose = $('nodes-prop-close');
    if (propClose) propClose.addEventListener('click', closePropPanel);

    // Sidebar toggle button (when collapsed)
    const sidebarToggle = $('nodes-sidebar-toggle');
    if (sidebarToggle) sidebarToggle.addEventListener('click', openPropPanelSidebar);

    // Observe theme changes (dark/light toggle)
    const observer = new MutationObserver(() => refreshCssVars());
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class']
    });
  }

  // ─── Sidebar Toggle (P key) ───

  function toggleSidebar() {
    const panel = $('nodes-prop-panel');
    const toggle = $('nodes-sidebar-toggle');
    if (!panel) return;
    const isCollapsed = panel.classList.contains('collapsed');
    if (isCollapsed) {
      openPropPanelSidebar();
    } else {
      closePropPanel();
    }
  }

  function openPropPanelSidebar() {
    const panel = $('nodes-prop-panel');
    const toggle = $('nodes-sidebar-toggle');
    if (panel) panel.classList.remove('collapsed');
    if (toggle) toggle.style.display = 'none';
    resizeCanvas();
  }

  // ─── Mouse Handlers ───

  function onMouseDown(e) {
    if (!canvas) return;
    // Finish a pending zoom gesture before another canvas action starts, so a
    // node drag or connection cannot accidentally merge with the zoom entry.
    flushViewHistory();
    const sp = getMousePos(e);
    const wp = screenToWorld(sp.x, sp.y);
    mouseSX = sp.x; mouseSY = sp.y;
    mouseWX = wp.x; mouseWY = wp.y;

    // Close search panel on canvas click (Bug 3: click outside closes search)
    if (searchOpen) closeSearchPanel();

    // Right-click (with or without Alt) is handled by contextmenu event — skip here
    if (e.button === 2) return;

    // Middle mouse or space+drag → pan
    if (e.button === 1 || (e.button === 0 && spaceHeld)) {
      beginHistory();
      isPanning = true;
      panStartX = sp.x; panStartY = sp.y;
      panStartPanX = panX; panStartPanY = panY;
      canvas.style.cursor = 'grabbing';
      e.preventDefault();
      return;
    }

    if (e.button !== 0) return;

    // Port hit → start connecting
    const port = hitTestPort(wp.x, wp.y);
    if (port) {
      connectingFrom = {
        nodeId: port.node.id,
        portIndex: port.portIndex,
        isOutput: port.isOutput
      };
      e.preventDefault();
      return;
    }

    // Connection line hit → select connection (for Delete key)
    const conn = hitTestConnection(wp.x, wp.y);
    if (conn) {
      selectedConnId = conn.id;
      selectedNodeId = null;
      selectedNodeIds.clear();
      return;
    }

    // Node hit → select & drag
    const node = hitTestNode(wp.x, wp.y);
    if (node) {
      // Shift+click → toggle multi-select
      if (e.shiftKey) {
        if (selectedNodeIds.has(node.id)) {
          selectedNodeIds.delete(node.id);
        } else {
          selectedNodeIds.add(node.id);
        }
        selectedNodeId = selectedNodeIds.size === 1 ? [...selectedNodeIds][0] : null;
      } else if (!selectedNodeIds.has(node.id)) {
        // Normal click — replace selection
        selectedNodeIds.clear();
        selectedNodeIds.add(node.id);
        selectedNodeId = node.id;
      }
      selectedConnId = null;
      beginHistory();
      dragNode = node;
      dragOffsetX = wp.x - node.x;
      dragOffsetY = wp.y - node.y;
      // Fix #1: Snapshot ALL selected node positions at mousedown (before any movement)
      dragStartPositions = [...selectedNodeIds].map(id => {
        const n = nodes.find(nn => nn.id === id);
        return n ? { id, x: n.x, y: n.y } : null;
      }).filter(Boolean);
    } else {
      // Empty canvas → start box selection (unless shift held to preserve)
      if (!e.shiftKey) {
        selectedNodeId = null;
        selectedNodeIds.clear();
        selectedConnId = null;
      }
      isBoxSelecting = true;
      boxStartX = sp.x; boxStartY = sp.y;
      boxEndX = sp.x; boxEndY = sp.y;
    }
  }

  function onMouseMove(e) {
    if (!canvas) return;
    const sp = getMousePos(e);
    const wp = screenToWorld(sp.x, sp.y);
    mouseSX = sp.x; mouseSY = sp.y;
    mouseWX = wp.x; mouseWY = wp.y;

    if (isPanning) {
      panX = panStartPanX + (sp.x - panStartX) / zoom;
      panY = panStartPanY + (sp.y - panStartY) / zoom;
      return;
    }

    // Box selection drag
    if (isBoxSelecting) {
      boxEndX = sp.x; boxEndY = sp.y;
      return;
    }

    if (dragNode) {
      // Fix #1: Use snapshot to compute delta, then apply to ALL selected nodes
      // Don't use dragNode.x after updating it — use the snapshot
      const newX = wp.x - dragOffsetX;
      const newY = wp.y - dragOffsetY;
      // Find the snapshot for the drag node
      const snap = dragStartPositions.find(s => s.id === dragNode.id);
      const deltaX = snap ? (newX - snap.x) : 0;
      const deltaY = snap ? (newY - snap.y) : 0;
      // Apply to ALL selected nodes using their snapshots
      for (const snp of dragStartPositions) {
        const n = nodes.find(nn => nn.id === snp.id);
        if (n) { n.x = snp.x + deltaX; n.y = snp.y + deltaY; }
      }
      return;
    }

    // Update cursor
    if (connectingFrom) {
      canvas.style.cursor = 'crosshair';
    } else {
      const port = hitTestPort(wp.x, wp.y);
      if (port) {
        canvas.style.cursor = 'pointer';
      } else {
        const node = hitTestNode(wp.x, wp.y);
        canvas.style.cursor = node ? 'move' : (spaceHeld ? 'grab' : 'default');
      }
    }
  }

  function onMouseUp(e) {
    if (isPanning) {
      isPanning = false;
      if (canvas) canvas.style.cursor = spaceHeld ? 'grab' : 'default';
      commitHistory();
      return;
    }

    // Complete box selection
    if (isBoxSelecting) {
      isBoxSelecting = false;
      // Compute selection rectangle in world coordinates
      const minX = Math.min(boxStartX, boxEndX);
      const maxX = Math.max(boxStartX, boxEndX);
      const minY = Math.min(boxStartY, boxEndY);
      const maxY = Math.max(boxStartY, boxEndY);
      // Only select if drag was meaningful (not just a click)
      if (Math.abs(boxEndX - boxStartX) > 3 || Math.abs(boxEndY - boxStartY) > 3) {
        if (!e.shiftKey) selectedNodeIds.clear();
        for (const n of nodes) {
          const s = worldToScreen(n.x, n.y);
          const s2 = worldToScreen(n.x + n.width, n.y + n.height);
          // Check if node rect intersects with box
          if (s.x < maxX && s2.x > minX && s.y < maxY && s2.y > minY) {
            selectedNodeIds.add(n.id);
          }
        }
        selectedNodeId = selectedNodeIds.size === 1 ? [...selectedNodeIds][0] : null;
      }
      return;
    }

    if (connectingFrom) {
      const sp = getMousePos(e);
      const wp = screenToWorld(sp.x, sp.y);
      const port = hitTestPort(wp.x, wp.y);

      if (port) {
        if (port.node.id === connectingFrom.nodeId) {
          showGraphNotice('不能把节点连接到自身', 'warning');
        } else if (connectingFrom.isOutput && !port.isOutput) {
          // Output → Input
          createConnection(
            connectingFrom.nodeId, connectingFrom.portIndex,
            port.node.id, port.portIndex
          );
        } else if (!connectingFrom.isOutput && port.isOutput) {
          // Input → Output (reverse direction)
          createConnection(
            port.node.id, port.portIndex,
            connectingFrom.nodeId, connectingFrom.portIndex
          );
        } else {
          showGraphNotice('连线必须从输出端口连接到输入端口', 'warning');
        }
      }

      connectingFrom = null;
      if (canvas) canvas.style.cursor = 'default';
    }

    dragNode = null;
    if (historyTransaction !== null) commitHistory();
  }

  function onWheel(e) {
    if (!canvas) return;
    e.preventDefault();
    beginHistory();
    const sp = getMousePos(e);
    const wp = screenToWorld(sp.x, sp.y);
    const delta = e.deltaY > 0 ? 0.9 : 1.1;
    const newZoom = Math.max(0.3, Math.min(2.0, zoom * delta));
    // Zoom toward cursor
    panX = sp.x / newZoom - wp.x;
    panY = sp.y / newZoom - wp.y;
    zoom = newZoom;
    if (viewHistoryTimer) clearTimeout(viewHistoryTimer);
    viewHistoryTimer = setTimeout(() => {
      viewHistoryTimer = null;
      commitHistory();
    }, 160);
  }

  function onDoubleClick(e) {
    if (!canvas) return;
    const sp = getMousePos(e);
    const wp = screenToWorld(sp.x, sp.y);
    // First check if double-clicking a connection line
    const conn = hitTestConnection(wp.x, wp.y);
    if (conn) {
      removeConnection(conn.id);
      return;
    }
    const node = hitTestNode(wp.x, wp.y);
    if (node) openPropPanel(node);
  }

  // ─── Keyboard Handlers ───

  function isNodesViewActive() {
    const v = $('view-nodes');
    return v && v.style.display !== 'none';
  }

  function isInFormField() {
    const ae = document.activeElement;
    if (!ae) return false;
    const tag = ae.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
  }

  function onKeyDown(e) {
    if (!isNodesViewActive()) return;

    if ((e.ctrlKey || e.metaKey) && !isInFormField()) {
      if (e.key.toLowerCase() === 'z') {
        e.preventDefault();
        if (e.shiftKey) redo(); else undo();
        return;
      }
      if (e.key.toLowerCase() === 'y') {
        e.preventDefault();
        redo();
        return;
      }
    }

    // Space → panning mode (only when not in form field and search closed)
    if (e.code === 'Space' && !isInFormField() && !searchOpen) {
      spaceHeld = true;
      if (canvas) canvas.classList.add('space-down');
      e.preventDefault();
      return;
    }

    // Tab → open search panel (but not when editing form fields)
    if (e.key === 'Tab' && !isInFormField()) {
      e.preventDefault();
      if (!searchOpen) openSearchPanel();
      return;
    }

    // Escape → close panel / deselect
    if (e.key === 'Escape') {
      if (searchOpen) {
        closeSearchPanel();
      } else if (propNodeId !== null) {
        closePropPanel();
      } else {
        selectedNodeId = null;
        selectedNodeIds.clear();
        selectedConnId = null;
        connectingFrom = null;
      }
      return;
    }

    // ? (Shift+/) → 快捷键/交互帮助浮层
    if ((e.key === '?' || (e.shiftKey && e.key === '/')) && !isInFormField()) {
      e.preventDefault();
      toggleHelpOverlay();
      return;
    }

    // P → toggle property sidebar
    if ((e.key === 'p' || e.key === 'P') && !isInFormField() && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      toggleSidebar();
      return;
    }

    // Fix #9: Ctrl+D → duplicate selected nodes
    if (e.ctrlKey && (e.key === 'd' || e.key === 'D') && !isInFormField()) {
      e.preventDefault();
      if (selectedNodeIds.size > 0) {
        duplicateSelected();
      }
      return;
    }

    // Fix #8: C → create comment node near selection center
    if (e.key.toLowerCase() === 'c' && !isInFormField() && !e.ctrlKey && !e.metaKey && selectedNodeIds.size > 0) {
      e.preventDefault();
      // Find bounding box of selection
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      for (const id of selectedNodeIds) {
        const n = nodes.find(nn => nn.id === id);
        if (n) {
          minX = Math.min(minX, n.x);
          minY = Math.min(minY, n.y);
          maxX = Math.max(maxX, n.x + n.width);
          maxY = Math.max(maxY, n.y + n.height);
        }
      }
      const cx = (minX + maxX) / 2;
      const commentNode = createNode('comment', cx - 120, maxY + 30);
      if (commentNode) {
        selectedNodeIds.clear();
        selectedNodeIds.add(commentNode.id);
        selectedNodeId = commentNode.id;
      }
      return;
    }

    // Delete / Backspace → delete selected node(s) or connection
    if ((e.key === 'Delete' || e.key === 'Backspace') && !isInFormField()) {
      e.preventDefault();
      if (selectedConnId !== null) {
        removeConnection(selectedConnId);
        selectedConnId = null;
        return;
      }
      if (selectedNodeIds.size > 0) {
        beginHistory();
        for (const id of [...selectedNodeIds]) {
          deleteNode(id);
        }
        commitHistory();
        selectedNodeIds.clear();
        selectedNodeId = null;
        return;
      }
      if (selectedNodeId !== null) {
        beginHistory();
        deleteNode(selectedNodeId);
        commitHistory();
        selectedNodeId = null;
        return;
      }
    }
  }

  function onKeyUp(e) {
    if (e.code === 'Space') {
      spaceHeld = false;
      if (canvas) {
        canvas.classList.remove('space-down');
        if (!isPanning) canvas.style.cursor = 'default';
      }
    }
  }

  // Fix #9: Duplicate selected nodes with +20px offset, duplicate connections between them
  function duplicateSelected() {
    const oldIds = [...selectedNodeIds];
    beginHistory();
    const idMap = {}; // oldId -> newId
    const newNodes = [];

    for (const oldId of oldIds) {
      const orig = nodes.find(n => n.id === oldId);
      if (!orig) continue;
      const newNode = {
        id: nextNodeId++,
        type: orig.type,
        x: orig.x + 20,
        y: orig.y + 20,
        width: orig.width,
        height: orig.height,
        title: orig.title,
        inputs: orig.inputs.map(p => ({ ...p })),
        outputs: orig.outputs.map(p => ({ ...p })),
        properties: JSON.parse(JSON.stringify(orig.properties))
      };
      // Clear transient execution state
      if (newNode.properties.resultUrl !== undefined) newNode.properties.resultUrl = '';
      if (newNode.properties.status !== undefined) newNode.properties.status = '';
      if (newNode.properties.errorMsg !== undefined) newNode.properties.errorMsg = '';
      if (newNode.properties.executionMessage !== undefined) newNode.properties.executionMessage = '';
      if (newNode.properties.elapsedMs !== undefined) newNode.properties.elapsedMs = 0;
      if (newNode.properties.lastOutput !== undefined) newNode.properties.lastOutput = '';
      if (newNode.properties.lastInput !== undefined) newNode.properties.lastInput = '';
      if (newNode.properties.lastContext !== undefined) newNode.properties.lastContext = '';
      if (newNode.properties.lastImage !== undefined) newNode.properties.lastImage = '';
      if (newNode.properties.lastSystemPrompt !== undefined) newNode.properties.lastSystemPrompt = '';
      if (newNode.properties.lastRunAt !== undefined) newNode.properties.lastRunAt = 0;
      idMap[oldId] = newNode.id;
      newNodes.push(newNode);
    }

    // Duplicate connections that exist between selected nodes
    const newConns = [];
    for (const conn of connections) {
      if (idMap[conn.fromNodeId] && idMap[conn.toNodeId]) {
        newConns.push({
          id: nextConnId++,
          fromNodeId: idMap[conn.fromNodeId],
          fromPortIdx: conn.fromPortIdx,
          toNodeId: idMap[conn.toNodeId],
          toPortIdx: connectionPortIndex(conn)
        });
      }
    }

    nodes.push(...newNodes);
    connections.push(...newConns);

    // Select only the new duplicates
    selectedNodeIds.clear();
    for (const newNode of newNodes) {
      selectedNodeIds.add(newNode.id);
    }
    selectedNodeId = newNodes.length === 1 ? newNodes[0].id : null;
    commitHistory();
  }

  // ─── Search Panel ───

  function openSearchPanel() {
    const panel = $('nodes-search-panel');
    const input = $('nodes-search-input');
    if (!panel) return;
    panel.style.display = 'flex';
    searchOpen = true;
    searchIdx = 0;
    if (input) {
      input.value = '';
      setTimeout(() => input.focus(), 0);
    }
    renderSearchList();
  }

  function closeSearchPanel() {
    const panel = $('nodes-search-panel');
    if (panel) panel.style.display = 'none';
    searchOpen = false;
    const input = $('nodes-search-input');
    if (input) input.blur();
  }

  function onSearchInput() {
    searchIdx = 0;
    renderSearchList();
  }

  function renderSearchList() {
    const list = $('nodes-search-list');
    const input = $('nodes-search-input');
    if (!list) return;

    const filter = (input ? input.value : '').toLowerCase().trim();
    const filtered = SEARCH_TYPES.filter(item =>
      item.name.toLowerCase().includes(filter) || item.type.includes(filter)
    );

    if (searchIdx >= filtered.length) searchIdx = 0;

    if (filtered.length === 0) {
      list.innerHTML = '<div class="nodes-search-empty">无匹配节点</div>';
      return;
    }

    list.innerHTML = filtered.map((item, i) =>
      '<div class="nodes-search-item' + (i === searchIdx ? ' selected' : '') +
      '" data-type="' + esc(item.type) + '">' +
      '<span class="nodes-search-item-name">' + esc(item.name) + '</span>' +
      '<span class="nodes-search-item-desc">' + esc(item.desc) + '</span>' +
      '</div>'
    ).join('');

    list.querySelectorAll('.nodes-search-item').forEach(el => {
      el.addEventListener('click', () => {
        createNodeAtCenter(el.dataset.type);
        closeSearchPanel();
      });
    });
  }

  function updateSearchSelection() {
    const list = $('nodes-search-list');
    if (!list) return;
    const items = list.querySelectorAll('.nodes-search-item');
    items.forEach((el, i) => {
      el.classList.toggle('selected', i === searchIdx);
    });
  }

  function onSearchKeyDown(e) {
    const list = $('nodes-search-list');
    if (!list) return;
    const items = list.querySelectorAll('.nodes-search-item');

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      searchIdx = Math.min(searchIdx + 1, items.length - 1);
      updateSearchSelection();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      searchIdx = Math.max(searchIdx - 1, 0);
      updateSearchSelection();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (items[searchIdx]) {
        createNodeAtCenter(items[searchIdx].dataset.type);
        closeSearchPanel();
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeSearchPanel();
    }
  }

  function createNodeAtCenter(type) {
    const center = screenToWorld(viewW / 2, viewH / 2);
    const offset = nodes.length * 24;
    const def = NODE_DEFINITIONS[type];
    if (!def) return;
    const node = createNode(
      type,
      center.x - def.width / 2 + offset,
      center.y - def.height / 2 + offset
    );
    if (node) {
      selectedNodeIds.clear();
      selectedNodeIds.add(node.id);
      selectedNodeId = node.id;
    }
  }

  // ─── Property Panel ───

  function openPropPanel(node) {
    const panel = $('nodes-prop-panel');
    const title = $('nodes-prop-title');
    const body = $('nodes-prop-body');
    if (!panel || !body) return;

    if (propNodeId !== node.id) {
      if (historyTransaction !== null) commitHistory();
      beginHistory();
    }
    propNodeId = node.id;
    if (title) title.textContent = node.title;
    renderPropBody(node);
    // Show sidebar (remove collapsed class, hide toggle button)
    panel.classList.remove('collapsed');
    panel.style.display = 'flex';
    const toggle = $('nodes-sidebar-toggle');
    if (toggle) toggle.style.display = 'none';
    resizeCanvas();
  }

  function closePropPanel() {
    if (!restoringHistory) commitHistory();
    const panel = $('nodes-prop-panel');
    const toggle = $('nodes-sidebar-toggle');
    if (panel) panel.classList.add('collapsed');
    // Show the toggle button so user can reopen
    if (toggle) toggle.style.display = 'flex';
    propNodeId = null;
    resizeCanvas();
  }

  function renderPropBody(node) {
    const body = $('nodes-prop-body');
    if (!body || !node) return;

    switch (node.type) {
      case 'prompt':
      case 'negative-prompt':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">提示词文本</label>' +
            '<textarea class="nodes-prop-textarea" id="np-prompt" rows="6" placeholder="输入提示词...">' +
              esc(node.properties.prompt) +
            '</textarea>' +
          '</div>';
        {
          const ta = $('np-prompt');
          if (ta) ta.addEventListener('input', () => {
            node.properties.prompt = ta.value;
          });
        }
        break;

      case 'image-input':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">上传图片</label>' +
            '<div class="nodes-prop-actions">' +
              '<button class="nodes-prop-btn" id="np-upload">选择图片</button>' +
            '</div>' +
            '<input type="file" id="np-file" accept="image/*" style="display:none">' +
            '<div class="nodes-prop-preview" id="np-preview"></div>' +
          '</div>';
        {
          const uploadBtn = $('np-upload');
          const fileInput = $('np-file');
          const preview = $('np-preview');

          if (node.properties.imageData) {
            preview.innerHTML = '<img src="' + esc(normalizeImageSource(node.properties.imageData)) + '" alt="预览">';
          } else {
            preview.innerHTML = '<div class="nodes-prop-preview-empty">未选择图片</div>';
          }

          if (uploadBtn && fileInput) {
            uploadBtn.addEventListener('click', () => fileInput.click());
            fileInput.addEventListener('change', (ev) => {
              const file = ev.target.files[0];
              if (!file) return;
              const reader = new FileReader();
              reader.onload = () => {
                const dataUrl = typeof reader.result === 'string' ? reader.result : '';
                node.properties.imageData = dataUrl;
                node.properties.imageName = file.name;
                node.properties.imageMime = file.type || '';
                if (preview) {
                  preview.innerHTML = '<img src="' + esc(dataUrl) + '" alt="预览">';
                }
              };
              reader.readAsDataURL(file);
            });
          }
        }
        break;

      case 'style-preset':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">风格</label>' +
            '<select class="nodes-prop-input" id="np-style">' +
              STYLE_PRESETS.map(s =>
                '<option value="' + esc(s.value) + '"' +
                (node.properties.style === s.value ? ' selected' : '') + '>' +
                esc(s.label) + '</option>'
              ).join('') +
            '</select>' +
          '</div>';
        {
          const sel = $('np-style');
          if (sel) sel.addEventListener('change', () => {
            node.properties.style = sel.value;
          });
        }
        break;

      // Fix #4: Resolution with 2048 + custom support
      case 'resolution':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">分辨率</label>' +
            '<select class="nodes-prop-input" id="np-size">' +
              RESOLUTIONS.map(r =>
                '<option value="' + esc(r.value) + '"' +
                (node.properties.size === r.value ? ' selected' : '') + '>' +
                esc(r.label) + '</option>'
              ).join('') +
            '</select>' +
          '</div>' +
          '<div id="np-custom-size" style="display:' + (node.properties.size === 'custom' ? 'block' : 'none') + '">' +
            '<div class="nodes-prop-field">' +
              '<label class="nodes-prop-label">自定义尺寸</label>' +
              '<div style="display:flex;gap:8px;align-items:center">' +
                '<input class="nodes-prop-input" id="np-size-w" type="number" value="' + (node.properties.customWidth || 1024) + '" style="flex:1" placeholder="宽">' +
                '<span style="color:var(--text-muted)">×</span>' +
                '<input class="nodes-prop-input" id="np-size-h" type="number" value="' + (node.properties.customHeight || 1024) + '" style="flex:1" placeholder="高">' +
              '</div>' +
            '</div>' +
          '</div>';
        {
          const sel = $('np-size');
          const customDiv = $('np-custom-size');
          if (sel) sel.addEventListener('change', () => {
            node.properties.size = sel.value;
            if (customDiv) customDiv.style.display = sel.value === 'custom' ? 'block' : 'none';
          });
          const wIn = $('np-size-w');
          if (wIn) wIn.addEventListener('input', () => {
            node.properties.customWidth = parseInt(wIn.value) || 1024;
          });
          const hIn = $('np-size-h');
          if (hIn) hIn.addEventListener('input', () => {
            node.properties.customHeight = parseInt(hIn.value) || 1024;
          });
        }
        break;

      case 'quality':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">质量</label>' +
            '<select class="nodes-prop-input" id="np-quality-val">' +
              QUALITY_OPTIONS.map(q =>
                '<option value="' + esc(q.value) + '"' +
                (node.properties.quality === q.value ? ' selected' : '') + '>' +
                esc(q.label) + '</option>'
              ).join('') +
            '</select>' +
          '</div>';
        {
          const sel = $('np-quality-val');
          if (sel) sel.addEventListener('change', () => {
            node.properties.quality = sel.value;
            refreshQualityDependents(node);
          });
        }
        break;

      case 'seed':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">随机种子</label>' +
            '<input class="nodes-prop-input" id="np-seed" type="number" value="' + node.properties.seed + '" placeholder="-1 为随机">' +
          '</div>' +
          '<div class="nodes-prop-hint">-1 = 每次随机，其他值 = 可复现</div>';
        {
          const seedIn = $('np-seed');
          if (seedIn) seedIn.addEventListener('input', () => {
            node.properties.seed = parseInt(seedIn.value) || -1;
          });
        }
        break;

      case 'merge':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">分隔符</label>' +
            '<input class="nodes-prop-input" id="np-sep" type="text" value="' + esc(node.properties.separator || ', ') + '">' +
          '</div>' +
          '<div class="nodes-prop-hint">当前 ' + node.inputs.length + ' 个输入端口。连接全部后自动扩展新端口。</div>';
        {
          const sepIn = $('np-sep');
          if (sepIn) sepIn.addEventListener('input', () => {
            node.properties.separator = sepIn.value;
          });
        }
        break;

      case 'text-template':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">模板文本</label>' +
            '<textarea class="nodes-prop-textarea" id="np-template" rows="4" placeholder="使用 {var1}, {var2} 引用输入...">' + esc(node.properties.template || '') + '</textarea>' +
          '</div>' +
          '<div class="nodes-prop-hint">用 {var1}, {var2}... 引用各输入端口的值。当前 ' + node.inputs.length + ' 个输入。</div>';
        {
          const ta = $('np-template');
          if (ta) ta.addEventListener('input', () => {
            node.properties.template = ta.value;
          });
        }
        break;

      case 'image-scale':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">缩放比例</label>' +
            '<select class="nodes-prop-input" id="np-scale">' +
              ['0.5', '0.75', '1.0', '1.5', '2.0'].map(v =>
                '<option value="' + v + '"' + (node.properties.scale === v ? ' selected' : '') + '>' + v + 'x</option>'
              ).join('') +
            '</select>' +
          '</div>';
        {
          const sel = $('np-scale');
          if (sel) sel.addEventListener('change', () => {
            node.properties.scale = sel.value;
          });
        }
        break;

      case 'canvas':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">画板</label>' +
            '<canvas id="np-draw" width="256" height="256" style="width:100%;max-width:236px;border-radius:6px;border:1px solid var(--border);background:#fff;cursor:crosshair;display:block"></canvas>' +
            '<div class="nodes-prop-actions">' +
              '<button class="nodes-prop-btn" id="np-draw-clear">清空</button>' +
              '<button class="nodes-prop-btn" id="np-draw-use">应用</button>' +
            '</div>' +
          '</div>';
        setupDrawCanvas(node);
        break;

      case 'generate':
        {
          const quality = effectiveQuality(node);
          const override = qualityOverride(node);
          const qualityFieldLabel = override ? '质量（由上游输入）' : '质量';
          body.innerHTML =
            '<div class="nodes-prop-field">' +
              '<label class="nodes-prop-label">服务商</label>' +
              '<select class="nodes-prop-input" id="np-provider">' +
                (providers.length === 0
                  ? '<option value="" disabled selected>加载中...</option>'
                  : providers.map(p =>
                      '<option value="' + esc(p.name) + '"' +
                      (node.properties.provider === p.name ? ' selected' : '') + '>' +
                      esc(p.name) + ' (' + esc(p.provider_type) + ')</option>'
                    ).join('')
                ) +
              '</select>' +
            '</div>' +
            '<div class="nodes-prop-field' + (override ? ' nodes-prop-field-linked' : '') + '">' +
              '<label class="nodes-prop-label">' + qualityFieldLabel + '</label>' +
              '<select class="nodes-prop-input" id="np-quality"' + (override ? ' disabled' : '') + '>' +
                QUALITY_OPTIONS.map(q =>
                  '<option value="' + esc(q.value) + '"' +
                  (quality === q.value ? ' selected' : '') + '>' +
                  esc(q.label) + '</option>'
                ).join('') +
              '</select>' +
              (override
                ? '<div class="nodes-prop-hint nodes-prop-linked-hint">已连接「' + esc(override.source.title) + '」节点，此处跟随上游质量。</div>'
                : '') +
            '</div>' +
            '<div class="nodes-prop-actions">' +
              '<button class="nodes-prop-btn" id="np-open-img-settings">AI生图设置</button>' +
            '</div>';
        }
        {
          const provSel = $('np-provider');
          if (provSel) provSel.addEventListener('change', () => {
            node.properties.provider = provSel.value;
          });
          const qualSel = $('np-quality');
          if (qualSel && !qualSel.disabled) qualSel.addEventListener('change', () => {
            node.properties.quality = qualSel.value;
          });
          const settingsBtn = $('np-open-img-settings');
          if (settingsBtn) settingsBtn.addEventListener('click', () => {
            // Navigate to image gen page to adjust settings — shared config
            if (typeof Router !== 'undefined') Router.navigate('/image');
            setTimeout(() => {
              if (typeof ImageApp !== 'undefined' && ImageApp.openSettings) ImageApp.openSettings();
            }, 200);
          });
        }
        break;

      // Fix #5: texture-asset node — merged game-dev nodes
      case 'texture-asset':
        {
          const quality = effectiveQuality(node);
          const override = qualityOverride(node);
          body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">服务商</label>' +
            '<select class="nodes-prop-input" id="np-ta-provider">' +
              (providers.length === 0
                ? '<option value="" disabled selected>加载中...</option>'
                : providers.map(p => '<option value="' + esc(p.name) + '"' + (node.properties.provider === p.name ? ' selected' : '') + '>' + esc(p.name) + '</option>').join('')) +
            '</select>' +
          '</div>' +
          '<div class="nodes-prop-field' + (override ? ' nodes-prop-field-linked' : '') + '">' +
            '<label class="nodes-prop-label">' + (override ? '质量（由上游输入）' : '质量') + '</label>' +
            '<select class="nodes-prop-input" id="np-ta-quality"' + (override ? ' disabled' : '') + '>' +
              QUALITY_OPTIONS.map(q => '<option value="' + esc(q.value) + '"' + (quality === q.value ? ' selected' : '') + '>' + esc(q.label) + '</option>').join('') +
            '</select>' +
            (override ? '<div class="nodes-prop-hint nodes-prop-linked-hint">已连接「' + esc(override.source.title) + '」节点，此处跟随上游质量。</div>' : '') +
          '</div>' +
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">贴图类型</label>' +
            '<select class="nodes-prop-input" id="np-ta-type">' +
              TEXTURE_TYPE_OPTIONS.map(t => '<option value="' + esc(t.value) + '"' + (node.properties.textureType === t.value ? ' selected' : '') + '>' + esc(t.label) + '</option>').join('') +
            '</select>' +
          '</div>' +
          '<div class="nodes-prop-field" id="np-ta-dir-field" style="display:' + (node.properties.textureType === 'diff_to_normal' ? 'block' : 'none') + '">' +
            '<label class="nodes-prop-label">法线方向</label>' +
            '<select class="nodes-prop-input" id="np-ta-dir">' +
              '<option value="opengl"' + (node.properties.normalDir === 'opengl' ? ' selected' : '') + '>OpenGL (Y+)</option>' +
              '<option value="dx"' + (node.properties.normalDir === 'dx' ? ' selected' : '') + '>DirectX (Y-)</option>' +
            '</select>' +
          '</div>' +
          '<label class="nodes-prop-check"><input type="checkbox" id="np-ta-seamless"' + (node.properties.seamless !== false ? ' checked' : '') + '> 无缝平铺</label>' +
          '<div class="nodes-prop-hint">输出类型覆盖 Diffuse、法线、Specular、Roughness、Metallic、AO、Height、Emissive 与 SSS/Thickness。灰度语义会写入模型提示词，法线方向按 OpenGL Y+ 或 DirectX Y- 发送。</div>';
        }
        {
          const ps = $('np-ta-provider'); if (ps) ps.addEventListener('change', () => { node.properties.provider = ps.value; });
          const qs = $('np-ta-quality'); if (qs && !qs.disabled) qs.addEventListener('change', () => { node.properties.quality = qs.value; });
          const ts = $('np-ta-type');
          const dirField = $('np-ta-dir-field');
          if (ts) ts.addEventListener('change', () => {
            node.properties.textureType = ts.value;
            if (dirField) dirField.style.display = ts.value === 'diff_to_normal' ? 'block' : 'none';
          });
          const ds = $('np-ta-dir'); if (ds) ds.addEventListener('change', () => { node.properties.normalDir = ds.value; });
          const seamless = $('np-ta-seamless'); if (seamless) seamless.addEventListener('change', () => { node.properties.seamless = seamless.checked; });
        }
        break;

      case 'texture-pack':
        body.innerHTML =
          '<div class="nodes-prop-field"><label class="nodes-prop-label">通道排列</label>' +
          '<select class="nodes-prop-input" id="np-pack-order"><option value="Roughness / Metallic / AO">R = Roughness，G = Metallic，B = AO</option><option value="AO / Roughness / Metallic">R = AO，G = Roughness，B = Metallic</option></select></div>' +
          (node.properties.lastOutput ? '<div class="nodes-prop-field"><label class="nodes-prop-label">最近输出</label><div class="nodes-prop-preview"><img src="' + esc(node.properties.lastOutput) + '" alt="ORM 打包结果"></div></div>' : '') +
          '<div class="nodes-prop-hint">将三张灰度图按像素通道合成为单张 PNG。未连接的通道使用默认值：R=255，G=0，B=255。适用于 ORM 或自定义材质通道打包。</div>';
        {
          const order = $('np-pack-order');
          if (order) {
            order.value = node.properties.channelOrder || 'Roughness / Metallic / AO';
            order.addEventListener('change', () => { node.properties.channelOrder = order.value; });
          }
        }
        break;

      case 'llm-call':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">模型</label>' +
            '<select class="nodes-prop-input" id="np-llm-model">' +
              (llmModels.length === 0
                ? '<option value="" disabled selected>加载中...</option>'
                : llmModels.map(m => '<option value="' + esc(m) + '"' + (node.properties.model === m ? ' selected' : '') + '>' + esc(m) + '</option>').join('')) +
            '</select>' +
          '</div>' +
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">系统提示词</label>' +
            '<textarea class="nodes-prop-textarea" id="np-llm-sysprompt" rows="4" placeholder="系统提示词（可选）">' + esc(node.properties.systemPrompt || '') + '</textarea>' +
          '</div>' +
          '<div class="nodes-prop-actions">' +
            '<button class="nodes-prop-btn" id="np-llm-optimize">生成优化提示词</button>' +
          '</div>' +
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">最终提示词</label>' +
            '<textarea class="nodes-prop-textarea nodes-prop-result-textarea" id="np-llm-optimized" rows="6" readonly placeholder="点击“生成优化提示词”后显示 LLM 实际生成的提示词">' + esc(node.properties.optimizedPrompt || '') + '</textarea>' +
          '</div>' +
          '<div class="nodes-prop-hint">prompt 接收提示词、模板、合并结果或上游 LLM 文本；context 接收补充资料；image 接收图片输入。执行工作流时，系统提示词作为 system 消息发送，最终提示词作为 user 消息发送。</div>';
        {
          const ms = $('np-llm-model'); if (ms) ms.addEventListener('change', () => { node.properties.model = ms.value; });
          const sp = $('np-llm-sysprompt'); if (sp) sp.addEventListener('input', () => { node.properties.systemPrompt = sp.value; });
          const optimizeBtn = $('np-llm-optimize');
          if (optimizeBtn) optimizeBtn.addEventListener('click', () => optimizeLlmPrompt(node, optimizeBtn));
        }
        break;

      case 'output':
        if (node.properties.status === 'loading') {
          body.innerHTML =
            '<div class="nodes-prop-field">' +
              '<div class="nodes-output-running">' +
                '<strong>正在执行工作流</strong>' +
                '<span>输出节点正在等待上游图片</span>' +
                '<b>' + esc(formatElapsed(node.properties.elapsedMs)) + '</b>' +
              '</div>' +
            '</div>';
        } else if (node.properties.status === 'error') {
          body.innerHTML =
            '<div class="nodes-prop-field">' +
              '<div style="color:var(--error);font-size:13px;padding:8px 0">' +
                esc(node.properties.errorMsg || '生成失败') +
              '</div>' +
            '</div>';
        } else if (node.properties.resultUrl) {
          body.innerHTML =
            '<div class="nodes-prop-field">' +
              '<label class="nodes-prop-label">生成结果</label>' +
              '<div class="nodes-prop-preview">' +
                '<img src="' + esc(node.properties.resultUrl) + '" alt="结果">' +
              '</div>' +
            '</div>';
        } else {
          body.innerHTML =
            '<div class="nodes-prop-field">' +
              '<div class="nodes-prop-preview">' +
                '<div class="nodes-prop-preview-empty">执行工作流后显示结果</div>' +
              '</div>' +
            '</div>';
        }
        break;

      // Fix #7: save-to-folder node
      case 'save-to-folder':
        if (node.properties.status === 'loading') {
          body.innerHTML =
            '<div class="nodes-prop-field">' +
              '<div class="nodes-output-running">' +
                '<strong>正在保存图片</strong>' +
                '<span>保存节点正在处理输入图片</span>' +
                '<b>' + esc(formatElapsed(node.properties.elapsedMs)) + '</b>' +
              '</div>' +
            '</div>';
          break;
        }
        if (node.properties.status === 'error') {
          body.innerHTML =
            '<div class="nodes-prop-field">' +
              '<div style="color:var(--error);font-size:13px;padding:8px 0">' +
                esc(node.properties.errorMsg || '保存失败') +
              '</div>' +
            '</div>';
          break;
        }
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">保存目录</label>' +
            '<div class="nodes-folder-picker-row"><button class="nodes-prop-btn" id="np-folder-pick" type="button">选择本地目录</button><span id="np-folder-status" class="nodes-folder-status">' + esc(node.properties.folderPath || node.properties.selectedDirectory || '默认生图目录') + '</span></div>' +
          '</div>' +
          '<div class="nodes-prop-hint">当前 ' + node.inputs.length + ' 个图片输入端口。每个端口只接收一条线，连接全部后自动扩展新端口。点击选择目录后，所选目录会成为本次运行的生图根目录。</div>' +
          (node.properties.lastSaved ?
            '<div class="nodes-prop-hint" style="color:var(--success);margin-top:8px">✓ 已保存: ' + esc(node.properties.lastSaved) + '</div>' : '');
        {
          const pick = $('np-folder-pick');
          if (pick) pick.addEventListener('click', async () => {
            const status = $('np-folder-status');
            try {
              pick.disabled = true;
              const tauri = typeof window !== 'undefined' ? window.__TAURI__ : undefined;
              const dialogOpen = tauri && tauri.dialog && typeof tauri.dialog.open === 'function'
                ? (options) => tauri.dialog.open(options)
                : tauri && tauri.core && typeof tauri.core.invoke === 'function'
                  ? (options) => tauri.core.invoke('plugin:dialog|open', { options })
                  : typeof window !== 'undefined' && window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function'
                    ? (options) => window.__TAURI_INTERNALS__.invoke('plugin:dialog|open', { options })
                  : null;
              if (!dialogOpen) {
                showGraphNotice('当前页面不是桌面版，浏览器无法获得可写入的本地目录。请使用 SKYLAND 桌面程序选择目录。', 'warning');
                return;
              }
              const selected = await dialogOpen({ directory: true, multiple: false, title: '选择图片保存目录' });
              if (!selected || Array.isArray(selected)) return;
              const result = await fetch('/api/image/path', {
                method: 'POST', headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ path: selected, allow_external: true })
              });
              const data = await result.json();
              if (!result.ok || data.status !== 'ok') throw new Error(data.message || '目录设置失败');
              node.properties.folderPath = '';
              node.properties.selectedDirectory = data.directory || selected;
              if (status) status.textContent = data.directory || selected;
              refreshPropPanel(node);
              showGraphNotice('保存目录已生效', 'success');
            } catch (error) {
              if (error && error.message) showGraphNotice(error.message, 'warning');
            } finally {
              pick.disabled = false;
            }
          });
        }
        break;

      // Fix #8: comment node
      case 'comment':
        body.innerHTML =
          '<div class="nodes-prop-field">' +
            '<label class="nodes-prop-label">注释内容</label>' +
            '<textarea class="nodes-prop-textarea" id="np-comment" rows="8" placeholder="输入注释内容..." style="font-size:14px;line-height:1.6">' +
              esc(node.properties.text || '') +
            '</textarea>' +
          '</div>' +
          '<div class="nodes-prop-hint">注释节点不会参与工作流执行，仅用于标注说明。</div>';
        {
          const ta = $('np-comment');
          if (ta) ta.addEventListener('input', () => {
            node.properties.text = ta.value;
          });
        }
        break;
    }
  }

  // ─── Drawing Canvas (for canvas node property) ───

  function setupDrawCanvas(node) {
    const dc = $('np-draw');
    if (!dc) return;
    const dctx = dc.getContext('2d');
    let drawing = false;
    let lx = 0, ly = 0;

    // White background
    dctx.fillStyle = '#FFFFFF';
    dctx.fillRect(0, 0, dc.width, dc.height);

    // Restore existing drawing
    if (node.properties.imageData) {
      const img = new Image();
      img.onload = () => dctx.drawImage(img, 0, 0, dc.width, dc.height);
      img.src = 'data:image/png;base64,' + node.properties.imageData;
    }

    function coords(e) {
      const rect = dc.getBoundingClientRect();
      return {
        x: (e.clientX - rect.left) * (dc.width / rect.width),
        y: (e.clientY - rect.top) * (dc.height / rect.height)
      };
    }

    function start(e) {
      drawing = true;
      const c = coords(e);
      lx = c.x; ly = c.y;
      dctx.beginPath();
      dctx.arc(c.x, c.y, 2, 0, Math.PI * 2);
      dctx.fillStyle = '#3370FF';
      dctx.fill();
    }

    function move(e) {
      if (!drawing) return;
      const c = coords(e);
      dctx.beginPath();
      dctx.moveTo(lx, ly);
      dctx.lineTo(c.x, c.y);
      dctx.strokeStyle = '#3370FF';
      dctx.lineWidth = 4;
      dctx.lineCap = 'round';
      dctx.lineJoin = 'round';
      dctx.stroke();
      lx = c.x; ly = c.y;
    }

    function stop() { drawing = false; }

    dc.addEventListener('mousedown', start);
    dc.addEventListener('mousemove', move);
    dc.addEventListener('mouseup', stop);
    dc.addEventListener('mouseleave', stop);
    // Touch support
    dc.addEventListener('touchstart', e => {
      e.preventDefault();
      const t = e.touches[0];
      start({ clientX: t.clientX, clientY: t.clientY });
    }, { passive: false });
    dc.addEventListener('touchmove', e => {
      e.preventDefault();
      const t = e.touches[0];
      move({ clientX: t.clientX, clientY: t.clientY });
    }, { passive: false });
    dc.addEventListener('touchend', stop);

    const clearBtn = $('np-draw-clear');
    if (clearBtn) clearBtn.addEventListener('click', () => {
      beginHistory();
      dctx.fillStyle = '#FFFFFF';
      dctx.fillRect(0, 0, dc.width, dc.height);
      node.properties.imageData = '';
      commitHistory();
    });

    const useBtn = $('np-draw-use');
    if (useBtn) useBtn.addEventListener('click', () => {
      beginHistory();
      const url = dc.toDataURL('image/png');
      const b64 = url.includes(',') ? url.split(',')[1] : url;
      node.properties.imageData = b64;
      commitHistory();
    });
  }

  // ─── Execution Engine (DAG topological sort + per-node evaluation) ───

  // Execution state per node: undefined = idle, 'running', 'ok', 'error'
  let nodeExecStatus = {};
  // Cached outputs: nodeId -> { text: string, image: string, url: string }
  let nodeOutputs = {};

  // Topological sort — returns ordered node IDs, or throws on cycle
  function topologicalSort() {
    const inDegree = {};
    const adjList = {};

    for (const n of nodes) {
      if (n.type === 'comment') continue;
      inDegree[n.id] = 0;
      adjList[n.id] = [];
    }

    for (const conn of connections) {
      if (!inDegree.hasOwnProperty(conn.fromNodeId) || !inDegree.hasOwnProperty(conn.toNodeId)) continue;
      adjList[conn.fromNodeId].push(conn.toNodeId);
      inDegree[conn.toNodeId]++;
    }

    const queue = [];
    for (const id in inDegree) {
      if (inDegree[id] === 0) queue.push(parseInt(id));
    }

    const sorted = [];
    while (queue.length > 0) {
      const id = queue.shift();
      sorted.push(id);
      if (adjList[id]) {
        for (const nextId of adjList[id]) {
          inDegree[nextId]--;
          if (inDegree[nextId] === 0) queue.push(nextId);
        }
      }
    }

    if (sorted.length !== Object.keys(inDegree).length) {
      throw new Error('工作流存在循环依赖，请检查连接');
    }
    return sorted;
  }

  function previewNodeOutput(node, visiting = new Set()) {
    if (!node) return {};
    if (nodeOutputs[node.id]) return nodeOutputs[node.id];
    if (visiting.has(node.id)) return {};
    const nextVisiting = new Set(visiting);
    nextVisiting.add(node.id);
    const incoming = connections
      .filter(connection => connection.toNodeId === node.id)
      .sort((a, b) => connectionPortIndex(a) - connectionPortIndex(b));
    const sourceForPort = portIdx => {
      const connection = incoming.find(item => connectionPortIndex(item) === portIdx);
      return connection ? nodes.find(candidate => candidate.id === connection.fromNodeId) : null;
    };
    const sourceOutput = portIdx => {
      const source = sourceForPort(portIdx);
      return source ? previewNodeOutput(source, nextVisiting) : {};
    };

    switch (node.type) {
      case 'prompt':
      case 'negative-prompt':
        return { text: node.properties.prompt || '' };
      case 'style-preset':
        return { text: STYLE_PREFIXES[node.properties.style] || '' };
      case 'image-input':
      case 'canvas':
        return { image: node.properties.imageData || '' };
      case 'resolution':
        return { text: node.properties.size === 'custom'
          ? (node.properties.customWidth || 1024) + 'x' + (node.properties.customHeight || 1024)
          : (node.properties.size || '') };
      case 'quality':
        return { text: node.properties.quality || 'standard' };
      case 'seed':
        return { text: String(node.properties.seed ?? -1) };
      case 'text-template': {
        let text = node.properties.template || '';
        for (const connection of incoming) {
          const portIdx = connectionPortIndex(connection);
          const value = previewNodeOutput(nodes.find(candidate => candidate.id === connection.fromNodeId), nextVisiting).text || '';
          text = text.replace(new RegExp('\\{var' + (portIdx + 1) + '\\}', 'g'), value);
        }
        return { text };
      }
      case 'merge':
        return { text: incoming
          .map(connection => previewNodeOutput(nodes.find(candidate => candidate.id === connection.fromNodeId), nextVisiting).text || '')
          .filter(Boolean)
          .join(node.properties.separator || ', ') };
      case 'llm-call':
        return { text: node.properties.lastOutput || '' };
      case 'generate':
      case 'texture-asset':
      case 'output':
        return node.properties.resultUrl ? { image: node.properties.resultUrl, url: node.properties.resultUrl } : {};
      case 'texture-pack':
        return node.properties.lastOutput
          ? { image: node.properties.lastOutput, url: node.properties.lastOutput }
          : {};
      case 'image-scale':
        return sourceOutput(0);
      case 'save-to-folder':
        return { text: node.properties.lastSaved || '' };
      default:
        return {};
    }
  }

  function collectNodeInputs(node) {
    const inputs = { text: '', texts: {}, image: '', images: {}, size: '', quality: '', seed: -1, negativePrompt: '' };
    for (const conn of connections) {
      if (conn.toNodeId !== node.id) continue;
      const srcNode = nodes.find(n => n.id === conn.fromNodeId);
      if (!srcNode) continue;
      const portIdx = connectionPortIndex(conn);
      const inputDef = node.inputs[portIdx];
      if (!inputDef) continue;
      const cached = nodeOutputs[conn.fromNodeId] || previewNodeOutput(srcNode);

      if (inputDef.type === 'text') {
        const val = cached && cached.text ? cached.text : '';
        if (srcNode.type === 'negative-prompt') {
          inputs.negativePrompt = val || srcNode.properties.prompt || '';
          // Negative prompt must NOT pollute the aggregate prompt text —
          // it is sent separately as body.negative_prompt.
          inputs.texts[portIdx] = val;
          continue;
        }
        // Store per-port for nodes that need to distinguish inputs (e.g. llm-call)
        inputs.texts[portIdx] = val;
        // Also merge into aggregate text for backward compatibility
        if (val) inputs.text = inputs.text ? inputs.text + ', ' + val : val;
      } else if (inputDef.type === 'image') {
        let imageValue = '';
        if (cached && (cached.image || cached.url)) {
          imageValue = cached.image || cached.url;
        } else if (srcNode.type === 'image-input' || srcNode.type === 'canvas') {
          imageValue = srcNode.properties.imageData || '';
        }
        inputs.images[portIdx] = imageValue;
        if (!inputs.image && imageValue) inputs.image = imageValue;
      } else if (inputDef.type === 'size') {
        if (srcNode.type === 'resolution') {
          if (srcNode.properties.size === 'custom') {
            inputs.size = (srcNode.properties.customWidth || 1024) + 'x' + (srcNode.properties.customHeight || 1024);
          } else {
            inputs.size = srcNode.properties.size || '';
          }
        }
      } else if (inputDef.type === 'quality') {
        if (srcNode.type === 'quality') inputs.quality = srcNode.properties.quality || 'standard';
      } else if (inputDef.type === 'seed') {
        if (srcNode.type === 'seed') inputs.seed = srcNode.properties.seed ?? -1;
      }
    }
    return inputs;
  }

  function workflowAbortError() {
    return new DOMException('工作流已取消', 'AbortError');
  }

  function llmInputPreview(node) {
    const inputs = collectNodeInputs(node);
    return {
      prompt: inputs.texts[0] || inputs.text || '',
      context: inputs.texts[1] || '',
      image: inputs.image || ''
    };
  }

  async function optimizeLlmPrompt(node, button) {
    if (!node || node.type !== 'llm-call') return;
    const input = llmInputPreview(node);
    if (!input.prompt.trim() && !input.context.trim() && !input.image) {
      showGraphNotice('请先连接或填写 prompt、context 或 image 输入', 'warning');
      return;
    }
    const originalLabel = button ? button.textContent : '';
    if (button) {
      button.disabled = true;
      button.textContent = '生成中...';
    }
    try {
      const task = [
        '请根据系统提示词和输入内容，生成一个可直接用于后续图像生成的最终提示词。',
        '只输出最终提示词正文，不要解释、不要加引号、不要输出 Markdown。',
        '保留用户的主体、动作、环境和风格意图；补全必要的画面细节，但不要凭空改变核心意图。',
        input.prompt ? '\n原始 prompt:\n' + input.prompt : '',
        input.context ? '\n补充 context:\n' + input.context : '',
        input.image ? '\n已有 image 输入：请结合图像内容进行理解。' : ''
      ].filter(Boolean).join('\n');
      const result = await fetchTextWithWorkflowAbort('/api/chat/simple', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          message: task,
          model: node.properties.model || undefined,
          system_prompt: node.properties.systemPrompt || undefined,
          image: input.image || undefined,
          temperature: node.properties.temperature
        })
      }, 120000);
      let data;
      try { data = JSON.parse(result.text); } catch (error) {
        throw new Error('LLM返回非 JSON: ' + result.text.substring(0, 200));
      }
      if (!result.response.ok) throw new Error(data.reply || 'LLM请求失败: HTTP ' + result.response.status);
      if (!data.reply || /^Error:\s*/i.test(data.reply)) throw new Error(data.reply || 'LLM返回为空');
      node.properties.optimizedPrompt = data.reply.trim();
      node.properties.lastInput = input.prompt;
      node.properties.lastContext = input.context;
      node.properties.lastImage = input.image ? '[已连接图片]' : '';
      node.properties.lastSystemPrompt = node.properties.systemPrompt || '';
      node.properties.lastRunAt = Date.now();
      refreshPropPanel(node);
      showGraphNotice('最终提示词已生成，可在属性面板查看', 'success');
    } catch (error) {
      if (error.name !== 'AbortError') showGraphNotice(error.message || '提示词优化失败', 'warning');
    } finally {
      if (button) {
        button.disabled = false;
        button.textContent = originalLabel || '生成优化提示词';
      }
    }
  }

  function normalizeImageSource(source) {
    if (!source) return '';
    const value = String(source).trim();
    if (!value) return '';
    if (value.startsWith('data:') || value.startsWith('/') || /^https?:\/\//i.test(value)) {
      return value;
    }
    return 'data:image/png;base64,' + value.replace(/\s+/g, '');
  }

  function isAbsoluteLocalPath(value) {
    const path = String(value || '').trim();
    return /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('\\\\') || path.startsWith('/');
  }

  // A saved workflow may be reopened after the app restarted. Re-apply the
  // selected directory immediately before writing so the displayed folder and
  // the server's actual image root cannot silently diverge.
  async function applySaveNodeDirectory(node) {
    const selectedDirectory = String(node?.properties?.selectedDirectory || '').trim();
    if (!selectedDirectory) return;
    const result = await fetchTextWithWorkflowAbort('/api/image/path', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        path: selectedDirectory,
        allow_external: isAbsoluteLocalPath(selectedDirectory)
      })
    }, 15000);
    let data = {};
    try { data = JSON.parse(result.text); } catch (error) {
      throw new Error('保存目录接口返回非 JSON');
    }
    if (!result.response.ok || data.status !== 'ok') {
      throw new Error(data.message || '保存节点的目录不可用，请重新选择');
    }
    node.properties.selectedDirectory = data.directory || selectedDirectory;
  }

  function scaleImageSource(source, scale) {
    const normalizedSource = normalizeImageSource(source);
    if (!normalizedSource) throw new Error('图片缩放节点没有有效的图片输入');
    if (Math.abs(scale - 1) < 0.0001) return Promise.resolve(source);

    const signal = workflowAbortController?.signal;
    if (signal?.aborted) return Promise.reject(workflowAbortError());

    return new Promise((resolve, reject) => {
      const image = new Image();
      let settled = false;

      const cleanup = () => {
        image.onload = null;
        image.onerror = null;
        if (signal) signal.removeEventListener('abort', abort);
      };
      const finish = (callback, value) => {
        if (settled) return;
        settled = true;
        cleanup();
        callback(value);
      };
      const abort = () => {
        image.src = '';
        finish(reject, workflowAbortError());
      };

      if (signal) signal.addEventListener('abort', abort, { once: true });
      // Cross-origin images need crossOrigin='anonymous' to avoid canvas
      // tainting when toDataURL is called after drawImage.
      if (/^https?:\/\//i.test(normalizedSource)) {
        const sourceUrl = new URL(normalizedSource, window.location.href);
        if (sourceUrl.origin !== window.location.origin) image.crossOrigin = 'anonymous';
      }
      image.onload = () => {
        if (signal?.aborted) {
          abort();
          return;
        }
        try {
          const width = Math.max(1, Math.round((image.naturalWidth || image.width) * scale));
          const height = Math.max(1, Math.round((image.naturalHeight || image.height) * scale));
          const output = document.createElement('canvas');
          output.width = width;
          output.height = height;
          const outputContext = output.getContext('2d');
          if (!outputContext) throw new Error('浏览器不支持图片缩放画布');
          outputContext.drawImage(image, 0, 0, width, height);
          finish(resolve, output.toDataURL('image/png'));
        } catch (error) {
          finish(reject, new Error('图片缩放失败: ' + (error.message || '无法读取图片')));
        }
      };
      image.onerror = () => finish(reject, new Error('图片缩放失败: 无法读取输入图片'));
      image.src = normalizedSource;
    });
  }

  function loadImageForTexture(source) {
    const normalized = normalizeImageSource(source);
    if (!normalized) return Promise.resolve(null);
    const signal = workflowAbortController?.signal;
    return new Promise((resolve, reject) => {
      const image = new Image();
      let settled = false;
      const cleanup = () => {
        image.onload = null;
        image.onerror = null;
        if (signal) signal.removeEventListener('abort', abort);
      };
      const finish = (fn, value) => {
        if (settled) return;
        settled = true;
        cleanup();
        fn(value);
      };
      const abort = () => finish(reject, workflowAbortError());
      if (signal) signal.addEventListener('abort', abort, { once: true });
      image.onload = () => finish(resolve, image);
      image.onerror = () => finish(reject, new Error('无法读取 ORM 灰度贴图'));
      // Same-origin generated files do not need CORS. Setting crossOrigin on
      // them can turn an otherwise valid image load into a CORS failure.
      if (/^https?:\/\//i.test(normalized)) {
        const sourceUrl = new URL(normalized, window.location.href);
        if (sourceUrl.origin !== window.location.origin) image.crossOrigin = 'anonymous';
      }
      image.src = normalized;
    });
  }

  async function packTextureChannels(sources, order) {
    const loaded = await Promise.all(sources.map(loadImageForTexture));
    const first = loaded.find(Boolean);
    if (!first) throw new Error('ORM 打包节点没有有效图片');
    const width = first.naturalWidth || first.width;
    const height = first.naturalHeight || first.height;
    for (let i = 0; i < loaded.length; i++) {
      const img = loaded[i];
      if (!img) continue;
      const w = img.naturalWidth || img.width;
      const h = img.naturalHeight || img.height;
      if (w !== width || h !== height) {
        console.warn('[ORM 打包] 输入贴图 #' + (i + 1) + ' 尺寸为 ' + w + 'x' + h +
          '，与基准尺寸 ' + width + 'x' + height + ' 不一致，已自动缩放');
      }
    }
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const out = canvas.getContext('2d', { willReadFrequently: true });
    if (!out) throw new Error('当前环境不支持通道打包');
    const channelData = [];
    for (const image of loaded) {
      if (!image) {
        channelData.push(null);
        continue;
      }
      const temp = document.createElement('canvas');
      temp.width = width;
      temp.height = height;
      const tempCtx = temp.getContext('2d', { willReadFrequently: true });
      if (!tempCtx) throw new Error('当前环境不支持读取 ORM 输入贴图');
      try {
        tempCtx.drawImage(image, 0, 0, width, height);
        channelData.push(tempCtx.getImageData(0, 0, width, height).data);
      } catch (error) {
        throw new Error('ORM 无法读取输入贴图像素: ' + (error.message || '图片来源受限'));
      }
    }
    const rgba = out.createImageData(width, height);
    const channelOrder = order === 'AO / Roughness / Metallic' ? [2, 0, 1] : [0, 1, 2];
    for (let i = 0; i < rgba.data.length; i += 4) {
      const channelValue = (data, fallback) => data
        ? Math.round(data[i] * 0.2126 + data[i + 1] * 0.7152 + data[i + 2] * 0.0722)
        : fallback;
      rgba.data[i] = channelValue(channelData[channelOrder[0]], 255);
      rgba.data[i + 1] = channelValue(channelData[channelOrder[1]], 0);
      rgba.data[i + 2] = channelValue(channelData[channelOrder[2]], 255);
      rgba.data[i + 3] = 255;
    }
    out.putImageData(rgba, 0, 0);
    return canvas.toDataURL('image/png');
  }

  async function evaluateNode(node) {
    const inputs = collectNodeInputs(node);

    switch (node.type) {
      case 'prompt':
        return { text: node.properties.prompt || '' };
      case 'negative-prompt':
        return { text: node.properties.prompt || '' };
      case 'style-preset':
        return { text: STYLE_PREFIXES[node.properties.style] || '' };
      case 'image-input':
      case 'canvas':
        return { image: node.properties.imageData || '' };
      case 'resolution':
        return { text: node.properties.size === 'custom'
          ? (node.properties.customWidth || 1024) + 'x' + (node.properties.customHeight || 1024)
          : (node.properties.size || '') };
      case 'quality':
        return { text: node.properties.quality || 'standard' };
      case 'seed':
        return { text: String(node.properties.seed ?? -1) };
      case 'image-scale': {
        const scale = Number.parseFloat(node.properties.scale || '1.0');
        if (!Number.isFinite(scale) || scale <= 0) {
          throw new Error('图片缩放比例必须是大于 0 的数字');
        }
        return { image: await scaleImageSource(inputs.image, scale) };
      }

      case 'texture-pack': {
        const sources = [inputs.images[0], inputs.images[1], inputs.images[2]];
        if (!sources.some(Boolean)) throw new Error('ORM 打包节点至少需要一张灰度贴图');
        const packed = await packTextureChannels(sources, node.properties.channelOrder);
        node.properties.lastOutput = packed;
        return { image: packed };
      }

      case 'text-template': {
        let tmpl = node.properties.template || '';
        for (const conn of connections) {
          if (conn.toNodeId !== node.id) continue;
          const tsrc = nodes.find(n => n.id === conn.fromNodeId);
          if (!tsrc) continue;
          const varName = 'var' + (connectionPortIndex(conn) + 1);
          const cached = nodeOutputs[conn.fromNodeId];
          let varValue = '';
          if (cached && cached.text) varValue = cached.text;
          else if (tsrc.type === 'prompt') varValue = tsrc.properties.prompt || '';
          else if (tsrc.type === 'style-preset') varValue = STYLE_PREFIXES[tsrc.properties.style] || '';
          else if (tsrc.type === 'negative-prompt') varValue = tsrc.properties.prompt || '';
          tmpl = tmpl.replace(new RegExp('\\{' + varName + '\\}', 'g'), varValue);
        }
        return { text: tmpl };
      }

      case 'merge': {
        const sep = node.properties.separator || ', ';
        let merged = '';
        const inputConnections = connections
          .filter(conn => conn.toNodeId === node.id)
          .sort((a, b) => connectionPortIndex(a) - connectionPortIndex(b));
        for (const conn of inputConnections) {
          const cached = nodeOutputs[conn.fromNodeId];
          if (cached && cached.text) {
            merged = merged ? merged + sep + cached.text : cached.text;
          }
        }
        return { text: merged };
      }

      case 'llm-call': {
        // Use per-port text: port 0 = prompt, port 1 = context
        const rawPrompt = inputs.texts[0] || inputs.text || '';
        if (!rawPrompt.trim()) throw new Error('LLM 推理节点缺少 prompt 输入');
        const context = inputs.texts[1] || '';
        const systemPrompt = node.properties.systemPrompt || '';
        const optimizedIsCurrent = Boolean(
          node.properties.optimizedPrompt &&
          node.properties.lastInput === rawPrompt &&
          node.properties.lastContext === context &&
          node.properties.lastImage === (inputs.image ? '[已连接图片]' : '') &&
          node.properties.lastSystemPrompt === systemPrompt
        );
        const prompt = optimizedIsCurrent ? node.properties.optimizedPrompt : rawPrompt;
        const message = context ? prompt + '\n\n上下文:\n' + context : prompt;
        const body = {
          message: message,
          model: node.properties.model || undefined,
          system_prompt: systemPrompt || undefined,
        };
        if (inputs.image) body.image = inputs.image;
        const result = await fetchTextWithWorkflowAbort('/api/chat/simple', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body)
        }, 120000);
        if (!result.response.ok) throw new Error('LLM请求失败: HTTP ' + result.response.status);
        let data;
        try {
          data = JSON.parse(result.text);
        } catch (error) {
          throw new Error('LLM返回非 JSON: ' + result.text.substring(0, 200));
        }
        if (!data.reply) throw new Error('LLM返回为空');
        if (/^Error:\s*/i.test(data.reply)) throw new Error(data.reply);
        node.properties.lastInput = rawPrompt;
        node.properties.lastContext = context;
        node.properties.lastImage = inputs.image ? '[已连接图片]' : '';
        node.properties.lastSystemPrompt = systemPrompt;
        node.properties.lastOutput = data.reply.trim();
        node.properties.lastRunAt = Date.now();
        return { text: data.reply || '' };
      }

      case 'generate':
      case 'texture-asset': {
        let finalPrompt = inputs.text || '';
        let mode = null;
        if (node.type === 'texture-asset') {
          const tt = node.properties.textureType;
          const textureSpecs = {
            diffuse_color: {
              instruction: 'Generate ONLY a diffuse/albedo color map. Fill the entire image with the material surface color. No lighting, no shadows, no ambient occlusion — pure surface albedo.',
              isGrayscale: false,
              format: 'Full-color RGB texture'
            },
            diff_to_normal: {
              instruction: 'Generate ONLY a tangent-space normal map. Use the standard purple-blue color scheme (R channel = X tangent, G channel = Y tangent, B channel = up). NO diffuse colors, NO lighting — this must be a pure normal map.',
              isGrayscale: false,
              format: 'Normal map (purple-blue tones)'
            },
            spec_mask: {
              instruction: 'Generate ONLY a grayscale specular reflectance mask. White areas = high reflectivity, black areas = no reflectivity. No color information — pure grayscale.',
              isGrayscale: true,
              format: 'Grayscale mask'
            },
            roughness: {
              instruction: 'Generate ONLY a grayscale roughness map. White areas = rough/matte, black areas = smooth/glossy. No color, no lighting — pure grayscale roughness data.',
              isGrayscale: true,
              format: 'Grayscale roughness map'
            },
            smoothness: {
              instruction: 'Generate ONLY a grayscale smoothness map. White areas = smooth/glossy, black areas = rough/matte. No color, no lighting — pure grayscale smoothness data (inverse of roughness).',
              isGrayscale: true,
              format: 'Grayscale smoothness map'
            },
            metallic: {
              instruction: 'Generate ONLY a grayscale metallic map. White areas = metallic, black areas = dielectric/non-metal. No color, no lighting — pure grayscale metallic data.',
              isGrayscale: true,
              format: 'Grayscale metallic map'
            },
            ao: {
              instruction: 'Generate ONLY a grayscale ambient occlusion map. White areas = unoccluded, black areas = fully occluded. No color, no direct lighting — pure AO data.',
              isGrayscale: true,
              format: 'Grayscale AO map'
            },
            opacity: {
              instruction: 'Generate ONLY a grayscale opacity/cutout mask. White areas = fully visible, black areas = transparent. Binary or smooth gradient — pure opacity data.',
              isGrayscale: true,
              format: 'Grayscale opacity mask'
            },
            curvature: {
              instruction: 'Generate ONLY a grayscale curvature mask. Bright = convex edges, dark = concave recesses. No color — pure curvature data.',
              isGrayscale: true,
              format: 'Grayscale curvature mask'
            },
            cavity: {
              instruction: 'Generate ONLY a grayscale micro-cavity mask. White = open surface, black = narrow recessed detail. No color — pure cavity data.',
              isGrayscale: true,
              format: 'Grayscale cavity mask'
            },
            height: {
              instruction: 'Generate ONLY a grayscale height/displacement map. White areas = raised/elevated, black areas = recessed/sunken. No color, no lighting — pure displacement data.',
              isGrayscale: true,
              format: 'Grayscale height map'
            },
            emissive: {
              instruction: 'Generate ONLY an emissive intensity map. Bright areas emit light, dark areas do not emit. Can use color for emission color or pure grayscale for intensity.',
              isGrayscale: false,
              format: 'Emissive map'
            },
            sss_thickness: {
              instruction: 'Generate ONLY a grayscale subsurface scattering thickness map. White areas = thicker material, black areas = thin/translucent. No color — pure thickness data.',
              isGrayscale: true,
              format: 'Grayscale SSS thickness map'
            },
            white_model_diffuse: {
              instruction: 'Generate a diffuse/albedo color texture for the supplied untextured white 3D model. Apply physically plausible material colors based on the model geometry. Fill the entire UV space.',
              isGrayscale: false,
              format: 'Full-color RGB texture'
            }
          };
          const spec = textureSpecs[tt] || textureSpecs.diffuse_color;
          // Build a strongly constrained prompt that forces single-texture output
          const negativeConstraints = 'Do NOT generate multiple texture types in one image. Do NOT create a texture atlas, sheet, or grid. Do NOT include labels, annotations, or borders. ';
          const physicalConstraints = 'Orthographic top-down texture view. No perspective scene, no isolated 3D object, no floor, no background, no text, no watermark, no logo, no UI elements. ';
          const finalInstruction = spec.instruction +
            (spec.isGrayscale ? ' The output must be pure grayscale — no color hue. ' : '') +
            (node.properties.seamless !== false ? ' Seamless tileable UV texture with matching edges. ' : '') +
            (tt === 'diff_to_normal' ? ' ' + (node.properties.normalDir === 'dx' ? 'Use DirectX Y- tangent convention.' : 'Use OpenGL Y+ tangent convention.') + ' ' : '');
          finalPrompt = finalInstruction + negativeConstraints + physicalConstraints +
            'Output format: ' + spec.format + '. ' +
            (finalPrompt ? 'Material description: ' + finalPrompt : '');
          if (tt === 'diffuse_color') {
            mode = 'seamless';
          } else if (tt === 'diff_to_normal') {
            mode = 'normal-map';
          } else if (tt === 'spec_mask') {
            mode = 'specular-mask';
          } else if (tt === 'white_model_diffuse') {
            mode = 'white-model';
          }
        }
        const body = {
          provider: node.properties.provider || '',
          prompt: finalPrompt,
          size: inputs.size || '1024x1024',
          quality: effectiveQuality(node)
        };
        if (mode) body.mode = mode;
        if (inputs.image) body.reference_image = inputs.image;
        if (inputs.negativePrompt) body.negative_prompt = inputs.negativePrompt;
        if (inputs.seed !== undefined && inputs.seed !== -1) body.seed = inputs.seed;

        if (!body.provider) throw new Error('生图节点未选择服务商');
        if (!finalPrompt.trim() && !inputs.image) throw new Error('生图节点缺少提示词或参考图片');

        // 重试已统一移至后端 /api/image/generate（[image_settings].retry_count，默认 5 次）
        // — 前端只保留中止感知与超时扩展，避免双重重试叠加（曾为前端 3 次 × 后端 5 次）
        if (workflowAbortController?.signal.aborted) {
          throw new DOMException('工作流已取消', 'AbortError');
        }
        const result = await fetchTextWithWorkflowAbort('/api/image/generate', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body)
        }, (1 + (Number.isFinite(imageRetryCount) ? imageRetryCount : 5)) * 120000 + 30000);
        const r = result.response;
        const respText = result.text;
        let data = null;
        try { data = JSON.parse(respText); } catch(e) {
          if (respText.startsWith('<!') || respText.startsWith('<html')) {
            throw new Error('API返回HTML页面，请检查Base URL配置（需包含/v1）');
          }
          throw new Error('服务器返回非JSON: ' + respText.substring(0, 300));
        }
        if (!r.ok) {
          const errMsg = (data && data.message) ? data.message :
            (data && data.error && data.error.message) ? data.error.message :
            (data && data.error) ? String(data.error) : respText.substring(0, 500);
          const err = new Error(errMsg);
          err.statusCode = r.status;
          throw err;
        }
        const isSuccess = data && (data.status === 'ok' || data.url || data.filename);
        if (!isSuccess) throw new Error((data && data.message) ? data.message : '生成失败');
        const imgUrl = data.url || ('/api/image/file/' + (data.filename || ''));
        return { url: imgUrl, image: imgUrl, text: imgUrl };
      }

      case 'output': {
        if (!inputs.image) throw new Error('输出节点没有有效的图片输入');
        const outputImage = normalizeImageSource(inputs.image);
        const loadedImage = await loadImageForTexture(outputImage);
        if (!loadedImage) throw new Error('输出节点无法读取图片结果');
        return { url: outputImage, image: outputImage };
      }
      case 'save-to-folder': {
        const imageInputs = Object.keys(inputs.images)
          .map(Number)
          .sort((a, b) => a - b)
          .map(index => inputs.images[index])
          .filter(Boolean);
        if (imageInputs.length === 0) {
          throw new Error('保存到文件夹节点没有有效的图片输入');
        }
        await applySaveNodeDirectory(node);
        const folder = (node.properties.folderPath || '').trim();
        const saved = [];
        for (const source of imageInputs) {
          if (workflowAbortController?.signal.aborted) {
            throw new DOMException('工作流已取消', 'AbortError');
          }
          const sourceForSave = source.startsWith('data:') || source.startsWith('/api/image/file/')
            ? source
            : 'data:image/png;base64,' + source;
          const result = await fetchTextWithWorkflowAbort('/api/image/save', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ source: sourceForSave, folder })
          }, 30000);
          let data;
          try {
            data = JSON.parse(result.text);
          } catch (error) {
            throw new Error('图片保存接口返回非 JSON');
          }
          if (!result.response.ok || data.status !== 'ok') {
            throw new Error(data.message || '图片保存失败');
          }
          saved.push(data.filename || source.split('/').pop() || 'image');
        }
        node.properties.lastSaved = saved.join(', ');
        node.properties.lastFolder = folder || node.properties.selectedDirectory || '当前生图目录';
        return { text: '已保存 ' + saved.length + ' 张图片', saved };
      }
      case 'comment':
        return {};
      default:
        return {};
    }
  }

  async function executeWorkflow() {
    if (isExecuting) {
      workflowCancelRequested = true;
      if (workflowAbortController) workflowAbortController.abort();
      return;
    }
    nodeExecStatus = {};
    const nodeExecErrors = {};
    nodeOutputs = {};
    for (const n of nodes) {
      nodeExecStatus[n.id] = 'idle';
      if (n.type === 'output') {
        n.properties.status = '';
        n.properties.resultUrl = '';
        n.properties.errorMsg = '';
        n.properties.executionMessage = '';
        n.properties.elapsedMs = 0;
        delete resultImages[n.id];
      } else if (n.type === 'save-to-folder') {
        n.properties.status = '';
        n.properties.errorMsg = '';
        n.properties.executionMessage = '';
        n.properties.elapsedMs = 0;
        n.properties.lastSaved = '';
      }
    }
    // Terminal nodes (output + save-to-folder) — every one gets execution status.
    const outputNodes = nodes.filter(n => TERMINAL_TYPES.includes(n.type));
    if (outputNodes.length === 0) { alert('请添加「输出」或「保存到文件夹」节点到画布'); return; }

    for (const outputNode of outputNodes) {
      outputNode.properties.status = 'loading';
      outputNode.properties.executionMessage = '正在准备工作流';
      outputNode.properties.elapsedMs = 0;
      refreshPropPanel(outputNode);
    }

    isExecuting = true;
    workflowCancelRequested = false;
    workflowAbortController = new AbortController();
    startOutputElapsed();
    const runBtn = $('nodes-btn-run');
    if (runBtn) {
      runBtn.classList.add('loading');
      runBtn.disabled = false;
      runBtn.innerHTML = '<span class="nodes-btn-spinner"></span> 取消执行';
      runBtn.title = '取消当前工作流';
    }

    try {
      const sortedIds = topologicalSort();
      const markOutputError = (node, message) => {
        if (!TERMINAL_TYPES.includes(node.type)) return;
        node.properties.status = 'error';
        node.properties.errorMsg = message;
        node.properties.resultUrl = '';
        node.properties.elapsedMs = Math.max(0, performance.now() - workflowStartedAt);
        node.properties.executionMessage = message;
        delete resultImages[node.id];
        refreshPropPanel(node);
      };
      for (const nodeId of sortedIds) {
        if (workflowAbortController.signal.aborted) {
          throw new DOMException('工作流已取消', 'AbortError');
        }
        const node = nodes.find(n => n.id === nodeId);
        if (!node || node.type === 'comment') continue;

        // A failed node only invalidates its descendants. Independent graph
        // branches must still execute so one bad provider or prompt does not
        // hide valid results from other output nodes.
        const failedUpstream = connections
          .filter(conn => conn.toNodeId === node.id)
          .map(conn => nodes.find(source => source.id === conn.fromNodeId))
          .find(source => source && nodeExecStatus[source.id] === 'error');
        if (failedUpstream) {
          const upstreamDetail = nodeExecErrors[failedUpstream.id] || '执行失败';
          const reason = '上游节点「' + failedUpstream.title + '」失败: ' + upstreamDetail;
          nodeExecStatus[node.id] = 'error';
          nodeExecErrors[node.id] = reason;
          markOutputError(node, reason);
          continue;
        }

        nodeExecStatus[node.id] = 'running';
        if (node.type === 'generate' || node.type === 'texture-asset') {
          for (const outputNode of outputNodes) {
            if (outputNode.properties.status === 'loading') {
              outputNode.properties.executionMessage = '正在生成图片';
              refreshPropPanel(outputNode);
            }
          }
        } else if (node.type === 'output') {
          node.properties.executionMessage = '正在处理输出图片';
          refreshPropPanel(node);
        } else if (node.type === 'save-to-folder') {
          node.properties.executionMessage = '正在保存图片';
          refreshPropPanel(node);
        }
        try {
          const output = await evaluateNode(node);
          nodeOutputs[node.id] = output;
          if (node.type === 'output') {
            const result = nodeOutputs[node.id];
            if (result && result.url) {
              node.properties.status = 'ok';
              node.properties.elapsedMs = Math.max(0, performance.now() - workflowStartedAt);
              node.properties.executionMessage = '输出完成';
              node.properties.resultUrl = result.url;
              node.properties.errorMsg = '';
              const img = new Image();
              img.src = result.url;
              resultImages[node.id] = img;
            } else if (result && result.text && !result.url) {
              node.properties.status = 'ok';
              node.properties.elapsedMs = Math.max(0, performance.now() - workflowStartedAt);
              node.properties.executionMessage = '输出完成';
              node.properties.resultUrl = '';
              node.properties.errorMsg = result.text.substring(0, 100);
            }
            refreshPropPanel(node);
          } else if (node.type === 'save-to-folder') {
            node.properties.status = 'ok';
            node.properties.elapsedMs = Math.max(0, performance.now() - workflowStartedAt);
            node.properties.executionMessage = '保存完成';
            node.properties.errorMsg = '';
            refreshPropPanel(node);
          }
          nodeExecStatus[node.id] = 'ok';
        } catch (e) {
          nodeExecStatus[node.id] = 'error';
          if (e.name === 'AbortError' || workflowCancelRequested) throw e;
          const reason = e.name === 'TimeoutError'
            ? '节点「' + node.title + '」请求超时'
            : '节点「' + node.title + '」执行失败: ' + (e.message || '执行失败');
          nodeExecErrors[node.id] = e.message || reason;
          markOutputError(node, reason);
          // Continue through the topological order. Descendants will be
          // marked as skipped by the failedUpstream check above.
          continue;
        }
      }
    } catch (e) {
      for (const n of outputNodes) {
        if (n.properties.status === 'ok') continue;
        n.properties.elapsedMs = Math.max(0, performance.now() - workflowStartedAt);
        n.properties.executionMessage = e.name === 'AbortError' || workflowCancelRequested
          ? '工作流已取消'
          : '工作流执行失败';
        n.properties.status = 'error';
        n.properties.errorMsg = e.name === 'AbortError' || workflowCancelRequested
          ? '工作流已取消'
          : (e.message || '工作流执行失败');
        refreshPropPanel(n);
      }
    } finally {
      // A topological or unexpected failure can bypass the per-node handler.
      // Never leave an output looking as if it is still running after cleanup.
      for (const n of outputNodes) {
        if (n.properties.status !== 'loading') continue;
        n.properties.status = 'error';
        n.properties.errorMsg = '工作流未完成';
        n.properties.executionMessage = '工作流未完成';
        n.properties.elapsedMs = Math.max(0, performance.now() - workflowStartedAt);
        n.properties.resultUrl = '';
        delete resultImages[n.id];
        refreshPropPanel(n);
      }
      stopOutputElapsed();
      isExecuting = false;
      if (workflowAbortController) workflowAbortController = null;
      if (runBtn) {
        runBtn.classList.remove('loading');
        runBtn.disabled = false;
        runBtn.textContent = '执行工作流';
        runBtn.title = '';
      }
    }
  }

  function refreshPropPanel(node) {
    if (propNodeId === node.id) renderPropBody(node);
  }


  // ─── Workflow Save/Load via File Picker (Fix #11) ───

  function serializeWorkflow() {
    const data = {
      nodes: nodes.map(n => {
        const copy = {
          id: n.id,
          type: n.type,
          x: n.x,
          y: n.y,
          width: n.width,
          height: n.height,
          title: n.title,
          inputs: n.inputs.map(p => ({ ...p })),
          outputs: n.outputs.map(p => ({ ...p })),
          properties: JSON.parse(JSON.stringify(n.properties))
        };
        // Strip large base64 image data — it will be empty on reload
        if (copy.properties.imageData) copy.properties.imageData = '';
        // Strip transient execution state
        if (copy.properties.resultUrl !== undefined) copy.properties.resultUrl = '';
        if (copy.properties.status !== undefined) copy.properties.status = '';
        if (copy.properties.errorMsg !== undefined) copy.properties.errorMsg = '';
        if (copy.properties.executionMessage !== undefined) copy.properties.executionMessage = '';
        if (copy.properties.elapsedMs !== undefined) copy.properties.elapsedMs = 0;
        if (copy.properties.lastSaved !== undefined) copy.properties.lastSaved = '';
        if (copy.properties.lastFolder !== undefined) copy.properties.lastFolder = '';
        if (copy.properties.lastOutput !== undefined) copy.properties.lastOutput = '';
        if (copy.properties.lastInput !== undefined) copy.properties.lastInput = '';
        if (copy.properties.lastContext !== undefined) copy.properties.lastContext = '';
        if (copy.properties.lastImage !== undefined) copy.properties.lastImage = '';
        if (copy.properties.lastSystemPrompt !== undefined) copy.properties.lastSystemPrompt = '';
        if (copy.properties.lastRunAt !== undefined) copy.properties.lastRunAt = 0;
        return copy;
      }),
      connections: connections.map(c => ({
        id: c.id,
        fromNodeId: c.fromNodeId,
        fromPortIdx: c.fromPortIdx,
        toNodeId: c.toNodeId,
        toPortIdx: connectionPortIndex(c)
      })),
      nextNodeId,
      nextConnId,
      panX, panY, zoom
    };

    return JSON.stringify(data, null, 2);
  }

  function getTauriApi() {
    return typeof window !== 'undefined' ? window.__TAURI__ : undefined;
  }

  function invokeTauri(command, args) {
    const tauri = getTauriApi();
    if (tauri && tauri.core && typeof tauri.core.invoke === 'function') {
      return tauri.core.invoke(command, args);
    }
    if (typeof window !== 'undefined' && window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
      return window.__TAURI_INTERNALS__.invoke(command, args);
    }
    return null;
  }

  async function saveWorkflow() {
    const content = serializeWorkflow();
    const tauri = getTauriApi();
    const saveDialog = tauri && tauri.dialog && typeof tauri.dialog.save === 'function'
      ? tauri.dialog.save.bind(tauri.dialog)
      : null;
    const now = new Date();
    const ts = now.getFullYear() + '' + String(now.getMonth() + 1).padStart(2, '0') + String(now.getDate()).padStart(2, '0') + '_' + String(now.getHours()).padStart(2, '0') + String(now.getMinutes()).padStart(2, '0');
    const defaultName = 'workflow_' + ts + '.json';

    if (saveDialog) {
      try {
        const selectedPath = await saveDialog({
          defaultPath: defaultName,
          title: '保存节点流工程',
          filters: [{ name: 'JSON 工作流', extensions: ['json'] }]
        });
        if (!selectedPath) return;
        const invokeResult = invokeTauri('save_workflow_file', { path: selectedPath, content });
        if (!invokeResult) throw new Error('桌面文件写入接口不可用');
        await invokeResult;
        showGraphNotice('工作流已保存', 'success');
        return;
      } catch (error) {
        showGraphNotice('工作流保存失败: ' + (error.message || error), 'warning');
        return;
      }
    }

    // Browser fallback: standard download behavior.
    const blob = new Blob([content], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = defaultName;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }

  function openWorkflow() {
    // Create hidden file input for loading
    const fileInput = document.createElement('input');
    fileInput.type = 'file';
    fileInput.accept = '.json,application/json';
    fileInput.style.display = 'none';
    document.body.appendChild(fileInput);

    fileInput.addEventListener('change', (e) => {
      const file = e.target.files[0];
      if (!file) {
        document.body.removeChild(fileInput);
        return;
      }
      const reader = new FileReader();
      reader.onload = () => {
        try {
          const data = JSON.parse(reader.result);
          if (data && Array.isArray(data.nodes)) {
            // Loading a workflow is a graph action too, so it can be undone.
            // Finish any pending property edit before taking the pre-load
            // snapshot and keep the existing undo history intact.
            if (historyTransaction !== null) commitHistory();
            closePropPanel();
            beginHistory();
            nodes = data.nodes;
            connections = Array.isArray(data.connections) ? data.connections : [];
            nextNodeId = Number.isInteger(Number(data.nextNodeId)) ? Number(data.nextNodeId) : 1;
            nextConnId = Number.isInteger(Number(data.nextConnId)) ? Number(data.nextConnId) : 1;
            panX = Number.isFinite(Number(data.panX)) ? Number(data.panX) : viewW / 2 / zoom;
            panY = Number.isFinite(Number(data.panY)) ? Number(data.panY) : 80 / zoom;
            zoom = Number.isFinite(Number(data.zoom)) ? Math.max(0.3, Math.min(2, Number(data.zoom))) : 1;
            normalizeGraph();
            selectedNodeId = null;
            selectedNodeIds.clear();
            selectedConnId = null;
            propNodeId = null;
            resultImages = {};
            commitHistory();
          } else {
            alert('无效的工作流文件');
          }
        } catch (err) {
          alert('加载失败: ' + err.message);
        }
        document.body.removeChild(fileInput);
      };
      reader.readAsText(file);
    });

    // Trigger file picker dialog
    fileInput.click();
  }

  function onHide() {
    if (historyTransaction !== null) commitHistory();
  }

  // ─── 快捷键/交互帮助浮层（? 键呼出）───
  let helpOverlayEl = null;
  function toggleHelpOverlay() {
    if (helpOverlayEl) { helpOverlayEl.remove(); helpOverlayEl = null; return; }
    helpOverlayEl = document.createElement('div');
    helpOverlayEl.className = 'kb-modal-overlay nodes-help-overlay';
    const rows = [
      ['P', '打开 / 收起属性面板'],
      ['C', '在选中节点附近创建注释'],
      ['Ctrl+D', '复制选中节点'],
      ['Ctrl+Z / Ctrl+Y', '撤销 / 重做'],
      ['Delete', '删除选中节点'],
      ['Alt+右键拖动', '裁剪连线（剪刀）'],
      ['滚轮', '缩放画布'],
      ['双击注释节点', '编辑注释文本'],
      ['?', '显示 / 隐藏本帮助'],
    ];
    helpOverlayEl.innerHTML =
      '<div class="kb-modal nodes-help-modal">' +
      '<div class="kb-modal-header"><span class="kb-modal-title">节点流 · 操作指南</span></div>' +
      '<div class="kb-modal-body"><table class="nodes-help-table">' +
      rows.map(r => '<tr><td class="nodes-help-key">' + r[0] + '</td><td>' + r[1] + '</td></tr>').join('') +
      '</table></div>' +
      '<div class="kb-modal-footer"><button class="kb-modal-btn kb-modal-btn-ok">知道了</button></div></div>';
    helpOverlayEl.addEventListener('click', e => {
      if (e.target === helpOverlayEl) { helpOverlayEl.remove(); helpOverlayEl = null; }
    });
    helpOverlayEl.querySelector('.kb-modal-btn-ok').onclick = () => { helpOverlayEl.remove(); helpOverlayEl = null; };
    document.body.appendChild(helpOverlayEl);
  }

  return { init, onShow, onHide, saveWorkflow, openWorkflow };
})();
