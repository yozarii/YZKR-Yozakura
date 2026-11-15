// Plane 2 — Graph / Mindmap: memory, filesystem and chat graphs on one canvas, linked through shared file nodes.
// Dependency-free force layout (Barnes–Hut-free O(n²) is fine for ≤ ~800 nodes with a cooling schedule).
import { api, emit, h, toast } from '../core.js';

const LAYERS = [
  { id: 'memory', label: 'Information & memory', css: '--graph-memory' },
  { id: 'filesystem', label: 'Data & filesystem', css: '--graph-filesystem' },
  { id: 'chat', label: 'Chat context & history', css: '--graph-chat' },
];
const cssVar = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();

const plane = {
  id: 'graph', title: 'Graph', icon: 'graph', shortcut: '⌘E', blurb: 'Memory · filesystem · chat mindmap',
  on: { memory: true, filesystem: true, chat: true },
  nodes: [], edges: [], view: { x: 0, y: 0, k: 1 }, hover: null, selected: null, query: '', alpha: 0,

  mount(el, ctx) {
    this.route = ctx.route;
    this.colors = Object.fromEntries(LAYERS.map((l) => [l.id, cssVar(l.css)]));
    this.colors.muted = cssVar('--text-muted'); this.colors.text = cssVar('--text'); this.colors.bg = cssVar('--bg');
    this.colors.accent = cssVar('--accent'); this.colors.warning = cssVar('--warning');
    this.chips = LAYERS.map((l) => {
      const b = h('button', { class: 'layer-chip on', onclick: () => { this.on[l.id] = !this.on[l.id]; b.classList.toggle('on', this.on[l.id]); this.applyFilter(); } },
        h('span', { class: 'sw', style: `background:var(${l.css})` }), l.label);
      return b;
    });
    this.search = h('input', { class: 'input', placeholder: 'Find node…', style: 'width:200px', oninput: (e) => { this.query = e.target.value.toLowerCase(); this.draw(); },
      onkeydown: (e) => { if (e.key === 'Enter') this.focusMatch(); } });
    this.countEl = h('span', { class: 'muted', style: 'font-size:12px' });
    const toolbar = h('div', { class: 'toolbar' }, ...this.chips, h('span', { class: 'grow' }), this.countEl, this.search,
      h('button', { class: 'btn', onclick: () => this.fit() }, 'Fit'), h('button', { class: 'btn', onclick: () => this.load(true) }, 'Rebuild'));
    this.canvas = h('canvas');
    this.inspector = h('aside', { class: 'inspector', hidden: true });
    this.legend = h('div', { class: 'graph-legend' }, 'drag to pan · scroll to zoom · click to inspect · double-click a file to edit');
    this.stage = h('div', { class: 'graph-stage' }, this.canvas, this.inspector, this.legend);
    el.append(toolbar, this.stage);
    this.ctx = this.canvas.getContext('2d');
    new ResizeObserver(() => this.resize()).observe(this.stage);
    this.bindPointer();
    this.load();
  },

  show(params) {
    if (params.force) this.load(true);
    if (params.focus || params.q) {
      this.search.value = params.focus || params.q;
      this.query = this.search.value.toLowerCase();
      this.whenLoaded(() => this.focusMatch());
    }
    this.resize();
  },
  whenLoaded(fn) { this.loaded ? fn() : (this._pending = fn); },

  async load(force = false) {
    this.countEl.textContent = 'building…';
    let data;
    try { data = await api(`/api/graph${force ? '?force=1' : ''}`); } catch (e) { return toast(`graph: ${e.message}`, 'error'); }
    const prev = Object.fromEntries(this.nodes.map((n) => [n.id, n]));
    const W = this.W || 800, H = this.H || 600;
    // seed each layer around its own anchor so the three graphs read as regions that interlink
    const anchor = { memory: [-1, -0.6], filesystem: [0, 0.7], chat: [1, -0.6] };
    this.nodes = data.nodes.map((n) => {
      const p = prev[n.id];
      const [ax, ay] = anchor[n.layer] || [0, 0];
      return { ...n, x: p?.x ?? ax * 260 + (Math.random() - 0.5) * 220, y: p?.y ?? ay * 220 + (Math.random() - 0.5) * 220, vx: 0, vy: 0,
        r: 3.5 + Math.min(10, Math.sqrt(n.degree || 0) * 1.6) + (n.type === 'entity' || n.type === 'session' ? 2 : 0) };
    });
    this.index = Object.fromEntries(this.nodes.map((n) => [n.id, n]));
    this.edges = data.edges.map((e) => ({ ...e, s: this.index[e.source], t: this.index[e.target] })).filter((e) => e.s && e.t);
    this.adj = {};
    for (const e of this.edges) { (this.adj[e.s.id] ||= []).push(e); (this.adj[e.t.id] ||= []).push(e); }
    if (Object.keys(data.errors || {}).length) toast(`graph errors: ${JSON.stringify(data.errors)}`, 'error');
    this.applyFilter();
    this.kick(1);
    if (!this.loaded) { this.loaded = true; setTimeout(() => { this.fit(); this._pending?.(); this._pending = null; }, 900); }
  },

  applyFilter() {
    for (const n of this.nodes) n.hidden = !n.layers.some((l) => this.on[l]);
    for (const e of this.edges) e.hidden = !this.on[e.layer] || e.s.hidden || e.t.hidden;
    const vis = this.nodes.filter((n) => !n.hidden).length;
    this.countEl.textContent = `${vis} nodes · ${this.edges.filter((e) => !e.hidden).length} links`;
    emit('subtitle', { plane: 'graph', text: LAYERS.filter((l) => this.on[l.id]).map((l) => l.id).join(' · ') });
    this.kick(0.3);
  },

  // ───── simulation ─────
  kick(a) { this.alpha = Math.max(this.alpha, a); if (!this.raf) this.raf = requestAnimationFrame(() => this.tick()); },
  tick() {
    this.raf = null;
    const ns = this.nodes.filter((n) => !n.hidden);
    const a = this.alpha;
    for (let i = 0; i < ns.length; i++) {
      const p = ns[i];
      for (let j = i + 1; j < ns.length; j++) {
        const q = ns[j];
        let dx = q.x - p.x, dy = q.y - p.y, d2 = dx * dx + dy * dy || 0.01;
        if (d2 > 90000) continue;
        const f = (900 * a) / d2;
        dx *= f; dy *= f;
        p.vx -= dx; p.vy -= dy; q.vx += dx; q.vy += dy;
      }
      p.vx -= p.x * 0.004 * a; p.vy -= p.y * 0.004 * a; // gentle gravity
    }
    for (const e of this.edges) {
      if (e.hidden) continue;
      const dx = e.t.x - e.s.x, dy = e.t.y - e.s.y, d = Math.sqrt(dx * dx + dy * dy) || 1;
      const rest = e.kind === 'contains' ? 34 : e.layer === 'chat' ? 70 : 55;
      const f = ((d - rest) / d) * 0.06 * a;
      e.s.vx += dx * f; e.s.vy += dy * f; e.t.vx -= dx * f; e.t.vy -= dy * f;
    }
    for (const n of ns) {
      if (n === this.dragNode) { n.vx = n.vy = 0; continue; }
      n.vx *= 0.82; n.vy *= 0.82;
      n.x += Math.max(-30, Math.min(30, n.vx)); n.y += Math.max(-30, Math.min(30, n.vy));
    }
    this.alpha *= 0.985;
    this.draw();
    if (this.alpha > 0.01) this.raf = requestAnimationFrame(() => this.tick());
  },

  // ───── rendering ─────
  resize() {
    const r = this.stage.getBoundingClientRect();
    if (!r.width) return;
    const dpr = window.devicePixelRatio || 1;
    this.W = r.width; this.H = r.height;
    this.canvas.width = r.width * dpr; this.canvas.height = r.height * dpr;
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.draw();
  },
  toScreen(n) { return [(n.x + this.view.x) * this.view.k + this.W / 2, (n.y + this.view.y) * this.view.k + this.H / 2]; },
  toWorld(sx, sy) { return [(sx - this.W / 2) / this.view.k - this.view.x, (sy - this.H / 2) / this.view.k - this.view.y]; },
  matches(n) { return this.query && (n.label.toLowerCase().includes(this.query) || (n.rel || '').toLowerCase().includes(this.query)); },
  draw() {
    const c = this.ctx, { k } = this.view;
    if (!c || !this.W) return;
    c.clearRect(0, 0, this.W, this.H);
    const focus = this.selected || this.hover;
    const near = focus ? new Set([focus.id, ...(this.adj[focus.id] || []).map((e) => (e.s === focus ? e.t.id : e.s.id))]) : null;
    c.lineWidth = 1;
    for (const e of this.edges) {
      if (e.hidden) continue;
      const lit = near && (e.s === focus || e.t === focus);
      c.globalAlpha = near ? (lit ? 0.85 : 0.05) : 0.22;
      c.strokeStyle = this.colors[e.layer];
      const [x1, y1] = this.toScreen(e.s), [x2, y2] = this.toScreen(e.t);
      c.beginPath(); c.moveTo(x1, y1); c.lineTo(x2, y2); c.stroke();
    }
    const showLabels = k > 0.9;
    for (const n of this.nodes) {
      if (n.hidden) continue;
      const [x, y] = this.toScreen(n);
      if (x < -40 || y < -40 || x > this.W + 40 || y > this.H + 40) continue;
      const m = this.matches(n);
      const dim = (near && !near.has(n.id)) || (this.query && !m);
      c.globalAlpha = dim ? 0.15 : 1;
      const r = n.r * Math.max(0.6, Math.min(1.6, k));
      c.fillStyle = this.colors[n.layer] || this.colors.muted;
      c.beginPath();
      if (n.type === 'commit' || n.type === 'branch') c.rect(x - r * 0.8, y - r * 0.8, r * 1.6, r * 1.6);
      else if (n.type === 'model') { c.moveTo(x, y - r); c.lineTo(x + r, y); c.lineTo(x, y + r); c.lineTo(x - r, y); c.closePath(); }
      else c.arc(x, y, r, 0, Math.PI * 2);
      c.fill();
      if (n.type === 'dir' || n.type === 'dangling') { c.fillStyle = this.colors.bg; c.beginPath(); c.arc(x, y, r * 0.45, 0, Math.PI * 2); c.fill(); }
      if (n.layers.length > 1) { c.strokeStyle = this.colors.text; c.globalAlpha *= 0.55; c.lineWidth = 1.2; c.beginPath(); c.arc(x, y, r + 2.5, 0, Math.PI * 2); c.stroke(); c.lineWidth = 1; }
      if (n.active || n.latest) { c.strokeStyle = this.colors.warning; c.globalAlpha = dim ? 0.2 : 1; c.lineWidth = 2; c.beginPath(); c.arc(x, y, r + 4, 0, Math.PI * 2); c.stroke(); c.lineWidth = 1; }
      if (m || n === focus || (near && near.has(n.id)) || (showLabels && (n.degree > 2 || k > 1.6)) || n.type === 'entity' || n.type === 'session') {
        c.globalAlpha = dim ? 0.25 : n === focus || m ? 1 : 0.8;
        c.fillStyle = n === focus || m ? this.colors.text : this.colors.muted;
        c.font = `${n === focus ? 600 : 400} 11px -apple-system, system-ui`;
        c.fillText(n.label.length > 34 ? n.label.slice(0, 33) + '…' : n.label, x + r + 4, y + 3.5);
      }
    }
    c.globalAlpha = 1;
  },
  fit() {
    const vis = this.nodes.filter((n) => !n.hidden);
    if (!vis.length) return;
    const xs = vis.map((n) => n.x), ys = vis.map((n) => n.y);
    const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
    this.view.k = Math.min(2.2, 0.9 * Math.min(this.W / (x1 - x0 + 80), this.H / (y1 - y0 + 80)));
    this.view.x = -(x0 + x1) / 2; this.view.y = -(y0 + y1) / 2;
    this.draw();
  },
  focusMatch() {
    const n = this.nodes.find((x) => !x.hidden && this.matches(x));
    if (!n) return toast('no matching node');
    this.select(n);
    this.view.x = -n.x; this.view.y = -n.y; this.view.k = Math.max(this.view.k, 1.3);
    this.draw();
  },

  // ───── interaction ─────
  hit(sx, sy) {
    let best = null, bd = 14;
    for (const n of this.nodes) {
      if (n.hidden) continue;
      const [x, y] = this.toScreen(n);
      const d = Math.hypot(x - sx, y - sy);
      if (d < Math.max(bd, n.r)) { best = n; bd = d; }
    }
    return best;
  },
  bindPointer() {
    const cv = this.canvas;
    let start = null, moved = false;
    cv.addEventListener('pointerdown', (e) => {
      cv.setPointerCapture(e.pointerId);
      const n = this.hit(e.offsetX, e.offsetY);
      start = { x: e.offsetX, y: e.offsetY, vx: this.view.x, vy: this.view.y, n };
      moved = false;
      if (n) this.dragNode = n;
      cv.classList.add('dragging');
    });
    cv.addEventListener('pointermove', (e) => {
      if (!start) {
        const n = this.hit(e.offsetX, e.offsetY);
        if (n !== this.hover) { this.hover = n; cv.style.cursor = n ? 'pointer' : ''; this.draw(); }
        return;
      }
      if (Math.hypot(e.offsetX - start.x, e.offsetY - start.y) > 3) moved = true;
      if (this.dragNode) { [this.dragNode.x, this.dragNode.y] = this.toWorld(e.offsetX, e.offsetY); this.kick(0.15); }
      else { this.view.x = start.vx + (e.offsetX - start.x) / this.view.k; this.view.y = start.vy + (e.offsetY - start.y) / this.view.k; this.draw(); }
    });
    cv.addEventListener('pointerup', () => {
      if (!moved) this.select(start?.n || null);
      start = null; this.dragNode = null; cv.classList.remove('dragging');
    });
    cv.addEventListener('dblclick', (e) => {
      const n = this.hit(e.offsetX, e.offsetY);
      if (n?.type === 'file') this.route('editor', { path: n.path });
    });
    cv.addEventListener('wheel', (e) => {
      e.preventDefault();
      const [wx, wy] = this.toWorld(e.offsetX, e.offsetY);
      this.view.k = Math.max(0.15, Math.min(5, this.view.k * Math.exp(-e.deltaY * 0.0015)));
      const [nx, ny] = this.toWorld(e.offsetX, e.offsetY);
      this.view.x += nx - wx; this.view.y += ny - wy;
      this.draw();
    }, { passive: false });
  },
  select(n) {
    this.selected = n;
    this.draw();
    const ins = this.inspector;
    if (!n) { ins.hidden = true; return; }
    ins.hidden = false;
    const neighbors = (this.adj[n.id] || []).map((e) => ({ e, o: e.s === n ? e.t : e.s }));
    const groups = {};
    for (const { e, o } of neighbors) (groups[e.kind] ||= []).push(o);
    const meta = [n.layer, n.type, n.rel, n.when, n.entityType, n.turns != null && `${n.turns} turns`, n.source, n.active && 'active now']
      .filter(Boolean).join(' · ');
    ins.replaceChildren(...[
      h('h4', {}, n.label),
      h('div', { class: 'meta' }, meta),
      ...(n.observations || []).map((o) => h('div', { class: 'obs' }, o)),
      n.subject ? h('div', { class: 'obs' }, n.subject) : null,
      n.type === 'file' ? h('button', { class: 'btn sm primary', onclick: () => this.route('editor', { path: n.path }) }, 'Open in editor') : null,
      n.type === 'session' && n.source === 'ai-os' ? h('button', { class: 'btn sm', onclick: () => this.route('chat', { id: n.id.slice(7) }) }, 'Open chat') : null,
      ...Object.entries(groups).map(([kind, os]) => [
        h('div', { class: 'meta', style: 'margin:10px 0 0' }, `${kind} (${os.length})`),
        h('ul', {}, ...os.slice(0, 30).map((o) => h('li', { onclick: () => { this.select(o); this.view.x = -o.x; this.view.y = -o.y; this.draw(); } },
          h('span', { class: 'dot', style: `background:${this.colors[o.layer]}` }), o.label))),
      ]).flat(),
    ].filter(Boolean));
  },
};
export default plane;
