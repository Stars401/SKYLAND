// ═══════════════════════════════════════════════════════════════
// SKYLAND 待时任务 — 定时任务列表 + 创建/编辑 + 立即执行
// 定时任务到点后由服务端 AI 完整执行，结果写入任务会话并通知
// ═══════════════════════════════════════════════════════════════

const TasksApp = (() => {
  const $ = id => document.getElementById(id);
  let tasks = [];
  let skills = [];
  let loaded = false;

  function init() {
    const nb = $('tasks-new-btn');
    if (nb) nb.addEventListener('click', () => showTaskDialog(null));
    const back = $('tasks-back-agent');
    if (back) back.addEventListener('click', () => { if (window.SkylandOpenLab) window.SkylandOpenLab(); else Router.navigate('/home'); });
  }

  function onShow() {
    loadTasks();
    loadSkills();
  }

  async function loadSkills() {
    try {
      const r = await fetch('/api/skills/list');
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      const data = await r.json();
      if (data.status === 'ok') skills = data.skills || [];
    } catch (e) {}
  }

  async function loadTasks() {
    const listEl = $('tasks-list');
    if (!listEl) return;
    try {
      const r = await fetch('/api/tasks/list');
      if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
      const data = await r.json();
      if (data.status !== 'ok') return;
      tasks = data.tasks || [];
      loaded = true;
      renderSummary();
      renderList();
    } catch (e) { console.error('loadTasks error:', e); }
  }

  function fmtTs(ts) {
    if (!ts) return '—';
    try {
      return new Date(ts * 1000).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
    } catch (e) { return String(ts); }
  }

  function scheduleDesc(s) {
    if (!s) return '—';
    if (s.type === 'once') return '单次 · ' + fmtTs(s.at);
    if (s.type === 'daily') return '每天 ' + (s.time || '');
    if (s.type === 'weekly') {
      const names = ['一', '二', '三', '四', '五', '六', '日'];
      const wd = (s.weekdays || []).map(d => '周' + (names[d - 1] || d)).join('、');
      return '每周 ' + wd + ' ' + (s.time || '');
    }
    if (s.type === 'interval') return '每 ' + s.minutes + ' 分钟';
    return JSON.stringify(s);
  }

  function renderSummary() {
    const el = $('tasks-summary');
    if (!el) return;
    const enabled = tasks.filter(t => t.enabled).length;
    const ran = tasks.reduce((acc, t) => acc + (t.run_count || 0), 0);
    el.innerHTML = `
      <div class="todo-stat-card"><div class="todo-stat-value">${tasks.length}</div><div class="todo-stat-label">全部任务</div></div>
      <div class="todo-stat-card"><div class="todo-stat-value">${enabled}</div><div class="todo-stat-label">进行中</div></div>
      <div class="todo-stat-card"><div class="todo-stat-value">${ran}</div><div class="todo-stat-label">累计执行</div></div>
      <div class="todo-stat-card"><div class="todo-stat-value">${skills.length}</div><div class="todo-stat-label">可用技能</div></div>`;
  }

  function renderList() {
    const el = $('tasks-list');
    if (!el) return;
    if (tasks.length === 0) {
      el.innerHTML = '<div class="todo-empty">暂无定时任务<br><span class="todo-empty-hint">点击右上角「新建任务」，或在 Agent 终端对话里让 AI 创建</span></div>';
      return;
    }
    el.innerHTML = tasks.map(t => {
      const status = t.enabled ? 'enabled' : 'disabled';
      const next = t.enabled ? fmtTs(t.next_run) : '已停用';
      const lastResult = t.last_result ? esc(t.last_result) : '尚未执行';
      return `
      <div class="task-card ${status}" data-id="${t.id}">
        <div class="task-card-main">
          <div class="task-card-head">
            <span class="task-toggle-dot ${status}" data-action="toggle" data-id="${t.id}" title="${t.enabled ? '点击停用' : '点击启用'}"></span>
            <span class="task-card-name">${esc(t.name)}</span>
            ${t.skill ? `<span class="task-skill-tag"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="11" height="11"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg> ${esc(t.skill)}</span>` : ''}
            <span class="task-schedule">${esc(scheduleDesc(t.schedule))}</span>
          </div>
          <div class="task-card-prompt">${esc(t.prompt)}</div>
          <div class="task-card-meta">
            <span>下次：${esc(next)}</span>
            <span>上次：${fmtTs(t.last_run)}</span>
            <span>执行 ${t.run_count || 0} 次</span>
          </div>
          <div class="task-card-result" title="${lastResult}">${lastResult}</div>
        </div>
        <div class="task-card-actions">
          <button data-action="run" data-id="${t.id}" title="立即执行">${ICONS.play}</button>
          <button data-action="open" data-id="${t.id}" title="查看结果会话">${ICONS.external}</button>
          <button data-action="edit" data-id="${t.id}" title="编辑">${ICONS.pencil}</button>
          <button data-action="delete" data-id="${t.id}" title="删除">${ICONS.trash}</button>
        </div>
      </div>`;
    }).join('');
    el.querySelectorAll('[data-action]').forEach(btn => {
      btn.addEventListener('click', e => {
        e.stopPropagation();
        const id = btn.dataset.id;
        const action = btn.dataset.action;
        if (action === 'toggle') toggleTask(id);
        else if (action === 'run') runTask(id);
        else if (action === 'edit') showTaskDialog(tasks.find(t => t.id === id));
        else if (action === 'delete') deleteTask(id);
        else if (action === 'open') openTaskConversation(id);
      });
    });
  }

  async function openTaskConversation(id) {
    const t = tasks.find(x => x.id === id);
    if (!t || !t.conversation_id) return;
    try {
      const r = await fetch('/api/conversations/' + t.conversation_id);
      if (!r.ok) throw new Error('HTTP ' + r.status);
      const data = await r.json();
      if (data.status !== 'ok') throw new Error(data.message || '读取失败');
      const msgs = (data.data && data.data.messages) || [];
      TasksApp.showResultModal(t.name, msgs);
    } catch (e) {
      showAlert('查看失败', e.message || String(e));
    }
  }

  function showResultModal(name, msgs) {
    const overlay = document.createElement('div');
    overlay.className = 'kb-modal-overlay';
    const modal = document.createElement('div');
    modal.className = 'kb-modal task-result-modal';
    const body = msgs.length === 0
      ? '<div class="todo-empty">任务还没有执行记录<br><span class="todo-empty-hint">到点自动执行，或点「立即执行」</span></div>'
      : msgs.map(m => `<div class="task-result-msg ${m.role === 'user' ? 'user' : 'ai'}">
          <div class="task-result-role">${m.role === 'user' ? '指令' : 'AI 结果'}</div>
          <div class="task-result-text">${typeof md === 'function' ? md(m.content || '') : esc(m.content || '')}</div>
        </div>`).join('');
    modal.innerHTML = `
      <div class="kb-modal-header"><span class="kb-modal-title">「${esc(name)}」执行记录</span></div>
      <div class="kb-modal-body task-result-body">${body}</div>
      <div class="kb-modal-footer"><button class="kb-modal-btn kb-modal-btn-ok" id="task-result-close">关闭</button></div>`;
    overlay.appendChild(modal);
    overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
    modal.querySelector('#task-result-close').onclick = () => overlay.remove();
    document.body.appendChild(overlay);
  }

  function showTaskDialog(existing) {
    const isEdit = !!existing;
    const overlay = document.createElement('div');
    overlay.className = 'kb-modal-overlay';
    const modal = document.createElement('div');
    modal.className = 'kb-modal task-dialog';
    const schedule = existing ? existing.schedule : { type: 'daily', time: '08:00' };
    modal.innerHTML = `
      <div class="kb-modal-header"><span class="kb-modal-title">${isEdit ? '编辑任务' : '新建定时任务'}</span></div>
      <div class="kb-modal-body">
        <input class="kb-modal-input" id="task-input-name" type="text" placeholder="任务名（如：晨间简报）" value="${isEdit ? esc(existing.name) : ''}" style="margin-bottom:8px">
        <textarea class="kb-modal-input" id="task-input-prompt" placeholder="到点后交给 AI 的执行指令（如：读取今日待办，生成一份晨间简报）" style="width:100%;min-height:70px;resize:vertical;margin-bottom:10px">${isEdit ? esc(existing.prompt) : ''}</textarea>
        <div class="todo-dialog-row">
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">关联技能（可选）</label>
            <select class="kb-modal-input" id="task-input-skill">
              <option value="">— 不关联 —</option>
              ${skills.map(s => `<option value="${esc(s.name)}" ${isEdit && existing.skill === s.name ? 'selected' : ''}>${esc(s.title)}（${esc(s.name)}）</option>`).join('')}
            </select>
          </div>
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">计划类型</label>
            <select class="kb-modal-input" id="task-input-type">
              <option value="daily" ${schedule.type === 'daily' ? 'selected' : ''}>每天</option>
              <option value="weekly" ${schedule.type === 'weekly' ? 'selected' : ''}>每周指定日</option>
              <option value="interval" ${schedule.type === 'interval' ? 'selected' : ''}>每 N 分钟</option>
              <option value="once" ${schedule.type === 'once' ? 'selected' : ''}>单次（指定时间）</option>
            </select>
          </div>
        </div>
        <div class="todo-dialog-row" id="task-row-time">
          <div class="todo-dialog-field">
            <label class="todo-dialog-label">时间</label>
            <input class="kb-modal-input" id="task-input-time" type="time" value="${esc(schedule.time || '08:00')}">
          </div>
          <div class="todo-dialog-field" id="task-field-weekdays">
            <label class="todo-dialog-label">星期</label>
            <div class="task-weekdays">
              ${['一','二','三','四','五','六','日'].map((n, i) => `
                <label class="task-wd-chip" data-wd="${i + 1}">${n}</label>`).join('')}
            </div>
          </div>
          <div class="todo-dialog-field" id="task-field-minutes">
            <label class="todo-dialog-label">间隔（分钟）</label>
            <input class="kb-modal-input" id="task-input-minutes" type="number" min="1" value="${schedule.minutes || 30}">
          </div>
          <div class="todo-dialog-field" id="task-field-at">
            <label class="todo-dialog-label">执行时间</label>
            <input class="kb-modal-input" id="task-input-at" type="datetime-local">
          </div>
        </div>
        <div class="task-dialog-hint">到点后 AI 将使用全部工具自动执行，结果写入任务会话并通知你。</div>
      </div>
      <div class="kb-modal-footer">
        <button class="kb-modal-btn kb-modal-btn-cancel" id="task-dialog-cancel">取消</button>
        <button class="kb-modal-btn kb-modal-btn-ok" id="task-dialog-save">保存</button>
      </div>`;
    overlay.appendChild(modal);
    overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
    modal.querySelector('#task-dialog-cancel').onclick = () => overlay.remove();

    // 星期多选
    const selectedWd = new Set(schedule.weekdays || [1, 2, 3, 4, 5]);
    modal.querySelectorAll('.task-wd-chip').forEach(chip => {
      if (selectedWd.has(parseInt(chip.dataset.wd))) chip.classList.add('active');
      chip.addEventListener('click', () => {
        const wd = parseInt(chip.dataset.wd);
        if (selectedWd.has(wd)) { selectedWd.delete(wd); chip.classList.remove('active'); }
        else { selectedWd.add(wd); chip.classList.add('active'); }
      });
    });

    // 计划类型切换可见性
    const typeSel = modal.querySelector('#task-input-type');
    const rows = {
      time: modal.querySelector('#task-row-time'),
      weekdays: modal.querySelector('#task-field-weekdays'),
      minutes: modal.querySelector('#task-field-minutes'),
      at: modal.querySelector('#task-field-at'),
    };
    const timeInput = modal.querySelector('#task-input-time');
    function syncTypeUI() {
      const t = typeSel.value;
      rows.time.style.display = (t === 'daily' || t === 'weekly') ? '' : 'none';
      rows.weekdays.style.display = t === 'weekly' ? '' : 'none';
      rows.minutes.style.display = t === 'interval' ? '' : 'none';
      rows.at.style.display = t === 'once' ? '' : 'none';
      timeInput.parentElement.style.display = (t === 'daily' || t === 'weekly') ? '' : 'none';
    }
    typeSel.addEventListener('change', syncTypeUI);
    syncTypeUI();
    if (schedule.type === 'once' && schedule.at) {
      const d = new Date(schedule.at * 1000);
      const pad = n => String(n).padStart(2, '0');
      modal.querySelector('#task-input-at').value =
        `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
    }

    modal.querySelector('#task-dialog-save').onclick = async () => {
      const saveBtn = modal.querySelector('#task-dialog-save');
      const name = modal.querySelector('#task-input-name').value.trim();
      const prompt = modal.querySelector('#task-input-prompt').value.trim();
      const skill = modal.querySelector('#task-input-skill').value || null;
      const type = typeSel.value;
      if (!name || !prompt) { showAlert('提示', '任务名与执行指令不能为空'); return; }
      let schedule2;
      if (type === 'daily') schedule2 = { type, time: timeInput.value || '08:00' };
      else if (type === 'weekly') {
        if (selectedWd.size === 0) { showAlert('提示', '请选择至少一个星期'); return; }
        schedule2 = { type, weekdays: Array.from(selectedWd).sort(), time: timeInput.value || '08:00' };
      } else if (type === 'interval') {
        const minutes = parseInt(modal.querySelector('#task-input-minutes').value) || 30;
        schedule2 = { type, minutes };
      } else {
        const atVal = modal.querySelector('#task-input-at').value;
        if (!atVal) { showAlert('提示', '请选择执行时间'); return; }
        schedule2 = { type, at: Math.floor(new Date(atVal).getTime() / 1000) };
      }
      saveBtn.disabled = true; saveBtn.textContent = '保存中...';
      try {
        const url = isEdit ? `/api/tasks/${existing.id}` : '/api/tasks/create';
        const method = isEdit ? 'PUT' : 'POST';
        const payload = isEdit
          ? { name, prompt, skill, schedule: schedule2 }
          : { name, prompt, skill, schedule: schedule2 };
        const r = await fetch(url, { method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(payload) });
        if (r.status === 401) { if (typeof Auth !== 'undefined' && Auth.onSessionLost) Auth.onSessionLost(); return; }
        const data = await r.json();
        if (data.status === 'ok') { overlay.remove(); loadTasks(); }
        else showAlert('保存失败', data.message || '未知错误');
      } catch (e) {
        showAlert('保存失败', e.message);
      } finally {
        if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = '保存'; }
      }
    };
    document.body.appendChild(overlay);
    setTimeout(() => modal.querySelector('#task-input-name')?.focus(), 50);
  }

  async function toggleTask(id) {
    const t = tasks.find(x => x.id === id);
    if (!t) return;
    try {
      const r = await fetch(`/api/tasks/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ enabled: !t.enabled }) });
      const d = await r.json().catch(() => null);
      // 后端对「任务不存在」回 200+error —— 必须等确认再翻 UI，否则假切换刷新弹回
      if (!r.ok || (d && d.status === 'error')) {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show((d && d.message) || '任务状态切换失败', 'error');
        loadTasks();
        return;
      }
      t.enabled = !t.enabled;
      renderList(); renderSummary();
    } catch (e) {
      console.error('toggleTask error:', e);
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('任务状态切换失败：' + e.message, 'error');
    }
  }

  let runningTask = null;
  async function runTask(id) {
    if (runningTask) return; // 防双击重复入队
    runningTask = id;
    try {
      const r = await fetch(`/api/tasks/${id}/run`, { method: 'POST' });
      let data;
      try { data = await r.json(); }
      catch (e) { throw new Error(r.status === 200 ? '响应解析失败' : ('HTTP ' + r.status)); }
      if (data.status === 'ok') showAlert('已加入执行队列', 'AI 正在执行，结果将写入任务会话，完成后会通知你。');
      else showAlert('执行失败', data.message || '未知错误');
      loadTasks();
    } catch (e) {
      console.error('runTask error:', e);
      showAlert('执行失败', e.message || '网络异常');
    } finally { runningTask = null; }
  }

  async function deleteTask(id) {
    const confirmed = await showConfirm('删除任务', '确定删除此定时任务？执行记录会话将保留。');
    if (!confirmed) return;
    try {
      const r = await fetch(`/api/tasks/${id}`, { method: 'DELETE' });
      const d = await r.json().catch(() => null);
      // 200+error（任务已在别处删除/不存在）→ 提示并刷新，不做假删除
      if (!r.ok || (d && d.status === 'error')) {
        if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show((d && d.message) || '任务删除失败', 'error');
        loadTasks();
        return;
      }
      tasks = tasks.filter(t => t.id !== id);
      renderList(); renderSummary();
    } catch (e) {
      console.error('deleteTask error:', e);
      if (typeof SkyToast === 'object' && SkyToast.show) SkyToast.show('任务删除失败：' + e.message, 'error');
    }
  }

  // ─── 弹窗（与 todo.js 同风格）───
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
          <button class="kb-modal-btn kb-modal-btn-cancel" id="tasks-confirm-cancel">取消</button>
          <button class="kb-modal-btn kb-modal-btn-ok" id="tasks-confirm-ok">确定</button>
        </div>`;
      overlay.appendChild(modal);
      overlay.addEventListener('click', e => { if (e.target === overlay) { overlay.remove(); resolve(false); } });
      modal.querySelector('#tasks-confirm-cancel').onclick = () => { overlay.remove(); resolve(false); };
      modal.querySelector('#tasks-confirm-ok').onclick = () => { overlay.remove(); resolve(true); };
      document.body.appendChild(overlay);
    });
  }

  function showAlert(title, message) {
    const overlay = document.createElement('div');
    overlay.className = 'kb-modal-overlay';
    const modal = document.createElement('div');
    modal.className = 'kb-modal';
    modal.innerHTML = `
      <div class="kb-modal-header"><span class="kb-modal-title">${esc(title)}</span></div>
      <div class="kb-modal-body"><p class="kb-modal-message">${esc(message)}</p></div>
      <div class="kb-modal-footer"><button class="kb-modal-btn kb-modal-btn-ok" id="tasks-alert-ok">知道了</button></div>`;
    overlay.appendChild(modal);
    modal.querySelector('#tasks-alert-ok').onclick = () => overlay.remove();
    document.body.appendChild(overlay);
  }


  const ICONS = {
    play: '<svg viewBox="0 0 24 24" fill="currentColor" width="12" height="12"><polygon points="6 3 20 12 6 21"/></svg>',
    external: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>',
    pencil: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg>',
    trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="12" height="12"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>',
  };

  function esc(s) { return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }

  return { init, onShow, loadTasks, showResultModal };
})();
