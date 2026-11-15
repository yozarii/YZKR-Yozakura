'use strict';

const {
  app, BrowserWindow, Menu, ipcMain, shell, session, nativeImage, net,
  screen, clipboard, dialog, components, desktopCapturer,
} = require('electron');
const path = require('node:path');
const fs = require('node:fs');
const crypto = require('node:crypto');

/* ------------------------------------------------------------------ *
 * Site configuration
 * ------------------------------------------------------------------ */

function parseFlags(argv) {
  const flags = {};
  for (const arg of argv.slice(1)) {
    if (!arg.startsWith('--')) continue;
    const eq = arg.indexOf('=');
    if (eq === -1) flags[arg.slice(2)] = true;
    else flags[arg.slice(2, eq)] = arg.slice(eq + 1);
  }
  return flags;
}

const flags = parseFlags(process.argv);

function slugify(value) {
  return String(value || '')
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64) || 'site';
}

function readJSON(file) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
}

function normalizeSite(raw) {
  const site = { ...(raw || {}) };
  if (!site.url && flags.url) site.url = flags.url;
  if (!site.name && flags.name) site.name = flags.name;
  if (!site.url) site.url = 'https://example.com';
  try {
    const parsed = new URL(site.url);
    site.host = parsed.host.replace(/^www\./, '');
  } catch {
    site.host = site.slug || slugify(site.name || 'site');
  }
  if (!site.name) site.name = site.host;
  site.slug = site.slug || slugify(site.name);
  site.drm = Boolean(site.drm);
  return site;
}

function loadSiteConfig() {
  const candidates = [];
  if (process.env.ZENPWA_SITE) candidates.push(process.env.ZENPWA_SITE);
  if (process.resourcesPath) candidates.push(path.join(process.resourcesPath, 'site.json'));
  candidates.push(path.join(__dirname, '..', 'site.json'));
  for (const file of candidates) {
    const parsed = readJSON(file);
    if (parsed) return normalizeSite(parsed);
  }
  return normalizeSite({});
}

const site = loadSiteConfig();
const profileSlug = flags.profile ? slugify(flags.profile) : site.slug;

app.setName(site.name || 'Zen PWA');
const userDataDir = path.join(app.getPath('appData'), 'Zen PWA', profileSlug);
try { fs.mkdirSync(userDataDir, { recursive: true }); } catch {}
app.setPath('userData', userDataDir);

const nativeUserAgent = String(app.userAgentFallback || '');
app.userAgentFallback = nativeUserAgent
  .replace(/\sElectron\/[^\s]+/i, '')
  .replace(/\sZenPWA\/[^\s]+/i, '')
  .replace(/\szen-pwa\/[^\s]+/i, '');

/*
 * Google refuses sign-in when the browser looks like a spoofed Chrome:
 * a laundered UA lands on /signin/rejected ("This browser or app may not
 * be secure"), while Electron's own UA is accepted. Keep the clean UA for
 * every other site and send Google's identity hosts the real one.
 */
const GOOGLE_AUTH_HOST = /^(accounts|consent)\.google\.[a-z.]+$|^accounts\.youtube\.com$|^myaccount\.google\.[a-z.]+$/i;

app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');

/* ------------------------------------------------------------------ *
 * State
 * ------------------------------------------------------------------ */

const state = {
  window: null,
  widevine: { state: 'idle', version: null, error: null, path: null, raw: null },
  pointerNearTop: false,
  iconDataUrl: null,
  accent: site.accent || null,
};

const boundsFile = path.join(userDataDir, 'window.json');
const settingsFile = path.join(userDataDir, 'settings.json');
const settings = readJSON(settingsFile) || {};

function saveSettings() {
  try { fs.writeFileSync(settingsFile, JSON.stringify(settings, null, 2)); } catch {}
}

site.drm = site.drm || /(^|\.)(netflix\.com|primevideo\.com|disneyplus\.com|max\.com|hbomax\.com|spotify\.com|music\.apple\.com)$/i.test(site.host || '');

/* ------------------------------------------------------------------ *
 * Colour helpers (shared with the favicon accent sampler)
 * ------------------------------------------------------------------ */

function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  let h = 0;
  let s = 0;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
    else if (max === g) h = ((b - r) / d + 2) / 6;
    else h = ((r - g) / d + 4) / 6;
  }
  return { h: h * 360, s, l };
}

function hslToHex(h, s, l) {
  h = ((h % 360) + 360) % 360 / 360;
  s = Math.min(1, Math.max(0, s));
  l = Math.min(1, Math.max(0, l));
  let r, g, b;
  if (s === 0) {
    r = g = b = l;
  } else {
    const hue2rgb = (p, q, t) => {
      if (t < 0) t += 1;
      if (t > 1) t -= 1;
      if (t < 1 / 6) return p + (q - p) * 6 * t;
      if (t < 1 / 2) return q;
      if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
      return p;
    };
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    r = hue2rgb(p, q, h + 1 / 3);
    g = hue2rgb(p, q, h);
    b = hue2rgb(p, q, h - 1 / 3);
  }
  const to = (v) => Math.round(v * 255).toString(16).padStart(2, '0');
  return `#${to(r)}${to(g)}${to(b)}`;
}

function tidyAccent({ h, s, l }) {
  if (!Number.isFinite(h) || !Number.isFinite(s) || !Number.isFinite(l)) return null;
  if (s < 0.16 || l < 0.06 || l > 0.95) return null;
  const sat = Math.min(0.92, Math.max(0.46, s));
  const light = Math.min(0.64, Math.max(0.4, l));
  return hslToHex(h, sat, light);
}

const FALLBACK_PALETTE = [
  '#e5484d', '#f76b15', '#ffb224', '#46a758', '#12a594',
  '#0d74ce', '#3e63dd', '#8e4ec6', '#d6409f', '#e93d82',
];

function fallbackAccent(host) {
  const digest = crypto.createHash('sha1').update(String(host || 'site')).digest();
  return FALLBACK_PALETTE[digest[0] % FALLBACK_PALETTE.length];
}

function dominantColor(image) {
  if (!image || image.isEmpty()) return null;
  let bitmap;
  let size;
  try {
    bitmap = image.toBitmap();
    size = image.getSize();
  } catch {
    return null;
  }
  const scale = typeof image.getScaleFactors === 'function'
    ? (image.getScaleFactors()[0] || 1)
    : 1;
  const width = Math.max(1, Math.round(size.width * scale));
  const height = Math.max(1, Math.round(size.height * scale));
  const total = Math.min(width * height, Math.floor(bitmap.length / 4));
  if (!total) return null;

  // Electron hands us BGRA on macOS.
  const step = Math.max(1, Math.floor(total / 24000));
  const buckets = new Map();
  for (let i = 0; i < total; i += step) {
    const o = i * 4;
    const b = bitmap[o];
    const g = bitmap[o + 1];
    const r = bitmap[o + 2];
    const a = bitmap[o + 3];
    if (a < 160) continue;
    const hsl = rgbToHsl(r, g, b);
    if (hsl.s < 0.2 || hsl.l < 0.08 || hsl.l > 0.94) continue;
    const key = Math.floor(hsl.h / 12);
    const bucket = buckets.get(key) || { count: 0, h: 0, s: 0, l: 0, weight: 0 };
    const weight = hsl.s * hsl.s;
    bucket.count += 1;
    bucket.weight += weight;
    bucket.h += hsl.h * weight;
    bucket.s += hsl.s * weight;
    bucket.l += hsl.l * weight;
    buckets.set(key, bucket);
  }
  if (!buckets.size) return null;
  let best = null;
  for (const bucket of buckets.values()) {
    if (!best || bucket.weight > best.weight) best = bucket;
  }
  if (!best || !best.weight) return null;
  return tidyAccent({
    h: best.h / best.weight,
    s: best.s / best.weight,
    l: best.l / best.weight,
  });
}

async function fetchImageBuffer(url, timeoutMs = 7000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await net.fetch(url, {
      signal: controller.signal,
      headers: {
        'User-Agent': app.userAgentFallback,
        Accept: 'image/avif,image/webp,image/png,image/svg+xml,image/*,*/*;q=0.8',
      },
    });
    if (!response.ok) return null;
    const buffer = Buffer.from(await response.arrayBuffer());
    if (buffer.length > 4 * 1024 * 1024) return null;
    return buffer;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

async function accentFromUrls(urls) {
  const list = Array.isArray(urls) ? urls.filter((u) => /^https?:/i.test(String(u))).slice(0, 6) : [];
  if (site.host) list.push(`https://www.google.com/s2/favicons?domain=${encodeURIComponent(site.host)}&sz=128`);
  for (const url of list) {
    const buffer = await fetchImageBuffer(url);
    if (!buffer) continue;
    let image = nativeImage.createFromBuffer(buffer);
    if (image.isEmpty()) continue;
    const color = dominantColor(image);
    if (color) return color;
  }
  return null;
}

/* ------------------------------------------------------------------ *
 * Widevine (castLabs ECS)
 * ------------------------------------------------------------------ */

const WIDEVINE_MIN_VERSION = '4.10.2830.0';
let resolveWidevine;
const widevineReady = new Promise((resolve) => { resolveWidevine = resolve; });

function setWidevine(next) {
  state.widevine = { ...state.widevine, ...next };
  sendToShell('widevine', state.widevine);
}

async function prepareWidevine() {
  if (!components || typeof components.whenReady !== 'function') {
    setWidevine({
      state: 'error',
      error: 'This build is not running on castLabs Electron, so Widevine is unavailable.',
    });
    resolveWidevine();
    return;
  }
  setWidevine({ state: 'installing', error: null });
  try {
    await components.whenReady([components.WIDEVINE_CDM_ID], {
      components: {
        [components.WIDEVINE_CDM_ID]: { minimumVersion: WIDEVINE_MIN_VERSION },
      },
    });
    const status = typeof components.status === 'function' ? components.status() : null;
    const entry = status && (status[components.WIDEVINE_CDM_ID] || Object.values(status)[0]) || {};
    setWidevine({
      state: 'ready',
      version: entry.version || entry.cdmVersion || null,
      path: entry.path || null,
      raw: status,
      error: null,
    });
  } catch (error) {
    const detail = error && error.errors && error.errors.length
      ? error.errors.map((e) => e.message || e.code || String(e)).join('; ')
      : (error && error.message) || String(error);
    setWidevine({ state: 'error', error: detail });
  } finally {
    resolveWidevine();
  }
}

/* ------------------------------------------------------------------ *
 * Window helpers
 * ------------------------------------------------------------------ */

function isShellSender(event) {
  return Boolean(state.window && !state.window.isDestroyed() && event.sender === state.window.webContents);
}

function sendToShell(channel, payload) {
  if (!state.window || state.window.isDestroyed()) return;
  try { state.window.webContents.send(channel, payload); } catch {}
}

function loadIcon() {
  const candidates = [
    process.resourcesPath ? path.join(process.resourcesPath, 'SiteIcon.png') : null,
    path.join(__dirname, '..', 'assets', 'icon.png'),
  ].filter(Boolean);
  for (const file of candidates) {
    try {
      if (fs.existsSync(file)) {
        const buffer = fs.readFileSync(file);
        state.iconDataUrl = `data:image/png;base64,${buffer.toString('base64')}`;
        return;
      }
    } catch {}
  }
}

function readBounds() {
  const stored = readJSON(boundsFile);
  const display = screen.getPrimaryDisplay().workArea;
  const fallback = {
    width: Math.min(1180, Math.round(display.width * 0.78)),
    height: Math.min(780, Math.round(display.height * 0.82)),
  };
  fallback.x = Math.round(display.x + (display.width - fallback.width) / 2);
  fallback.y = Math.round(display.y + (display.height - fallback.height) / 2);
  if (!stored || !Number.isFinite(stored.width) || !Number.isFinite(stored.height)) {
    return { ...fallback, maximized: false, fullscreen: false };
  }
  const bounds = {
    x: stored.x, y: stored.y, width: stored.width, height: stored.height,
    maximized: Boolean(stored.maximized), fullscreen: Boolean(stored.fullscreen),
  };
  const area = screen.getDisplayMatching(bounds).workArea;
  if (bounds.x + 80 < area.x || bounds.x > area.x + area.width - 80) bounds.x = fallback.x;
  if (bounds.y + 80 < area.y || bounds.y > area.y + area.height - 80) bounds.y = fallback.y;
  bounds.width = Math.max(420, Math.min(bounds.width, area.width));
  bounds.height = Math.max(280, Math.min(bounds.height, area.height));
  return bounds;
}

let saveBoundsTimer = null;
function saveBounds() {
  if (!state.window || state.window.isDestroyed()) return;
  try {
    const normal = state.window.getNormalBounds();
    const payload = {
      ...normal,
      maximized: state.window.isMaximized(),
      fullscreen: state.window.isFullScreen(),
    };
    fs.writeFileSync(boundsFile, JSON.stringify(payload));
  } catch {}
}

function scheduleBoundsSave() {
  clearTimeout(saveBoundsTimer);
  saveBoundsTimer = setTimeout(saveBounds, 400);
}

function windowState() {
  const win = state.window;
  return {
    maximized: Boolean(win && win.isMaximized()),
    fullscreen: Boolean(win && win.isFullScreen()),
    alwaysOnTop: Boolean(win && win.isAlwaysOnTop()),
    focused: Boolean(win && win.isFocused()),
  };
}

function broadcastWindowState() {
  sendToShell('window-state', windowState());
}

function mixWithBackground(hex, ratio = 0.22) {
  const match = /^#?([0-9a-f]{6})$/i.exec(String(hex || ''));
  if (!match) return '#0e0e12';
  const value = parseInt(match[1], 16);
  const r = (value >> 16) & 255;
  const g = (value >> 8) & 255;
  const b = value & 255;
  const base = { r: 14, g: 14, b: 18 };
  const to = (channel, baseChannel) => Math.round(baseChannel * (1 - ratio) + channel * ratio);
  return `#${[to(r, base.r), to(g, base.g), to(b, base.b)]
    .map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

function createWindow() {
  const bounds = readBounds();
  const win = new BrowserWindow({
    x: bounds.x,
    y: bounds.y,
    width: bounds.width,
    height: bounds.height,
    minWidth: 420,
    minHeight: 280,
    frame: false,
    transparent: false,
    backgroundColor: '#0e0e12',
    roundedCorners: true,
    hasShadow: true,
    acceptFirstMouse: true,
    titleBarStyle: 'hidden',
    title: site.name,
    show: false,
    fullscreenable: true,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      webviewTag: true,
      spellcheck: true,
      backgroundThrottling: false,
    },
  });
  state.window = win;
  if (typeof win.setWindowButtonVisibility === 'function') win.setWindowButtonVisibility(false);

  win.loadFile(path.join(__dirname, 'shell', 'index.html'));
  win.once('ready-to-show', () => {
    win.show();
    if (bounds.maximized) win.maximize();
    if (bounds.fullscreen) win.setFullScreen(true);
    win.focus();
  });

  win.on('focus', () => { broadcastWindowState(); });
  win.on('blur', () => {
    broadcastWindowState();
    state.pointerNearTop = false;
    sendToShell('pointer', false);
  });
  win.on('enter-full-screen', () => sendToShell('fullscreen', true));
  win.on('leave-full-screen', () => sendToShell('fullscreen', false));
  win.on('maximize', broadcastWindowState);
  win.on('unmaximize', broadcastWindowState);
  win.on('always-on-top-changed', broadcastWindowState);
  win.on('moved', scheduleBoundsSave);
  win.on('resized', scheduleBoundsSave);
  win.on('close', saveBounds);
  win.on('closed', () => {
    state.window = null;
    stopPointerWatch();
  });
  win.webContents.on('will-navigate', (event, url) => {
    if (!/^https?:/i.test(url)) {
      event.preventDefault();
      shell.openExternal(url);
    }
  });
  startPointerWatch();
}

function startPointerWatch() {
  stopPointerWatch();
  state.pointerTimer = setInterval(() => {
    const win = state.window;
    if (!win || win.isDestroyed()) return;
    if (!win.isFocused()) {
      if (state.pointerNearTop) {
        state.pointerNearTop = false;
        sendToShell('pointer', false);
      }
      return;
    }
    const point = screen.getCursorScreenPoint();
    const frame = win.getBounds();
    const insideX = point.x >= frame.x - 4 && point.x <= frame.x + frame.width + 4;
    const near = insideX && point.y >= frame.y - 14 && point.y <= frame.y + 44;
    if (near !== state.pointerNearTop) {
      state.pointerNearTop = near;
      sendToShell('pointer', near);
    }
  }, 60);
}

function stopPointerWatch() {
  if (state.pointerTimer) clearInterval(state.pointerTimer);
  state.pointerTimer = null;
}

/* ------------------------------------------------------------------ *
 * Menus
 * ------------------------------------------------------------------ */

const isDev = Boolean(flags.dev) || process.env.ZENPWA_DEV === '1';

function sendCommand(command) {
  sendToShell('cmd', command);
}

function buildMenu() {
  const name = site.name || 'Zen PWA';
  const template = [
    {
      label: name,
      submenu: [
        { role: 'about', label: `About ${name}` },
        { type: 'separator' },
        {
          label: 'Always on Top',
          type: 'checkbox',
          checked: Boolean(state.window && state.window.isAlwaysOnTop()),
          click: (item) => {
            if (!state.window) return;
            state.window.setAlwaysOnTop(item.checked);
            settings.alwaysOnTop = item.checked;
            saveSettings();
          },
        },
        {
          label: 'Widevine Playback Setup…',
          click: () => showWidevineSetup(),
        },
        { label: 'Check Widevine…', click: () => sendCommand('widevine-check') },
        { type: 'separator' },
        { role: 'hide', label: `Hide ${name}` },
        { role: 'hideOthers' },
        { role: 'unhide' },
        { type: 'separator' },
        { role: 'quit', label: `Quit ${name}` },
      ],
    },
    {
      label: 'File',
      submenu: [
        { label: 'Open Current Page in Zen', accelerator: 'CmdOrCtrl+Shift+O', click: () => sendCommand('open-in-zen') },
        { label: 'Copy Current URL', accelerator: 'CmdOrCtrl+Shift+C', click: () => sendCommand('copy-url') },
        { type: 'separator' },
        { role: 'close', label: 'Close Window' },
      ],
    },
    { role: 'editMenu' },
    {
      label: 'View',
      submenu: [
        { label: 'Focus Address Bar', accelerator: 'CmdOrCtrl+L', click: () => sendCommand('focus-address') },
        { label: 'App Home', accelerator: 'CmdOrCtrl+Shift+H', click: () => sendCommand('home') },
        { type: 'separator' },
        { label: 'Reload', accelerator: 'CmdOrCtrl+R', click: () => sendCommand('reload') },
        { label: 'Force Reload', accelerator: 'CmdOrCtrl+Shift+R', click: () => sendCommand('hard-reload') },
        { label: 'Back', accelerator: 'CmdOrCtrl+[', click: () => sendCommand('back') },
        { label: 'Forward', accelerator: 'CmdOrCtrl+]', click: () => sendCommand('forward') },
        { type: 'separator' },
        { label: 'Actual Size', accelerator: 'CmdOrCtrl+0', click: () => sendCommand('zoom-reset') },
        { label: 'Zoom In', accelerator: 'CmdOrCtrl+Plus', click: () => sendCommand('zoom-in') },
        { label: 'Zoom Out', accelerator: 'CmdOrCtrl+-', click: () => sendCommand('zoom-out') },
        { type: 'separator' },
        { role: 'togglefullscreen' },
      ],
    },
    ...(isDev ? [{
      label: 'Develop',
      submenu: [
        { role: 'reload', label: 'Reload Shell' },
        { role: 'toggleDevTools' },
        { type: 'separator' },
        { label: 'Open Guest DevTools', click: () => sendCommand('guest-devtools') },
      ],
    }] : []),
    { role: 'windowMenu' },
  ];
  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
}

function showWidevineSetup() {
  const command = 'zen --pwa --sign';
  const message = state.widevine.state === 'ready'
    ? [
        'Widevine is installed and ready.',
        '',
        `CDM version: ${state.widevine.version || 'unknown'}`,
        '',
        'Netflix, Prime Video and similar services additionally require a production VMP signature. Castlabs provides that for free:',
        '',
        '1. zen --pwa --sign',
        '2. Create a castlabs EVS account when prompted (one time).',
        '3. Run zen --pwa --rebuild when the signature completes.',
      ].join('\n')
    : [
        'Widevine could not be prepared.',
        '',
        state.widevine.error || 'Unknown error.',
        '',
        `Run “${command}” in Terminal to install the signing tools and retry.`,
      ].join('\n');
  const buttons = state.widevine.state === 'ready'
    ? ['Copy Command', 'OK']
    : ['Copy Command', 'OK'];
  dialog.showMessageBox(state.window || undefined, {
    type: state.widevine.state === 'ready' ? 'info' : 'warning',
    title: 'Widevine Playback',
    message: state.widevine.state === 'ready' ? 'Widevine is ready' : 'Widevine needs attention',
    detail: message,
    buttons,
    defaultId: 1,
    cancelId: 1,
  }).then((result) => {
    if (result.response === 0) clipboard.writeText(command);
  });
}

/* ------------------------------------------------------------------ *
 * Sessions, popups, context menus
 * ------------------------------------------------------------------ */

const ALLOWED_PERMISSIONS = new Set([
  'fullscreen', 'media', 'audioCapture', 'videoCapture', 'clipboard-read',
  'clipboard-sanitized-write', 'notifications', 'idle-detection',
  'pointerLock', 'openExternal', 'geolocation',
]);

function setupSession() {
  const ses = session.defaultSession;
  ses.setUserAgent(app.userAgentFallback);
  // Google sign-in needs the untrimmed Electron UA; see GOOGLE_AUTH_HOST.
  ses.webRequest.onBeforeSendHeaders({ urls: ['<all_urls>'] }, (details, callback) => {
    try {
      if (GOOGLE_AUTH_HOST.test(new URL(details.url).hostname)) {
        details.requestHeaders['User-Agent'] = nativeUserAgent;
      }
    } catch {}
    callback({ requestHeaders: details.requestHeaders });
  });
  ses.setPermissionRequestHandler((_wc, permission, callback) => {
    callback(ALLOWED_PERMISSIONS.has(permission));
  });
  ses.setPermissionCheckHandler((_wc, permission) => ALLOWED_PERMISSIONS.has(permission));
  if (typeof ses.setDisplayMediaRequestHandler === 'function') {
    ses.setDisplayMediaRequestHandler((request, callback) => {
      desktopCapturer.getSources({ types: ['screen'] })
        .then((sources) => callback({ video: sources[0] }))
        .catch(() => callback({}));
    }, { useSystemPicker: true });
  }
  ses.on('will-download', (_event, item) => {
    notify(`Downloading ${item.getFilename()}`);
  });
}

function notify(message, kind = 'info') {
  sendToShell('toast', { message, kind });
}

function popupOptions() {
  return {
    width: 540,
    height: 740,
    minWidth: 380,
    minHeight: 420,
    titleBarStyle: 'hiddenInset',
    backgroundColor: '#141419',
    autoHideMenuBar: true,
    webPreferences: {
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      spellcheck: true,
    },
  };
}

function guestContextMenu(contents, params) {
  const template = [];
  if (params.linkURL) {
    template.push(
      { label: 'Open Link in Zen', click: () => openInZen(params.linkURL) },
      { label: 'Open Link in Default Browser', click: () => shell.openExternal(params.linkURL) },
      { label: 'Copy Link', click: () => clipboard.writeText(params.linkURL) },
      { type: 'separator' },
    );
  }
  if (params.isEditable) {
    template.push(
      { role: 'undo' }, { role: 'redo' }, { type: 'separator' },
      { role: 'cut' }, { role: 'copy' }, { role: 'paste' }, { role: 'selectAll' },
      { type: 'separator' },
    );
  } else if (params.selectionText) {
    template.push(
      { role: 'copy' },
      { label: 'Search with Zen', click: () => openInZen(`https://duckduckgo.com/?q=${encodeURIComponent(params.selectionText)}`) },
      { type: 'separator' },
    );
  }
  if (params.mediaType === 'video' || params.mediaType === 'audio') {
    template.push({
      label: 'Picture in Picture',
      click: () => contents.executeJavaScript('document.querySelector("video")?.requestPictureInPicture?.()').catch(() => {}),
    });
    template.push({ type: 'separator' });
  }
  template.push(
    { label: 'Back', enabled: contents.canGoBack(), click: () => contents.goBack() },
    { label: 'Forward', enabled: contents.canGoForward(), click: () => contents.goForward() },
    { label: 'Reload', click: () => contents.reload() },
  );
  if (isDev) {
    template.push(
      { type: 'separator' },
      { label: 'Inspect Element', click: () => contents.inspectElement(params.x, params.y) },
    );
  }
  const window = BrowserWindow.fromWebContents(contents);
  Menu.buildFromTemplate(template).popup({ window: window || undefined });
}

function openInZen(url) {
  if (!/^https?:/i.test(String(url))) return;
  const candidates = [
    '/Applications/Zen.app/Contents/MacOS/zen',
    '/Applications/Zen Browser.app/Contents/MacOS/zen',
  ];
  const binary = candidates.find((p) => fs.existsSync(p));
  if (binary) {
    try {
      const { spawn } = require('node:child_process');
      spawn(binary, [url], { detached: true, stdio: 'ignore' }).unref();
      return;
    } catch {}
  }
  shell.openExternal(url);
}

app.on('web-contents-created', (_event, contents) => {
  if (flags.selftest && contents.getType() === 'webview') {
    contents.once('did-finish-load', () => {
      setTimeout(async () => {
        const target = typeof flags.selftest === 'string' ? flags.selftest : '/tmp/zenpwa-selftest.png';
        try {
          if (state.window && !state.window.isDestroyed()) {
            await state.window.webContents.executeJavaScript(
              "(() => {" +
              "const bar = document.getElementById('toolbar');" +
              "if (bar) {" +
              "bar.classList.add('visible');" +
              "bar.setAttribute('aria-hidden','false');" +
              "bar.style.opacity = '1';" +
              "bar.style.transform = 'none';" +
              "bar.style.pointerEvents = 'auto';" +
              "}" +
              "document.body.classList.remove('booting');" +
              "})()",
            ).catch(() => {});
            await new Promise((resolve) => setTimeout(resolve, 600));
            const image = await state.window.webContents.capturePage();
            fs.writeFileSync(target, image.toPNG());
          }
          const theme = await contents.executeJavaScript(`(() => {
            const meta = document.querySelector('meta[name="theme-color"]');
            const style = getComputedStyle(document.body);
            return { themeColor: meta && meta.content, bg: style.backgroundColor };
          })()`).catch(() => null);
          const eme = await contents.executeJavaScript(`(async () => {
            if (!navigator.requestMediaKeySystemAccess) return 'unsupported';
            try {
              const access = await navigator.requestMediaKeySystemAccess('com.widevine.alpha', [{
                initDataTypes: ['cenc'],
                audioCapabilities: [{ contentType: 'audio/mp4; codecs="mp4a.40.2"' }],
                videoCapabilities: [{ contentType: 'video/mp4; codecs="avc1.42E01E"' }]
              }]);
              return 'ready:' + access.keySystem;
            } catch (error) {
              return 'failed:' + error.name + ':' + error.message;
            }
          })()`).catch((error) => `error:${error && error.message}`);
          console.log('SELFTEST ' + JSON.stringify({
            captured: target,
            title: contents.getTitle(),
            windowTitle: state.window && !state.window.isDestroyed() ? state.window.getTitle() : null,
            url: contents.getURL(),
            widevine: state.widevine,
            eme,
            accent: state.accent,
            guestTheme: theme,
            userAgent: await contents.executeJavaScript('navigator.userAgent').catch(() => null),
          }));
        } catch (error) {
          console.log('SELFTEST_ERROR ' + String((error && error.stack) || error));
        }
        app.exit(0);
      }, 2600);
    });
  }

  contents.setWindowOpenHandler(({ url, disposition }) => {    if (!/^https?:/i.test(url)) {
      if (/^(mailto|tel|sms):/i.test(url)) shell.openExternal(url);
      return { action: 'deny' };
    }
    if (disposition === 'save-to-disk') return { action: 'deny' };
    return { action: 'allow', overrideBrowserWindowOptions: popupOptions() };
  });

  contents.on('context-menu', (_e, params) => guestContextMenu(contents, params));

  contents.on('enter-html-full-screen', () => {
    if (contents.getType() === 'webview') sendToShell('fullscreen', true);
  });
  contents.on('leave-html-full-screen', () => {
    if (contents.getType() === 'webview') sendToShell('fullscreen', false);
  });
});

/* ------------------------------------------------------------------ *
 * IPC
 * ------------------------------------------------------------------ */

ipcMain.handle('site:init', async (event) => {
  if (!isShellSender(event)) return null;
  await Promise.race([widevineReady, new Promise((resolve) => setTimeout(resolve, 12000))]);
  return {
    site: {
      name: site.name,
      url: site.url,
      slug: site.slug,
      host: site.host,
      drm: site.drm,
      accent: state.accent,
    },
    widevine: state.widevine,
    icon: state.iconDataUrl,
    window: windowState(),
    platform: process.platform,
    versions: {
      electron: process.versions.electron,
      chromium: process.versions.chrome,
      node: process.versions.node,
    },
    alwaysOnTop: Boolean(settings.alwaysOnTop),
  };
});

ipcMain.handle('win:action', (event, action, value) => {
  const win = state.window;
  if (!win || !isShellSender(event)) return null;
  switch (action) {
    case 'close': win.close(); break;
    case 'minimize': win.minimize(); break;
    case 'zoom':
      if (win.isFullScreen()) win.setFullScreen(false);
      else if (win.isMaximized()) win.unmaximize();
      else win.maximize();
      break;
    case 'fullscreen':
      win.setFullScreen(!win.isFullScreen());
      break;
    case 'always-on-top': {
      const next = typeof value === 'boolean' ? value : !win.isAlwaysOnTop();
      win.setAlwaysOnTop(next);
      settings.alwaysOnTop = next;
      saveSettings();
      buildMenu();
      broadcastWindowState();
      break;
    }
    case 'state': return windowState();
    default: break;
  }
  return windowState();
});

ipcMain.handle('accent:favicon', async (event, urls) => {
  if (!isShellSender(event)) return null;
  const cacheKey = crypto.createHash('sha1').update(JSON.stringify(urls || [])).digest('hex');
  settings.accentCache = settings.accentCache || {};
  if (settings.accentCache[cacheKey]) return settings.accentCache[cacheKey];
  const color = await accentFromUrls(urls);
  if (color) {
    settings.accentCache[cacheKey] = color;
    saveSettings();
  }
  return color;
});

ipcMain.handle('clipboard:write', (event, text) => {
  if (!isShellSender(event)) return false;
  clipboard.writeText(String(text || ''));
  return true;
});

ipcMain.handle('open:zen', (event, url) => {
  if (!isShellSender(event) || !/^https?:/i.test(String(url))) return false;
  openInZen(String(url));
  return true;
});

ipcMain.handle('open:external', (event, url) => {
  if (!isShellSender(event) || !/^(https?|mailto):/i.test(String(url))) return false;
  shell.openExternal(String(url));
  return true;
});

ipcMain.handle('widevine:check', async (event) => {
  if (!isShellSender(event)) return null;
  const guest = require('electron').webContents.getAllWebContents().find((wc) => wc.getType() === 'webview');
  if (!guest) return { ok: false, error: 'No page loaded' };
  try {
    const result = await guest.executeJavaScript(`
      (async () => {
        if (!navigator.requestMediaKeySystemAccess) return 'No EME support';
        try {
          const access = await navigator.requestMediaKeySystemAccess('com.widevine.alpha', [{
            initDataTypes: ['cenc'],
            audioCapabilities: [{ contentType: 'audio/mp4; codecs="mp4a.40.2"' }],
            videoCapabilities: [{ contentType: 'video/mp4; codecs="avc1.42E01E"' }]
          }]);
          return 'Widevine ready: ' + access.keySystem;
        } catch (error) {
          return 'Widevine failed: ' + error.name + ' ' + error.message;
        }
      })();
    `, true);
    return { ok: /^Widevine ready/.test(result), message: result, widevine: state.widevine };
  } catch (error) {
    return { ok: false, error: String(error && error.message || error) };
  }
});

ipcMain.handle('menu:show', (event, anchor) => {
  if (!isShellSender(event)) return false;
  const template = [
    { label: 'Copy URL', click: () => sendCommand('copy-url') },
    { label: 'Open in Zen', click: () => sendCommand('open-in-zen') },
    { type: 'separator' },
    { label: 'Actual Size', click: () => sendCommand('zoom-reset') },
    { label: 'Zoom In', click: () => sendCommand('zoom-in') },
    { label: 'Zoom Out', click: () => sendCommand('zoom-out') },
    { type: 'separator' },
    {
      label: 'Always on Top',
      type: 'checkbox',
      checked: Boolean(state.window && state.window.isAlwaysOnTop()),
      click: () => {
        if (!state.window) return;
        const next = !state.window.isAlwaysOnTop();
        state.window.setAlwaysOnTop(next);
        settings.alwaysOnTop = next;
        saveSettings();
        broadcastWindowState();
        buildMenu();
      },
    },
    { type: 'separator' },
    { label: 'Check Widevine…', click: () => sendCommand('widevine-check') },
    { label: 'Widevine Playback Setup…', click: () => showWidevineSetup() },
    { type: 'separator' },
    { label: 'Reload', click: () => sendCommand('reload') },
    { label: 'App Home', click: () => sendCommand('home') },
    { type: 'separator' },
    { label: 'Quit', click: () => app.quit() },
  ];
  const point = anchor && Number.isFinite(anchor.x) && Number.isFinite(anchor.y)
    ? { x: Math.round(anchor.x), y: Math.round(anchor.y) }
    : undefined;
  Menu.buildFromTemplate(template).popup({
    window: state.window || undefined,
    x: point && point.x,
    y: point && point.y,
  });
  return true;
});

ipcMain.on('win:title', (event, title) => {
  if (!isShellSender(event)) return;
  const clean = String(title || '').trim() || site.name;
  if (state.window) state.window.setTitle(clean);
});

ipcMain.on('win:accent', (event, color) => {
  if (!isShellSender(event)) return;
  if (!/^#[0-9a-f]{6}$/i.test(String(color || ''))) return;
  state.accent = color;
  if (state.window) state.window.setBackgroundColor(mixWithBackground(color));
});

ipcMain.on('toast', (event, payload) => {
  if (!isShellSender(event)) return;
  if (payload && payload.message) notify(payload.message, payload.kind);
});

/* ------------------------------------------------------------------ *
 * Boot
 * ------------------------------------------------------------------ */

const lock = app.requestSingleInstanceLock({ profile: profileSlug });
if (!lock) {
  app.quit();
} else {
  app.on('second-instance', () => {
    if (!state.window) return;
    if (state.window.isMinimized()) state.window.restore();
    state.window.show();
    state.window.focus();
  });

  app.whenReady().then(() => {
    setupSession();
    loadIcon();
    if (state.iconDataUrl && app.dock && typeof app.dock.setIcon === 'function') {
      try { app.dock.setIcon(nativeImage.createFromDataURL(state.iconDataUrl)); } catch {}
    }
    app.setAboutPanelOptions({
      applicationName: site.name,
      applicationVersion: '1.0',
      credits: `Zen PWA · ${site.host}`,
    });
    createWindow();
    buildMenu();
    if (settings.alwaysOnTop && state.window) state.window.setAlwaysOnTop(true);
    prepareWidevine();
    if (flags.selftest) {
      setTimeout(() => {
        console.log('SELFTEST_ERROR timeout');
        app.exit(2);
      }, 45000);
    }
    app.on('activate', () => {
      if (!state.window) createWindow();
    });
  });

  app.on('window-all-closed', () => app.quit());
  app.on('before-quit', saveBounds);
}
