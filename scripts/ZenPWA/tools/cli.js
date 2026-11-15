#!/usr/bin/env node
'use strict';

/*
 * zen-pwa — installs Zen-style site-specific browsers on top of castLabs
 * Electron (ECS) so Widevine playback works outside Zen.
 *
 * Each installed app is a clone of the shared ECS runtime with:
 *   Contents/Resources/app       -> symlink to the shared shell code
 *   Contents/Resources/site.json -> that site's configuration
 *   Contents/Resources/SiteIcon.icns
 *   Contents/Info.plist          -> per-site name / bundle id / icon
 *
 * APFS clones keep the extra bundles almost free, and because `app` is a
 * symlink every app picks up shell updates immediately.
 */

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const HOME = os.homedir();
const ROOT = path.join(HOME, 'YZKR', 'scripts', 'ZenPWA');
const RUNTIME = path.join(ROOT, 'node_modules', 'electron', 'dist', 'Electron.app');
const RUNTIME_DIST = path.join(ROOT, 'node_modules', 'electron', 'dist');
const APP_CODE = path.join(ROOT, 'app');
const SITEINFO = path.join(ROOT, 'tools', 'siteinfo.js');
const ICONMAKER = path.join(ROOT, 'tools', '.build', 'iconmaker');
const ICONMAKER_SRC = path.join(ROOT, 'tools', 'iconmaker.swift');
const EVS_VENV = path.join(ROOT, '.evs');
const EVS_PY = path.join(EVS_VENV, 'bin', 'python');
const EVS_CONFIG = path.join(HOME, '.config', 'evs', 'config.json');
// Per-PWA window tint overrides, { "<slug>": "#rrggbb" }. Read by the shell at launch.
const ACCENTS_FILE = path.join(HOME, '.config', 'zen-pwa', 'accents.json');
const USER_DIR = path.join(HOME, 'Applications');
const SYSTEM_DIR = '/Applications';
const MARKER = 'ZenPWA.json';
const VERSION = '5.0.0';
const LSREGISTER = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister';
const VMP_SIG = path.join('Contents', 'Frameworks', 'Electron Framework.framework', 'Versions', 'A', 'Resources', 'Electron Framework.sig');

const COMMANDS = new Set(['install', 'list', 'remove', 'rebuild', 'doctor', 'sign', 'update', 'open', 'help']);
const VALUE_FLAGS = new Set(['site', 'url', 'name', 'dir', 'accent']);
const KNOWN_FLAGS = new Set([
  ...VALUE_FLAGS,
  'drm', 'force', 'help', 'purge', 'quiet', 'replace', 'system', 'no-open', 'version',
  ...COMMANDS,
]);
const LEGACY_IDS = /^app\.zen\.pwa(\.|$)/i;

const color = process.stdout.isTTY && !process.env.NO_COLOR;
const paint = (code, text) => (color ? `\x1b[${code}m${text}\x1b[0m` : String(text));
const bold = (text) => paint('1', text);
const dim = (text) => paint('2', text);
const red = (text) => paint('31', text);
const green = (text) => paint('32', text);
const yellow = (text) => paint('33', text);
const blue = (text) => paint('36', text);

function say(...lines) {
  for (const line of lines) process.stdout.write(`${line}\n`);
}

function fail(message) {
  process.stderr.write(`${red('zen-pwa:')} ${message}\n`);
  process.exit(1);
}

function parseFlags(argv) {
  const flags = {};
  const words = [];
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '-h') { flags.help = true; continue; }
    if (!arg.startsWith('--')) { words.push(arg); continue; }
    const eq = arg.indexOf('=');
    let key;
    let value;
    if (eq !== -1) {
      key = arg.slice(2, eq);
      value = arg.slice(eq + 1);
    } else {
      key = arg.slice(2);
      value = true;
    }
    if (VALUE_FLAGS.has(key) && value === true && argv[i + 1] && !argv[i + 1].startsWith('--')) {
      value = argv[++i];
    }
    flags[key] = value;
  }

  // `zen --pwa --list` / `--doctor` / `--sign` / `--rebuild` style switches.
  const switchCommands = ['list', 'doctor', 'sign', 'rebuild', 'update', 'remove', 'open'];
  let command = 'install';
  if (words.length && COMMANDS.has(words[0])) command = words.shift();
  else {
    const switched = switchCommands.find((name) => flags[name]);
    if (switched) command = switched;
  }
  return { command, flags, args: words };
}

function run(cmd, args, options = {}) {
  return spawnSync(cmd, args, { encoding: 'utf8', ...options });
}

function runOrFail(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, { stdio: 'inherit', ...options });
  if (result.status !== 0) throw new Error(`${cmd} exited with ${result.status}`);
  return result;
}

function readJSON(file, fallback = null) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return fallback; }
}

function writeJSON(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function slugify(value) {
  return String(value || '')
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64) || 'site';
}

function sanitizeName(value) {
  const name = String(value || '')
    .replace(/[\u0000-\u001f]/g, '')
    .replace(/[/:]/g, '-')
    .replace(/\s+/g, ' ')
    .replace(/^\.+/, '')
    .trim()
    .slice(0, 64);
  return name || 'Zen PWA';
}

function frameworkSignature(app) {
  return path.join(app, VMP_SIG);
}

function isOurBundle(app) {
  return fs.existsSync(path.join(app, 'Contents', 'Resources', MARKER));
}

function bundleIdentifier(app) {
  const plist = path.join(app, 'Contents', 'Info.plist');
  if (!fs.existsSync(plist)) return '';
  const result = run('plutil', ['-extract', 'CFBundleIdentifier', 'raw', '-o', '-', plist]);
  return result.status === 0 ? String(result.stdout || '').trim() : '';
}

function isLegacyBundle(app) {
  const id = bundleIdentifier(app);
  if (LEGACY_IDS.test(id)) return true;
  try {
    const exe = path.join(app, 'Contents', 'MacOS', path.basename(app, '.app'));
    const body = fs.readFileSync(exe, 'utf8');
    return body.includes('ZenPWA-bin');
  } catch {
    return false;
  }
}

function bundleSite(app) {
  return readJSON(path.join(app, 'Contents', 'Resources', 'site.json'), null);
}

function listBundles() {
  const found = [];
  const seen = new Set();
  for (const dir of [USER_DIR, SYSTEM_DIR]) {
    if (!fs.existsSync(dir)) continue;
    let entries = [];
    try { entries = fs.readdirSync(dir); } catch { continue; }
    for (const entry of entries) {
      if (!entry.endsWith('.app')) continue;
      const app = path.join(dir, entry);
      if (!isOurBundle(app) && !isLegacyBundle(app)) continue;
      if (seen.has(app)) continue;
      seen.add(app);
      found.push({
        path: app,
        name: entry.slice(0, -4),
        site: bundleSite(app) || {},
        legacy: !isOurBundle(app),
      });
    }
  }
  return found.sort((a, b) => a.name.localeCompare(b.name));
}

function matches(bundle, query) {
  const text = String(query || '').toLowerCase();
  return bundle.name.toLowerCase() === text
    || String(bundle.site.slug || '').toLowerCase() === text
    || slugify(bundle.name) === slugify(text);
}

function removePath(target) {
  try {
    fs.rmSync(target, { recursive: true, force: true });
    return;
  } catch (error) {
    if (!['EPERM', 'EACCES'].includes(error.code)) throw error;
  }
  if (!process.stdin.isTTY) {
    fail(`Need administrator rights to remove ${target}.\nRun: sudo rm -rf "${target}"`);
  }
  say(yellow(`macOS will ask for your password to remove ${target}`));
  const script = `do shell script "/bin/rm -rf " & quoted form of ${JSON.stringify(target)} with administrator privileges`;
  const result = spawnSync('osascript', ['-e', script], { stdio: 'inherit' });
  if (result.status !== 0 || fs.existsSync(target)) {
    fail(`Could not remove ${target}.`);
  }
}

function ensureRuntime() {
  if (!fs.existsSync(RUNTIME)) {
    fail(`The Electron runtime is missing.\nRun ${bold(path.join(ROOT, 'build.sh'))} to install it.`);
  }
  if (!fs.existsSync(path.join(APP_CODE, 'main.js'))) {
    fail(`The Zen PWA shell is missing at ${APP_CODE}.`);
  }
}

function runtimeVersion() {
  try { return fs.readFileSync(path.join(RUNTIME_DIST, 'version'), 'utf8').trim(); } catch { return 'unknown'; }
}

function hasVmpSignature(app) {
  return fs.existsSync(frameworkSignature(app));
}

function cloneRuntime(dest) {
  fs.rmSync(dest, { recursive: true, force: true });
  let result = spawnSync('cp', ['-cR', RUNTIME, dest], { stdio: 'pipe' });
  if (result.status !== 0) {
    result = spawnSync('cp', ['-R', RUNTIME, dest], { stdio: 'inherit' });
  }
  if (result.status !== 0 || !fs.existsSync(dest)) {
    throw new Error('Could not clone the Electron runtime (is the disk full?)');
  }
}

function injectShell(dest) {
  const resources = path.join(dest, 'Contents', 'Resources');
  fs.rmSync(path.join(resources, 'default_app.asar'), { force: true });
  fs.rmSync(path.join(resources, 'app.asar'), { force: true });
  fs.rmSync(path.join(resources, 'app'), { recursive: true, force: true });
  fs.symlinkSync(APP_CODE, path.join(resources, 'app'), 'dir');
  // The clone inherits castLabs' signature; only the outer bundle changes.
  fs.rmSync(path.join(dest, 'Contents', '_CodeSignature'), { recursive: true, force: true });
}

function writeSiteFiles(dest, site, name) {
  const resources = path.join(dest, 'Contents', 'Resources');
  const installedAt = new Date().toISOString();
  writeJSON(path.join(resources, 'site.json'), {
    generator: 'zen-pwa',
    version: VERSION,
    name,
    url: site.url,
    slug: site.slug || slugify(name),
    host: site.host,
    drm: Boolean(site.drm),
    accent: site.themeColor || site.accent || '',
    accentSource: site.accentSource || (site.themeColor || site.accent ? 'site' : ''),
    iconUrl: site.iconUrl || '',
    installedAt,
  });
  writeJSON(path.join(resources, MARKER), {
    generator: 'zen-pwa',
    version: VERSION,
    slug: site.slug || slugify(name),
    name,
    installedAt,
  });
}

function siteAccent(site) {
  const value = String(site.themeColor || site.accent || '');
  return /^#[0-9a-f]{6}$/i.test(value) ? value : '#5b8cff';
}

function setAccentOverride(slug, value) {
  if (!slug || value === undefined) return;
  const accents = readJSON(ACCENTS_FILE, {}) || {};
  if (value === true || /^(auto|site|none|clear)$/i.test(String(value))) {
    delete accents[slug];
  } else {
    const hex = String(value).trim().toLowerCase();
    if (!/^#[0-9a-f]{6}$/.test(hex)) fail(`--accent wants a colour like #e50914 (or "auto" to clear), got "${value}".`);
    accents[slug] = hex;
  }
  fs.mkdirSync(path.dirname(ACCENTS_FILE), { recursive: true });
  writeJSON(ACCENTS_FILE, accents);
}

async function fetchBuffer(url) {
  if (!/^https?:/i.test(String(url || ''))) return null;
  try {
    const response = await fetch(url, {
      redirect: 'follow',
      signal: AbortSignal.timeout(20000),
      headers: {
        'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36',
        Accept: 'image/avif,image/webp,image/png,image/svg+xml,image/*,*/*;q=0.8',
      },
    });
    if (!response.ok) return null;
    const buffer = Buffer.from(await response.arrayBuffer());
    return buffer.length > 64 ? buffer : null;
  } catch {
    return null;
  }
}

function imageWidth(file) {
  const result = run('sips', ['-g', 'pixelWidth', file]);
  if (result.status !== 0) return 0;
  const match = /pixelWidth:\s*(\d+)/.exec(result.stdout || '');
  return match ? Number(match[1]) : 0;
}

function ensureIconmaker() {
  if (fs.existsSync(ICONMAKER)) return true;
  if (!fs.existsSync(ICONMAKER_SRC)) return false;
  fs.mkdirSync(path.dirname(ICONMAKER), { recursive: true });
  const result = spawnSync('swiftc', ['-O', ICONMAKER_SRC, '-o', ICONMAKER], { stdio: 'inherit' });
  return result.status === 0 && fs.existsSync(ICONMAKER);
}

async function buildIcons(site, dest, name) {
  const resources = path.join(dest, 'Contents', 'Resources');
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'zen-pwa-icon-'));
  const source = path.join(work, 'source');
  const canvas = path.join(work, 'canvas.png');
  try {
    const candidates = Array.isArray(site.icons) ? site.icons.map((icon) => icon && icon.url) : [];
    const iconURL = candidates.find((url) => /^https?:/i.test(String(url))) || site.iconUrl;
    let input = null;
    let mode = 'monogram';
    if (iconURL) {
      const buffer = await fetchBuffer(iconURL);
      if (buffer) {
        fs.writeFileSync(source, buffer);
        const width = imageWidth(source);
        if (width > 0) {
          input = source;
          mode = width >= 220 ? 'full' : 'tile';
        }
      }
    }

    const accent = siteAccent(site);
    if (!ensureIconmaker()) throw new Error('iconmaker is not available');
    const monogram = String(name || 'Z').trim().charAt(0).toUpperCase() || 'Z';
    const make = (args) => spawnSync(ICONMAKER, args, { stdio: 'inherit' });

    if (input) {
      let result = make([input, canvas, accent, mode, monogram]);
      if (result.status !== 0) result = make(['-', canvas, accent, 'monogram', monogram]);
    } else {
      make(['-', canvas, accent, 'monogram', monogram]);
    }
    if (!fs.existsSync(canvas)) throw new Error('could not render the app icon');

    // SiteIcon.png powers the splash screen and the runtime Dock icon.
    runOrFail('sips', ['-z', '512', '512', canvas, '--out', path.join(resources, 'SiteIcon.png')], { stdio: 'pipe' });

    const iconset = path.join(work, 'SiteIcon.iconset');
    fs.mkdirSync(iconset, { recursive: true });
    const sizes = [
      [16, 'icon_16x16.png'],
      [32, 'icon_16x16@2x.png'],
      [32, 'icon_32x32.png'],
      [64, 'icon_32x32@2x.png'],
      [128, 'icon_128x128.png'],
      [256, 'icon_128x128@2x.png'],
      [256, 'icon_256x256.png'],
      [512, 'icon_256x256@2x.png'],
      [512, 'icon_512x512.png'],
      [1024, 'icon_512x512@2x.png'],
    ];
    for (const [size, file] of sizes) {
      run('sips', ['-z', String(size), String(size), canvas, '--out', path.join(iconset, file)], { stdio: 'pipe' });
    }
    const icns = run('iconutil', ['-c', 'icns', iconset, '-o', path.join(resources, 'SiteIcon.icns')]);
    if (icns.status !== 0) throw new Error('iconutil could not build SiteIcon.icns');
    return true;
  } catch (error) {
    say(yellow(`  ! Icon generation failed (${error.message}); using a monogram.`));
    try {
      const accent = siteAccent(site);
      const monogram = String(name || 'Z').trim().charAt(0).toUpperCase() || 'Z';
      if (ensureIconmaker() && spawnSync(ICONMAKER, ['-', canvas, accent, 'monogram', monogram], { stdio: 'inherit' }).status === 0) {
        run('sips', ['-z', '512', '512', canvas, '--out', path.join(resources, 'SiteIcon.png')], { stdio: 'pipe' });
        const iconset = path.join(work, 'Fallback.iconset');
        fs.mkdirSync(iconset, { recursive: true });
        for (const [size, file] of [[16, 'icon_16x16.png'], [32, 'icon_16x16@2x.png'], [128, 'icon_128x128.png'], [256, 'icon_128x128@2x.png'], [512, 'icon_512x512.png'], [1024, 'icon_512x512@2x.png']]) {
          run('sips', ['-z', String(size), String(size), canvas, '--out', path.join(iconset, file)], { stdio: 'pipe' });
        }
        run('iconutil', ['-c', 'icns', iconset, '-o', path.join(resources, 'SiteIcon.icns')]);
      }
    } catch {}
    return false;
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

function patchInfoPlist(dest, site, name) {
  const plist = path.join(dest, 'Contents', 'Info.plist');
  const converted = run('plutil', ['-convert', 'json', '-o', '-', plist]);
  if (converted.status !== 0) throw new Error('could not read Info.plist');
  const info = JSON.parse(converted.stdout);
  info.CFBundleName = name;
  info.CFBundleDisplayName = name;
  info.CFBundleIdentifier = `app.zenpwa.${site.slug || slugify(name)}`;
  info.CFBundleIconFile = 'SiteIcon';
  info.CFBundleShortVersionString = VERSION;
  info.CFBundleVersion = '5';
  info.LSApplicationCategoryType = 'public.app-category.productivity';
  const tmp = path.join(dest, 'Contents', 'Info.json');
  fs.writeFileSync(tmp, JSON.stringify(info, null, 2));
  runOrFail('plutil', ['-convert', 'xml1', '-o', plist, tmp], { stdio: 'pipe' });
  fs.rmSync(tmp, { force: true });
}

function signBundle(dest) {
  // Only the outer bundle is re-signed. Nested frameworks keep castLabs'
  // signature (and any VMP signature) untouched.
  const args = ['--force', '--sign', '-', '--preserve-metadata=entitlements,requirements,flags', dest];
  let result = run('xattr', ['-cr', dest]);
  if (result.status !== 0) { /* xattr is best effort */ }
  result = run('codesign', args);
  if (result.status === 0) return 'ad-hoc';
  result = run('codesign', ['--force', '--sign', '-', dest]);
  if (result.status === 0) return 'ad-hoc';
  return null;
}

function register(dest) {
  try { fs.utimesSync(dest, new Date(), new Date()); } catch {}
  run(LSREGISTER, ['-f', dest]);
}

async function resolveSite(input, flags) {
  const target = flags.url || flags.site || input;
  if (!target) fail('Tell me which site to install, for example: zen --pwa "Netflix"');
  const tmp = path.join(os.tmpdir(), `zen-pwa-site-${process.pid}-${Date.now()}.json`);
  const args = [SITEINFO, String(target), '--out', tmp];
  if (flags.name) args.push('--name', String(flags.name));
  if (flags.drm) args.push('--drm');
  if (flags.url || flags.site) args.push('--pin');
  const result = spawnSync(process.execPath, args, { encoding: 'utf8' });
  if (result.status !== 0 || !fs.existsSync(tmp)) {
    fs.rmSync(tmp, { force: true });
    fail(`Could not work out the site details.\n${(result.stderr || '').trim()}`);
  }
  const site = readJSON(tmp);
  fs.rmSync(tmp, { force: true });
  if (!site || !site.url) fail('Could not work out the site URL.');
  site.slug = site.slug || slugify(site.name);
  return site;
}

function installDirectory(flags) {
  if (flags.dir) return path.resolve(String(flags.dir).replace(/^~(?=$|\/)/, HOME));
  if (flags.system) return SYSTEM_DIR;
  return USER_DIR;
}

async function buildApp(site, flags, nameOverride) {
  ensureRuntime();
  const name = sanitizeName(nameOverride || flags.name || site.name);
  const dir = installDirectory(flags);
  fs.mkdirSync(dir, { recursive: true });
  const dest = path.join(dir, `${name}.app`);
  const existed = fs.existsSync(dest);

  if (existed) {
    if (isOurBundle(dest) || isLegacyBundle(dest) || flags.replace || flags.force) {
      removePath(dest);
    } else {
      fail(`${dest} already exists and was not created by zen-pwa.\nRe-run with --replace if you really want to overwrite it.`);
    }
  }

  // Remove older copies of this app from either install location.
  for (const other of listBundles()) {
    if (other.path === dest) continue;
    if (other.name.toLowerCase() !== name.toLowerCase()) continue;
    removePath(other.path);
  }

  cloneRuntime(dest);
  injectShell(dest);
  writeSiteFiles(dest, { ...site, slug: site.slug || slugify(name) }, name);
  await buildIcons(site, dest, name);
  patchInfoPlist(dest, { ...site, slug: site.slug || slugify(name) }, name);
  const signature = signBundle(dest);
  register(dest);

  return { dest, name, existed, signature, vmp: hasVmpSignature(dest) };
}

function printInstalled(result, site) {
  say('');
  say(`${green('Installed')} ${bold(result.name)} ${dim(`(${site.host})`)}`);
  say(`  ${dim(result.dest)}`);
  if (site.drm) {
    if (result.vmp) say(`  Widevine: ${green('VMP signature present')}`);
    else say(`  Widevine: ${yellow('CDM ready, production VMP signature missing')} ${dim('(run: zen --pwa --sign)')}`);
  }
  if (!result.signature) say(yellow('  Warning: the app could not be re-signed and may not launch.'));
}

async function commandInstall(flags, args) {
  const site = await resolveSite(args.join(' '), flags);
  say(`${blue('zen-pwa')} ${dim(`· ${site.name}`)}`);
  say(`  ${site.url}`);
  const result = await buildApp(site, flags);
  setAccentOverride(site.slug, flags.accent);
  printInstalled(result, site);
  if (!flags['no-open'] && !flags.quiet) {
    const opened = run('open', [result.dest]);
    if (opened.status !== 0) say(yellow(`  Could not launch automatically: open ${result.dest}`));
  }
}

function commandList() {
  const apps = listBundles();
  if (!apps.length) {
    say(`No Zen PWAs installed yet. Try: ${bold('zen --pwa "Netflix"')}`);
    return;
  }
  const evsReady = fs.existsSync(EVS_CONFIG);
  const width = Math.min(30, Math.max(...apps.map((app) => app.name.length)));
  say(bold(`Zen PWAs ${dim(`(${apps.length})`)}`));
  say('');
  for (const app of apps) {
    const host = app.site.host || (app.legacy ? 'old WebKit build' : '');
    const flags = [];
    if (app.legacy) flags.push(yellow('legacy'));
    if (app.site.drm) {
      const signed = hasVmpSignature(app.path);
      flags.push(signed && evsReady ? green('widevine') : yellow('needs signing'));
    }
    say(`  ${app.name.padEnd(width)}  ${dim(String(host).padEnd(24))}  ${dim(app.path)}${flags.length ? `  [${flags.join(', ')}]` : ''}`);
  }
  if (apps.some((app) => app.site.drm) && !evsReady) {
    say('');
    say(dim('DRM apps need a one-time castLabs EVS signature: ') + bold('zen --pwa --sign'));
  }
}

function commandRemove(flags, args) {
  const query = args.join(' ').trim();
  if (!query) fail('Which app? Try: zen --pwa --remove Netflix');
  const apps = listBundles().filter((app) => matches(app, query));
  if (!apps.length) fail(`No installed PWA matches "${query}".`);
  for (const app of apps) {
    removePath(app.path);
    say(`${green('Removed')} ${app.name} ${dim(app.path)}`);
    if (flags.purge && app.site.slug) {
      const profile = path.join(HOME, 'Library', 'Application Support', 'Zen PWA', app.site.slug);
      fs.rmSync(profile, { recursive: true, force: true });
      say(dim(`  deleted profile ${profile}`));
    }
  }
}

async function commandRebuild(flags, args) {
  const apps = listBundles().filter((app) => !args.length || matches(app, args.join(' ')));
  if (!apps.length) fail(args.length ? `No installed PWA matches "${args.join(' ')}".` : 'No PWAs to rebuild.');
  ensureRuntime();
  for (const app of apps) {
    let site = app.site;
    if (!site || !site.url) {
      say(dim(`Resolving metadata for ${app.name}…`));
      site = await resolveSite(app.name, { ...flags, name: app.name });
    }
    say(`${blue('rebuilding')} ${app.name} ${dim(`(${site.host || site.url})`)}`);
    const result = await buildApp(site, { ...flags, replace: true }, app.name);
    setAccentOverride(site.slug || slugify(app.name), flags.accent);
    say(`  ${green('done')} ${dim(result.dest)}${result.vmp ? `  ${green('VMP')}` : ''}`);
  }
  say('');
  say('Shell updates apply immediately; the bundles now point at the current runtime.');
}

function widevineCdmVersions() {
  const base = path.join(HOME, 'Library', 'Application Support', 'Zen PWA');
  const versions = new Set();
  if (!fs.existsSync(base)) return [];
  for (const profile of fs.readdirSync(base)) {
    const cdm = path.join(base, profile, 'WidevineCdm');
    if (!fs.existsSync(cdm)) continue;
    for (const entry of fs.readdirSync(cdm)) {
      if (/^\d+\./.test(entry)) versions.add(entry);
    }
  }
  return [...versions].sort();
}

function commandDoctor() {
  const rows = [];
  const add = (label, ok, detail) => rows.push({ label, ok, detail });

  add('Node.js', true, process.version);
  const runtime = fs.existsSync(RUNTIME);
  add('Electron runtime', runtime, runtime ? `${runtimeVersion()} ${dim(RUNTIME)}` : 'missing — run build.sh');

  const ecs = runtime && /wvcus/i.test(runtimeVersion());
  add('castLabs ECS build', ecs, ecs ? 'Widevine-capable' : 'not the castLabs build');

  const shell = ['main.js', 'preload.js', 'package.json'].every((file) => fs.existsSync(path.join(APP_CODE, file)));
  add('Zen PWA shell', shell, shell ? APP_CODE : 'missing app files');

  const iconmaker = fs.existsSync(ICONMAKER);
  add('Icon composer', iconmaker, iconmaker ? ICONMAKER : 'will be compiled on first install');

  const cdm = widevineCdmVersions();
  add('Widevine CDM', cdm.length > 0, cdm.length ? cdm.join(', ') : 'will download on first launch');

  const signedRuntime = runtime && fs.existsSync(path.join(RUNTIME, VMP_SIG));
  const evsReady = fs.existsSync(EVS_CONFIG);
  add('VMP signature', signedRuntime && evsReady, signedRuntime
    ? (evsReady ? 'EVS-signed for production playback' : 'development signature — run zen --pwa --sign for Netflix')
    : 'not signed — Netflix will refuse production playback');

  add('EVS client', fs.existsSync(EVS_PY), fs.existsSync(EVS_PY) ? EVS_PY : 'installed by zen --pwa --sign');
  add('EVS account', evsReady, evsReady ? EVS_CONFIG : 'not signed up yet');

  say(bold('zen-pwa doctor'));
  say('');
  for (const row of rows) {
    const mark = row.ok ? green('✓') : yellow('•');
    say(`  ${mark} ${row.label.padEnd(20)} ${dim(row.detail)}`);
  }

  const apps = listBundles();
  say('');
  say(bold(`Installed PWAs ${dim(`(${apps.length})`)}`));
  if (!apps.length) say(dim('  none'));
  for (const app of apps) {
    const widevine = app.site.drm && (hasVmpSignature(app.path) && evsReady ? green('widevine ready') : yellow('needs EVS signature'));
    const bits = [
      app.legacy ? yellow('legacy WebKit bundle') : 'Electron',
      widevine || null,
    ].filter(Boolean);
    say(`  ${app.name.padEnd(24)} ${dim(bits.join(' · '))}`);
  }

  if (!evsReady) {
    say('');
    say('Next step for Netflix:');
    say(`  ${bold('zen --pwa --sign')}   ${dim('free castLabs account, one-time, unlocks Netflix/Prime/Disney+')}`);
    say(dim('  The signature is written into the runtime and every installed app.'));
  }
}

function ensureEvs() {
  if (!fs.existsSync(EVS_PY)) {
    say(dim('Creating a private Python environment for castLabs EVS…'));
    runOrFail('python3', ['-m', 'venv', EVS_VENV], { stdio: 'inherit' });
  }
  const check = run(EVS_PY, ['-c', 'import castlabs_evs']);
  if (check.status !== 0) {
    say(dim('Installing castlabs-evs…'));
    runOrFail(EVS_PY, ['-m', 'pip', 'install', '--upgrade', 'castlabs-evs'], { stdio: 'inherit' });
  }
}

function commandSign(flags, args) {
  if (!fs.existsSync(RUNTIME)) fail('Install the runtime first: run build.sh');
  ensureEvs();

  if (!fs.existsSync(EVS_CONFIG)) {
    if (!process.stdin.isTTY) {
      fail('No EVS account yet. Run “zen --pwa --sign” in Terminal and follow the signup prompts.');
    }
    say('');
    say(bold('castLabs EVS signup'));
    say(dim('Free account. You will be asked for an email address, a name, an account name'));
    say(dim('and a password, then for the confirmation code sent to your email.'));
    say('');
    runOrFail(EVS_PY, ['-m', 'castlabs_evs.account', 'signup'], { stdio: 'inherit' });
    if (!fs.existsSync(EVS_CONFIG)) fail('Signup did not complete.');
  }

  say('');
  say(bold('Signing the shared Electron runtime with VMP…'));
  const result = spawnSync(EVS_PY, ['-m', 'castlabs_evs.vmp', 'sign-pkg', RUNTIME_DIST], { stdio: 'inherit' });
  if (result.status !== 0) {
    fail('VMP signing failed. Check your connection and EVS account, then retry.');
  }
  if (!fs.existsSync(path.join(RUNTIME, VMP_SIG))) {
    fail('EVS reported success but no signature was written.');
  }

  const apps = listBundles().filter((app) => !app.legacy);
  let synced = 0;
  for (const app of apps) {
    const target = frameworkSignature(app.path);
    if (!fs.existsSync(path.dirname(target))) continue;
    fs.copyFileSync(path.join(RUNTIME, VMP_SIG), target);
    synced += 1;
  }

  say('');
  say(`${green('Done')} — the runtime and ${synced} installed app${synced === 1 ? '' : 's'} now carry a VMP signature.`);
  say(dim('Netflix and other DRM services will accept production playback on the next launch.'));
  if (args.length) say(dim(`(Requested: ${args.join(' ')})`));
}

function commandUpdate() {
  ensureRuntime();
  say(bold('Updating the ECS runtime and shell…'));
  runOrFail('npm', ['install', '--no-audit', '--no-fund'], { cwd: ROOT, stdio: 'inherit' });
  runOrFail(process.execPath, [path.join(ROOT, 'scripts', 'fix-electron-symlinks.js')], { stdio: 'inherit' });
  if (!fs.existsSync(path.join(RUNTIME, VMP_SIG))) {
    say('');
    say(yellow('The runtime was replaced, so its VMP signature is gone.'));
    say(`Run ${bold('zen --pwa --sign')} once more to restore Netflix playback.`);
  }
}

function commandOpen(args) {
  const query = args.join(' ').trim();
  if (!query) fail('Which app? Try: zen --pwa --open Netflix');
  const app = listBundles().find((entry) => matches(entry, query));
  if (!app) fail(`No installed PWA matches "${query}".`);
  runOrFail('open', [app.path], { stdio: 'ignore' });
}

function usage() {
  say(`${bold('zen-pwa')} ${dim(VERSION)} — Zen-style site apps with real Widevine`);
  say('');
  say(bold('Install'));
  say(`  zen --pwa "Netflix"                  ${dim('resolve the site and install it')}`);
  say(`  zen --pwa --site https://example.com  ${dim('install an exact URL')}`);
  say(`  zen --pwa "Google Docs" --name Docs   ${dim('override the app name')}`);
  say(`  zen --pwa --system "Netflix"          ${dim('install into /Applications')}`);
  say(`  zen --pwa --rebuild Netflix --accent "#e50914"  ${dim('pin the window tint (auto clears)')}`);
  say('');
  say(bold('Manage'));
  say(`  zen --pwa --list                      ${dim('show installed apps')}`);
  say(`  zen --pwa --open Netflix              ${dim('launch an installed app')}`);
  say(`  zen --pwa --remove Netflix            ${dim('delete an app (add --purge for its profile)')}`);
  say(`  zen --pwa --rebuild                   ${dim('rebuild every app against the current runtime')}`);
  say(`  zen --pwa --update                    ${dim('update Electron and rebuild the shell')}`);
  say('');
  say(bold('Widevine'));
  say(`  zen --pwa --doctor                    ${dim('check the runtime, CDM and signatures')}`);
  say(`  zen --pwa --sign                      ${dim('free castLabs EVS signup + VMP signature')}`);
  say('');
  say(dim('Apps live in ~/Applications by default. Each has its own profile in'));
  say(dim('~/Library/Application Support/Zen PWA, and shares the shell code with all others.'));
}

async function main() {
  const { command, flags, args } = parseFlags(process.argv.slice(2));
  const unknown = Object.keys(flags).filter((key) => !KNOWN_FLAGS.has(key));
  if (unknown.length) {
    fail(`Unknown option${unknown.length > 1 ? 's' : ''}: ${unknown.map((key) => `--${key}`).join(', ')}\nRun: zen --pwa --help`);
  }
  if (flags.help || flags.version || command === 'help') return usage();

  switch (command) {
    case 'list': return commandList();
    case 'remove': return commandRemove(flags, args);
    case 'rebuild': return commandRebuild(flags, args);
    case 'doctor': return commandDoctor();
    case 'sign': return commandSign(flags, args);
    case 'update': return commandUpdate();
    case 'open': return commandOpen(args);
    case 'install':
    default:
      return commandInstall(flags, args);
  }
}

main().catch((error) => {
  fail(error && error.message ? error.message : String(error));
});
