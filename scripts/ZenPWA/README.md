# ZenPWA

Site-specific apps (e.g. `~/Applications/Netflix.app`) on the castLabs Electron runtime, so Widevine works. CLI: `zen --pwa …` (`tools/cli.js`); shell code in `app/` is symlinked into every installed app, so changes apply on next launch without a rebuild.

## Look: Zen is the source of truth

- Zen is the source of truth for the PWA look (`app/zen-config.js`). At launch, and live whenever Zen rewrites them, the shell reads the Zen profile (`installs.ini` default; override `ZENPWA_ZEN_PROFILE=/path`): `prefs.js`/`user.js` (gap `zen.theme.content-element-separation`, `zen.theme.border-radius`, `zen.view.compact.hide-toolbar`, hide delay, `zen.view.window.scheme`, grey-out-inactive), the active space's colour from `zen-sessions.jsonlz4`, and mods (`chrome/zen-themes.css` + `chrome/userChrome.css`). Layout uses Zen's own variables (`--zen-element-separation`, `--zen-native-inner-radius`, `--zen-toolbar-height`, …) and Zen's macOS paint maths (translucent layer over native vibrancy).
- Mods: rules are mapped onto the shell (`#urlbar`→`#address`, `#nav-bar`→`#toolbar`, `.toolbarbutton-1`→`.zbtn`, `#tabbrowser-tabbox`→`#page`, `:root` vars kept; `-moz-pref`/`-moz-bool-pref` evaluated against Zen prefs). Rules for parts a PWA doesn't have (tabs, sidebar) and nested CSS rules are dropped. Map lives in `ALIASES` in `zen-config.js`.
- Colour: the active space colour with the site's hue (manifest `theme_color` → meta `theme-color` → brand colour → favicon; per-PWA override `~/.config/zen-pwa/accents.json`, `zen --pwa --rebuild <app> --accent "#hex"`, `auto` clears). Only the space's primary colour is used, not the full gradient or grain.
- Widevine/castLabs runtime untouched; Netflix EME reports `ready` with the bridge on.

## Icons

Clean, Chrome-style app icons (`tools/iconmaker.swift`, compiled to `tools/.build/iconmaker`): the site's own icon inside the macOS icon shape (824/1024 squircle, standard shadow, no gloss). Choice (`pickIcon` in `tools/cli.js`, measured sizes): manifest maskable ≥192 fills the shape → manifest icon ≥144 → apple-touch-icon ≥120 → any site icon ≥32 → App Store art → Google favicon → flat monogram. Opaque square artwork fills the shape; transparent logos sit on a white tile. `zen --pwa --rebuild <app>` regenerates an app's icon.
