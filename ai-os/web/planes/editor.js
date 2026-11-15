// Plane 1 — Editor: lazy file tree bound to ~/YZKR (+ ai-memory shortcut), tabbed CodeMirror, live Markdown preview.
import { api, emit, h, qs, renderMarkdown, toast } from '../core.js';

const MODES = {
  md: 'gfm', markdown: 'gfm', py: 'python', sh: 'shell', zsh: 'shell', bash: 'shell', js: 'javascript', mjs: 'javascript',
  ts: { name: 'javascript', typescript: true }, json: { name: 'javascript', json: true }, jsonl: { name: 'javascript', json: true },
  css: 'css', html: 'htmlmixed', swift: 'swift', yml: 'yaml', yaml: 'yaml', toml: 'toml',
};
const modeFor = (path) => {
  const name = path.split('/').pop();
  const ext = name.includes('.') ? name.split('.').pop().toLowerCase() : '';
  return MODES[ext] || (['zen', 'npw', 'zen-pwa', 'yzkr-hotkey'].includes(name) ? 'shell' : null);
};

const plane = {
  id: 'editor', title: 'Editor', icon: 'editor', shortcut: '⌘1', blurb: 'Markdown & code in ~/YZKR',
  tabs: [], active: null, root: '',

  mount(el, ctx) {
    this.root = ctx.root;
    this.treeBody = h('div', { class: 'tree-body' });
    const rootBtn = (label, sub) => h('button', { class: 'btn sm ghost', onclick: () => this.loadTree(sub ? `${this.root}/${sub}` : this.root) }, label);
    this.tree = h('aside', { class: 'tree' },
      h('div', { class: 'tree-roots' }, rootBtn('YZKR'), rootBtn('ai-memory', 'ai-memory'), rootBtn('scripts', 'scripts'),
        h('span', { class: 'grow' }), h('button', { class: 'btn sm ghost', title: 'New file', onclick: () => this.newFile() }, '+')),
      this.treeBody);
    this.tabsEl = h('div', { class: 'tabs' });
    this.split = h('div', { class: 'ed-split' });
    this.preview = h('div', { class: 'preview md', hidden: true });
    this.statusEl = h('div', { class: 'ed-status' });
    this.previewBtn = h('button', { class: 'btn sm', title: '⌘⇧V', onclick: () => this.togglePreview() }, 'Preview');
    this.saveBtn = h('button', { class: 'btn sm primary', onclick: () => this.save() }, 'Save');
    this.toolbar = h('div', { class: 'toolbar', style: 'min-height:36px;padding:4px 10px' },
      h('span', { class: 'muted mono', style: 'font-size:12px' }, ''), h('span', { class: 'grow' }), this.previewBtn, this.saveBtn);
    this.empty = h('div', { class: 'empty' }, h('h3', {}, 'No file open'), 'Pick a file in the tree, or ⌘K and type its name.');
    this.main = h('div', { class: 'ed-main' }, this.tabsEl, this.toolbar, this.split, this.statusEl);
    el.append(this.tree, this.main);

    this.cm = window.CodeMirror(this.split, {
      lineNumbers: true, lineWrapping: true, styleActiveLine: true, matchBrackets: true, autoCloseBrackets: true,
      indentUnit: 2, tabSize: 2, indentWithTabs: false,
      extraKeys: {
        'Cmd-S': () => this.save(), 'Cmd-Shift-V': () => this.togglePreview(),
        Enter: 'newlineAndIndentContinueMarkdownList', Tab: (cm) => cm.execCommand(cm.somethingSelected() ? 'indentMore' : 'insertSoftTab'),
      },
    });
    this.split.append(this.preview, this.empty);
    this.cm.on('change', () => {
      const t = this.activeTab();
      if (!t || this.loading) return;
      t.doc = this.cm.getValue();
      this.markDirty(t, t.doc !== t.saved);
      if (!this.preview.hidden) this.renderPreview();
    });
    this.cm.on('cursorActivity', () => this.renderStatus());
    window.addEventListener('beforeunload', (e) => { if (this.tabs.some((t) => t.dirty)) e.preventDefault(); });
    this.loadTree(this.root);
    this.renderTabs();
  },

  show(params) {
    if (params.path) this.open(params.path);
    setTimeout(() => this.cm.refresh(), 0);
  },

  // ───── tree ─────
  async loadTree(path) {
    this.treeBody.replaceChildren(await this.dirNode(path, 0, true));
  },
  async dirNode(path, depth, expanded) {
    const wrap = h('div');
    const { entries } = await api(`/api/fs/list?${qs({ path })}`);
    for (const e of entries) {
      const row = h('div', { class: `tree-row ${e.dir ? 'is-dir' : ''}`, style: `padding-left:${8 + depth * 13}px`, title: e.path },
        h('span', { class: 'chev' }, e.dir ? '▶' : ''), h('span', { class: 'name' }, e.name));
      row.dataset.path = e.path;
      wrap.append(row);
      if (e.dir) {
        let kids = null;
        row.onclick = async () => {
          if (kids) { kids.remove(); kids = null; row.classList.remove('expanded'); return; }
          kids = await this.dirNode(e.path, depth + 1);
          row.after(kids);
          row.classList.add('expanded');
        };
      } else row.onclick = () => this.open(e.path);
    }
    return wrap;
  },

  // ───── tabs ─────
  activeTab() { return this.tabs.find((t) => t.path === this.active); },
  async open(path) {
    let t = this.tabs.find((x) => x.path === path);
    if (!t) {
      try {
        const f = await api(`/api/fs/read?${qs({ path })}`);
        t = { path: f.path, rel: f.rel, doc: f.text, saved: f.text, mtime: f.mtime, dirty: false, cmDoc: null };
        t.cmDoc = window.CodeMirror.Doc(f.text, modeFor(f.path));
        this.tabs.push(t);
      } catch (e) { return toast(`${path.split('/').pop()}: ${e.message}`, 'error'); }
    }
    this.activate(t.path);
  },
  activate(path) {
    const t = this.tabs.find((x) => x.path === path);
    this.active = t?.path || null;
    this.loading = true;
    if (t) this.cm.swapDoc(t.cmDoc);
    this.loading = false;
    const isMd = t && /\.(md|markdown)$/i.test(t.path);
    if (t && isMd && this.preview.hidden && this.autoPreview !== false) this.togglePreview(true);
    if (t && !isMd && !this.preview.hidden) this.togglePreview(false);
    this.empty.hidden = !!t;
    this.cm.getWrapperElement().classList.toggle('cm-md', !!isMd);
    this.cm.getWrapperElement().hidden = !t;
    this.toolbar.firstChild.textContent = t ? `~/YZKR/${t.rel}` : '';
    this.saveBtn.disabled = !t;
    emit('subtitle', { plane: 'editor', text: t?.rel || '' });
    this.renderTabs();
    this.renderStatus();
    document.querySelectorAll('.tree-row.open-file').forEach((r) => r.classList.remove('open-file'));
    document.querySelector(`.tree-row[data-path="${CSS.escape(path || '')}"]`)?.classList.add('open-file');
    if (t) setTimeout(() => { this.cm.refresh(); this.cm.focus(); }, 0);
  },
  close(path) {
    const t = this.tabs.find((x) => x.path === path);
    if (t?.dirty && !confirm(`Discard unsaved changes to ${t.rel}?`)) return;
    const i = this.tabs.indexOf(t);
    this.tabs.splice(i, 1);
    if (this.active === path) this.activate(this.tabs[Math.max(0, i - 1)]?.path);
    else this.renderTabs();
  },
  markDirty(t, dirty) { if (t.dirty !== dirty) { t.dirty = dirty; this.renderTabs(); } },
  renderTabs() {
    this.tabsEl.replaceChildren(...this.tabs.map((t) =>
      h('div', { class: `tab ${t.path === this.active ? 'active' : ''} ${t.dirty ? 'dirty' : ''}`, title: t.rel, onclick: () => this.activate(t.path),
        onauxclick: (e) => e.button === 1 && this.close(t.path) },
      t.path.split('/').pop(),
      h('button', { class: 'x', 'aria-label': 'Close', onclick: (e) => { e.stopPropagation(); this.close(t.path); } }))));
  },
  renderStatus() {
    const t = this.activeTab();
    if (!t) return this.statusEl.replaceChildren();
    const c = this.cm.getCursor();
    const mode = this.cm.getOption('mode');
    this.statusEl.replaceChildren(h('span', {}, `Ln ${c.line + 1}, Col ${c.ch + 1}`), h('span', {}, `${this.cm.lineCount()} lines`),
      h('span', {}, typeof mode === 'object' ? mode.name : mode || 'text'), h('span', { class: 'grow' }), h('span', {}, t.dirty ? '● unsaved' : 'saved'));
  },

  // ───── actions ─────
  async save() {
    const t = this.activeTab();
    if (!t) return;
    try {
      const r = await api('/api/fs/write', { path: t.path, text: t.doc, mtime: t.mtime });
      t.saved = t.doc; t.mtime = r.mtime;
      this.markDirty(t, false);
      this.renderStatus();
      toast(`Saved ${t.rel}`, 'ok');
    } catch (e) { toast(e.message, 'error'); }
  },
  togglePreview(force) {
    const show = force ?? this.preview.hidden;
    if (force === undefined) this.autoPreview = show;
    this.preview.hidden = !show;
    this.previewBtn.classList.toggle('primary', show);
    if (show) this.renderPreview();
    setTimeout(() => this.cm.refresh(), 0);
  },
  renderPreview() {
    clearTimeout(this._pv);
    this._pv = setTimeout(() => { this.preview.innerHTML = renderMarkdown(this.cm.getValue()); }, 60);
  },
  async newFile() {
    const name = prompt('New file path (relative to ~/YZKR):', 'ai-memory/notes/untitled.md');
    if (!name) return;
    try {
      await api('/api/fs/write', { path: name, text: name.endsWith('.md') ? `# ${name.split('/').pop().replace(/\.md$/, '')}\n\n` : '' });
      await this.open(`${this.root}/${name}`);
      this.loadTree(this.root);
    } catch (e) { toast(e.message, 'error'); }
  },
};
export default plane;
