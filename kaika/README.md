# kaika

The universal package manager. One command for everything on your Mac: it picks the right backend
(Homebrew formula or cask, npm, uv, the App Store), and for software that has no package it finds the
vendor's download, verifies it, installs it, and keeps it up to date.

```
kaika install ripgrep                  # backend chosen for you; shows the plan, asks, then runs
kaika install "Sublime Merge"          # spaces and case do not matter (finds the sublime-merge cask)
kaika install cask:zen                 # force a backend with source:name
kaika install https://vendor.com/app   # an app no database knows: kaika scans the page for the download
kaika install rectangle -n             # print the plan only
kaika install ghost -y                 # no prompt (needed when there is no terminal)
kaika upgrade --all                    # every backend, plus vendor-site apps via their release feeds
kaika uninstall zen --zap              # also remove support files, caches and preferences
kaika list | search | info | outdated | doctor | cleanup | config
```

## Flags

Established meanings, as in brew, apt, npm and uv.

| Flag | Meaning |
|---|---|
| `-n`, `--dry-run` | Print the plan; change nothing |
| `-y`, `--yes` | Skip the confirmation prompt |
| `-f`, `--force` | Override safety: failed verification, conflicts, an existing install |
| `--all` | Everything (`upgrade --all`) |
| `--zap` | Also remove an app's support files (`uninstall`) |
| `-q`, `-v`, `--json` | Quiet, verbose, machine-readable |

With no terminal to answer the prompt (an agent, a script) kaika prints the plan and exits 4 without
changing anything, unless `-y` is given. Exit codes: 0 ok, 1 failure, 2 usage, 3 verification failed, 4 needs confirmation.

## Where things live

Everything kaika owns is in one folder, `~/.kaika` (move it with `KAIKA_HOME`):

```
~/.kaika/
  kaika.toml      config                recipes/    your recipes and the ones kaika learned
  manifest.toml   what kaika installed  cache/      downloads and staging
  bin/            command-line tools: the only folder to put on PATH
  packages/<name>/<version>/            command-line packages, unpacked whole, linked into bin/
```

Apps go to `/Applications`.

## How it decides

**Which source?** Never a model. Curated sources (brew, cask, App Store, recipes) always come before open
registries (npm, uv), whose names can be squatted. A name that is both a formula and a cask, or a tie between
open registries, keeps the preference order and the plan shows each tied candidate with a one-line
description, so you decide (`source:name` forces one).

**An app no backend carries?** Wikidata records an official website for most notable software; kaika reads
that (search engines answer scripts with bot challenges, so it does not scrape them). Or give the page's https
URL. Then it lists the page's download links (never plain HTTP, never unresolved `${version}` templates),
follows a "download" page or a GitHub release feed if needed, and picks the macOS build by a deterministic score.

**Where a model helps.** Only for choosing between peers: which site is official, which of two close
downloads. A small local model (Ollama, `qwen2.5:3b`) sees lettered candidates and answers one token; the
probability of each letter comes from the token log-probabilities, and the answer only counts if it survives
reversing the candidate order. About 0.3 s per decision. A model can only name a letter: it never writes a
URL or a command. If it is unsure, a bigger model is asked (your own `claude` CLI, locked down: no tools, no
project, no hooks; it sees only the query and public candidate text), and then you are.

**Whatever is chosen is verified.** Every vendor download is checked for a valid signature, Apple
notarization, the signing Team ID (a recipe's, or the one recorded on first use) and a published checksum.
A failed check stops the install; only `-f` overrides it. A found app is saved as a recipe, so next time it
needs no lookup.

`KAIKA_MODELS=off` (or `kaika config set model none`) turns off every model call and the Wikidata lookup.
`KAIKA_ESCALATION=none` turns off only the bigger model.

## Layout of the code

- `src/` Rust: CLI, backends, manifest, verification, recipes, decision step (`decision.rs`), discovery (`discovery.rs`)
- `recipes/` curated vendor recipes
- `poc/` the Python proof of concept the decision step was ported from: `resolver.py`, `escalate.py`, the labelled `corpus/`, and `eval.py` to measure a model
- `tests/` hermetic integration tests; `poc/tests/` the PoC's tests

## Build and test

```bash
cargo build --release                         # bin/kaika launches this build
cargo test                                    # unit + hermetic integration tests
python3 -I -m unittest discover -s poc/tests  # the PoC
python3 -I poc/eval.py                        # measure the small model (needs Ollama running)
cargo test -- --ignored                       # real codesign/spctl on an installed app (read-only)
```

The automated tests are hermetic: a temporary HOME and apps folder, stub executables for brew/npm/uv/mas/curl/
codesign/spctl first on PATH, real `ditto`/`tar` on fixture archives, a fake Ollama. They never touch your
installs, open an app or a window, mount an image, take focus, ask for a password, or use the network.
