// Shared helpers for every plane: authenticated API, toasts, safe markdown, icons, event bus.
export const TOKEN = document.querySelector('meta[name="yzkr-token"]').content;
export const bus = new EventTarget();
export const emit = (type, detail) => bus.dispatchEvent(new CustomEvent(type, { detail }));
export const on = (type, fn) => bus.addEventListener(type, (e) => fn(e.detail));

export async function api(path, body, { method } = {}) {
  const res = await fetch(path, {
    method: method || (body === undefined ? 'GET' : 'POST'),
    headers: { 'X-YZKR-Token': TOKEN, ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || `${res.status} ${res.statusText}`);
  return data;
}

export const qs = (o) => new URLSearchParams(Object.entries(o).filter(([, v]) => v != null && v !== '')).toString();

/** POST and yield parsed ndjson events as they stream in. */
export async function* stream(path, body) {
  const res = await fetch(path, {
    method: 'POST', body: JSON.stringify(body),
    headers: { 'X-YZKR-Token': TOKEN, 'Content-Type': 'application/json' },
  });
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
  const reader = res.body.getReader();
  const dec = new TextDecoder();
  let buf = '';
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += dec.decode(value, { stream: true });
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i).trim();
      buf = buf.slice(i + 1);
      if (line) yield JSON.parse(line);
    }
  }
}

export function toast(text, kind = '') {
  const el = h('div', { class: `toast ${kind}` }, text);
  document.getElementById('toasts').append(el);
  setTimeout(() => el.remove(), kind === 'error' ? 6000 : 3200);
}

/** Tiny hyperscript: h('div', {class, onclick, ...}, ...children) */
export function h(tag, attrs = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'html') el.innerHTML = v;
    else if (k in el && typeof v !== 'string') el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) el.append(kid instanceof Node ? kid : String(kid));
  return el;
}

export const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

// Markdown: raw HTML dropped, only http(s)/mailto/relative links — model output is untrusted.
const SAFE_URL = /^(https?:|mailto:|#|\/(?!\/)|\.{0,2}\/?[\w.-])/i;
window.marked.use({
  gfm: true,
  breaks: false,
  renderer: {
    html: () => '',
    link({ href, title, tokens }) {
      const text = this.parser.parseInline(tokens);
      if (!SAFE_URL.test(href || '')) return text;
      return `<a href="${esc(href)}"${title ? ` title="${esc(title)}"` : ''} target="_blank" rel="noopener noreferrer">${text}</a>`;
    },
    image({ href, text }) {
      return /^https?:/i.test(href || '') ? `<img src="${esc(href)}" alt="${esc(text)}" style="max-width:100%">` : esc(text);
    },
  },
});
// DOMPurify as defense-in-depth on top of the renderer rules (and the page CSP blocks inline script).
export const renderMarkdown = (src) => window.DOMPurify.sanitize(window.marked.parse(src || ''), { ADD_ATTR: ['target'] });

export const rel = (p, root) => (root && p.startsWith(root) ? p.slice(root.length + 1) : p);

const svg = (d) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${d}</svg>`;
export const ICONS = {
  editor: svg('<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M9 13h6M9 17h4"/>'),
  graph: svg('<circle cx="6" cy="6" r="2.5"/><circle cx="18" cy="8" r="2.5"/><circle cx="9" cy="18" r="2.5"/><path d="M8.2 7.2 15.6 7.7M7 8.4l1.4 7.2M16.5 10.1l-5.6 6.2"/>'),
  home: svg('<path d="M4 13a8 8 0 1 1 16 0"/><path d="M12 13l4-4"/><circle cx="12" cy="13" r="1.3"/><path d="M5 19h14"/>'),
  chat: svg('<path d="M4 5h16v10H9l-5 4z"/><path d="M8 9.5l2 1.5-2 1.5M12.5 12.5H15"/>'),
  palette: svg('<path d="M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z"/>'),
};

export const KIND_ICON = {
  app: '◆', note: '✎', file: '▤', script: '❯', folder: '▸', repo: '⎇', alias: '$', macro: '⚡', bookmark: '☆',
  memory: '◉', plane: '◈', cmd: '⌘',
};
