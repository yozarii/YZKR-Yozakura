// YZKR AI-OS shell: plane registry, router (hash + yzkr:// native bridge), global shortcuts, ⌘K palette.
import { api, h, ICONS, KIND_ICON, on, toast, emit } from './core.js';
import editor from './planes/editor.js';
import graph from './planes/graph.js';
import home from './planes/home.js';
import chat from './planes/chat.js';

const PLANES = [editor, graph, home, chat];
const byId = Object.fromEntries(PLANES.map((p) => [p.id, p]));
const state = { current: null, root: '' };

// ───────── layout ─────────
const nav = document.getElementById('plane-nav');
const planesEl = document.getElementById('planes');
for (const p of PLANES) {
  const btn = h('button', { class: 'nav-btn', 'aria-label': p.title, onclick: () => route(p.id) });
  btn.innerHTML = ICONS[p.icon];
  btn.append(h('span', { class: 'tip' }, p.title, h('kbd', {}, p.shortcut)));
  p.navBtn = btn;
  nav.append(btn);
  p.el = h('div', { class: `plane ${p.id}-plane` });
  planesEl.append(p.el);
}
const palBtn = document.getElementById('palette-btn');
palBtn.innerHTML = ICONS.palette;
palBtn.onclick = () => openPalette();

on('subtitle', ({ plane, text }) => { if (plane === state.current) document.getElementById('plane-sub').textContent = text || ''; });
on('status', (text) => { document.getElementById('status-pill').textContent = text; });

// ───────── router ─────────
const mounted = new Set();
export function route(name, params = {}) {
  if (name === 'palette') return openPalette(params.q);
  const p = byId[name] || byId.home;
  if (!mounted.has(p.id)) { p.mount(p.el, { route, root: state.root }); mounted.add(p.id); }
  for (const q of PLANES) {
    q.el.classList.toggle('active', q === p);
    q.navBtn.classList.toggle('active', q === p);
  }
  const changed = state.current !== p.id;
  state.current = p.id;
  document.getElementById('plane-title').textContent = p.title;
  if (changed) document.getElementById('plane-sub').textContent = '';
  p.show?.(params);
  const hash = `#/${p.id}`;
  if (location.hash.split('?')[0] !== hash) history.replaceState(null, '', hash);
  return true;
}

function parseRoute(str) {
  const m = str.match(/^(?:yzkr:\/\/|#\/?)([\w-]*)\/?\??(.*)$/);
  if (!m) return null;
  return [m[1] || 'home', Object.fromEntries(new URLSearchParams(m[2]))];
}

// Native shell calls window.YZKR.openURL('yzkr://editor?path=…') for URL-scheme launches.
window.YZKR = {
  route,
  // URLs that arrive before boot finishes are replayed after it (boot would otherwise override them with the hash)
  openURL(url) { const r = parseRoute(url); if (!r) return; state.booted ? route(...r) : (state.pending = r); },
};
window.addEventListener('hashchange', () => { const r = parseRoute(location.hash); if (r && r[0] !== state.current) route(...r); });

// ───────── shortcuts ─────────
const SHORTCUTS = {
  k: () => (palette.hidden ? openPalette() : closePalette()),
  e: () => route('graph'),
  t: () => { route('chat'); emit('focus-terminal'); },
  1: () => route('editor'), 2: () => route('graph'), 3: () => route('home'), 4: () => route('chat'),
};
window.addEventListener('keydown', (e) => {
  if (!e.metaKey || e.altKey || e.ctrlKey) return;
  const fn = SHORTCUTS[e.key.toLowerCase()];
  if (fn && !(e.shiftKey && e.key.toLowerCase() !== 'k')) { e.preventDefault(); e.stopPropagation(); fn(); }
}, true);

// ───────── palette ─────────
const palette = document.getElementById('palette');
const input = document.getElementById('palette-input');
const list = document.getElementById('palette-list');
const spin = document.getElementById('palette-spin');
let items = [], sel = 0, searchSeq = 0;

const COMMANDS = [
  ...PLANES.map((p) => ({ title: `Go to ${p.title}`, sub: p.blurb, key: p.shortcut, kind: 'plane', run: () => route(p.id) })),
  { title: 'New chat', sub: 'Start a fresh conversation', kind: 'cmd', run: () => route('chat', { new: '1' }) },
  { title: 'Focus terminal', sub: 'Embedded zsh', key: '⌘T', kind: 'cmd', run: () => { route('chat'); emit('focus-terminal'); } },
  { title: 'Record macro…', sub: 'Describe steps in plain language', kind: 'macro', run: (q) => route('home', { record: q || '' }) },
  { title: 'Reindex spotlight', sub: 'Apps, files, repos, macros, bookmarks', kind: 'cmd',
    run: async () => { const r = await api('/api/spotlight/index', {}); toast(`Indexed ${r.docs} items in ${r.seconds}s`, 'ok'); } },
  { title: 'Rebuild graph', sub: 'Memory · filesystem · chat', kind: 'cmd', run: () => route('graph', { force: '1' }) },
];

function fuzzy(q, text) {
  q = q.toLowerCase(); text = text.toLowerCase();
  if (!q) return 1;
  if (text.includes(q)) return 3 - text.indexOf(q) / 100;
  let i = 0;
  for (const c of text) if (c === q[i]) i++;
  return i === q.length ? 1 : 0;
}

function openPalette(q = '') {
  palette.hidden = false;
  input.value = q;
  input.focus();
  input.select();
  refresh();
}
function closePalette() { palette.hidden = true; }
palette.addEventListener('mousedown', (e) => { if (e.target === palette) closePalette(); });

function render() {
  list.replaceChildren();
  let lastSect = null;
  items.forEach((it, i) => {
    if (it.sect !== lastSect) { list.append(h('li', { class: 'sect' }, it.sect)); lastSect = it.sect; }
    const li = h('li', { class: i === sel ? 'sel' : '', role: 'option', onmousemove: () => { if (sel !== i) { sel = i; render(); } }, onclick: () => choose(i) },
      h('span', { class: 'ico' }, it.icon || KIND_ICON[it.kind] || '•'),
      h('span', { class: 't' }, it.title),
      h('span', { class: 's' }, it.sub || ''),
      it.key ? h('kbd', {}, it.key) : h('span', { class: 'k' }, it.kind));
    list.append(li);
  });
  list.querySelector('.sel')?.scrollIntoView({ block: 'nearest' });
}

async function refresh() {
  const q = input.value.trim();
  const cmds = COMMANDS.map((c) => ({ ...c, score: fuzzy(q, `${c.title} ${c.sub}`), sect: 'Commands' }))
    .filter((c) => c.score > 0).sort((a, b) => b.score - a.score).slice(0, q ? 4 : 8);
  const ask = q ? [{ title: `Ask: “${q}”`, sub: 'Resolve with local AI (⌘↵) or send to chat (⇥)', kind: 'cmd', icon: '✦', sect: 'Intent', resolve: true }] : [];
  items = [...ask, ...cmds];
  sel = 0;
  render();
  if (q.length < 2) return;
  const seq = ++searchSeq;
  const hits = await api(`/api/spotlight/search?q=${encodeURIComponent(q)}&limit=10`).catch(() => []);
  if (seq !== searchSeq || palette.hidden) return;
  const docs = hits.map((d) => ({ title: d.title, sub: d.snippet && d.kind !== 'app' ? d.uri.replace(/^\/Users\/[^/]+/, '~') : d.uri.replace(/^\/Users\/[^/]+/, '~'), kind: d.kind, doc: d, sect: 'Spotlight' }));
  // strong lexical hits outrank the generic "Ask" row
  const strong = docs.length && hits[0].score >= 12;
  items = strong ? [...docs.slice(0, 1), ...ask, ...cmds, ...docs.slice(1)] : [...ask, ...cmds, ...docs];
  items.forEach((it) => { if (it === docs[0] && strong) it.sect = 'Top hit'; });
  render();
}

// In-app handlers for resolved actions; anything not handled here executes server-side (open apps, URLs, macros…).
const INAPP = {
  edit: (doc) => doc && state.root && doc.uri.startsWith(state.root) && route('editor', { path: doc.uri }),
  graph: (doc, q) => route('graph', { focus: doc?.title || '', q: doc ? '' : q }),
  ask_ai: (doc, q) => route('chat', { q }),
  record_macro: (doc, q) => route('home', { record: q }),
  open: (doc) => doc?.uri.startsWith('yzkr://') && route(...parseRoute(doc.uri)),
};

async function runDoc(doc, action, q) {
  if (INAPP[action || doc.action]?.(doc, q)) return;
  const r = await api('/api/spotlight/exec', { doc, action, q });
  toast(`${r.action} → ${r.target || r.opened || 'done'}`, r.ok === false ? 'error' : 'ok');
}

async function resolveAndRun(q) {
  spin.hidden = false;
  try {
    const res = await api('/api/spotlight/resolve', { q });
    closePalette();
    if (INAPP[res.action]?.(res.target, res.query)) return;
    if (res.target) return runDoc(res.target, res.action, res.query);
    const r = await api('/api/spotlight/exec', { doc: null, action: res.action, q: res.query }).catch((e) => ({ ok: false, error: e.message }));
    toast(r.error ? r.error : `${res.action}: ${q}`, r.error ? 'error' : 'ok');
  } catch (e) {
    toast(e.message, 'error');
  } finally { spin.hidden = true; }
}

async function choose(i, { resolve = false } = {}) {
  const it = items[i];
  const q = input.value.trim();
  if (!it) return q && resolveAndRun(q);
  if (resolve || it.resolve) return resolveAndRun(q);
  closePalette();
  try {
    if (it.doc) await runDoc(it.doc, null, q);
    else await it.run(q);
  } catch (e) { toast(e.message, 'error'); }
}

input.addEventListener('input', refresh);
input.addEventListener('keydown', (e) => {
  if (e.key === 'ArrowDown') { sel = Math.min(sel + 1, items.length - 1); render(); e.preventDefault(); }
  else if (e.key === 'ArrowUp') { sel = Math.max(sel - 1, 0); render(); e.preventDefault(); }
  else if (e.key === 'Enter') { e.preventDefault(); choose(sel, { resolve: e.metaKey }); }
  else if (e.key === 'Tab') { e.preventDefault(); const q = input.value.trim(); closePalette(); if (q) route('chat', { q }); }
  else if (e.key === 'Escape') { e.preventDefault(); closePalette(); }
});

// ───────── boot ─────────
(async () => {
  try { state.root = (await api('/api/fs/list')).path; } catch (e) { toast(`backend: ${e.message}`, 'error'); }
  route(...(state.pending || parseRoute(location.hash) || ['home', {}]));
  state.booted = true;
})();
