# ZenSkin

Zen look for **Safari web apps** (Safari → File → Add to Dock) without touching them. Safari keeps doing playback (FairPlay DRM, so Netflix works), logins and updates. ZenSkin turns each web-app window into a **lens**: the page sits in one thin Zen rim with rounded corners, and everything else of the window (Safari's title bar, its square corners and its native drop shadow) is replaced by a live 60 fps view of what is behind it, so the page looks like it is under the desktop and the lens reveals it.

- rim in your active Zen space colour with the site's hue (from the Zen profile via `ZenPWA/app/zen-config.js`), soft lens shadow;
- hover the top edge: the hidden title bar fades in (Zen compact), with the real traffic lights;
- nothing in fullscreen, so video is untouched.

The live backdrop uses ScreenCaptureKit with the web app and ZenSkin excluded, so the web app's window (and its video) is never captured. It needs the **Screen Recording** permission (asked once); without it, the title bar strip is painted with `titleBarColor` / the frame colour instead. Cost: roughly 10–15% of one core while a web app is on screen.

All clicks fall through to Safari. Targets every running app whose bundle id starts with `com.apple.Safari.WebApp` (Stage Manager thumbnails are skipped).

```
zen-skin start    # refresh colours from Zen, build if needed, start the agent
zen-skin stop
zen-skin config   # rewrite ~/.config/zen-pwa/skin.json
```

`~/.config/zen-pwa/skin.json` (written by `ZenPWA/tools/skin-config.js`): layout from Zen prefs, one entry per web app with `frame` colour. The page is exactly the window's visible content and is never covered; the rim (the Zen gap, 8) sits outside the window on the left, right and bottom and inside the hidden title bar on top, so only the title bar is hidden. `layout.topExtra` (default 4) lifts just the top rim's outer edge; `layout.bottomLift` (default 3) raises just the bottom rim's outer edge. Corners: the lens uses Zen's window radius (`zen.theme.border-radius`, 11 on macOS Tahoe) and the page ~6; override with `layout.lensRadius` / `layout.cardRadius`. These survive `zen-skin config`. Set `"titleBar": <points>` on an app entry to match its title bar height (default 32 on macOS 26+; taller when Safari's navigation controls are shown); it survives `zen-skin config`. Per-site colour override: `~/.config/zen-pwa/accents.json` (same as the Electron PWAs). Test against any app with `ZENSKIN_TARGET=<bundle id prefix>`.

Not started at login yet (that needs a LaunchAgent).
