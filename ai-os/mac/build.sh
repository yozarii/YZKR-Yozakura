#!/bin/zsh
# build.sh — compile the native AI-OS shell and install ~/Applications/YZKR AI-OS.app (registers yzkr://).
set -euo pipefail

HERE=${0:A:h}
YZKR=${HERE:h:h}
APP="$HOME/Applications/YZKR AI-OS.app"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

swiftc -O -swift-version 5 -framework AppKit -framework WebKit -framework Carbon "$HERE/main.swift" -o "$WORK/YZKR AI-OS"

# icon: theme accent monogram via the ZenPWA iconmaker (built on demand)
ICONMAKER="$YZKR/scripts/ZenPWA/tools/.build/iconmaker"
[[ -x $ICONMAKER ]] || swiftc -O "$YZKR/scripts/ZenPWA/tools/iconmaker.swift" -o "$ICONMAKER"
ACCENT=$(/opt/homebrew/bin/jq -r '.colors.accentStrong' "$YZKR/ai-os/theme.json")
"$ICONMAKER" - "$WORK/icon.png" "$ACCENT" monogram Y
mkdir -p "$WORK/AppIcon.iconset"
for s in 16 32 128 256 512; do
  sips -z $s $s "$WORK/icon.png" --out "$WORK/AppIcon.iconset/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) "$WORK/icon.png" --out "$WORK/AppIcon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$WORK/AppIcon.iconset" -o "$WORK/AppIcon.icns"

osascript -e 'quit app "YZKR AI-OS"' 2>/dev/null || true
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
mv "$WORK/YZKR AI-OS" "$APP/Contents/MacOS/"
mv "$WORK/AppIcon.icns" "$APP/Contents/Resources/"
cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>YZKR AI-OS</string>
  <key>CFBundleDisplayName</key><string>YZKR AI-OS</string>
  <key>CFBundleIdentifier</key><string>dev.yzkr.aios</string>
  <key>CFBundleExecutable</key><string>YZKR AI-OS</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSAppTransportSecurity</key><dict><key>NSAllowsLocalNetworking</key><true/></dict>
  <key>CFBundleURLTypes</key><array><dict>
    <key>CFBundleURLName</key><string>dev.yzkr.aios</string>
    <key>CFBundleURLSchemes</key><array><string>yzkr</string></array>
  </dict></array>
</dict></plist>
PLIST
codesign --force --sign - "$APP" >/dev/null
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP"
print "installed $APP"
