// Plane 3 — Home control panel: daemon status, service toggles, spotlight index, macros + recorder.
import { api, emit, h, toast } from '../core.js';

const STEP_TYPES = ['shell', 'open_app', 'open_url', 'open_path', 'quit_app', 'sleep', 'notify', 'yzkr'];

const plane = {
  id: 'home', title: 'Home', icon: 'home', shortcut: '⌘3', blurb: 'Daemons, toggles, macros',

  mount(el, ctx) {
    this.route = ctx.route;
    this.statusEl = h('div', { class: 'cards' });
    this.macrosEl = h('div', { class: 'macro-list' });
    this.recordText = h('textarea', { class: 'input', rows: 2, placeholder: 'e.g. "open Safari and Notes, wait 2 seconds, then notify me focus time"' });
    this.recordName = h('input', { class: 'input', placeholder: 'Name (optional)', style: 'width:190px' });
    this.histN = h('input', { class: 'input', type: 'number', min: 1, max: 50, value: 5, style: 'width:64px' });
    this.histName = h('input', { class: 'input', placeholder: 'Macro name', style: 'width:190px' });
    this.previewEl = h('div');
    this.refreshBtn = h('button', { class: 'btn sm ghost', onclick: () => this.refresh() }, '↻ refresh');
    el.append(h('div', { class: 'home-wrap' },
      h('section', {}, h('h2', {}, 'System', this.refreshBtn), this.statusEl),
      h('section', {}, h('h2', {}, 'Macros', h('span', { class: 'muted', style: 'text-transform:none;letter-spacing:0;font-weight:400' }, '~/YZKR/scripts/macros · mirrored into Sol')),
        this.macrosEl),
      h('section', {}, h('h2', {}, 'Record a macro'),
        h('div', { class: 'recorder' },
          this.recordText,
          h('div', { class: 'row' }, this.recordName,
            h('button', { class: 'btn primary', onclick: () => this.record() }, '✦ Draft from description'),
            h('span', { class: 'grow' }),
            h('span', { class: 'muted' }, 'or capture last'), this.histN, h('span', { class: 'muted' }, 'shell commands as'), this.histName,
            h('button', { class: 'btn', onclick: () => this.fromHistory() }, '⏺ Capture')),
          this.previewEl)),
      h('section', {}, h('h2', {}, 'Launcher'),
        h('div', { class: 'card' },
          h('div', { class: 'card-head' }, h('span', { class: 'name' }, '⌘Space → Sol → YZKR Spotlight')),
          h('div', { class: 'detail', style: '-webkit-line-clamp:unset' },
            'Sol is configured for ⌘Space and lists “YZKR Spotlight”, “YZKR Record Macro”, “YZKR Reindex” and every macro. To free ⌘Space from macOS Spotlight, run yzkr-hotkey once (it edits a system keyboard shortcut, so it’s yours to run). ⌥Space opens this palette from anywhere while AI-OS runs.'),
          h('div', {}, h('button', { class: 'btn sm', onclick: () => this.sendToTerminal('~/YZKR/bin/yzkr-hotkey') }, 'Paste into terminal'))))));
    this.refresh();
    this.timer = setInterval(() => { if (el.classList.contains('active')) this.refresh(true); }, 15000);
  },

  show(params) {
    if (params.record != null) {
      this.recordText.value = params.record.replace(/^(record|make|create)( me)?( a)? macro( that| to)?:?\s*/i, '');
      this.recordText.focus();
      if (this.recordText.value) this.record();
    }
  },

  async refresh(quiet) {
    if (!quiet) this.refreshBtn.textContent = '… loading';
    try {
      const [cards, macros] = await Promise.all([api('/api/system'), api('/api/macros')]);
      this.renderCards(cards);
      this.renderMacros(macros);
      const ups = cards.filter((c) => c.state === 'up').length;
      emit('subtitle', { plane: 'home', text: `${ups}/${cards.length} healthy` });
      const oll = cards.find((c) => c.id === 'ollama');
      emit('status', `ollama ${oll?.state === 'up' || oll?.state === 'warn' ? '●' : '○'}  ·  ${macros.length} macros`);
    } catch (e) { if (!quiet) toast(e.message, 'error'); }
    this.refreshBtn.textContent = '↻ refresh';
  },

  renderCards(cards) {
    const groups = {};
    for (const c of cards) (groups[c.group || 'Other'] ||= []).push(c);
    this.statusEl.replaceChildren(...Object.values(groups).flat().map((c) => {
      const right = [];
      if (c.toggle) {
        const isOn = c.on ?? (c.state === 'up' || c.state === 'warn');
        const sw = h('button', { class: `switch ${isOn ? 'on' : ''}`, title: isOn ? 'Turn off' : 'Turn on', 'aria-pressed': String(!!isOn),
          onclick: async () => {
            sw.disabled = true;
            try {
              const r = await api('/api/system/toggle', { id: c.toggle, on: !isOn });
              if (r.terminal) { this.sendToTerminal(r.terminal); toast(r.note); }
              else toast(r.ok ? `${c.label}: ${isOn ? 'off' : 'on'}` : r.output || 'failed', r.ok ? 'ok' : 'error');
            } catch (e) { toast(e.message, 'error'); }
            setTimeout(() => this.refresh(true), 1200);
          } });
        right.push(sw);
      }
      if (c.action === 'reindex') right.push(h('button', { class: 'btn sm', onclick: async () => {
        const r = await api('/api/spotlight/index', {}); toast(`Indexed ${r.docs} items in ${r.seconds}s`, 'ok'); this.refresh(true);
      } }, 'Reindex'));
      return h('div', { class: 'card', title: c.hint || '' },
        h('div', { class: 'card-head' }, h('span', { class: `dot ${c.state}` }), h('span', { class: 'name' }, c.label), ...right),
        h('div', { class: 'detail' }, c.detail),
        h('div', { class: 'muted', style: 'font-size:11px' }, c.group));
    }));
  },

  renderMacros(macros) {
    if (!macros.length) return this.macrosEl.replaceChildren(h('div', { class: 'muted' }, 'No macros yet — record one below.'));
    this.macrosEl.replaceChildren(...macros.map((m) => h('div', { class: 'macro' },
      h('span', { class: 'icon' }, m.icon),
      h('div', { class: 'body' }, h('div', {}, h('b', {}, m.name), ' ', h('span', { class: 'muted' }, m.description)),
        h('div', { class: 'steps' }, m.steps.map((s) => `${s.type}: ${s.value}`).join('  →  '))),
      h('button', { class: 'btn sm', onclick: () => this.route('editor', { path: m.path }) }, 'Edit'),
      h('button', { class: 'btn sm primary', onclick: async () => {
        toast(`Running ${m.name}…`);
        const r = await api('/api/macros/run', { slug: m.slug }).catch((e) => ({ ok: false, output: e.message }));
        toast(r.ok ? `${m.name} ✓` : `${m.name} failed: ${(r.output || '').slice(-160)}`, r.ok ? 'ok' : 'error');
      } }, '▶ Run'),
      h('button', { class: 'btn sm ghost danger', title: 'Delete', onclick: async () => {
        if (!confirm(`Delete macro “${m.name}”?`)) return;
        await api('/api/macros/delete', { slug: m.slug }); this.refresh(true);
      } }, '✕'))));
  },

  async record() {
    const text = this.recordText.value.trim();
    if (!text) return this.recordText.focus();
    this.previewEl.replaceChildren(h('div', { class: 'row muted' }, h('span', { class: 'spin' }), 'Drafting steps with local model…'));
    try {
      const m = await api('/api/macros/record', { request: text, name: this.recordName.value.trim(), dry_run: true });
      this.renderDraft(m);
    } catch (e) { this.previewEl.replaceChildren(h('div', { class: 'muted' }, `Couldn’t draft: ${e.message}`)); }
  },

  renderDraft(m) {
    const name = h('input', { class: 'input', value: m.name, style: 'width:200px' });
    const icon = h('input', { class: 'input', value: m.icon, style: 'width:48px;text-align:center' });
    const rows = m.steps.map((s) => ({ ...s }));
    const list = h('div', { style: 'display:grid;gap:6px' });
    const draw = () => list.replaceChildren(...rows.map((s, i) => h('div', { class: 'row' },
      h('select', { class: 'input', onchange: (e) => { s.type = e.target.value; } }, ...STEP_TYPES.map((t) => h('option', { value: t, selected: t === s.type }, t))),
      h('input', { class: 'input mono', value: s.value, style: 'flex:1;min-width:200px', oninput: (e) => { s.value = e.target.value; } }),
      h('button', { class: 'btn sm ghost', onclick: () => { rows.splice(i, 1); draw(); } }, '✕'))));
    draw();
    this.previewEl.replaceChildren(h('div', { style: 'display:grid;gap:8px;margin-top:4px' },
      h('div', { class: 'row' }, icon, name, h('span', { class: 'muted' }, m.description)),
      list,
      h('div', { class: 'row' },
        h('button', { class: 'btn sm ghost', onclick: () => { rows.push({ type: 'shell', value: '' }); draw(); } }, '+ step'),
        h('span', { class: 'grow' }),
        h('button', { class: 'btn ghost', onclick: () => this.previewEl.replaceChildren() }, 'Discard'),
        h('button', { class: 'btn primary', onclick: async () => {
          try {
            const saved = await api('/api/macros/save', { name: name.value.trim() || m.name, icon: icon.value || '⚡', description: m.description,
              steps: rows.filter((r) => r.value.trim()) });
            toast(`Saved ${saved.name} — also in Sol`, 'ok');
            this.previewEl.replaceChildren(); this.recordText.value = ''; this.refresh(true);
          } catch (e) { toast(e.message, 'error'); }
        } }, 'Save macro'))));
  },

  async fromHistory() {
    const name = this.histName.value.trim();
    if (!name) return this.histName.focus();
    try {
      const m = await api('/api/macros/history', { n: Number(this.histN.value) || 5, name });
      toast(`Captured ${m.steps.length} commands as ${m.name}`, 'ok');
      this.refresh(true);
    } catch (e) { toast(e.message, 'error'); }
  },

  sendToTerminal(cmd) { this.route('chat'); emit('terminal-paste', { cmd, run: false }); },
};
export default plane;
