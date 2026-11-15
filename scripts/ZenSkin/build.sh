#!/bin/bash
# Builds ZenSkin.app (background agent, no Dock icon) next to this script.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$here/build/ZenSkin.app"
mkdir -p "$app/Contents/MacOS"
swiftc -O "$here/main.swift" -o "$app/Contents/MacOS/ZenSkin"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.yzkr.zenskin</string>
  <key>CFBundleName</key><string>ZenSkin</string>
  <key>CFBundleExecutable</key><string>ZenSkin</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>LSUIElement</key><true/>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
</dict></plist>
PLIST
codesign --force --sign - "$app" >/dev/null
echo "$app"
