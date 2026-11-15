#!/usr/bin/env node
'use strict';

/*
 * Resolves a site name or URL into everything the installer needs:
 * canonical URL, display name, theme colour and icon candidates.
 *
 * Usage: node siteinfo.js "<name or url>" [--name "..."] [--drm] [--out file]
 */

const fs = require('node:fs');

const UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36';

/*
 * Aliases pin both the install URL and the identity the app should keep.
 * Following a sign-in redirect (accounts.google.com) must never rename a
 * Docs/Drive/Sheets/Slides PWA or swap its icon for a Google "G".
 */
const ALIASES = {
  netflix: { url: 'https://www.netflix.com', name: 'Netflix', accent: '#e50914' },
  gemini: { url: 'https://gemini.google.com', name: 'Gemini', accent: '#4285f4' },
  bard: { url: 'https://gemini.google.com', name: 'Gemini', accent: '#4285f4' },
  drive: { url: 'https://drive.google.com', name: 'Google Drive', accent: '#1a73e8' },
  'google drive': { url: 'https://drive.google.com', name: 'Google Drive', accent: '#1a73e8' },
  docs: { url: 'https://docs.google.com', name: 'Google Docs', accent: '#4285f4' },
  'google docs': { url: 'https://docs.google.com', name: 'Google Docs', accent: '#4285f4' },
  sheets: { url: 'https://docs.google.com/spreadsheets', name: 'Google Sheets', accent: '#0f9d58' },
  'google sheets': { url: 'https://docs.google.com/spreadsheets', name: 'Google Sheets', accent: '#0f9d58' },
  slides: { url: 'https://docs.google.com/presentation', name: 'Google Slides', accent: '#f4b400' },
  'google slides': { url: 'https://docs.google.com/presentation', name: 'Google Slides', accent: '#f4b400' },
  gmail: { url: 'https://mail.google.com', name: 'Gmail', accent: '#ea4335' },
  mail: { url: 'https://mail.google.com', name: 'Gmail', accent: '#ea4335' },
  calendar: { url: 'https://calendar.google.com', name: 'Google Calendar', accent: '#1a73e8' },
  meet: { url: 'https://meet.google.com', name: 'Google Meet', accent: '#00897b' },
  chat: { url: 'https://chat.google.com', name: 'Google Chat', accent: '#00ac47' },
  classroom: { url: 'https://classroom.google.com', name: 'Google Classroom', accent: '#0f9d58' },
  keep: { url: 'https://keep.google.com', name: 'Google Keep', accent: '#fbbc04' },
  photos: { url: 'https://photos.google.com', name: 'Google Photos', accent: '#4285f4' },
  maps: { url: 'https://maps.google.com', name: 'Google Maps', accent: '#1a73e8' },
  translate: { url: 'https://translate.google.com', name: 'Google Translate', accent: '#4285f4' },
  youtube: { url: 'https://www.youtube.com', name: 'YouTube', accent: '#ff0000' },
  whatsapp: { url: 'https://web.whatsapp.com', name: 'WhatsApp', accent: '#25d366' },
  'whatsapp web': { url: 'https://web.whatsapp.com', name: 'WhatsApp', accent: '#25d366' },
  spotify: { url: 'https://open.spotify.com', name: 'Spotify', accent: '#1db954' },
  chatgpt: { url: 'https://chatgpt.com', name: 'ChatGPT', accent: '#10a37f' },
  claude: { url: 'https://claude.ai', name: 'Claude', accent: '#d97757' },
  notion: { url: 'https://www.notion.so', name: 'Notion', accent: '' },
  figma: { url: 'https://www.figma.com', name: 'Figma', accent: '#f24e1e' },
  github: { url: 'https://github.com', name: 'GitHub', accent: '#2da44e' },
  reddit: { url: 'https://www.reddit.com', name: 'Reddit', accent: '#ff4500' },
  x: { url: 'https://x.com', name: 'X', accent: '' },
  twitter: { url: 'https://x.com', name: 'X', accent: '' },
  instagram: { url: 'https://www.instagram.com', name: 'Instagram', accent: '#e1306c' },
  linkedin: { url: 'https://www.linkedin.com', name: 'LinkedIn', accent: '#0a66c2' },
  slack: { url: 'https://app.slack.com', name: 'Slack', accent: '#611f69' },
  discord: { url: 'https://discord.com/app', name: 'Discord', accent: '#5865f2' },
  prime: { url: 'https://www.primevideo.com', name: 'Prime Video', accent: '#00a8e1' },
  'prime video': { url: 'https://www.primevideo.com', name: 'Prime Video', accent: '#00a8e1' },
  disney: { url: 'https://www.disneyplus.com', name: 'Disney+', accent: '#113ccf' },
  'disney+': { url: 'https://www.disneyplus.com', name: 'Disney+', accent: '#113ccf' },
  max: { url: 'https://www.max.com', name: 'Max', accent: '#002be7' },
  hbo: { url: 'https://www.max.com', name: 'Max', accent: '#002be7' },
};

/*
 * High-resolution app icons from the iTunes catalogue. Only exact name
 * matches are accepted, so a search for "Gemini" can never pick "Google".
 */
const STORE_TERMS = {
  'netflix': 'Netflix',
  'gemini': 'Google Gemini',
  'google drive': 'Google Drive',
  'google docs': 'Google Docs',
  'google sheets': 'Google Sheets',
  'google slides': 'Google Slides',
  'gmail': 'Gmail',
  'google calendar': 'Google Calendar',
  'google meet': 'Google Meet',
  'google chat': 'Google Chat',
  'google classroom': 'Google Classroom',
  'google keep': 'Google Keep',
  'google photos': 'Google Photos',
  'google maps': 'Google Maps',
  'google translate': 'Google Translate',
  'youtube': 'YouTube',
  'whatsapp': 'WhatsApp Messenger',
  'spotify': 'Spotify',
  'chatgpt': 'ChatGPT',
  'claude': 'Claude',
  'notion': 'Notion',
  'figma': 'Figma',
  'github': 'GitHub',
  'reddit': 'Reddit',
  'x': 'X',
  'instagram': 'Instagram',
  'linkedin': 'LinkedIn',
  'slack': 'Slack',
  'discord': 'Discord',
  'prime video': 'Amazon Prime Video',
  'disney+': 'Disney+',
  'max': 'Max: Stream HBO, TV, & Movies',
};

const DRM_HOSTS = [
  'netflix.com', 'primevideo.com', 'disneyplus.com', 'max.com', 'hbomax.com',
  'spotify.com', 'music.apple.com', 'tv.apple.com', 'hulu.com', 'paramountplus.com',
  'peacocktv.com', 'crunchyroll.com', 'twitch.tv',
];

const BRANDS = {
  'netflix': 'Netflix',
  'gemini': 'Gemini',
  'google drive': 'Google Drive',
  'google docs': 'Google Docs',
  'google sheets': 'Google Sheets',
  'google slides': 'Google Slides',
  'youtube': 'YouTube',
  'github': 'GitHub',
  'whatsapp': 'WhatsApp',
  'chatgpt': 'ChatGPT',
  'claude': 'Claude',
  'notion': 'Notion',
  'figma': 'Figma',
  'reddit': 'Reddit',
  'spotify': 'Spotify',
  'gmail': 'Gmail',
  'discord': 'Discord',
  'slack': 'Slack',
};

function parseArgs(argv) {
  const args = { input: '', name: '', drm: false, out: '', pin: false };
  const rest = [];
  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--name') { args.name = argv[++i] || ''; continue; }
    if (arg === '--out') { args.out = argv[++i] || ''; continue; }
    if (arg === '--drm') { args.drm = true; continue; }
    if (arg === '--pin') { args.pin = true; continue; }
    rest.push(arg);
  }
  args.input = rest.join(' ').trim();
  return args;
}

const args = parseArgs(process.argv);

function decodeEntities(value) {
  return String(value || '')
    .replace(/&#x([0-9a-f]+);/gi, (_, hex) => String.fromCodePoint(parseInt(hex, 16)))
    .replace(/&#(\d+);/g, (_, num) => String.fromCodePoint(parseInt(num, 10)))
    .replace(/&nbsp;/gi, ' ')
    .replace(/&amp;/gi, '&')
    .replace(/&lt;/gi, '<')
    .replace(/&gt;/gi, '>')
    .replace(/&quot;/gi, '"')
    .replace(/&#39;|&apos;/gi, "'");
}

function slugify(value) {
  return String(value || '')
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64) || 'site';
}

function prettyName(text) {
  const key = String(text || '').trim().toLowerCase();
  if (BRANDS[key]) return BRANDS[key];
  return String(text || '').trim().replace(/\s+/g, ' ');
}

function looksLikeURL(value) {
  return /^https?:\/\//i.test(value);
}

function looksLikeDomain(value) {
  return /^[a-z0-9][a-z0-9.-]*\.[a-z]{2,}(:\d+)?(\/.*)?$/i.test(value);
}

function normalizeURL(value) {
  const text = String(value || '').trim();
  if (looksLikeURL(text)) return text;
  return `https://${text.replace(/^\/+/, '')}`;
}

const HOST_ALIASES = {
  'netflix.com': 'netflix',
  'gemini.google.com': 'gemini',
  'drive.google.com': 'drive',
  'mail.google.com': 'gmail',
  'calendar.google.com': 'calendar',
  'meet.google.com': 'meet',
  'chat.google.com': 'chat',
  'classroom.google.com': 'classroom',
  'keep.google.com': 'keep',
  'photos.google.com': 'photos',
  'maps.google.com': 'maps',
  'translate.google.com': 'translate',
  'youtube.com': 'youtube',
  'web.whatsapp.com': 'whatsapp',
  'open.spotify.com': 'spotify',
  'chatgpt.com': 'chatgpt',
  'claude.ai': 'claude',
  'notion.so': 'notion',
  'figma.com': 'figma',
  'github.com': 'github',
  'reddit.com': 'reddit',
  'x.com': 'x',
  'twitter.com': 'x',
  'instagram.com': 'instagram',
  'linkedin.com': 'linkedin',
  'slack.com': 'slack',
  'discord.com': 'discord',
  'primevideo.com': 'prime',
  'disneyplus.com': 'disney',
  'max.com': 'max',
};

/* Recognise a URL as one of the known apps, even when it carries a path. */
function aliasFor(input) {
  const key = String(input || '').trim().toLowerCase();
  if (ALIASES[key]) return ALIASES[key];
  let parsed;
  try {
    const text = looksLikeURL(input) ? input : (looksLikeDomain(input) ? `https://${input}` : '');
    parsed = new URL(text);
  } catch {
    return null;
  }
  const host = parsed.host.replace(/^www\./, '');
  if (host === 'docs.google.com') {
    if (parsed.pathname.startsWith('/spreadsheets')) return ALIASES.sheets;
    if (parsed.pathname.startsWith('/presentation')) return ALIASES.slides;
    return ALIASES.docs;
  }
  const aliasKey = HOST_ALIASES[host];
  return aliasKey ? ALIASES[aliasKey] : null;
}

async function fetchWithTimeout(url, options = {}, timeout = 15000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);
  try {
    return await fetch(url, {
      redirect: 'follow',
      ...options,
      signal: controller.signal,
      headers: {
        'User-Agent': UA,
        'Accept-Language': 'en-US,en;q=0.9',
        Accept: 'text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8',
        ...(options.headers || {}),
      },
    });
  } finally {
    clearTimeout(timer);
  }
}

async function isReachable(url) {
  try {
    const response = await fetchWithTimeout(url, { method: 'GET' }, 12000);
    return response.ok ? response.url : null;
  } catch {
    return null;
  }
}

function normalizeName(value) {
  return String(value || '').toLowerCase().replace(/\s+/g, ' ').trim();
}

/*
 * The App Store artwork is 512×512, which is far better than the 16–32px
 * favicons most sign-in pages expose. We only accept an exact title match.
 */
async function storeIcon(name) {
  const term = STORE_TERMS[normalizeName(name)];
  if (!term) return null;
  try {
    const response = await fetchWithTimeout(
      `https://itunes.apple.com/search?term=${encodeURIComponent(term)}&entity=software&limit=8&country=us`,
      { headers: { Accept: 'application/json' } },
      10000,
    );
    if (!response.ok) return null;
    const data = await response.json();
    const wanted = normalizeName(term);
    const hit = (Array.isArray(data.results) ? data.results : [])
      .find((entry) => normalizeName(entry && entry.trackName) === wanted && entry.artworkUrl512);
    return hit ? hit.artworkUrl512 : null;
  } catch {
    return null;
  }
}

async function resolveURL(input, alias) {
  if (alias && alias.url) return alias.url;
  if (looksLikeURL(input)) return input;
  if (looksLikeDomain(input)) {
    return (await isReachable(normalizeURL(input))) || normalizeURL(input);
  }
  const direct = `https://www.${slugify(input).replace(/-/g, '')}.com`;
  const directHit = await isReachable(direct);
  if (directHit) return directHit;

  for (const search of [
    `https://html.duckduckgo.com/html/?q=${encodeURIComponent(`${input} official site`)}`,
    `https://lite.duckduckgo.com/lite/?q=${encodeURIComponent(input)}`,
  ]) {
    try {
      const response = await fetchWithTimeout(search, {}, 12000);
      const html = await response.text();
      const candidates = [];
      const anchorPattern = /<a[^>]+href="([^"]+)"[^>]*>(.*?)<\/a>/gi;
      let match;
      while ((match = anchorPattern.exec(html))) {
        let href = decodeEntities(match[1]);
        if (href.includes('uddg=')) {
          const query = href.split('uddg=')[1].split('&')[0];
          try { href = decodeURIComponent(query); } catch {}
        }
        if (!/^https?:\/\//i.test(href)) continue;
        if (/duckduckgo\.com|wikipedia\.org|youtube\.com\/watch/i.test(href)) continue;
        candidates.push(href);
        if (candidates.length >= 5) break;
      }
      for (const candidate of candidates) {
        const hit = await isReachable(candidate);
        if (hit) return hit;
      }
    } catch {}
  }
  return direct;
}

function pickMeta(html, name) {
  const patterns = [
    new RegExp(`<meta[^>]+name=["']${name}["'][^>]*content=["']([^"']*)["']`, 'i'),
    new RegExp(`<meta[^>]+content=["']([^"']*)["'][^>]*name=["']${name}["']`, 'i'),
    new RegExp(`<meta[^>]+property=["']${name}["'][^>]*content=["']([^"']*)["']`, 'i'),
    new RegExp(`<meta[^>]+content=["']([^"']*)["'][^>]*property=["']${name}["']`, 'i'),
  ];
  for (const pattern of patterns) {
    const match = pattern.exec(html);
    if (match) return decodeEntities(match[1]);
  }
  return '';
}

/*
 * Window tint. A theme colour only counts as a highlight if it has some
 * colour in it: many sites ship #000 / #fff as theme-color, which would
 * give every app the same grey rim.
 */
function normalizeHex(value) {
  const text = String(value || '').trim().toLowerCase();
  let m = /^#([0-9a-f]{3,4})$/.exec(text);
  if (m) return `#${m[1].slice(0, 3).split('').map((c) => c + c).join('')}`;
  m = /^#([0-9a-f]{6})([0-9a-f]{2})?$/.exec(text);
  if (m) return `#${m[1]}`;
  m = /^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/.exec(text);
  if (m) return `#${m.slice(1, 4).map((n) => Math.min(255, Number(n)).toString(16).padStart(2, '0')).join('')}`;
  return '';
}

function isHighlight(hex) {
  if (!hex) return false;
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const s = max === min ? 0 : (max - min) / (1 - Math.abs(2 * l - 1));
  return s >= 0.2 && l >= 0.12 && l <= 0.9;
}

// manifest theme_color → <meta name="theme-color"> → curated brand colour.
function pickAccent(candidates) {
  for (const [source, value] of candidates) {
    const hex = normalizeHex(value);
    if (isHighlight(hex)) return { accent: hex, source };
  }
  return { accent: '', source: '' };
}

function pickTitle(html) {
  const match = /<title[^>]*>([\s\S]*?)<\/title>/i.exec(html);
  return match ? decodeEntities(match[1]).replace(/\s+/g, ' ').trim() : '';
}

function pickLink(html, relPattern) {
  // Group the alternatives: an ungrouped `a|b` would split the whole regex
  // and match tags without their href (which dropped every icon link).
  const pattern = new RegExp(`<link[^>]+rel=["'](?:${relPattern})["'][^>]*>`, 'gi');
  const results = [];
  let match;
  while ((match = pattern.exec(html))) {
    const tag = match[0];
    const href = /href=["']([^"']+)["']/i.exec(tag);
    if (!href) continue;
    const sizes = /sizes=["']([^"']+)["']/i.exec(tag);
    const type = /type=["']([^"']+)["']/i.exec(tag);
    results.push({
      href: decodeEntities(href[1]),
      sizes: sizes ? sizes[1] : '',
      type: type ? type[1] : '',
    });
  }
  return results;
}

function sizeValue(sizes) {
  if (!sizes) return 0;
  const match = /(\d+)\s*x\s*(\d+)/i.exec(sizes);
  if (match) return Math.max(parseInt(match[1], 10), parseInt(match[2], 10));
  if (/any/i.test(sizes)) return 512;
  return 0;
}

function cleanTitle(title, host) {
  const text = String(title || '').replace(/\s+/g, ' ').trim();
  if (!text) return host;
  const segments = text.split(/\s+[|·—–]\s+|\s+-\s+|\s*::\s*/).map((s) => s.trim()).filter(Boolean);
  let candidate = segments[0] || text;
  if (candidate.includes(':')) candidate = candidate.split(':')[0].trim();
  if (!candidate || candidate.length > 42) candidate = segments[segments.length - 1] || text;
  if (candidate.length > 42) candidate = `${candidate.slice(0, 39).trim()}…`;
  return candidate;
}

async function fetchManifest(url) {
  try {
    const response = await fetchWithTimeout(url, {}, 12000);
    if (!response.ok) return null;
    const json = await response.json();
    return { json, base: response.url || url };
  } catch {
    return null;
  }
}

function absoluteURL(base, value) {
  try { return new URL(value, base).href; } catch { return value; }
}

async function main() {
  if (!args.input) {
    console.error('Usage: siteinfo.js "<name or url>" [--name "..."] [--drm] [--pin] [--out file]');
    process.exit(2);
  }

  const alias = aliasFor(args.input);
  const directURL = args.pin && (looksLikeURL(args.input) || looksLikeDomain(args.input))
    ? normalizeURL(args.input)
    : null;

  // For aliases the pinned URL is the identity: the account chooser that a
  // fetch lands on is only used for metadata, never for name/host/icon.
  const sourceURL = directURL || (alias ? alias.url : await resolveURL(args.input, null));
  const displayURL = directURL || (alias ? alias.url : sourceURL);
  const parsed = new URL(displayURL);
  const displayHost = parsed.host.replace(/^www\./, '');

  let html = '';
  let finalURL = sourceURL;
  try {
    const response = await fetchWithTimeout(sourceURL);
    if (response.ok) {
      finalURL = response.url || sourceURL;
      html = await response.text();
    } else if (response.status >= 300 && response.status < 400) {
      finalURL = response.headers.get('location') || sourceURL;
    }
  } catch {}

  const finalParsed = new URL(finalURL);
  const finalHost = finalParsed.host.replace(/^www\./, '');

  const appNameMeta = pickMeta(html, 'application-name');
  const ogSiteName = pickMeta(html, 'og:site_name');
  const appleTitle = pickMeta(html, 'apple-mobile-web-app-title');
  const pageTitle = pickTitle(html);
  const themeColor = pickMeta(html, 'theme-color') || pickMeta(html, 'msapplication-TileColor');

  let manifest = null;
  const manifestLink = pickLink(html, 'manifest')[0];
  const manifestURL = manifestLink ? absoluteURL(finalURL, manifestLink.href) : '';
  if (manifestURL) {
    const fetched = await fetchManifest(manifestURL);
    if (fetched && fetched.json && typeof fetched.json === 'object') {
      manifest = { ...fetched.json, __url: fetched.base };
    }
  }

  let name = args.name || (alias && alias.name) || appNameMeta
    || (manifest && (manifest.short_name || manifest.name)) || appleTitle || ogSiteName;
  if (!name) name = cleanTitle(pageTitle, displayHost);
  name = prettyName(name);
  if (!name) name = displayHost.split('.')[0].replace(/^\w/, (c) => c.toUpperCase());

  const icons = [];
  if (manifest && Array.isArray(manifest.icons)) {
    for (const icon of manifest.icons) {
      if (!icon || !icon.src) continue;
      icons.push({
        url: absoluteURL(manifest.__url || finalURL, icon.src),
        size: sizeValue(icon.sizes),
        type: icon.type || '',
        source: 'manifest',
        purpose: String(icon.purpose || 'any'),
      });
    }
  }
  for (const link of pickLink(html, 'apple-touch-icon|apple-touch-icon-precomposed')) {
    icons.push({ url: absoluteURL(finalURL, link.href), size: sizeValue(link.sizes) || 180, type: link.type || '', source: 'apple-touch' });
  }
  for (const link of pickLink(html, 'icon|shortcut icon|fluid-icon|mask-icon')) {
    icons.push({ url: absoluteURL(finalURL, link.href), size: sizeValue(link.sizes), type: link.type || '', source: 'icon' });
  }
  // Chrome only uses the site's own icons; these two are last resorts
  // (App Store art often has a bezel baked in, so it is never preferred).
  const storeIconURL = await storeIcon(name);
  if (storeIconURL) icons.push({ url: storeIconURL, size: 1024, type: 'image/jpeg', source: 'store' });
  icons.push({ url: `https://www.google.com/s2/favicons?domain=${encodeURIComponent(alias ? displayHost : finalHost)}&sz=256`, size: 256, type: 'image/png', source: 'google' });
  const rank = { manifest: 0, 'apple-touch': 1, icon: 2, store: 3, google: 4 };
  icons.sort((a, b) => (rank[a.source] - rank[b.source]) || (b.size - a.size));

  const slug = slugify(name);
  const drm = args.drm || DRM_HOSTS.some((suffix) => displayHost === suffix || displayHost.endsWith(`.${suffix}`));

  const tint = pickAccent([
    ['manifest', manifest && manifest.theme_color],
    ['meta', themeColor],
    ['brand', alias && alias.accent],
  ]);

  const output = {
    input: args.input,
    url: displayURL,
    name,
    host: displayHost,
    slug,
    drm,
    themeColor: tint.accent,
    accentSource: tint.source,
    title: alias ? alias.name : (pageTitle || name),
    resolvedUrl: finalURL,
    icons,
    iconUrl: icons[0] ? icons[0].url : '',
    manifest: manifestURL || '',
  };

  const json = JSON.stringify(output, null, 2);
  if (args.out) fs.writeFileSync(args.out, json);
  process.stdout.write(json);
}

main().catch((error) => {
  console.error(`siteinfo: ${error && error.stack || error}`);
  process.exit(1);
});
