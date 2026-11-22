# kaika: the universal package manager

Status: built and working on `feature/kaika`; see "As built" at the end for what changed from the first draft. Date: 2026-11-21.

## Intent

One command that installs, updates and removes anything on the user's Mac, from the terminal. kaika picks the right backend for each tool (Homebrew formula or cask, npm, uv, the App Store). For software with no package, such as apps that only exist on a vendor's website, kaika finds, downloads, verifies and installs it, then keeps it up to date like any other package. Everything it installs is tracked in one place, so managing apps stops being a scavenger hunt.

The intelligence is deliberately narrow: deterministic tools do the work, and a small local decision model is consulted only when kaika has to choose among plausible third-party candidates (which site is official, which download fits). Models never touch package managers, never run at startup, and never decide trust. Models choose; they never act.

## Goals

- Install, uninstall, upgrade, list, search and inspect across all backends through one interface.
- Pick the right backend automatically. If one tool is available from several, choose by a documented preference order that the user can override.
- Install apps that exist only on vendor sites, with verification, from the terminal.
- Track every managed app, including its source, version, install location and how to update it.
- Fast: a native Rust binary, with the common commands answering in well under a second when no model is involved.
- Scriptable: stable `--json` output and exit codes, so agents and scripts can drive it.

## Non-goals

- Replacing Homebrew, npm or uv. kaika drives them; it does not reimplement them.
- Buying or paying for apps. App Store purchases and paid licences stay with the user.
- Linux or Windows support in v1. macOS on Apple silicon first. The design must not make other platforms impossible.
- A resident background service. Scheduled upgrades are a plain launchd job that runs and exits (`kaika schedule`).
- Managing system settings or anything that is not a package.

## Anti-goals

These would make kaika a failure even if it technically works.

- A model ever supplying a URL, a command, or a file path that kaika then runs. Models pick from candidates that deterministic code produced.
- Installing unverified software silently.
- Inventing vocabulary or redefining flags. Whimsical names are for user-facing apps and tools, never for commands, flags, functions or file formats.
- Slow startup or per-command model calls on the common path.

## Conventions this design follows

- **Language:** Python is for proof of concept; Rust is for the fast end product. (The Python proofs of concept are finished and deleted; everything shipped is Rust.)
- **Names and flags:** established conventions only.
- **Paths:** one dedicated home, `~/.kaika/`, as cargo (`~/.cargo`), rustup and bun do: `kaika.toml` (config), `recipes/`, `manifest.toml`, `bin/` (command-line tools; the only folder to put on PATH), `packages/<name>/<version>/` (unpacked command-line packages), `cache/` (downloads, staging). Apps go to `/Applications`. `KAIKA_HOME` moves the tree.
- **Names:** no whimsical vocabulary in commands, flags, functions or files. The earlier "garden", "petal", "bloom" names are retired.

### Commands

| Command | Meaning (as in brew, apt, npm, uv) |
|---|---|
| `install <name>...` | Install packages |
| `uninstall <name>...` (alias `remove`) | Remove packages |
| `upgrade [name...]` | Upgrade the named packages, or all with `--all` |
| `update` | Refresh indexes, recipes and update sources |
| `list` | Installed packages, with source and version |
| `search <query>` | Search all backends and vendor recipes |
| `info <name>` | Details: source, version, install path, update source, verification |
| `outdated` | Packages with newer versions available |
| `doctor` | Check the machine: backends present, models reachable, state consistent |
| `cleanup` | Remove old downloads and caches |
| `config` | Read and write configuration |

A target can be named `source:name` (for example `cask:zen`) to bypass automatic backend choice, as the current prototype does.

### Flags (global and per command)

| Flag | Meaning |
|---|---|
| `-n`, `--dry-run` | Print the plan; change nothing |
| `-y`, `--yes` | Skip the confirmation prompt |
| `-f`, `--force` | Override safety: proceed past failed verification, conflicts, or an existing install |
| `--all` | Apply to everything (`upgrade --all`) |
| `--zap` | On uninstall, also remove the app's support files, caches and preferences, as `brew uninstall --zap` does |
| `-q`, `-v` | Quiet, verbose |
| `--json` | Machine-readable output |

Default behaviour: commands run, show the plan, and ask for confirmation when the action is installing or removing. If there is no terminal to answer the prompt (an agent or a script), kaika prints the plan and exits non-zero without changing anything, unless `-y` is given. `-f` alone does not skip the prompt.

Exit codes: 0 success, 1 failure, 2 usage error, 3 verification failed (nothing installed), 4 needs confirmation but no terminal.

## Architecture

```
kaika install <name | source:name | https URL>
  0. Offline?     a 2 s probe; offline exits 69 before anything starts
  1. Resolve      deterministic: backend lookups (names compared loosely), recipes
  2. Trust        curated sources before open registries; ties are shown, not guessed
  3. Discover     only for what no backend carries: Wikidata site, or the URL given; links, release feeds
  4. Quarantine   candidates that talk to the chooser are dropped, deterministically
  5. Score        a deterministic score settles clear downloads (about 9 in 10) with no model
  6. Decide       only if still unclear: a small local model, started lazily, stopped after
  7. Verify       deterministic: signature, notarization, Team ID, checksum
  8. Plan         shown to the user; run on confirmation or -y
  9. Execute      backend command, or the download-and-install pipeline
 10. Record       manifest entry; a confirmed discovery is saved as a recipe
```

### Parts

1. **CLI and core (Rust).** Argument parsing, plan/confirm flow, output (human and `--json`), exit codes.
2. **Backends (Rust).** One adapter per source behind a common trait: `installed()`, `search()`, `info()`, `install()`, `uninstall()`, `upgrade()`, `outdated()`. v1: brew formula, brew cask, npm global, uv tool, `mas` (App Store), and `direct` (vendor-site downloads). Adapters shell out to the real tool and parse its machine-readable output.
3. **Manifest and state (Rust).** The manifest records, for every managed package, its source, version, install path, update source, how it was resolved, and when. The old `garden/petals/` notes held only empty tags and were retired without import.
4. **Verification (Rust).** Described below.
5. **Discovery and the decision model (Rust).** Described below: `discovery.rs`, `decision.rs`, `quarantine.rs`, `model.rs`.
6. **Recipes.** Small TOML files describing a vendor app: name, official domain, expected Team ID, where releases are announced (GitHub releases, Sparkle appcast, a page to scrape), asset pattern. A recipe is deterministic and bypasses the models entirely. Curated recipes live in the repo; recipes kaika learns are saved to the user's config directory.

### Backend choice

For a bare name, kaika asks every available backend whether it has it, comparing names the way people mean them (case, spaces and hyphens ignored, so "Sublime Merge" finds `sublime-merge`). If exactly one does, it takes that one. If several do, a fixed preference order decides: brew, cask, uv, npm, mas, then direct recipes; the user can change the order in config and can always force a source with `source:name`.

**No model decides which source to use.** Measured on real descriptions, the small model picked an npm wrapper over the Homebrew tool at probability 1.00, because the question is about trust and ambiguity, not about what a description says. Two rules in code instead:

1. **Trust tiers.** Curated sources (brew, cask, App Store, recipes) come before open registries (uv, npm), whose names can be squatted. An open-registry package is never chosen over a curated one, by any means.
2. **Ambiguity is shown, not guessed.** A name that is both a formula and a cask (a command-line tool and a desktop app), or a tie between open registries, keeps the preference order, and the plan names every tied candidate with a one-line description so the human decides; `source:name` forces another.

### The decision model

**Scope.** Models decide *relevance among peers* for third-party and non-package apps, and nothing else: which site is the official one, which download fits this Mac. They never decide which package source to use (trust and ambiguity are rules in code), never touch verification, and are never used on a package-manager path (install of a packaged name, list, upgrade, uninstall, cleanup) or at startup. A test asserts no model process is spawned on any of those paths.

**The model.** H2O-Lightning-4B, Q4_K_M (Apache-2.0, 2.7 GB, h2oai's official GGUF), served by `llama-server` (Homebrew `llama.cpp`, build 11773 or newer, which answers typed decisions itself on `POST /v1/systemone`). It was chosen by a bake-off (see below), not by size: the two 9B models and a 27B-class family scored lower on kaika's own decisions, and the top leaderboard model (Torchcast Decision 4B) cannot run on a Mac.

**Lifecycle.** Discovery calls `preload()` first, so the server starts while Wikidata and the vendor page are fetched and loading costs no extra time; the command stops it when it ends. A supervisor shell kills it within a second if kaika dies by any means (SIGKILL included), so it cannot be left behind. Nothing is resident at rest; about 3 GB while it runs. `KAIKA_MODELS=off` or `kaika config set models off` disables every model call and the web lookups that feed it.

**Setup is explicit.** If `llama-server` or the model file is missing, discovery says exactly what to run (`kaika install llama.cpp`, `kaika install h2o-lightning`) and decides nothing; it never installs silently. The model is an ordinary direct package: an embedded recipe pins one URL and one SHA-256 (the LFS object id Hugging Face publishes), and `.gguf` files without a checksum are refused unless `-f`.

**The question.** A typed `choice` over the candidates plus "none of the other options is right". Option keys are informative and unique (a download URL becomes its file name); texts are clipped to 220 characters and described to the model as data, not instructions. At most 8 candidates.

**The verdict.** A single pass at probability 0.8 or more is accepted. Between 0.6 and 0.8 the options are reversed and asked again, and the answer counts only if it still agrees and the average is at least 0.6. Below 0.6, or "none", is unsure, and kaika asks the user. A model can only name a candidate that deterministic code produced.

**Injection quarantine.** Candidate texts come from the open web. Before any scoring or model call, candidates whose text addresses the chooser ("ignore the other options", "the correct answer is A", chat-turn markers, and the same in German, French, Spanish, Italian and Portuguese) are dropped, and the plan reports how many and which. This also closes the deterministic shortcut: a hostile link cannot win by scoring well. A corpus test requires every marked candidate to be flagged and none of 1,390 clean ones.

**Deterministic first.** A lone candidate, or a download that clearly outscores the next, needs no model; the score rules out other operating systems, source, debug symbols and checksum files, and demotes nightly and beta builds. On the corpus this settles 270 of 298 download decisions with no wrong answer.

**No cloud.** There is no escalation to a remote model. A decision kaika cannot make locally is put to the user.

**Evidence.** A five-model bake-off on 52 hard decisions (committed in history under `kaika/poc/bakeoff`, since removed): H2O-Lightning-4B answered 46 of 46 with no wrong answer and refused all 6 no-answer cases; tev1:4b 44; Nimble 9B 42; Qwen3.5-4B and 9B 41 (the 9B fell for an injection); the earlier Qwen2.5-3B 39 with a confident injection miss. The reversed pass proved unnecessary for H2O at high confidence, hence the single-pass rule above. The shipped corpus has 626 cases (`corpus/discovery.json`) and an opt-in test runs the real model over all of it with gates fixed in advance. Last run: 611 of 626 correct (97.6%), 2 wrong, 13 abstained; 270 cases settled by the deterministic shortcut, 0.32 s per case on average. All explicit injections and all no-answer cases were handled correctly; the two wrong answers are persuasive lookalike sites (`*-genuine.example`), a known weak spot.

### Vendor-site pipeline (the `direct` backend)

1. **Find the official site, without scraping a search engine** (they answer scripts with bot challenges, which kaika does not work around). Candidates come from **Wikidata**, which records an official website for most notable software (a structured, keyless API; exact label match, entities without a website dropped, so "Obsidian" the app is not "Obsidian" the mineral or the video games). One match is taken as is; several go to the decision layer. Or the user gives an **https URL** as the target, for apps Wikidata does not know.
2. **Find the download.** Fetch the page and list links: unresolved script placeholders (`${version}`) and plain-HTTP links are never candidates. If the page has none, follow at most two same-site "download" pages one level deep; if it only links to a GitHub project, use its release feed, which also names checksums. Close calls between links go to the decision layer. A plain fetch and parse is the default. Only when a page has no links at all (it builds them with JavaScript) and a Chromium-family browser is already installed (Chrome, Chromium, Brave, Edge, Vivaldi, Arc; `KAIKA_BROWSER` chooses one, `off` disables), that browser renders the page headless with a throwaway profile and no extensions, and the resulting page goes through the same link scan and quarantine. No browser is ever downloaded for this; without one, the error says what would help. Recipes re-render the same way at upgrade time.

   **Second source: GitHub.** When Wikidata knows no official site, or its site yields no download, kaika searches GitHub repositories by name. Only a repository whose own name equals the app's name, that is not a fork or archived and has at least 50 stars is offered (most-starred first, at most five); a lone one is taken, several go to the decision layer (which may refuse all), and descriptions pass the same quarantine. The release feed then supplies the download, and every normal verification applies.
3. **Pick the asset** for macOS on Apple silicon by a deterministic score (formats: dmg, zip, pkg, tar.gz command-line packages).
4. Download over HTTPS to `~/.kaika/cache/downloads/`, then verify (below).
5. **Install.** Apps: copy out of a mounted dmg or an unzipped archive into `/Applications`; pkg: the system installer (administrator rights are stated in the plan, never taken silently). Command-line packages: unpacked whole into `~/.kaika/packages/<name>/<version>/` with their executables linked into `~/.kaika/bin/`; a link kaika did not create is never overwritten without `-f`; an upgrade replaces the old version.
6. Record in the manifest with the update source, so `upgrade` and `outdated` check for newer versions through the recipe's release feed (GitHub releases, Sparkle appcast, or a page re-check). A confirmed discovery is saved as a recipe, so the same app is resolved deterministically next time with no lookup.

### App Store

Through `mas`. It can install apps already associated with the user's Apple ID and update them. Purchases are out of scope; kaika tells the user to buy in the App Store themselves.

### Verification (no model is involved)

For every direct download, before anything is installed:

- Code signature is valid.
- The app is notarized by Apple.
- The signing Team ID matches the recipe's expected one; for an unknown app, the Team ID is shown in the plan and recorded for future checks.
- A published checksum matches, when the vendor provides one.

Any failure stops the install with exit code 3. `-f` overrides it, and only with the failure printed prominently in the plan. kaika never sends a model's opinion in place of a failed check.

## Constraints

- Apple silicon macOS (currently macOS 27). Rust toolchain installed through rustup.
- Models are local only. Wikidata lookups send the app name to wikidata.org; `KAIKA_MODELS=off` stops them.
- `bin/kaika` is a launcher for the built Rust binary.

## Testing

All automated tests are hermetic: a temporary HOME and applications folder, stub executables for every external tool (including `llama-server`) first on PATH, real `ditto`/`tar` on fixture archives. They never touch the user's installs, load a model, open an app or a window, mount an image, ask for a password, take focus, or use the network.

- Unit tests per module: parsers, scoring, trust tiers, quarantine, the request and verdict rules, verification, discovery against recorded Wikidata and page fixtures.
- CLI integration tests on exit codes, plans and the manifest, including: no terminal, `-f`, every download format, trust-on-first-use upgrades; offline exits 69 before anything starts; **no model process on any package-manager path**; exactly one model start per discovery and it is gone afterwards; the model is killed even when kaika receives SIGKILL; missing prerequisites yield instructions, not crashes; the model installs, verifies and uninstalls like any package.
- Corpus tests (`corpus/discovery.json`, 626 cases): well-formedness, the quarantine against 84 hostile and 1,390 clean candidates, a hostile candidate never reaches a decider or wins, and the deterministic download shortcut is never wrong.
- Opt-in: `cargo test --test real_model -- --ignored --nocapture` runs the real model over the whole corpus (gates: no wrong answer on explicit injections or no-answer cases, at most 1% wrong overall, at least 95% correct overall); `cargo test -- --ignored` runs the real `codesign` and `spctl` on an installed app.
- Not automated: a signed-in App Store through `mas` (tests cover command construction, parsing and the failure guidance with a stub; the real call needs the user's Apple ID).
- The update check is tested with stub `launchctl` and `osascript`; a real notification and an unattended 03:30 run are not automated. The browser fallback is tested with a stub browser; a real Chrome is not driven.

## Decided by me (not by the user)

- `uninstall` with `remove` as an alias; `update` refreshes indexes and `upgrade` upgrades packages, as in brew and apt.
- `~/.kaika` as the dedicated home and TOML for config, recipes and manifest.
- The preference order among backends, the trust tiers, and the rule that no model decides a source.
- Wikidata as the discovery source, and URL targets for apps it does not know.
- H2O-Lightning-4B served by `llama-server`, lazily, per command; the 0.8 and 0.6 verdict thresholds; the injection quarantine and its phrase list.
- Exit code 69 for offline (sysexits), and the 2 second probe.
- The exit-code numbers 0 to 4.

## As built

- **Layout:** one Rust package in `kaika/` (library plus a thin binary). External tools run through a `Runner` trait so tests substitute stubs. `bin/kaika` launches the built binary.
- **HTTP:** every fetch and download is `curl` (HTTPS only, one User-Agent); the dependencies are clap, serde, serde_json (with `preserve_order`), toml, sha2 and anyhow.
- **Formats:** zip and dmg (image always detached), pkg (system installer via `sudo`, announced in the plan), tar.gz command-line packages (unpacked whole into `~/.kaika/packages`, linked into `~/.kaika/bin`), and `.gguf` model files (checksum required).
- **Trust on first use:** a recipe without a `team_id` shows the Team ID and records it; every later upgrade must match it.
- **Verified against reality:** the `codesign`/`spctl` parsers were checked on an installed, notarized app; real discovery runs found a template-URL trap (`${version}`) and a bot-challenge wall on search engines, both handled; the decision model was measured on real macOS hardware.
- **Test hooks:** `KAIKA_TEST_MODE=1` permits `file://` URLs; `KAIKA_GITHUB_API` redirects the GitHub API; `KAIKA_ASSUME_TTY=1` makes the prompt readable from stdin; `KAIKA_LLAMA_SERVER` names the server program; `KAIKA_OFFLINE`, `KAIKA_MODELS`, `KAIKA_HOME`, `KAIKA_APPS_DIR` configure the rest. None weakens a check on the normal path.
- **App Store (`mas`):** `mas` uses the Apple ID signed in to the App Store app and can only fetch apps that account already obtained once. kaika never handles Apple ID credentials: a failed `mas` command explains the sign-in, and `doctor` says so.
- **Update suggestions, not automatic upgrades:** like macOS Software Update, kaika only suggests. `kaika schedule on [--at HH:MM]` writes `~/Library/LaunchAgents/io.kaika.upgrade.plist` (daily, default 03:00) that runs `kaika outdated --notify`: refresh the indexes, and when the set of available updates is new since the last notice, post one macOS notification ("N updates available … Run: kaika upgrade --all"); the same set is announced once, and being up to date resets it. Nothing is installed unattended. Offline runs exit 69 and stay quiet. `off` unloads and removes it; `status` (default) reads it back.
- **Verified live (2026-11-22):** a real launchd run of `schedule` (kickstarted, log read); `mas` 7 installing WhatsApp with the user signed in (found that `mas` 7 needs administrator rights and asks for them itself, and that the App Store listing name differs from the installed app's name: both fixed); headless Chrome rendering `obsidian.md/download`, a page with no links in its plain text, to find `Obsidian-1.15.1.dmg`. `kaika install chrome` no longer guesses npm's unrelated `chrome`: an exact name that only an open registry has is never taken when a reviewed source has a near name.
- **Plain-text names (`names.rs`):** `kaika install chrome`, `chome` and `vscode` mean google-chrome and visual-studio-code. When a name is not an exact reviewed package, or only an open registry has it exactly, kaika matches it against the reviewed formulae and casks as a whole word of the name, as its usual acronym (`vs`+`code`) or as a one- or two-letter typo (words of at least 5 letters), and takes the match only when it is overwhelmingly the most installed by Homebrew's public 365-day install counts (5x the runner-up, 10x for a typo; at least 1,000 installs a year). The counts are cached for a week in `~/.kaika/cache/popularity.tsv`. The plan says how the name was read. No model decides this; `-y` never confirms a guess at a typo (a person sees the plan, or names the package); with no clear winner it falls through to the old behaviour (a refusal listing the options, or discovery).
- **Download buttons with no file name (`recipe.rs`):** links such as `/api/downloads/...?platform=osx` or `/download/desktop/mac` are followed to the real file with a one-byte ranged request; test channels (ptb, canary, beta) are skipped, the Apple-silicon link goes first, and helpers, updaters and uninstallers are never taken for the app. Verified live on cursor.com, notion.com, discord.com and figma.com.
- **Found by the live Obsidian install:** a disk image's `Applications` shortcut was followed when looking for the app, so kaika could pick (and verify, and copy) an app from the user's own `/Applications` instead of the download. Symlinks are never followed now, with a test. A full install through discovery from a bare URL (render, redirect-free link, download, mount, verify, install, learned recipe, manifest) then worked on obsidian.md.
- **Not done yet:** a real unattended 03:30 launchd run; pages that need clicks or logins rather than just scripts, and download links that serve a file under no name at all (warp.dev's `get_warp?package=dmg`); installing from a name when the vendor page only offers the App Store (kaika says so and installs nothing).

## Left to the builder

- Which cases to add to the corpus next, and when to raise the gates.
