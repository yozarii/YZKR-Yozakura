#!/usr/bin/env node
'use strict';

/*
 * skin-config — writes ~/.config/zen-pwa/skin.json for the ZenSkin overlay.
 *
 * ZenSkin reskins Safari web apps (File → Add to Dock) by floating a
 * click-through overlay over their windows. This file gives it the Zen look:
 * layout from the Zen profile and, per web app, the Zen space colour with the
 * site's hue (same rules as the Electron shell: accents.json override →
 * site colour from siteinfo → plain space colour).
 *
 * The frame colour is flattened to an opaque colour: Zen paints a translucent
 * layer over the macOS window blur, which reads as roughly mid grey; the
 * overlay sits on Safari's own title bar instead, so it composites onto that
 * grey itself (measured against the user's Zen frame).
 *
 *   node tools/skin-config.js            # write the file, print it
 */

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const ROOT = path.join(__dirname, '..');
const zen = require(path.join(ROOT, 'app', 'zen-config.js'));
const OUT = path.join(os.homedir(), '.config', 'zen-pwa', 'skin.json');
const ACCENTS = path.join(os.homedir(), '.config', 'zen-pwa', 'accents.json');
const APPS_DIR = path.join(os.homedir(), 'Applications');
const BLUR_BASE = [127, 127, 127];

function readJSON(file, fallback) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return fallback; }
}

function slugify(value) {
  return String(value || '').toLowerCase().normalize('NFKD').replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'site';
}

function bundleId(app) {
  const r = spawnSync('plutil', ['-extract', 'CFBundleIdentifier', 'raw', '-o', '-', path.join(app, 'Contents', 'Info.plist')], { encoding: 'utf8' });
  return r.status === 0 ? r.stdout.trim() : '';
}

function webApps() {
  let entries = [];
  try { entries = fs.readdirSync(APPS_DIR); } catch {}
  return entries
    .filter((e) => e.endsWith('.app'))
    .map((e) => ({ name: e.slice(0, -4), path: path.join(APPS_DIR, e) }))
    .map((a) => ({ ...a, id: bundleId(a.path) }))
    .filter((a) => a.id.startsWith('com.apple.Safari.WebApp'));
}

function flatten(css) {
  const m = /rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+))?\s*\)/.exec(css || '');
  if (!m) return null;
  const rgb = [m[1], m[2], m[3]].map(Number);
  const a = m[4] === undefined ? 1 : Number(m[4]);
  const out = rgb.map((c, i) => Math.round(c * a + BLUR_BASE[i] * (1 - a)));
  return `#${out.map((c) => c.toString(16).padStart(2, '0')).join('')}`;
}

function siteColour(name, cache) {
  const slug = slugify(name.replace(/\s+(web|app)$/i, ''));
  const accents = readJSON(ACCENTS, {});
  if (/^#[0-9a-f]{6}$/i.test(accents[slug] || '')) return accents[slug];
  if (cache[slug] !== undefined) return cache[slug];
  const r = spawnSync(process.execPath, [path.join(ROOT, 'tools', 'siteinfo.js'), name.replace(/\s+(web|app)$/i, '')], { encoding: 'utf8', timeout: 45000 });
  let colour = '';
  try { colour = JSON.parse(r.stdout).themeColor || ''; } catch {}
  cache[slug] = colour;
  return colour;
}

function main() {
  const config = zen.load();
  const dark = spawnSync('defaults', ['read', '-g', 'AppleInterfaceStyle'], { encoding: 'utf8' }).stdout.trim() === 'Dark';
  const previous = readJSON(OUT, {});
  const cache = previous.siteColours || {};

  const apps = {};
  for (const app of webApps()) {
    const colour = siteColour(app.name, cache);
    const frame = zen.frameFor(config, colour || null, dark);
    const kept = (previous.apps || {})[app.name] || {};
    apps[app.name] = {
      bundleId: app.id,
      site: colour || null,
      frame: flatten(frame.css),
      ink: frame.inkScheme || 'dark',
      // Hand-tuned per app (Safari's bar is taller with navigation controls on).
      ...(Number.isFinite(kept.titleBar) ? { titleBar: kept.titleBar } : {}),
      ...(kept.titleBarColor ? { titleBarColor: kept.titleBarColor } : {}),
    };
  }

  const fallback = zen.frameFor(config, null, dark);
  const out = {
    generatedAt: new Date().toISOString(),
    zenProfile: config.profile ? path.basename(config.profile) : null,
    space: config.space ? config.space.name : null,
    layout: {
      separation: config.layout.separation,
      innerRadius: config.layout.innerRadius,
      borderRadius: config.layout.borderRadius,
      // Hand-tuned lens values survive regeneration.
      ...(['lensLift', 'lensRadius', 'cardRadius'].reduce((o, k) => (Number.isFinite((previous.layout || {})[k]) ? { ...o, [k]: previous.layout[k] } : o), {})),
      hideDelay: config.layout.hideDelay,
      hideToolbar: config.layout.hideToolbar,
    },
    defaultFrame: flatten(fallback.css) || '#6f5b69',
    apps,
    siteColours: cache,
  };
  fs.mkdirSync(path.dirname(OUT), { recursive: true });
  fs.writeFileSync(OUT, `${JSON.stringify(out, null, 2)}\n`);
  process.stdout.write(`${JSON.stringify(out, null, 2)}\n`);
}

main();
