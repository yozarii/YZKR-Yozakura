# kaika

The universal package manager. One command for everything on your Mac: it picks the right backend
(Homebrew formula or cask, npm, uv, the App Store), and for software that has no package it finds the
vendor's download, verifies it, installs it, and keeps it up to date.

```
kaika install ripgrep            # backend chosen for you; shows the plan, asks, then runs
kaika install cask:zen           # force a backend with source:name
kaika install rectangle -n       # print the plan only
kaika install ghost -y           # no prompt (needed when there is no terminal)
kaika upgrade --all              # every backend, plus vendor-site apps via their release feeds
kaika uninstall zen --zap        # also remove support files, caches and preferences
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

## How it decides

```
deterministic lookups and recipes  ->  small local decision model  ->  bigger model  ->  ask you
```

Models only ever return the id of a candidate that deterministic code produced, or "none". Every vendor
download is checked for a valid signature, Apple notarization, the signing Team ID (a recipe's, or the one
recorded on first use) and a published checksum. A failed check stops the install; only `-f` overrides it.

## Layout

- `src/` Rust: CLI, backends, manifest, verification, recipes, the resolver client
- `recipes/` curated vendor recipes (learned ones are saved in `~/.config/kaika/recipes/`)
- `poc/resolver.py` the decision layer (Python proof of concept; steps get ported to Rust once proven)
- `tests/` hermetic integration tests; `poc/tests/` resolver tests

State lives in `~/.config/kaika` (config, recipes), `~/.local/share/kaika` (manifest), `~/.cache/kaika` (downloads).

## Build and test

```bash
cargo build --release
cargo test                                   # unit + hermetic integration tests
python3 -I -m unittest discover -s poc/tests # decision layer
```

The tests are hermetic: a temporary HOME and apps folder, stub executables for brew/npm/uv/mas/curl/codesign/spctl
first on PATH, real `ditto` on fixture archives, a fake Ollama on a localhost port. They never touch your installs,
open an app or a window, mount an image, ask for a password, or use the network.
