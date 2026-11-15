'use strict';

/*
 * Zen config bridge.
 *
 * The PWA shell takes its look from the user's Zen browser profile, so
 * whatever is configured in Zen (layout prefs, the active space's colour,
 * mods, userChrome.css) is the source of truth for every PWA window.
 *
 *   profile   ~/Library/Application Support/zen  (installs.ini → profiles.ini)
 *             override with ZENPWA_ZEN_PROFILE=/path/to/profile
 *   prefs     prefs.js + user.js over Zen's built-in defaults
 *   space     zen-sessions.jsonlz4 → the active space's theme colour
 *   mods      chrome/zen-themes.css (Zen Mods) + chrome/userChrome.css
 *
 * Plain Node, no Electron: it can be tested with `node -e`.
 */

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const ZEN_ROOT = path.join(os.homedir(), 'Library', 'Application Support', 'zen');

// Zen 1.24 built-in defaults (greprefs.js / zenThemeModifier.js / zen-theme.css).
const DEFAULTS = {
  'zen.theme.content-element-separation': 8,
  'zen.theme.border-radius': -1, // -1 = native: 11px on macOS Tahoe+, 10px before
  'zen.theme.acrylic-sidebar': false,
  'zen.theme.use-system-colors': false,
  'zen.view.compact.hide-toolbar': false,
  'zen.view.compact.toolbar-hide-after-hover.duration': 1000,
  'zen.view.grey-out-inactive-windows': true,
  'zen.view.window.scheme': 2, // 0 dark, 1 light, 2 follow system
  'zen.view.borderless-fullscreen': true,
};

// Zen's gradient maths (ZenGradientGenerator.mjs).
const OPACITY_MIN = 0.3;
const OPACITY_MAX = 0.8;
const MIN_LAYER_ALPHA = 0.2;
const MAX_LAYER_ALPHA = 0.88;
const MIN_TRANSLUCENT_LAYER_ALPHA = 0.85;
const MIN_WINDOW_ALPHA = 0.35;
const MAX_WINDOW_ALPHA = 1;
const SQUIRCLE = 1.3;
const TOOLBAR_HEIGHT_MAC = 42;

/* ------------------------------------------------------------------ *
 * Files
 * ------------------------------------------------------------------ */

function readText(file) {
  try { return fs.readFileSync(file, 'utf8'); } catch { return ''; }
}

function iniSections(text) {
  const out = {};
  let current = null;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    const head = /^\[(.+)\]$/.exec(line);
    if (head) { current = out[head[1]] = {}; continue; }
    const eq = line.indexOf('=');
    if (current && eq > 0) current[line.slice(0, eq)] = line.slice(eq + 1);
  }
  return out;
}

function findProfile() {
  if (process.env.ZENPWA_ZEN_PROFILE) return process.env.ZENPWA_ZEN_PROFILE;
  const resolve = (rel) => path.join(ZEN_ROOT, rel);
  // installs.ini names the profile the installed Zen actually opens.
  for (const section of Object.values(iniSections(readText(resolve('installs.ini'))))) {
    if (section.Default && fs.existsSync(resolve(section.Default))) return resolve(section.Default);
  }
  const profiles = Object.values(iniSections(readText(resolve('profiles.ini')))).filter((s) => s.Path);
  const pick = profiles.find((s) => s.Default === '1') || profiles[0];
  if (!pick) return null;
  return pick.IsRelative === '0' ? pick.Path : resolve(pick.Path);
}

function readPrefs(profile) {
  const prefs = { ...DEFAULTS };
  for (const file of ['prefs.js', 'user.js']) {
    const text = readText(path.join(profile, file));
    const re = /user_pref\(\s*"([^"]+)"\s*,\s*(.+?)\s*\);/g;
    let m;
    while ((m = re.exec(text))) {
      try { prefs[m[1]] = JSON.parse(m[2]); } catch { prefs[m[1]] = m[2]; }
    }
  }
  return prefs;
}

// mozLz40: "mozLz40\0" + u32 size + one raw LZ4 block.
function mozlz4(buf) {
  if (buf.subarray(0, 8).toString('binary') !== 'mozLz40\0') throw new Error('not mozlz4');
  const out = Buffer.alloc(buf.readUInt32LE(8));
  const src = buf.subarray(12);
  let i = 0;
  let o = 0;
  while (i < src.length) {
    const token = src[i++];
    let literals = token >> 4;
    if (literals === 15) { let b; do { b = src[i++]; literals += b; } while (b === 255); }
    src.copy(out, o, i, i + literals);
    i += literals;
    o += literals;
    if (i >= src.length) break;
    const offset = src[i] | (src[i + 1] << 8);
    i += 2;
    let match = token & 15;
    if (match === 15) { let b; do { b = src[i++]; match += b; } while (b === 255); }
    match += 4;
    for (let k = 0; k < match; k += 1, o += 1) out[o] = out[o - offset];
  }
  return out;
}

function readSpaceTheme(profile, prefs) {
  let sessions;
  try {
    sessions = JSON.parse(mozlz4(fs.readFileSync(path.join(profile, 'zen-sessions.jsonlz4'))).toString('utf8'));
  } catch {
    return null;
  }
  const spaces = Array.isArray(sessions.spaces) ? sessions.spaces : [];
  const active = spaces.find((s) => s.uuid === prefs['zen.workspaces.active']) || spaces[0];
  if (!active || !active.theme) return null;
  const colors = Array.isArray(active.theme.gradientColors) ? active.theme.gradientColors : [];
  const primary = colors.find((c) => c && c.isPrimary) || colors[0];
  const rgb = primary && Array.isArray(primary.c) ? primary.c.map(Number) : null;
  return {
    name: active.name || '',
    rgb,
    custom: Boolean(primary && primary.isCustom),
    opacity: Number.isFinite(active.theme.opacity) ? active.theme.opacity : 0.5,
  };
}

function readMods(profile) {
  const chrome = path.join(profile, 'chrome');
  const themes = (() => { try { return JSON.parse(readText(path.join(profile, 'zen-themes.json'))) || {}; } catch { return {}; } })();
  const names = Object.values(themes)
    .filter((mod) => mod && mod.enabled !== false)
    .map((mod) => mod.name || mod.id)
    .filter(Boolean);
  return {
    names,
    css: [readText(path.join(chrome, 'zen-themes.css')), readText(path.join(chrome, 'userChrome.css'))].join('\n'),
  };
}

/* ------------------------------------------------------------------ *
 * Mods: Zen's chrome CSS → the PWA shell
 *
 * Zen mods style Firefox's chrome (#urlbar, #nav-bar, …). The shell has
 * one element for each of the parts a PWA keeps, so selectors are mapped
 * onto those; rules aimed at anything else (tabs, sidebar, …) are dropped.
 * `-moz-pref()` / `-moz-bool-pref` conditions are evaluated against prefs.
 * ------------------------------------------------------------------ */

const ALIASES = [
  [/^(?::root|html|#main-window)/, ':root'],
  [/^(?:#zen-main-app-wrapper|#browser|body)/, 'body'],
  [/^(?:#zen-appcontent-navbar-wrapper|#zen-appcontent-navbar-container|#nav-bar|#navigator-toolbox)/, '#toolbar'],
  [/^(?:#urlbar-background|#urlbar)/, '#address'],
  [/^#urlbar-input/, '#url'],
  [/^#back-button/, '#btn-back'],
  [/^#forward-button/, '#btn-forward'],
  [/^(?:#reload-button|#stop-reload-button)/, '#btn-reload'],
  [/^(?:#PanelUI-menu-button|#PanelUI-button)/, '#btn-menu'],
  [/^\.toolbarbutton-1/, '.zbtn'],
  [/^(?:#tabbrowser-tabbox|#tabbrowser-tabpanels|#zen-tabbox-wrapper|\.browserSidebarContainer|\.browserStack|#zen-appcontent-wrapper|browser)/, '#page'],
];

function mapCompound(compound) {
  if (!compound) return compound;
  for (const [pattern, replacement] of ALIASES) {
    const m = pattern.exec(compound);
    if (!m) continue;
    const rest = compound.slice(m[0].length);
    // Keep pseudo-classes / attribute filters; drop further ids/classes we can't place.
    if (/^[#.][\w-]/.test(rest)) return null;
    return replacement + rest;
  }
  return null;
}

function mapSelector(selector) {
  const parts = selector.trim().split(/(\s*[>+~]\s*|\s+)/);
  const out = [];
  for (const part of parts) {
    if (!part.trim() || /^\s*[>+~]\s*$/.test(part)) { out.push(part); continue; }
    if (part === '&' || part.startsWith('&')) { out.push(part); continue; }
    const mapped = mapCompound(part);
    if (mapped === null) return null;
    out.push(mapped);
  }
  return out.join('');
}

function evalPrefCondition(prelude, prefs) {
  const on = (name) => {
    const v = prefs[name];
    return v === true || (typeof v === 'number' && v !== 0) || (typeof v === 'string' && v !== '' && v !== 'false');
  };
  return prelude
    .replace(/not\s*\(\s*-moz-bool-pref:\s*["']([^"']+)["']\s*\)/g, (_, n) => (on(n) ? 'not all' : 'all'))
    .replace(/\(\s*-moz-bool-pref:\s*["']([^"']+)["']\s*\)/g, (_, n) => (on(n) ? 'all' : 'not all'))
    .replace(/not\s+-moz-pref\(\s*["']([^"']+)["']\s*(?:,\s*["']([^"']*)["'])?\s*\)/g, (_, n, v) => ((v === undefined ? on(n) : String(prefs[n]) === v) ? 'not all' : 'all'))
    .replace(/-moz-pref\(\s*["']([^"']+)["']\s*(?:,\s*["']([^"']*)["'])?\s*\)/g, (_, n, v) => ((v === undefined ? on(n) : String(prefs[n]) === v) ? 'all' : 'not all'))
    .replace(/\(\s*-moz-platform:\s*macos\s*\)/g, 'all')
    .replace(/\(\s*-moz-platform:\s*\w+\s*\)/g, 'not all');
}

// Minimal CSS block parser: [{ prelude, body }] where body is text or children.
function parseBlocks(css) {
  const blocks = [];
  let i = 0;
  const text = css.replace(/\/\*[\s\S]*?\*\//g, '');
  while (i < text.length) {
    const open = text.indexOf('{', i);
    if (open === -1) break;
    const prelude = text.slice(i, open).trim();
    let depth = 1;
    let j = open + 1;
    while (j < text.length && depth) {
      if (text[j] === '{') depth += 1;
      else if (text[j] === '}') depth -= 1;
      j += 1;
    }
    const inner = text.slice(open + 1, j - 1);
    // A statement like `@import …;` before the block is skipped with the prelude.
    blocks.push({ prelude: prelude.replace(/^[\s\S]*;/, '').trim(), inner });
    i = j;
  }
  return blocks;
}

function translateCSS(css, prefs, depth = 0) {
  if (depth > 6) return '';
  const out = [];
  for (const { prelude, inner } of parseBlocks(css)) {
    if (!prelude) continue;
    if (/^@(media|supports)/.test(prelude)) {
      const condition = evalPrefCondition(prelude, prefs);
      if (/^@media\s+not all\s*$/.test(condition)) continue;
      const body = translateCSS(inner, prefs, depth + 1);
      if (body.trim()) out.push(`${condition} {\n${body}\n}`);
      continue;
    }
    if (prelude.startsWith('@')) continue; // @keyframes, @font-face, @-moz-document …
    const selectors = prelude.split(',').map(mapSelector).filter(Boolean);
    if (!selectors.length) continue;
    // Nested rules (CSS nesting) are not followed; declarations only.
    const declarations = inner.replace(/[^;{}]*\{[^}]*\}/g, '').trim();
    if (declarations) out.push(`${[...new Set(selectors)].join(',\n')} {\n  ${declarations}\n}`);
  }
  return out.join('\n');
}

/* ------------------------------------------------------------------ *
 * Colour
 * ------------------------------------------------------------------ */

function rgbToHsl([r, g, b]) {
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
  return [h, s, l];
}

function hslToRgb([h, s, l]) {
  if (s === 0) return [l, l, l].map((v) => Math.round(v * 255));
  const hue = (p, q, t) => {
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  return [hue(p, q, h + 1 / 3), hue(p, q, h), hue(p, q, h - 1 / 3)].map((v) => Math.round(v * 255));
}

function hexToRgb(hex) {
  const n = parseInt(String(hex).slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function luminance(rgb) {
  const [r, g, b] = rgb.map((v) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/*
 * The frame colour for one PWA: the active Zen space's colour with the
 * site's hue, painted exactly as Zen paints a space on macOS (translucent
 * layer over the native window blur). No site colour → the space colour.
 */
function frameFor(config, siteHex, systemDark) {
  const scheme = config.prefs['zen.view.window.scheme'];
  const dark = scheme === 0 ? true : scheme === 1 ? false : Boolean(systemDark);
  const underlay = dark ? [19, 19, 19] : [255, 255, 255];
  const space = config.space;

  let rgb = space && space.rgb;
  if (rgb && /^#[0-9a-f]{6}$/i.test(String(siteHex || ''))) {
    const [, s, l] = rgbToHsl(rgb);
    const [h, siteS] = rgbToHsl(hexToRgb(siteHex));
    // A grey space would wash every site out; borrow some of the site's saturation.
    rgb = hslToRgb([h, Math.max(s, Math.min(siteS, 0.45) * 0.6), l]);
  } else if (!rgb && /^#[0-9a-f]{6}$/i.test(String(siteHex || ''))) {
    const [h, siteS] = rgbToHsl(hexToRgb(siteHex));
    rgb = hslToRgb([h, Math.min(siteS, 0.45), 0.32]);
  }

  if (!rgb) {
    // Zen's own background for a space without a theme (macOS, translucent).
    return { css: dark ? 'rgba(0, 0, 0, 0.4)' : 'rgba(255, 255, 255, 0)', dark, ink: dark ? 'rgba(255, 255, 255, 0.8)' : 'rgba(0, 0, 0, 0.8)' };
  }

  const opacity = space ? space.opacity : 0.5;
  const intensity = Math.min(1, Math.max(0, (opacity - OPACITY_MIN) / (OPACITY_MAX - OPACITY_MIN)));
  const layer = MIN_TRANSLUCENT_LAYER_ALPHA + (MAX_LAYER_ALPHA - MIN_TRANSLUCENT_LAYER_ALPHA) * intensity ** 2;
  const windowAlpha = MIN_WINDOW_ALPHA + (MAX_WINDOW_ALPHA - MIN_WINDOW_ALPHA) * intensity ** 2;
  const painted = space && space.custom && space.rgb === rgb
    ? rgb
    : rgb.map((c, i) => Math.round(c * layer + underlay[i] * (1 - layer)));
  // Text colour like Zen's shouldBeDarkMode: whichever reads better on the paint.
  const lightText = luminance(painted) < 0.18;
  return {
    css: `rgba(${painted.join(', ')}, ${windowAlpha.toFixed(4)})`,
    dark,
    ink: lightText ? 'rgba(255, 255, 255, 0.8)' : 'rgba(0, 0, 0, 0.8)',
    inkScheme: lightText ? 'dark' : 'light',
  };
}

/* ------------------------------------------------------------------ *
 * Public
 * ------------------------------------------------------------------ */

function load() {
  const profile = findProfile();
  const prefs = profile ? readPrefs(profile) : { ...DEFAULTS };
  const space = profile ? readSpaceTheme(profile, prefs) : null;
  const mods = profile ? readMods(profile) : { names: [], css: '' };

  const separation = Math.max(0.1, Number(prefs['zen.theme.content-element-separation']) || 0);
  const radiusPref = Number(prefs['zen.theme.border-radius']);
  const tahoe = Number(os.release().split('.')[0]) >= 25; // Darwin 25 = macOS 26
  const borderRadius = radiusPref >= 0 ? radiusPref : (tahoe ? 11 : 10);
  const innerRadius = Math.max(5, (borderRadius - separation / 2) * SQUIRCLE);

  return {
    profile,
    prefs,
    space,
    mods: { names: mods.names, css: translateCSS(mods.css, prefs) },
    layout: {
      separation,
      borderRadius,
      innerRadius,
      toolbarHeight: TOOLBAR_HEIGHT_MAC,
      hideToolbar: Boolean(prefs['zen.view.compact.hide-toolbar']),
      hideDelay: Number(prefs['zen.view.compact.toolbar-hide-after-hover.duration']) || 0,
      greyInactive: Boolean(prefs['zen.view.grey-out-inactive-windows']),
    },
  };
}

// Calls back (debounced) whenever Zen writes prefs, sessions or mod CSS.
function watch(config, onChange) {
  if (!config.profile) return () => {};
  let timer = null;
  const fire = () => {
    clearTimeout(timer);
    timer = setTimeout(onChange, 300);
  };
  const watchers = [];
  const add = (target, only) => {
    try {
      watchers.push(fs.watch(target, (_event, name) => {
        if (!only || only.has(String(name || ''))) fire();
      }));
    } catch {}
  };
  // The profile folder churns constantly (cache, sqlite); only these matter.
  add(config.profile, new Set(['prefs.js', 'user.js', 'zen-sessions.jsonlz4', 'zen-themes.json']));
  add(path.join(config.profile, 'chrome'));
  return () => { clearTimeout(timer); for (const w of watchers) { try { w.close(); } catch {} } };
}

module.exports = { load, watch, frameFor, translateCSS, mozlz4, findProfile };
