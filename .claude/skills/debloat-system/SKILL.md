---
name: debloat-system
description: Audit and prune macOS bloat (launch agents, Homebrew casks/formulae, PWA wrappers, launcher apps like Raycast) while preserving the Netflix Zen PWA and core YZKR daemons. Use when the user says "debloat", "clean up apps", "remove PWAs", "purge <app>", or "audit background agents".
---

# debloat-system

Audit first, report, then prune. Never delete before listing exactly what will go.

## Targets (audit, read-only)
1. `~/Library/LaunchAgents`, `/Library/LaunchAgents`, `/Library/LaunchDaemons` — `ls` + `launchctl list | grep -v com.apple`
2. Homebrew: `brew list --cask`, `brew leaves`
3. PWAs: `~/YZKR/bin/zen-pwa --list` (apps in `~/Applications`, profiles in `~/Library/Application Support/Zen PWA/<slug>`)
4. Browser profiles: `~/Library/Application Support/Firefox`, `~/Library/Application Support/zen` (inspect only, never delete profiles)
5. Residue for a named app `<bundle-id>`: `~/Library/{Application Support,Caches,Preferences,Containers,Group Containers,HTTPStorages,Logs,WebKit,Cookies}/*<bundle-id>*`

## Safety exclusions (NEVER remove)
- Netflix PWA: `~/Applications/Netflix.app`, `~/Library/Application Support/Zen PWA/netflix`, its `userChrome.css` and launch wrapper
- `~/YZKR/**` (scripts, bin, ai-memory, ai-os), `~/YZKR/bin/zen`, `~/YZKR/bin/zen-pwa`
- `sh.brew.ollama` launch agent, Ollama models
- Sol, Rectangle, NordPass (`np`), Zen browser, uv, node
- Anything under `com.apple.*`

## Prune procedure
1. Present a before/after table of candidates; get user confirmation for anything not already named in the request.
2. Homebrew-managed app: `brew uninstall --cask --zap <name>` (zap removes residue). Otherwise quit the app (`pkill -x`), move the `.app` out of `/Applications`, then remove residue paths above.
3. PWAs: `zen-pwa --remove "<Name>" --purge` one at a time. Never delete `Zen PWA/netflix`.
4. Launch agents: `launchctl bootout gui/$UID <plist>` then remove the plist — only for items the user named.

## Verification
- `zen-pwa --list` shows only Netflix; `ls ~/Applications` shows only `Netflix.app`
- `ls /Applications | grep -i <app>` is empty; `pgrep -il <app>` is empty
- Residue glob (step 5) returns nothing
- `launchctl list | grep ollama` still present; `np whoami` still works
- Report exact items removed.
