'use strict';

/* ------------------------------------------------------------------ *
 * Zen PWA shell renderer
 * ------------------------------------------------------------------ */

const $ = (selector) => document.querySelector(selector);

const els = {
  host: $('#webview-host'),
  toolbar: $('#toolbar'),
  address: $('#address'),
  title: $('#title'),
  hostLabel: $('#host'),
  url: $('#url'),
  favicon: $('#favicon'),
  monogram: $('#monogram'),
  lock: $('#lock'),
  back: $('#btn-back'),
  forward: $('#btn-forward'),
  reload: $('#btn-reload'),
  menu: $('#btn-menu'),
  progress: $('#progress'),
  splash: $('#splash'),
  splashIcon: $('#splash-icon'),
  splashName: $('#splash-name'),
  splashHost: $('#splash-host'),
  splashStatus: $('#splash-status-text'),
  toast: $('#toast'),
  error: $('#error'),
  errorTitle: $('#error-title'),
  errorDetail: $('#error-detail'),
};

let view = null;
let config = null;
let currentURL = '';
let currentTitle = '';
let pageHost = '';
let favicons = [];
let faviconAccent = null;
let lastFaviconKey = '';
let pageAccent = null;
let baseAccent = '';
let appliedAccent = '';
let appliedScheme = '';
let editing = false;
let loading = false;

/*
 * Sign-in and consent pages belong to the identity provider, not to the
 * app. Keep the app's name, icon and tint while the user is on one.
 */
const AUTH_HOST = /^(accounts\.google\..+|consent\.google\..+|accounts\.youtube\.com|myaccount\.google\.com|login\.microsoftonline\.com|login\.live\.com|appleid\.apple\.com|idmsa\.apple\.com|auth\..+)$/i;

function isAuthHost(host) {
  const value = String(host || '').replace(/^www\./, '');
  return Boolean(value) && AUTH_HOST.test(value);
}
let htmlFullscreen = false;
let isWindowFullscreen = false;
let menuOpen = false;

/* ------------------------------------------------------------------ *
 * Colour utilities
 * ------------------------------------------------------------------ */

const canvas = document.createElement('canvas');
canvas.width = 1;
canvas.height = 1;
const ctx = canvas.getContext('2d', { willReadFrequently: true });

function parseColor(value) {
  if (!value || typeof value !== 'string') return null;
  if (value === 'transparent') return null;
  ctx.clearRect(0, 0, 1, 1);
  ctx.fillStyle = '#000000';
  ctx.fillStyle = value;
  ctx.fillRect(0, 0, 1, 1);
  const data = ctx.getImageData(0, 0, 1, 1).data;
  if (data[3] === 0) return null;
  return { r: data[0], g: data[1], b: data[2], a: data[3] / 255 };
}

function rgbToHsl({ r, g, b }) {
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

function hexToRgb(hex) {
  const match = /^#?([0-9a-f]{6})$/i.exec(String(hex || ''));
  if (!match) return null;
  const value = parseInt(match[1], 16);
  return { r: (value >> 16) & 255, g: (value >> 8) & 255, b: value & 255, a: 1 };
}

function isVivid(hsl) {
  return hsl && hsl.s >= 0.18 && hsl.l >= 0.10 && hsl.l <= 0.92;
}

function vividAccentFromColor(value) {
  const rgb = typeof value === 'string' ? parseColor(value) : value;
  if (!rgb) return null;
  const hsl = rgbToHsl(rgb);
  if (!isVivid(hsl)) return null;
  return hsl;
}

function tintAccent(hex, scheme) {
  const rgb = hexToRgb(hex) || hexToRgb('#5b8cff');
  const hsl = rgbToHsl(rgb);
  const s = Math.max(0.5, Math.min(0.9, hsl.s));
  const l = scheme === 'light'
    ? Math.max(0.34, Math.min(0.48, hsl.l))
    : Math.max(0.48, Math.min(0.66, hsl.l));
  return hslToHex(hsl.h, s, l);
}

const FALLBACK_PALETTE = [
  '#e5484d', '#f76b15', '#ffb224', '#46a758', '#12a594',
  '#0d74ce', '#3e63dd', '#8e4ec6', '#d6409f', '#e93d82',
];

function hashString(value) {
  let hash = 5381;
  for (let i = 0; i < value.length; i += 1) {
    hash = ((hash << 5) + hash + value.charCodeAt(i)) >>> 0;
  }
  return hash;
}

function fallbackAccent(host) {
  return FALLBACK_PALETTE[hashString(host || 'zen') % FALLBACK_PALETTE.length];
}

function applyAccent(hex, scheme) {
  if (!hex || !/^#[0-9a-f]{6}$/i.test(hex)) return;
  baseAccent = hex;
  const tuned = tintAccent(hex, scheme);
  if (tuned === appliedAccent) return;
  appliedAccent = tuned;
  document.documentElement.style.setProperty('--accent', tuned);
  const hsl = rgbToHsl(hexToRgb(tuned));
  document.documentElement.style.setProperty('--accent-bright', hslToHex(hsl.h, 0.85, 0.7));
  zen.setAccent(tuned);
}

function applyScheme(scheme) {
  if (scheme === appliedScheme) return;
  appliedScheme = scheme;
  document.documentElement.dataset.scheme = scheme;
  if (baseAccent) applyAccent(baseAccent, scheme);
  else if (config && config.site.accent) applyAccent(config.site.accent, scheme);
}

/* ------------------------------------------------------------------ *
 * Toolbar visibility
 * ------------------------------------------------------------------ */

let nearTop = false;
let hovering = false;
let hideTimer = null;

function reveal() {
  clearTimeout(hideTimer);
  els.toolbar.classList.add('visible');
  els.toolbar.setAttribute('aria-hidden', 'false');
}

function hide() {
  if (editing || menuOpen) return;
  els.toolbar.classList.remove('visible');
  els.toolbar.setAttribute('aria-hidden', 'true');
}

function scheduleHide(delay = 420) {
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => {
    if (!nearTop && !hovering && !editing && !menuOpen) hide();
  }, delay);
}

function setNearTop(value) {
  nearTop = value;
  if (value) reveal();
  else scheduleHide();
}

/* ------------------------------------------------------------------ *
 * Toasts
 * ------------------------------------------------------------------ */

let toastTimer = null;

function toast(message, kind = 'info') {
  if (!message) return;
  els.toast.hidden = false;
  els.toast.textContent = message;
  els.toast.classList.toggle('error', kind === 'error');
  requestAnimationFrame(() => els.toast.classList.add('visible'));
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    els.toast.classList.remove('visible');
    setTimeout(() => { els.toast.hidden = true; }, 220);
  }, kind === 'error' ? 4200 : 2400);
}

/* ------------------------------------------------------------------ *
 * Address bar
 * ------------------------------------------------------------------ */

function normalizeTarget(text) {
  const value = String(text || '').trim();
  if (!value) return null;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(value)) return value;
  if (/^localhost(:\d+)?(\/|$)/i.test(value)) return `http://${value}`;
  if (/^[^\s/]+\.[^\s/]{2,}(\/|$|\?)/.test(value)) return `https://${value}`;
  return `https://duckduckgo.com/?q=${encodeURIComponent(value)}`;
}

function startEdit() {
  if (!view) return;
  reveal();
  editing = true;
  els.address.classList.add('editing');
  els.url.value = currentURL || (config && config.site.url) || '';
  els.url.focus();
  els.url.select();
}

function endEdit(commit) {
  if (!editing) return;
  editing = false;
  els.address.classList.remove('editing');
  els.url.blur();
  if (commit) {
    const target = normalizeTarget(els.url.value);
    if (target) load(target);
  }
  scheduleHide(260);
}

function updateAddress(url, title) {
  if (url) currentURL = url;
  const parsed = (() => { try { return new URL(currentURL); } catch { return null; } })();
  pageHost = parsed ? parsed.host.replace(/^www\./, '') : (config ? config.site.host : '');
  const siteHost = config ? config.site.host : '';
  const auth = isAuthHost(pageHost);
  const foreign = Boolean(pageHost) && pageHost !== siteHost;

  // The host is only worth showing when it differs from the app's own site.
  const hostText = foreign ? pageHost : '';
  els.hostLabel.textContent = hostText;
  els.hostLabel.hidden = !hostText;

  els.lock.classList.toggle('secure', Boolean(parsed && parsed.protocol === 'https:'));
  const fallback = (config && config.site.name) || pageHost || 'Zen PWA';
  const pageTitle = (title || '').trim();
  // A sign-in page must not rename the PWA; keep the app identity instead.
  const label = (auth && config) ? config.site.name : (pageTitle || pageHost || fallback);
  currentTitle = label;
  els.title.textContent = label;
  els.address.title = currentURL;
  zen.setTitle(label);
  if (editing) return;
  els.url.value = currentURL;
}

/* ------------------------------------------------------------------ *
 * Theme extraction
 * ------------------------------------------------------------------ */

let themeTimer = null;

function scheduleTheme(delay = 40) {
  clearTimeout(themeTimer);
  themeTimer = setTimeout(refreshTheme, delay);
}

const THEME_SCRIPT = `(() => {
  const metas = [];
  for (const el of document.querySelectorAll('meta[name="theme-color"], meta[name="msapplication-TileColor"], meta[property="og:theme-color"]')) {
    if (el.content) metas.push(el.content);
  }
  const htmlStyle = getComputedStyle(document.documentElement);
  const bodyStyle = document.body ? getComputedStyle(document.body) : null;
  return {
    metas,
    htmlBg: htmlStyle.backgroundColor,
    bodyBg: bodyStyle ? bodyStyle.backgroundColor : null,
    colorScheme: (htmlStyle.colorScheme || (bodyStyle && bodyStyle.colorScheme) || ''),
    href: location.href,
    title: document.title
  };
})()`;

async function refreshTheme() {
  if (!view) return;
  let info;
  try {
    info = await view.executeJavaScript(THEME_SCRIPT, true);
  } catch {
    return;
  }
  if (!info) return;

  const bodyBg = parseColor(info.bodyBg);
  const htmlBg = parseColor(info.htmlBg);
  const pageBg = (bodyBg && bodyBg.a > 0.35 && bodyBg) || (htmlBg && htmlBg.a > 0.35 && htmlBg) || null;

  let scheme;
  if (pageBg) {
    const { r, g, b } = pageBg;
    const luminance = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
    scheme = luminance < 0.5 ? 'dark' : 'light';
  } else if (info.colorScheme) {
    scheme = /dark/.test(info.colorScheme) && !/light/.test(info.colorScheme) ? 'dark' : 'light';
  } else {
    scheme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  applyScheme(scheme);

  let accent = null;
  for (const meta of info.metas) {
    const hsl = vividAccentFromColor(meta);
    if (hsl) { accent = hsl; break; }
  }
  if (accent) pageAccent = hslToHex(accent.h, accent.s, accent.l);
  else pageAccent = null;

  let chosen;
  if (config && isAuthHost(pageHost)) {
    // Keep the app's own colour through a third-party sign-in flow.
    chosen = config.site.accent || pageAccent || faviconAccent || fallbackAccent(config.site.host);
  } else {
    chosen = pageAccent || faviconAccent || (config && config.site.accent) || fallbackAccent(pageHost);
  }
  // A near-white or near-black "theme color" is not a theme; skip it.
  if (chosen) {
    const hsl = rgbToHsl(hexToRgb(chosen) || { r: 91, g: 140, b: 255 });
    if (!isVivid(hsl)) chosen = fallbackAccent(pageHost);
  }
  applyAccent(chosen, scheme);

  if (info.title && !currentTitle) updateAddress(info.href, info.title);
}

async function considerFavicons(list) {
  if (!Array.isArray(list) || !list.length) return;
  const key = list.join('|');
  if (key === lastFaviconKey) return;
  lastFaviconKey = key;
  favicons = list;
  const color = await zen.faviconAccent(list);
  if (color) {
    faviconAccent = color;
    if (!pageAccent) scheduleTheme(0);
  }
}

/* ------------------------------------------------------------------ *
 * Webview plumbing
 * ------------------------------------------------------------------ */

function updateFavicon() {
  // Identity providers must not replace the app's icon with their own.
  const pinned = config && config.icon && isAuthHost(pageHost) ? config.icon : null;
  const url = pinned || favicons[favicons.length - 1];
  if (!url) {
    els.favicon.hidden = true;
    els.monogram.hidden = false;
    return;
  }
  els.favicon.onerror = () => {
    els.favicon.hidden = true;
    els.monogram.hidden = false;
  };
  els.favicon.onload = () => {
    els.favicon.hidden = false;
    els.monogram.hidden = true;
  };
  els.favicon.src = url;
}

function updateNav() {
  if (!view) return;
  try {
    els.back.disabled = !view.canGoBack();
    els.forward.disabled = !view.canGoForward();
  } catch {}
}

function startProgress() {
  loading = true;
  els.progress.classList.remove('done');
  els.progress.classList.add('loading');
  els.progress.style.transform = 'scaleX(0.10)';
  setTimeout(() => { if (loading) els.progress.style.transform = 'scaleX(0.55)'; }, 260);
  setTimeout(() => { if (loading) els.progress.style.transform = 'scaleX(0.78)'; }, 1100);
}

function endProgress() {
  loading = false;
  els.progress.style.transform = 'scaleX(1)';
  els.progress.classList.remove('loading');
  els.progress.classList.add('done');
  setTimeout(() => {
    if (!loading) els.progress.style.transform = 'scaleX(0)';
  }, 400);
}

function showError(title, detail) {
  els.errorTitle.textContent = title || "This page didn't load";
  els.errorDetail.textContent = detail || 'Check your connection and try again.';
  els.error.hidden = false;
  hideSplash();
}

function clearError() {
  els.error.hidden = true;
}

function hideSplash() {
  document.body.classList.remove('booting');
  els.splash.classList.add('hidden');
}

function setSplashStatus(text) {
  if (text) els.splashStatus.textContent = text;
}

function load(url) {
  if (!view || !url) return;
  clearError();
  view.loadURL(url).catch(() => {});
}

function wireWebview() {
  view.addEventListener('did-start-loading', () => {
    startProgress();
  });

  view.addEventListener('did-stop-loading', () => {
    endProgress();
    updateNav();
    if (view.getURL()) updateAddress(view.getURL(), view.getTitle() || currentTitle);
    scheduleTheme(0);
    hideSplash();
  });

  view.addEventListener('did-start-navigation', (event) => {
    if (!event.isMainFrame) return;
    clearError();
    startProgress();
  });

  view.addEventListener('did-navigate', (event) => {
    updateAddress(event.url, view.getTitle() || '');
    favicons = [];
    lastFaviconKey = '';
    faviconAccent = null;
    pageAccent = null;
    updateFavicon();
    updateNav();
    scheduleTheme(60);
  });

  view.addEventListener('did-navigate-in-page', (event) => {
    if (!event.isMainFrame) return;
    updateAddress(event.url, view.getTitle() || '');
    updateNav();
    scheduleTheme(80);
  });

  view.addEventListener('page-title-updated', (event) => {
    updateAddress(view.getURL() || currentURL, event.title);
  });

  view.addEventListener('page-favicon-updated', (event) => {
    favicons = Array.isArray(event.favicons) ? event.favicons : [];
    updateFavicon();
    considerFavicons(favicons);
    if (view.getURL()) updateAddress(view.getURL(), view.getTitle() || currentTitle);
  });

  view.addEventListener('did-fail-load', (event) => {
    if (!event.isMainFrame) return;
    if (event.errorCode === -3) return;
    const host = (() => { try { return new URL(event.validatedURL || currentURL).host; } catch { return pageHost; } })();
    showError(`Can't reach ${host || 'this page'}`, event.errorDescription || `Error ${event.errorCode}`);
    endProgress();
  });

  view.addEventListener('render-process-gone', () => {
    showError('This page stopped responding', 'The page was reloaded after its process ended.');
    setTimeout(() => load(currentURL), 600);
  });

  view.addEventListener('dom-ready', () => {
    scheduleTheme(0);
    hideSplash();
  });

  view.addEventListener('enter-html-full-screen', () => {
    htmlFullscreen = true;
    document.body.classList.add('fullscreen');
    if (!isWindowFullscreen) zen.windowAction('fullscreen');
  });

  view.addEventListener('leave-html-full-screen', () => {
    htmlFullscreen = false;
    if (isWindowFullscreen) zen.windowAction('fullscreen');
  });

  view.addEventListener('focus', () => updateNav());
}

function createWebview(url) {
  const webview = document.createElement('webview');
  webview.id = 'view';
  webview.setAttribute('allowpopups', '');
  webview.setAttribute('webpreferences', 'contextIsolation=yes,nodeIntegration=no,sandbox=yes,spellcheck=yes');
  webview.setAttribute('src', url);
  els.host.appendChild(webview);
  view = webview;
  wireWebview();
}

/* ------------------------------------------------------------------ *
 * Commands from the app menu
 * ------------------------------------------------------------------ */

function zoomBy(delta) {
  if (!view) return;
  const next = Math.max(0.3, Math.min(3, (view.getZoomFactor() || 1) + delta));
  view.setZoomFactor(next);
  toast(`Zoom ${Math.round(next * 100)}%`);
}

async function checkWidevine() {
  const result = await zen.checkWidevine();
  if (result && result.ok) toast(result.message || 'Widevine is ready');
  else toast((result && (result.message || result.error)) || 'Widevine is not available', 'error');
}

function handleCommand(command) {
  switch (command) {
    case 'focus-address': startEdit(); break;
    case 'back': if (view && view.canGoBack()) view.goBack(); break;
    case 'forward': if (view && view.canGoForward()) view.goForward(); break;
    case 'reload': if (view) view.reload(); break;
    case 'hard-reload': if (view) view.reloadIgnoringCache(); break;
    case 'home': load(config && config.site.url); break;
    case 'zoom-in': zoomBy(0.1); break;
    case 'zoom-out': zoomBy(-0.1); break;
    case 'zoom-reset': if (view) { view.setZoomFactor(1); toast('Zoom 100%'); } break;
    case 'copy-url':
      zen.copy(currentURL);
      toast('URL copied');
      break;
    case 'open-in-zen': zen.openInZen(currentURL); break;
    case 'widevine-check': checkWidevine(); break;
    case 'guest-devtools': if (view && view.openDevTools) view.openDevTools(); break;
    default: break;
  }
}

/* ------------------------------------------------------------------ *
 * Wire the shell
 * ------------------------------------------------------------------ */

async function boot() {
  let cfg = null;
  try {
    cfg = await zen.init();
  } catch (error) {
    showError('Zen PWA could not start', String(error && error.message || error));
    return;
  }
  if (!cfg) return;
  config = cfg;
  document.title = cfg.site.name;

  els.splashName.textContent = cfg.site.name;
  els.splashHost.textContent = cfg.site.host;
  if (cfg.icon) {
    const img = document.createElement('img');
    img.src = cfg.icon;
    img.alt = '';
    els.splashIcon.textContent = '';
    els.splashIcon.appendChild(img);
  } else {
    const monogram = document.createElement('span');
    monogram.textContent = (cfg.site.name || 'Z').trim().charAt(0).toUpperCase();
    els.splashIcon.textContent = '';
    els.splashIcon.appendChild(monogram);
  }
  els.monogram.textContent = (cfg.site.name || 'Z').trim().charAt(0).toUpperCase();

  applyScheme(window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  if (cfg.site.accent) applyAccent(cfg.site.accent, appliedScheme);

  updateAddress(cfg.site.url, cfg.site.name);
  els.hostLabel.textContent = cfg.site.host;

  document.body.classList.toggle('blurred', !(cfg.window && cfg.window.focused));

  if (cfg.widevine.state === 'installing') {
    setSplashStatus('Preparing secure playback…');
    await Promise.race([
      waitForWidevineReady(),
      new Promise((resolve) => setTimeout(resolve, 12000)),
    ]);
  } else if (cfg.widevine.state === 'error') {
    setSplashStatus('Secure playback unavailable');
  }

  setSplashStatus(`Loading ${cfg.site.host}…`);
  createWebview(cfg.site.url);
  if (cfg.widevine.state === 'ready' && cfg.site.drm) {
    setSplashStatus('Secure playback ready');
  }
  setTimeout(hideSplash, 7000);

  // Show the toolbar briefly at launch, like Zen revealing its toolbar.
  setTimeout(() => { reveal(); scheduleHide(2200); }, 380);
}

function waitForWidevineReady() {
  return new Promise((resolve) => {
    const off = zen.on('widevine', (widevine) => {
      if (widevine.state !== 'installing') {
        off();
        resolve();
      }
    });
    window.setTimeout(() => { off(); resolve(); }, 12000);
  });
}

/* Toolbar behaviour */
els.toolbar.addEventListener('mouseenter', () => { hovering = true; reveal(); });
els.toolbar.addEventListener('mouseleave', () => { hovering = false; scheduleHide(320); });

document.addEventListener('pointerenter', () => {});
document.addEventListener('mousemove', (event) => {
  if (event.clientY <= 2) reveal();
}, { passive: true });

$('#dragstrip').addEventListener('dblclick', () => zen.windowAction('zoom'));

document.addEventListener('click', (event) => {
  const button = event.target.closest('button[data-action]');
  if (!button) return;
  const action = button.dataset.action;
  switch (action) {
    case 'close': zen.windowAction('close'); break;
    case 'minimize': zen.windowAction('minimize'); break;
    case 'zoom': zen.windowAction('zoom'); break;
    case 'back': if (view && view.canGoBack()) view.goBack(); break;
    case 'forward': if (view && view.canGoForward()) view.goForward(); break;
    case 'reload':
      if (!view) break;
      if (loading) view.stop(); else view.reload();
      break;
    case 'copy':
      zen.copy(currentURL);
      toast('URL copied');
      break;
    case 'menu':
      reveal();
      menuOpen = true;
      zen.showMenu().finally(() => {
        setTimeout(() => {
          menuOpen = false;
          scheduleHide(200);
        }, 700);
      });
      break;
    default: break;
  }
});

/* Address editing */
els.address.addEventListener('click', () => {
  if (!editing) startEdit();
});

els.address.addEventListener('keydown', (event) => {
  if (editing) return;
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    startEdit();
  }
});

els.url.addEventListener('keydown', (event) => {
  if (event.key === 'Enter') {
    event.preventDefault();
    endEdit(true);
  } else if (event.key === 'Escape') {
    event.preventDefault();
    endEdit(false);
  }
});

els.url.addEventListener('blur', () => {
  if (editing) endEdit(false);
});

/* Error actions */
$('#error-retry').addEventListener('click', () => {
  clearError();
  if (currentURL) load(currentURL);
});
$('#error-zen').addEventListener('click', () => zen.openInZen(currentURL));

/* Events from main */
zen.on('pointer', (near) => setNearTop(Boolean(near)));
zen.on('focus', (focused) => document.body.classList.toggle('blurred', !focused));
zen.on('window-state', (state) => {
  isWindowFullscreen = Boolean(state.fullscreen);
  document.body.classList.toggle('blurred', !state.focused);
  document.body.classList.toggle('fullscreen', Boolean(state.fullscreen) || htmlFullscreen);
});
zen.on('fullscreen', (on) => {
  isWindowFullscreen = Boolean(on);
  document.body.classList.toggle('fullscreen', Boolean(on) || htmlFullscreen);
  if (on) hide();
});
zen.on('cmd', handleCommand);
zen.on('toast', (payload) => toast(payload && payload.message, payload && payload.kind));
zen.on('widevine', (widevine) => {
  if (widevine.state === 'ready') {
    if (config && config.site.drm) toast('Widevine is ready');
  } else if (widevine.state === 'error') {
    toast(`Widevine unavailable: ${widevine.error || 'unknown error'}`, 'error');
  }
});

window.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && editing) endEdit(false);
  if (event.metaKey && event.key === 'l') {
    event.preventDefault();
    startEdit();
  }
});

window.addEventListener('blur', () => document.body.classList.add('blurred'));

boot();
