#!/usr/bin/env node
'use strict';

/*
 * npm installs castLabs' Electron from a GitHub tarball, which loses the
 * relative symlinks inside the .framework bundles on macOS. Without them
 * the Electron binary crashes on launch with a dyld error.
 */

const fs = require('node:fs');
const path = require('node:path');

const dist = path.join(__dirname, '..', 'node_modules', 'electron', 'dist', 'Electron.app');
const frameworks = path.join(dist, 'Contents', 'Frameworks');

function fixFramework(framework) {
  const versions = path.join(framework, 'Versions');
  if (!fs.existsSync(versions)) return;
  const entries = fs.readdirSync(versions).filter((entry) => entry !== 'Current' && !entry.startsWith('.'));
  const version = entries[0];
  if (!version) return;
  const current = path.join(versions, 'Current');
  try { fs.unlinkSync(current); } catch {}
  fs.symlinkSync(version, current);
  const versionDir = path.join(versions, version);
  for (const item of fs.readdirSync(versionDir)) {
    const target = path.join(framework, item);
    try { fs.unlinkSync(target); } catch {}
    fs.symlinkSync(path.join('Versions', 'Current', item), target);
  }
}

if (fs.existsSync(frameworks)) {
  for (const entry of fs.readdirSync(frameworks)) {
    if (!entry.endsWith('.framework')) continue;
    fixFramework(path.join(frameworks, entry));
  }
  console.log('[zen-pwa] repaired Electron framework symlinks');
}
