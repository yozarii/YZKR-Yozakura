// Plane 4 — Unified AI control chat + embedded terminal (⌘T).
// Chat routes between Claude and local Ollama (auto or pinned); `!cmd` runs in the terminal; `cls`//cls clears.
import { api, emit, h, on, renderMarkdown, stream, TOKEN, toast } from '../core.js';

const plane = {
  id: 'chat', title: 'Chat & Terminal', icon: 'chat', shortcut: '⌘T', blurb: 'Claude · Ollama · zsh',
  chatId: null, busy: false,

  mount(el) {
    this.messages = h('div', { class: 'messages' });
    this.provider = h('select', { class: 'input', title: 'Model routing' }, h('option', { value: 'auto' }, 'Auto route'));
    this.chatSel = h('select', { class: 'input', style: 'max-width:220px', onchange: (e) => this.loadChat(e.target.value) });
    this.input = h('textarea', { rows: 1, placeholder: 'Message…   (!cmd runs in terminal · cls clears)', oninput: () => this.autosize(), onkeydown: (e) => this.onKey(e) });
    this.termToggle = h('button', { class: 'btn sm', title: 'Show/hide terminal', onclick: () => this.toggleTerm() }, '▣ Terminal');
    const chatCol = h('div', { class: 'chat-col' },
      h('div', { class: 'toolbar' }, this.chatSel, h('button', { class: 'btn sm', onclick: () => this.newChat() }, '+ New'),
        h('span', { class: 'grow' }), this.provider, this.termToggle),
      this.messages,
      h('div', { class: 'composer' }, this.input,
        h('div', { class: 'hint' }, h('span', {}, h('kbd', {}, '↵'), ' send'), h('span', {}, h('kbd', {}, '⇧↵'), ' newline'),
          h('span', {}, h('kbd', {}, '!'), ' terminal'), h('span', {}, h('kbd', {}, 'cls'), ' clear'), h('span', {}, h('kbd', {}, '⌘T'), ' focus terminal'))));
    this.splitter = h('div', { class: 'splitter', role: 'separator', 'aria-orientation': 'vertical' });
    this.termHost = h('div', { class: 'term-host' });
    this.termStatus = h('span', { class: 'muted', style: 'font-size:12px' }, 'connecting…');
    this.termCol = h('div', { class: 'term-col' },
      h('div', { class: 'toolbar', style: 'background:var(--bg-elevated)' }, h('span', { class: 'mono', style: 'font-size:12px' }, 'zsh — ~/YZKR'), h('span', { class: 'grow' }), this.termStatus,
        h('button', { class: 'btn sm ghost', title: 'Restart shell', onclick: () => this.connectTerm(true) }, '↻')),
      this.termHost);
    el.append(chatCol, this.splitter, this.termCol);
    this.bindSplitter(chatCol);
    this.initTerm();
    this.loadProviders();
    this.refreshChats();
    this.renderEmpty();
    on('focus-terminal', () => { this.toggleTerm(true); setTimeout(() => this.term.focus(), 30); });
    on('terminal-paste', ({ cmd, run }) => { this.toggleTerm(true); this.termSend(cmd + (run ? '\r' : '')); setTimeout(() => this.term.focus(), 30); });
  },

  show(params) {
    setTimeout(() => this.fit(), 0);
    if (params.new) this.newChat();
    if (params.id) this.loadChat(params.id);
    if (params.term) emit('terminal-paste', { cmd: params.term, run: false });
    if (params.q) { this.input.value = params.q; this.autosize(); this.send(); }
    else if (!params.term) this.input.focus();
  },

  // ───── terminal ─────
  initTerm() {
    const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
    this.term = new window.Terminal({
      fontFamily: "'SF Mono', 'JetBrains Mono', Menlo, monospace", fontSize: 12.5, lineHeight: 1.2, cursorBlink: true, allowProposedApi: false,
      macOptionIsMeta: true, scrollback: 5000,
      theme: { background: '#08080c', foreground: css('--text'), cursor: css('--accent'), selectionBackground: '#a78bfa55',
        black: '#171722', red: css('--danger'), green: css('--success'), yellow: css('--warning'), blue: css('--info'), magenta: css('--accent'),
        cyan: '#67e8f9', white: css('--text'), brightBlack: css('--text-muted') },
    });
    this.fitAddon = new window.FitAddon.FitAddon();
    this.term.loadAddon(this.fitAddon);
    this.term.open(this.termHost);
    this.term.onData((d) => this.termSend(d));
    // ⌘K/⌘E/⌘T must reach the app, not the shell
    this.term.attachCustomKeyEventHandler((e) => !(e.metaKey && ['k', 'e', 't', '1', '2', '3', '4'].includes(e.key.toLowerCase())));
    new ResizeObserver(() => this.fit()).observe(this.termHost);
    this.connectTerm();
  },
  connectTerm(restart) {
    if (restart) { this.ws?.close(); this.term.reset(); }
    const ws = new WebSocket(`ws://${location.host}/ws/term?token=${encodeURIComponent(TOKEN)}`);
    this.ws = ws;
    ws.onopen = () => { this.termStatus.textContent = '● live'; this.fit(true); };
    ws.onmessage = (e) => this.term.write(e.data);
    ws.onclose = () => { if (this.ws === ws) { this.termStatus.textContent = '○ exited — ↻ to restart'; } };
  },
  termSend(d) { if (this.ws?.readyState === 1) this.ws.send(JSON.stringify({ t: 'i', d })); },
  fit(force) {
    if (this.termCol.classList.contains('collapsed') || !this.termHost.offsetWidth) return;
    try { this.fitAddon.fit(); } catch { return; }
    const { cols, rows } = this.term;
    if (force || cols !== this._cols || rows !== this._rows) {
      this._cols = cols; this._rows = rows;
      if (this.ws?.readyState === 1) this.ws.send(JSON.stringify({ t: 'r', c: cols, r: rows }));
    }
  },
  toggleTerm(force) {
    const show = force ?? this.termCol.classList.contains('collapsed');
    this.termCol.classList.toggle('collapsed', !show);
    this.splitter.hidden = !show;
    this.termToggle.classList.toggle('primary', show);
    setTimeout(() => this.fit(true), 0);
  },
  bindSplitter(left) {
    this.splitter.addEventListener('pointerdown', (e) => {
      this.splitter.setPointerCapture(e.pointerId);
      this.splitter.classList.add('drag');
      const total = left.parentElement.getBoundingClientRect();
      const move = (ev) => { const pct = Math.min(80, Math.max(25, ((ev.clientX - total.left) / total.width) * 100)); left.style.flex = `0 0 ${pct}%`; this.termCol.style.flex = '1 1 0'; this.fit(); };
      const up = () => { this.splitter.classList.remove('drag'); this.splitter.removeEventListener('pointermove', move); this.splitter.removeEventListener('pointerup', up); };
      this.splitter.addEventListener('pointermove', move);
      this.splitter.addEventListener('pointerup', up);
    });
  },

  // ───── chat ─────
  async loadProviders() {
    try {
      const { providers, claude_configured } = await api('/api/chat/providers');
      for (const p of providers) {
        const label = p.kind === 'claude' && !claude_configured ? `${p.label} (needs key)` : p.label;
        this.provider.append(h('option', { value: p.id }, `${p.kind === 'claude' ? '✦' : '◇'} ${label}`));
      }
    } catch (e) { toast(`providers: ${e.message}`, 'error'); }
  },
  async refreshChats() {
    const chats = await api('/api/chat/list').catch(() => []);
    this.chatSel.replaceChildren(h('option', { value: '' }, this.chatId ? '— history —' : 'New conversation'),
      ...chats.map((c) => h('option', { value: c.id, selected: c.id === this.chatId }, c.title)));
  },
  newChat() {
    this.chatId = null;
    this.renderEmpty();
    this.refreshChats();
    emit('subtitle', { plane: 'chat', text: 'new conversation' });
    this.input.focus();
  },
  async loadChat(id) {
    if (!id) return this.newChat();
    this.chatId = id;
    const msgs = await api(`/api/chat/history?id=${encodeURIComponent(id)}`);
    this.messages.replaceChildren();
    for (const m of msgs) {
      if (m.role === 'user') this.addUser(m.content);
      else { const b = this.addAssistant(m.model?.startsWith('claude') ? 'claude' : 'ollama', m.model || ''); this.setBody(b, m.content); }
    }
    this.refreshChats();
    emit('subtitle', { plane: 'chat', text: msgs.find((m) => m.role === 'user')?.content.slice(0, 60) || id });
    this.scroll();
  },
  renderEmpty() {
    this.messages.replaceChildren(h('div', { class: 'empty' }, h('h3', {}, 'One pane for every model'),
      'Auto routes quick asks to local Ollama, heavier work to Claude, and shell intents to a command you can run in the terminal.'));
  },
  autosize() { this.input.style.height = 'auto'; this.input.style.height = Math.min(220, this.input.scrollHeight) + 'px'; },
  onKey(e) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); this.send(); }
  },
  scroll() { this.messages.scrollTop = this.messages.scrollHeight; },
  addUser(text) {
    this.messages.querySelector('.empty')?.remove();
    this.messages.append(h('div', { class: 'msg user' }, h('div', { class: 'bubble' }, text)));
    this.scroll();
  },
  addAssistant(kind, label) {
    this.messages.querySelector('.empty')?.remove();
    const who = h('div', { class: 'who' }, h('span', { class: `route ${kind}` }, label || kind));
    const body = h('div', { class: 'md typing' });
    this.messages.append(h('div', { class: 'msg' }, who, body));
    this.scroll();
    return body;
  },
  setBody(body, text) {
    body.innerHTML = renderMarkdown(text);
    for (const pre of body.querySelectorAll('pre')) {
      const code = pre.textContent.trim();
      pre.append(h('div', { class: 'pre-actions' },
        h('button', { class: 'btn sm', onclick: () => navigator.clipboard.writeText(code).then(() => toast('Copied')) }, 'Copy'),
        h('button', { class: 'btn sm', title: 'Paste into terminal (not executed)', onclick: () => emit('terminal-paste', { cmd: code, run: false }) }, '▸ Terminal')));
    }
  },

  async send() {
    const text = this.input.value.trim();
    if (!text || this.busy) return;
    this.input.value = ''; this.autosize();
    // local verbs: an explicit leading ! means the user typed a command → run it
    if (/^(cls|\/cls|clear|\/clear)$/i.test(text)) { this.messages.replaceChildren(); this.term.clear(); return this.newChat(); }
    if (/^\/new$/i.test(text)) return this.newChat();
    if (text.startsWith('!')) { emit('terminal-paste', { cmd: text.slice(1).trim(), run: true }); return; }

    this.busy = true;
    this.addUser(text);
    let body = null, acc = '', raf = 0;
    const flush = () => { raf = 0; this.setBody(body, acc); body.classList.add('typing'); this.scroll(); };
    try {
      for await (const ev of stream('/api/chat/send', { id: this.chatId, text, provider: this.provider.value })) {
        if (ev.t === 'chat') { if (!this.chatId) { this.chatId = ev.id; this.refreshChats(); } }
        else if (ev.t === 'route') {
          const kind = ev.provider.split(':')[0];
          body = this.addAssistant(kind, ev.provider === 'terminal' ? 'terminal' : `${ev.label}${this.provider.value === 'auto' ? ' · auto' : ''}`);
        } else if (ev.t === 'delta') { acc += ev.text; if (!raf) raf = requestAnimationFrame(flush); }
        else if (ev.t === 'command') {
          body.classList.remove('typing');
          body.append(h('div', { class: 'cmd-card' }, h('code', {}, ev.cmd),
            h('button', { class: 'btn sm', onclick: () => emit('terminal-paste', { cmd: ev.cmd, run: false }) }, 'Paste'),
            h('button', { class: 'btn sm primary', onclick: () => emit('terminal-paste', { cmd: ev.cmd, run: true }) }, '▶ Run')));
        } else if (ev.t === 'error') { (body || this.addAssistant('error', 'error')).append(h('div', { class: 'err' }, ev.msg)); }
      }
    } catch (e) { (body || this.addAssistant('error', 'error')).append(h('div', { class: 'err' }, e.message)); }
    if (raf) cancelAnimationFrame(raf);
    if (body && acc) this.setBody(body, acc);
    body?.classList.remove('typing');
    this.busy = false;
    this.scroll();
  },
};
export default plane;
