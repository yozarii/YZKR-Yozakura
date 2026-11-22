# kaika

The universal package manager. One command for everything on your Mac: it picks the right backend
(Homebrew formula or cask, npm, uv, the App Store), and for software that has no package it finds the
vendor's download, verifies it, installs it, and keeps it up to date.

```
kaika install ripgrep                  # backend chosen for you; shows the plan, asks, then runs
kaika install "Sublime Merge"          # spaces and case do not matter (finds the sublime-merge cask)
kaika install cask:zen                 # force a backend with source:name
kaika install https://vendor.com/app  # an app no database knows: kaika scans the page for the download
kaika install rectangle -n             # print the plan only
kaika install ghost -y                 # no prompt (needed when there is no terminal)
kaika upgrade --all                    # every backend, plus vendor-site apps via their release feeds
kaika uninstall zen --zap              # also remove support files, caches and preferences
kaika list | search | info | outdated | doctor | cleanup | config
```

## Flags and exit codes

Established meanings, as in brew, apt, npm and uv.

| Flag | Meaning |
|---|---|
| `-n`, `--dry-run` | Print the plan; change nothing |
| `-y`, `--yes` | Skip the confirmation prompt |
| `-f`, `--force` | Override safety: failed verification, conflicts, an existing install |
| `--all` | Everything (`upgrade --all`) |
| `--zap` | Also remove an app's support files (`uninstall`) |
| `-q`, `-v`, `--json` | Quiet, verbose, machine-readable |

With no terminal to answer the prompt (an agent, a script) kaika prints the plan and exits 4 without changing anything,
unless `-y` is given.

| Code | Meaning |
|---|---|
| 0 | ok |
| 1 | failure |
| 2 | usage error |
| 3 | verification failed, nothing installed |
| 4 | needs confirmation but there is no terminal |
| 69 | offline (sysexits `EX_UNAVAILABLE`) |

`install`, `upgrade`, `update`, `outdated` and `search` check the network first (a 2 second probe) and exit 69 before
starting anything. `list`, `info`, `uninstall`, `cleanup`, `doctor` and `config` work offline. `KAIKA_OFFLINE=1` forces it.

## Update suggestions and the App Store

Like macOS Software Update, kaika only suggests; it never upgrades by itself.

```bash
kaika schedule on --at 03:30   # a launchd job: refresh indexes, then notify if there are new updates
kaika schedule status
kaika schedule off
kaika outdated --notify        # what the job runs; posts one notification per new set of updates
kaika upgrade --all            # the only thing that installs; you run it
```

App Store apps go through `mas`, which uses the Apple ID signed in to the App Store app, can only fetch apps that
account has already obtained once, and asks for your administrator password itself. kaika never handles Apple ID
credentials and explains this when `mas` fails.

## Where things live

Everything kaika owns is in one folder, `~/.kaika` (move it with `KAIKA_HOME`):

```
~/.kaika/
  kaika.toml      config                recipes/    your recipes and the ones kaika learned
  manifest.toml   what kaika installed  cache/      downloads and staging
  bin/            command-line tools: the only folder to put on PATH
  packages/<name>/<version>/            command-line packages, unpacked whole, linked into bin/
  models/h2o-lightning/                 the decision model (only if you use third-party apps)
```

Apps go to `/Applications`.

## How it decides

**Plain text works.** `kaika install chrome`, `chome`, `vscode`, `discord`, `obsidian`: an exact package is used as is;
otherwise the name is matched, as a word, abbreviation or typo, against reviewed Homebrew formulae and casks and taken
only if it is by far the most installed match. `-y` never confirms a guess at a typo: you look at the plan or name the package. The plan says how it was read (`'chome' is read as cask:google-chrome`).

**Which source?** Never a model. Curated sources (brew, cask, App Store, recipes) always come before open registries
(npm, uv), whose names can be squatted. A name that is both a formula and a cask, or a tie between open registries,
keeps the preference order and the plan shows each tied candidate with a one-line description, so you decide
(`source:name` forces one).

**An app no backend carries?** Wikidata records an official website for most notable software (search engines answer
scripts with bot challenges, so kaika does not scrape them). Or give the page's https URL. Then kaika lists the page's
download links (never plain HTTP, never unresolved `${version}` templates, never debug symbols or checksum files),
follows a "download" page or a GitHub release feed if needed, and picks the macOS build by a deterministic score,
which settles about nine downloads in ten with no model at all.

If Wikidata has no site (or the site yields no download), kaika falls back to GitHub: a repository named exactly like the
app, not a fork or archived, with at least 50 stars. A page with no links at all is rendered once in a headless Chromium
browser if one is installed (never downloaded for this; `KAIKA_BROWSER=off` disables), then scanned like any page.

**Where a model helps, and only there.** When several sites or downloads are plausible, a small local model
(H2O-Lightning-4B, Apache-2.0) chooses among them. It is used only for third-party apps, never on a package-manager
path and never at startup:

- It is started when discovery begins, so loading overlaps the network fetches, and stopped when the command ends:
  nothing is resident at rest, about 3 GB while it runs. A supervisor stops it even if kaika is killed.
- It runs through `llama-server` (`kaika install llama.cpp`) and is installed like any package
  (`kaika install h2o-lightning`, a 2.7 GB download pinned to one SHA-256). If either is missing, `kaika` says so; it
  never installs anything silently. `kaika doctor` shows the state without starting anything.
- It answers a typed choice with probabilities. A sure answer (0.8 or more) is taken in one pass; a middling one
  (0.6 to 0.8) must survive reversing the options; anything weaker, or "none of these", means kaika asks you.
- Candidate texts come from the open web, so any that talk to the chooser ("ignore the other options", "the correct
  answer is A", in several languages) are dropped before the model sees them, and the plan says so.
- No cloud model is ever asked. `KAIKA_MODELS=off` (or `kaika config set models off`) turns off every model call and the
  web lookups that feed them; recipes, backends and URL targets still work.

**Whatever is chosen is verified.** Every vendor download is checked for a valid signature, Apple notarization, the
signing Team ID (a recipe's, or the one recorded on first use) and a published checksum. A failed check stops the
install; only `-f` overrides it. A found app is saved as a recipe, so next time it needs no lookup.

## Layout of the code

- `src/` Rust: CLI, backends, manifest, verification, recipes, discovery, the model manager (`model.rs`), the decision
  rules (`decision.rs`, `quarantine.rs`), the offline check (`net.rs`)
- `recipes/` curated recipes, including the pinned model
- `corpus/discovery.json` 626 labelled third-party-app decisions: official site and right download, with lookalikes,
  typosquats, nightly/beta/checksum/debug files, localized builds and injections
- `tests/` hermetic integration tests, corpus-driven tests, and one opt-in test against the real model

## Build and test

```bash
cargo build --release                                      # bin/kaika launches this build
cargo test                                                 # unit, integration and corpus tests
cargo test --test real_model -- --ignored --nocapture      # the real model over the whole corpus (needs llama-server and the model)
cargo test -- --ignored                                    # also: real codesign/spctl on an installed app (read-only)
```

The automated tests are hermetic: a temporary HOME and apps folder, stub executables for brew/npm/uv/mas/curl/codesign/
spctl/llama-server first on PATH, real `ditto`/`tar` on fixture archives. They never touch your installs, open an app or
a window, mount an image, take focus, ask for a password, load a model, or use the network.
