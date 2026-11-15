#!/bin/bash
set -euo pipefail

ROOT="$HOME/YZKR/scripts/ZenPWA"
BIN="$HOME/YZKR/bin/zen-pwa"
ZEN="$HOME/YZKR/bin/zen"

say() { printf '%s\n' "$@"; }

if ! command -v node >/dev/null 2>&1; then
  say "Node.js is required for the Zen PWA runtime."
  if command -v brew >/dev/null 2>&1; then
    say "Installing Node with Homebrew…"
    brew install node
  else
    say "Install Node 20+ from https://nodejs.org and run this script again."
    exit 1
  fi
fi

NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]')"
if [ "$NODE_MAJOR" -lt 20 ]; then
  say "Node $(node --version) is too old; Node 20 or newer is required."
  exit 1
fi

if ! command -v swiftc >/dev/null 2>&1; then
  say "Xcode Command Line Tools are required (swiftc was not found)."
  say "Install them with: xcode-select --install"
  exit 1
fi

say "zen-pwa: installing the castLabs Electron runtime…"
cd "$ROOT"
npm install --no-audit --no-fund

say "zen-pwa: repairing framework symlinks…"
node "$ROOT/scripts/fix-electron-symlinks.js"

say "zen-pwa: compiling the icon composer…"
mkdir -p "$ROOT/tools/.build"
swiftc -O "$ROOT/tools/iconmaker.swift" -o "$ROOT/tools/.build/iconmaker"

chmod +x "$BIN" "$ZEN" "$ROOT/build.sh" "$ROOT/tools/cli.js" "$ROOT/tools/siteinfo.js"

# Make sure ~/YZKR/bin is on PATH for future shells.
if [ -f "$HOME/.zshrc" ]; then
  if ! grep -q 'YZKR/bin' "$HOME/.zshrc"; then
    printf '\nexport PATH="$HOME/YZKR/bin:$PATH"\n' >> "$HOME/.zshrc"
  fi
else
  printf 'export PATH="$HOME/YZKR/bin:$PATH"\n' > "$HOME/.zshrc"
fi

say ""
say "Done. Zen PWA $(node -p "require('$ROOT/package.json').version") is ready."
say ""
say "  zen --pwa \"Netflix\"      install a site"
say "  zen --pwa --list          see installed apps"
say "  zen --pwa --doctor        check Widevine and signatures"
say "  zen --pwa --sign          free castLabs EVS signature for Netflix"
say ""
say "If the zen command is not found in a new terminal, run: source ~/.zshrc"
