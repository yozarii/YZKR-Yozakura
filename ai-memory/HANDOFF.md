# YZKR Stage A → Stage B Handoff

## Done (Phases 1–4)
- Raycast removed (`brew uninstall --cask --zap raycast`) + residue cleaned.
- PWAs: only **Netflix** remains (`~/Applications/Netflix.app`, profile `~/Library/Application Support/Zen PWA/netflix`). Removed: Gemini, Google Docs/Drive/Sheets/Slides (+profiles). Native `google-drive` cask untouched. Netflix still "needs signing" (`zen --pwa --sign`).
- Skill: `.claude/skills/debloat-system/SKILL.md`.
- Scripts consolidated: one-shot bootstrap scripts → `scripts/archive/`; `scripts/install-zen-pwa.sh` (v5 entry). `~/installs` did not exist.
- `scripts/np_tui.sh` sourced from `~/.zshrc` (guarded, interactive+TTY only).
- Installed already: Sol, Rectangle, ollama, fzf, jq, node, uv, np.

## Paths
- `scripts/np_mcp.py` (np-manager MCP), `scripts/worker_mcp.py` (worker MCP: `route_intent`, `quick_worker`, `tri_search`)
- `bin/zen`, `bin/zen-pwa`, `bin/npw`; PWA toolkit `scripts/ZenPWA/` (node tool `tools/cli.js`)
  - Look: copies Zen compact mode (measured from the user's Zen window): a 9px frame around an 8px-radius page card, no handle; the top bar is hidden until the pointer reaches the top edge, then drops in with native traffic lights, flat back/forward/reload icons and a Zen-style URL bar showing the address.
  - Frame colour: the site's highlight colour in the tone of the Zen workspace colour (`#6f5b69`): same tone, hue from the site, so Netflix is dusty red and GitHub dusty green (`zenTone()` in `app/main.js`). Highlight source order: `~/.config/zen-pwa/accents.json` override (`{"<slug>": "#rrggbb"}`, set with `zen --pwa --rebuild <app> --accent "#hex"`, `auto` clears) → `site.json` `accent` (manifest `theme_color` → `<meta name="theme-color">` → curated brand colour in `tools/siteinfo.js`; greys/black/white skipped) → favicon colour sampled once at runtime → plain Zen `#6f5b69`. Overrides apply on next launch, no rebuild needed.
- `ai-memory/memory.jsonl` (memory MCP store), `ai-os/theme.json`
- Claude Desktop config: `~/Library/Application Support/Claude/claude_desktop_config.json` → servers: filesystem, git, memory, np-manager, worker (all via `/opt/homebrew/bin/uv run` / npx / uvx)

## Ollama
- Endpoint `http://localhost:11434` (brew service `sh.brew.ollama`)
- Router: `qwen2.5:3b-instruct-q4_K_M` (JSON-schema `format`), worker: `llama3.2:3b`

## Theme tokens (`ai-os/theme.json`)
bg `#0b0b10`, bgElevated `#12121a`, surface `#171722`, border `#262636`, text `#e6e6ee`, muted `#8b8b99`, accent `#a78bfa`, success `#7ce7ac`, warning `#f5c451`, danger `#f87171`, info `#7dd3fc`. Graph colors: memory `#a78bfa`, filesystem `#7dd3fc`, chat `#7ce7ac`. Shortcuts: Cmd+K palette, Cmd+E graph, Cmd+T terminal.

## Caveats for Stage B
- `tri_search` keys: env `YDC_API_KEY` / `PERPLEXITY_API_KEY` / `FIRECRAWL_API_KEY`, else NordPass items whose name matches "You.com" / "Perplexity" / "Firecrawl". No such items existed at build time — create them or set env vars. Untested live.
- `route_intent`/`quick_worker` depend on model pulls completing; verify with `ollama list`.
- Sol and Rectangle are installed but **theming/config of Sol and Rectangle not applied** (no stable config files; only the shell banner uses the tokens).
- Restart Claude Desktop to load the `worker` server.

---

# Stage B (Phases 5–6) — done 2026-11-15

## Phase 5 · Semantic spotlight (`scripts/spotlight_engine.py`)
- Pipeline: source registry (apps, ~/YZKR files/folders, bin, git repos, zsh aliases, macros, Zen/Firefox/Safari/Chromium bookmarks, memory entities, AI-OS planes) → SQLite FTS5 **trigram** index (`ai-os/data/spotlight.db`, typo-tolerant) → lexical rerank (+usage boost) → `qwen2.5:3b` picks `{candidate, action}` from schema **enums built from the registries** (no free-text fields: 3B models degenerate in long strings) → action registry executes.
- Exact title hits skip the model (~0.1s); LLM path ~1.5s; cached per query; falls back to lexical top hit if Ollama is down.
- Safety: `run` needs an explicit verb unless it's the item's default (macros); actions declare allowed kinds.
- CLI: `uv run scripts/spotlight_engine.py index|search|resolve|run|ask|macro …|sol-sync [--hotkey]`.
- Macros (`scripts/yzkr_macros.py`): `scripts/macros/<slug>.zsh`, typed steps in `# yzkr-steps:` header (step-type registry: shell/open_app/open_url/open_path/quit_app/sleep/notify/yzkr). Record from NL (qwen, schema-constrained, normalized) or from last N zsh history lines.
- Sol: scripts in `~/.config/sol/scripts/` (YZKR Spotlight, Record Macro, Reindex, one per macro, auto-synced); `~/.config/sol/config.json` → `globalShortcut: "command"` (⌘Space).
- **User action left:** run `~/YZKR/bin/yzkr-hotkey` once to disable macOS Spotlight's ⌘Space (system shortcut; `--undo` reverts), then launch Sol.

## Phase 6 · AI-OS (`ai-os/`)
- `server.py` (aiohttp, 127.0.0.1:7777): per-launch token in index.html, Host allowlist (DNS rebinding), WS Origin check, CSP + DOMPurify for model output.
- `backend/`: `files` (sandboxed to ~/YZKR, mtime conflict check), `graph` (3 layers sharing `file:<path>` ids), `system` (probe + toggle registries), `chat` (Claude via anthropic SDK `claude-opus-5-5` default + server-side refusal fallback; Ollama; auto-router local/claude/terminal), `term` (pty zsh over WS; `cls` via ZDOTDIR shim).
- `web/`: no bundler; ES modules + vendored CodeMirror 5, xterm.js, marked, DOMPurify. Planes: Editor ⌘1, Graph ⌘E, Home ⌘3, Chat+Terminal ⌘T; palette ⌘K.
- `mac/main.swift` + `mac/build.sh` → `~/Applications/YZKR AI-OS.app`: WKWebView window, starts/stops the server, `yzkr://` scheme (palette/editor/graph/chat/home), ⌥Space global hotkey, hides on close.
- Claude chat needs credentials: `ANTHROPIC_API_KEY`, `ant auth login`, or NordPass item named "Anthropic". Untested live (no key present).
