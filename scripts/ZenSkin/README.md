# ZenSkin

Zen look for **Safari web apps** (Safari → File → Add to Dock) without touching them. Safari keeps doing playback (FairPlay DRM, so Netflix works), logins and updates; ZenSkin floats a click-through overlay over each web-app window:

- Zen frame in your active Zen space colour, with the site's hue (from the Zen profile via `ZenPWA/app/zen-config.js`),
- a band over Safari's title bar that fades out while the pointer is at the top edge (Zen compact), revealing the real title bar and traffic lights,
- nothing in fullscreen, so video is untouched.

All clicks fall through to Safari. Targets every running app whose bundle id starts with `com.apple.Safari.WebApp` (Stage Manager thumbnails are skipped).

```
zen-skin start    # refresh colours from Zen, build if needed, start the agent
zen-skin stop
zen-skin config   # rewrite ~/.config/zen-pwa/skin.json
```

`~/.config/zen-pwa/skin.json` (written by `ZenPWA/tools/skin-config.js`): layout from Zen prefs, one entry per web app with `frame` colour. Set `"titleBar": <points>` on an app entry to match its title bar height (default 28; taller when Safari's navigation controls are shown); it survives `zen-skin config`. Per-site colour override: `~/.config/zen-pwa/accents.json` (same as the Electron PWAs). Test against any app with `ZENSKIN_TARGET=<bundle id prefix>`.

Not started at login yet (that needs a LaunchAgent).
