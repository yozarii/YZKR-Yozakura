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
