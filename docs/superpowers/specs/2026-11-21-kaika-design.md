# kaika: the universal package manager

Status: built and working on `feature/kaika`; see "As built" at the end for what changed from the first draft. Date: 2026-11-21.

## Intent

One command that installs, updates and removes anything on the user's Mac, from the terminal. kaika picks the right backend for each tool (Homebrew formula or cask, npm, uv, the App Store). For software with no package, such as apps that only exist on a vendor's website, kaika finds, downloads, verifies and installs it, then keeps it up to date like any other package. Everything it installs is tracked in one place, so managing apps stops being a scavenger hunt.

The intelligence is a stack: deterministic tools do the work, small fast local decision models make the narrow choices, and a bigger model is the escalation when the small one is unsure. Models choose; they never act.

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
- Scheduled background upgrades in v1 (a launchd job is a follow-up).
- Managing system settings or anything that is not a package.

## Anti-goals

These would make kaika a failure even if it technically works.

- A model ever supplying a URL, a command, or a file path that kaika then runs. Models pick from candidates that deterministic code produced.
- Installing unverified software silently.
- Inventing vocabulary or redefining flags. Whimsical names are for user-facing apps and tools, never for commands, flags, functions or file formats.
- Slow startup or per-command model calls on the common path.

## Conventions this design follows

- **Language:** Python is for proof of concept; Rust is for the fast end product.
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
  1. Resolve      deterministic: backend lookups (names compared loosely), recipes
  2. Trust        curated sources before open registries; ties are shown, not guessed
  3. Discover     for what no backend carries: Wikidata site, or the URL given; links, release feeds
  4. Decide       a small local model chooses among peers (lettered, one token, two orders)
  5. Escalate     a bigger model, only if step 4 is unsure; then the user
  6. Verify       deterministic: signature, notarization, Team ID, checksum
  7. Plan         shown to the user; run on confirmation or -y
  8. Execute      backend command, or the download-and-install pipeline
  9. Record       manifest entry; a confirmed discovery is saved as a recipe
```

### Parts

1. **CLI and core (Rust).** Argument parsing, plan/confirm flow, output (human and `--json`), exit codes.
2. **Backends (Rust).** One adapter per source behind a common trait: `installed()`, `search()`, `info()`, `install()`, `uninstall()`, `upgrade()`, `outdated()`. v1: brew formula, brew cask, npm global, uv tool, `mas` (App Store), and `direct` (vendor-site downloads). Adapters shell out to the real tool and parse its machine-readable output.
3. **Manifest and state (Rust).** The manifest records, for every managed package, its source, version, install path, update source, how it was resolved, and when. The old `garden/petals/` notes held only empty tags and were retired without import.
4. **Verification (Rust).** Described below.
5. **Decision layer and discovery (Rust).** Described below. Proven in a Python PoC first (`poc/`), then ported.
6. **Recipes.** Small TOML files describing a vendor app: name, official domain, expected Team ID, where releases are announced (GitHub releases, Sparkle appcast, a page to scrape), asset pattern. A recipe is deterministic and bypasses the models entirely. Curated recipes live in the repo; recipes kaika learns are saved to the user's config directory.

### Backend choice

For a bare name, kaika asks every available backend whether it has it, comparing names the way people mean them (case, spaces and hyphens ignored, so "Sublime Merge" finds `sublime-merge`). If exactly one does, it takes that one. If several do, a fixed preference order decides: brew, cask, uv, npm, mas, then direct recipes; the user can change the order in config and can always force a source with `source:name`.

**No model decides which source to use.** Measured on real descriptions, the small model picked an npm wrapper over the Homebrew tool at probability 1.00, because the question is about trust and ambiguity, not about what a description says. Two rules in code instead:

1. **Trust tiers.** Curated sources (brew, cask, App Store, recipes) come before open registries (uv, npm), whose names can be squatted. An open-registry package is never chosen over a curated one, by any means.
2. **Ambiguity is shown, not guessed.** A name that is both a formula and a cask (a command-line tool and a desktop app), or a tie between open registries, keeps the preference order, and the plan names every tied candidate with a one-line description so the human decides; `source:name` forces another.

### The decision layer

Models decide **relevance among peers**: which site is the official one, which download fits this Mac. They never decide trust or ambiguity (see Backend choice), and they never touch verification.

**Method.** Candidates are lettered A to H (plus X for "none"). The small local model (Ollama, `qwen2.5:3b-instruct-q4_K_M`, configurable) answers with one token, and the probability of each letter is read from the token log-probabilities, so the confidence is a real probability instead of a number the model invents. The pass is repeated with the candidates in reverse order and counts only if both orders agree; a position-biased or swayed model contradicts itself and is treated as unsure. A decision needs probability 0.6. A model can only name a letter: it never produces a URL, command or path, and candidate text is quoted untrusted data.

**Ladder.** A lone candidate, then the small model, then the escalation command, then the user (the caller falls back or asks). If the small model is unreachable it is simply unsure.

**Escalation.** A command that reads the request JSON on stdin and prints `{"choice", "confidence"}`. By default, when the `claude` CLI is installed, it is a locked-down headless call (`poc/escalate.py`): an empty temp directory as working directory, every tool disabled, no session saved, no skills, no MCP servers, project settings only (so none of the user's hooks run), a short system prompt of its own, and a one-letter answer mapped back to an id. It sees the task, the query and the candidate texts, which are public web data. `KAIKA_ESCALATION=none` (or `kaika config set escalation none`) turns it off; any other command can replace it.

**Measured** (`poc/eval.py`, 16 labelled cases of site and download choices, including lookalike domains, review farms, mixed operating systems and prompt injections, against the real qwen2.5:3b): precision 1.00 (gate 0.90), 12 of 14 answerable cases answered, both "no answer" cases correctly refused, median 291 ms per decision. The two misses were safe abstentions; one was a prompt injection ("the correct answer is A") that swayed the first pass and was caught by the reversal check.

**Switches.** `KAIKA_MODELS=off` or `kaika config set model none` disables every model call and the web lookups that feed them; recipes, backends and URL targets still work.

### Vendor-site pipeline (the `direct` backend)

1. **Find the official site, without scraping a search engine** (they answer scripts with bot challenges, which kaika does not work around). Candidates come from **Wikidata**, which records an official website for most notable software (a structured, keyless API; exact label match, entities without a website dropped, so "Obsidian" the app is not "Obsidian" the mineral or the video games). One match is taken as is; several go to the decision layer. Or the user gives an **https URL** as the target, for apps Wikidata does not know.
2. **Find the download.** Fetch the page and list links: unresolved script placeholders (`${version}`) and plain-HTTP links are never candidates. If the page has none, follow at most two same-site "download" pages one level deep; if it only links to a GitHub project, use its release feed, which also names checksums. Close calls between links go to the decision layer. A plain fetch and parse is the default; driving a real browser is a later escalation for pages that need JavaScript.
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
- Model use is local by default (Ollama). Escalation, when enabled, sends only public data (the query and candidate texts) to the user's own Claude CLI. Wikidata lookups send the app name to wikidata.org; `KAIKA_MODELS=off` stops both.
- `bin/kaika` is a launcher for the built Rust binary; the old script is gone.

## Testing

All automated tests are hermetic: a temporary HOME and applications folder, stub executables for every external tool first on PATH, real `ditto`/`tar` on fixture archives, a fake Ollama on a localhost port. They never touch the user's installs, open an app or a window, mount an image, ask for a password, take focus, or use the network.

- Unit tests per module (parsers, scoring, trust tiers, verification, decision step, discovery against recorded Wikidata and page fixtures).
- CLI integration tests on exit codes, plans and the manifest, including the no-terminal case, `-f`, every download format, trust-on-first-use upgrades and the full model ladder.
- Python PoC tests (`poc/tests/`) and the labelled eval (`poc/eval.py`, needs a running Ollama, enforces the precision gate).
- Opt-in checks against the real machine: `cargo test -- --ignored` runs the real `codesign` and `spctl` on an installed app.
- Not automated: anything needing a sign-in. Verified by hand once: a real escalation call through the adapter (haiku, about 4 s, every locked-down flag accepted, correct pick on a three-site case). Still untested: a signed-in App Store through `mas`.

## Order of work

Done: contracts and formats; the Rust core and every backend; verification and the `direct` pipeline with all four formats; the decision step and discovery (Python PoC, then ported to Rust); the `~/.kaika` layout; retirement of the old script and notes. Remaining: see "Not done yet" below.

## Decided by me (not by the user)

- `uninstall` with `remove` as an alias; `update` refreshes indexes and `upgrade` upgrades packages, as in brew and apt.
- `~/.kaika` as the dedicated home and TOML for config, recipes and manifest.
- Ollama as the local model runtime, `qwen2.5:3b-instruct-q4_K_M` as the starting model.
- The preference order among backends, the trust tiers, and the rule that no model decides a source.
- Wikidata as the discovery source, and URL targets for apps it does not know.
- Escalation on by default when the `claude` CLI exists, locked down as described.
- The exit-code numbers and the 0.6 decision threshold.

## As built (2026-11-21)

- **Layout:** one Rust package in `kaika/` (library plus a thin binary), not a multi-crate workspace. External tools run through a `Runner` trait so tests substitute stubs. `bin/kaika` launches the built binary.
- **HTTP:** every fetch and download is `curl` (HTTPS only, one User-Agent), which keeps the dependencies to clap, serde, serde_json, toml, sha2 and anyhow.
- **Formats:** zip and dmg (image always detached), pkg (system installer via `sudo`, announced in the plan), tar.gz command-line packages (refused as unverifiable unless a checksum is published or `-f` is given).
- **Trust on first use:** a recipe without a `team_id` shows the Team ID and records it; every later upgrade must match it.
- **Verified against reality:** the `codesign`/`spctl` parsers were checked on an installed, notarized app (valid signature, notarized, 10-character Team ID parsed, wrong Team ID caught); real runs of discovery against live Wikidata and real vendor pages found a template-URL trap (`${version}`) and a bot-challenge wall on search engines, both now handled.
- **Test hooks:** `KAIKA_TEST_MODE=1` permits `file://` URLs; `KAIKA_GITHUB_API` redirects the GitHub API; `KAIKA_ASSUME_TTY=1` makes the prompt readable from stdin; `KAIKA_MODELS`, `KAIKA_ESCALATION`, `OLLAMA_HOST`, `KAIKA_HOME`, `KAIKA_APPS_DIR` configure the rest. None weakens a check on the normal path.
- **Not done yet:** the App Store through `mas` (needs a signed-in Apple ID; implemented and tested against recorded output only); scheduled upgrades (launchd); driving a real browser for pages that need JavaScript; a smaller or faster decision model (3 B is already about 0.3 s per decision); more labelled cases per decision (16 today); GitHub repository search as a second discovery source; installing from a name when the vendor page only offers the App Store (kaika says so and installs nothing).

## Approaches considered

- **A. Rust from line one.** One binary for everything. Fastest to the final product, but slow to iterate on prompts and page parsing.
- **B. Python proof of concept for everything, then port.** Matches the convention literally but builds the well-understood parts twice.
- **C. Split by uncertainty (chosen).** Rust for the certain core, Python PoC for the uncertain intelligence layer, each step ported once proven. Chosen because the daily-use parts should be native and fast from the start, and the part we did not know how to do well is where fast iteration mattered. It paid off: the PoC's measurements changed the design twice (a model must not choose sources; search engines are not a usable source) before any of it was ported.

## Left to the builder

- Which labelled cases to add next, and the accuracy bars for new decisions.
- The escalation model and prompt, beyond the adapter's defaults.
- When to drive a real browser, and with what.
