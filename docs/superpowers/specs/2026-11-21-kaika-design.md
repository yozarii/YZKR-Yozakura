# kaika: the universal package manager

Status: draft for approval. Date: 2026-11-21.

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
- **Paths:** XDG-style, as uv and cargo use. Config in `~/.config/kaika/`, state in `~/.local/share/kaika/`, cache and downloads in `~/.cache/kaika/`.

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
kaika install <name>
  1. Resolve      deterministic: exact backend lookups, recipes
  2. Decide       decision model picks among candidates (constrained JSON, confidence)
  3. Fetch        deterministic: pages, links, release feeds (no model)
  4. Escalate     bigger model, only when step 2 is unsure or a page needs real browsing
  5. Verify       deterministic: notarization, signature Team ID, checksum
  6. Plan         shown to the user; run on confirmation or -y
  7. Execute      backend command, or download-and-install pipeline
  8. Record       manifest entry; resolution cached as a recipe
```

### Parts

1. **CLI and core (Rust).** Argument parsing, plan/confirm flow, output (human and `--json`), exit codes.
2. **Backends (Rust).** One adapter per source behind a common trait: `installed()`, `search()`, `info()`, `install()`, `uninstall()`, `upgrade()`, `outdated()`. v1: brew formula, brew cask, npm global, uv tool, `mas` (App Store), and `direct` (vendor-site downloads). Adapters shell out to the real tool and parse its machine-readable output.
3. **Manifest and state (Rust).** The manifest records, for every managed package, its source, version, install path, update source, how it was resolved, and when. It starts from a one-time import of the existing `garden/petals/` notes, after which that vocabulary is retired.
4. **Verification (Rust).** Described below.
5. **Resolver and decision layer.** Described below. A Python PoC at first, ported to Rust behind the same interface once proven.
6. **Recipes.** Small TOML files describing a vendor app: name, official domain, expected Team ID, where releases are announced (GitHub releases, Sparkle appcast, a page to scrape), asset pattern. A recipe is deterministic and bypasses the models entirely. Curated recipes live in the repo; recipes kaika learns are saved to the user's config directory.

### Backend choice

For a bare name, kaika asks every available backend whether it has it. If exactly one does, it takes that one. If several do, a fixed preference order decides: brew cask for GUI apps, brew formula for command-line tools, then uv or npm for language tools, then `mas`, then direct. The user can change the order in config and can always force a source with `source:name`. Where the deterministic order is ambiguous (for example, a name that is a CLI tool in one backend and an unrelated app in another), the decision model picks from the enumerated candidates.

### The decision layer

Small local models (Ollama, qwen2.5:3b to start, configurable) make narrow decisions, each with a defined candidate list, a constrained JSON answer, and a confidence. If the confidence is below that decision's threshold, kaika escalates to the larger model; if that is unsure too, it asks the user.

| Decision | Candidates from | Model picks |
|---|---|---|
| Which backend carries this tool | backend searches | one candidate, or none |
| Which site is the official vendor site | search results plus domain signals | one domain, or none |
| Which link on the page is the download | link list parsed by code | one link ID |
| Which asset fits this Mac (arm64, macOS, format) | asset list from a release feed | one asset ID |
| What state is this page (download, login wall, captcha, error, redirect) | page features | one state |
| Which new release is newer or a pre-release | version list | one release |

Rules:

- A model only ever returns an ID from the candidate list or "none". Anything else is rejected and counts as unsure.
- Page text and search results are untrusted data. They are given to a model only as quoted candidate text and never as instructions.
- Each decision has a labelled test corpus. A decision is only allowed to rely on the small model if it meets an accuracy bar on that corpus; otherwise it always escalates. This is measured, not assumed.
- The escalation model is configurable and receives only public data: the tool name, candidate lists and page excerpts. No personal information leaves the machine.
- A confirmed resolution is saved as a recipe, so the same app is resolved deterministically next time.

### Interface between the Rust core and the Python PoC

The resolver is a subprocess that reads one JSON request on stdin and writes one JSON response on stdout. The request carries the task and the candidates; the response carries the chosen ID, a confidence, the evidence it used, and the model that produced it. This keeps the seam narrow, lets the Rust core be tested with a fake resolver, and means each resolver step can be ported to Rust one at a time without changing the core.

### Vendor-site pipeline (the `direct` backend)

1. Locate the official site: from a recipe, otherwise from search results, with the model choosing among candidate domains using signals such as a matching name, a link from the project's repository, and an expected code-signing identity.
2. Find the download: fetch the page, list links, pick the download. A plain HTTP fetch and HTML parse is the default; a real headless browser is used only when the page needs JavaScript, and is an escalation.
3. Pick the asset for macOS on Apple silicon (formats in v1: dmg, zip, pkg, and tar.gz command-line binaries).
4. Download over HTTPS to `~/.cache/kaika/downloads/`, then verify (below).
5. Install: copy the app out of a mounted dmg or an unzipped archive into `/Applications`; run a pkg with the system installer (this needs administrator rights, so the plan states it and kaika never elevates silently).
6. Record in the manifest with the update source, so `upgrade` can check for newer versions through the recipe's release feed (GitHub releases, Sparkle appcast, or a page re-check).

### App Store

Through `mas`. It can install apps already associated with the user's Apple ID and update them. Purchases are out of scope; kaika tells the user to buy in the App Store themselves.

### Verification (no model is involved)

For every direct download, before anything is installed:

- Code signature is valid.
- The app is notarized by Apple.
- The signing Team ID matches the recipe's expected one; for an unknown app, the Team ID is shown in the plan and recorded for future checks.
- A published checksum matches, when the vendor provides one.

Any failure stops the install with exit code 3. `-f` overrides it, and only with the failure printed prominently in the plan. kaika never sends a model's opinion in place of a failed check.

## As built (2026-11-21)

The first working version follows this design. Where it fills in what the design left to the builder, or differs:

- **Layout:** one Rust package in `kaika/` (library plus a thin binary), not a multi-crate workspace. External tools run through a `Runner` trait so tests substitute stubs. Python PoC in `kaika/poc/`; the legacy `bin/kaika` is untouched.
- **HTTP:** kaika shells out to `curl` for every fetch and download (HTTPS only), which keeps the dependency list to clap, serde, toml, sha2 and anyhow.
- **Formats:** zip and dmg (app copied out, image always detached), pkg (system installer via `sudo`, announced in the plan as needing administrator rights), and tar.gz command-line tools (installed to `~/.local/bin`, refused as unverifiable unless a checksum is published or `-f` is given).
- **Trust on first use:** a recipe without a `team_id` shows the Team ID and records it in the manifest; every later upgrade must match it.
- **Resolver contract:** request `{task, query, candidates?, model, escalation, threshold}`; answer `{choice | url/page/github/link_pattern/domain/version/sha256/app, confidence, evidence, model}`. Rust rejects any choice outside the candidate list, any non-HTTPS URL, and anything below the 0.6 threshold. The Python side asks the small model twice with the candidate order reversed and treats a contradiction as unsure.
- **Learned recipes:** a confirmed vendor-site resolution is saved to `~/.config/kaika/recipes/` (GitHub-hosted projects follow their release feed; others follow the page).
- **Test hooks:** `KAIKA_TEST_MODE=1` permits `file://` URLs; `KAIKA_GITHUB_API` redirects the GitHub API; `KAIKA_RESOLVER`/`KAIKA_RESOLVER_FIXTURES` replace the resolver and its web pages; `KAIKA_ASSUME_TTY=1` makes the prompt readable from stdin. None of these weakens a check on the normal path.
- **Not done yet:** the escalation model is a configurable command with no default (kaika asks you instead); `mas` is implemented and tested against recorded output but not against a signed-in App Store; scheduled upgrades (launchd) and the one-time import of the old `garden/petals/` notes are follow-ups; the Python resolver steps are not yet ported to Rust; the small model's accuracy bars (per-decision labelled corpora) are not yet measured. A five-case spot check against the real qwen2.5:3b through Ollama confirmed the API shapes and the safe behaviour (three clear cases picked correctly with confidence 0.90 to 1.00; "nothing fits" and a genuinely ambiguous case came back unsure). Latency is the open weakness against the "extremely fast" goal: about 3.5 to 5 s per decision (two passes), 11 s with a cold model. Next steps are a smaller or faster decision model, a single pass for decisions that have a strong deterministic signal, and keeping the model warm.

## Approaches considered

- **A. Rust from line one.** One binary for everything. Fastest to the final product, but slow to iterate on prompts and page parsing.
- **B. Python proof of concept for everything, then port.** Matches the convention literally but builds the well-understood parts twice.
- **C. Split by uncertainty (chosen).** Rust for the certain core (CLI, backends, manifest, verification, plan/confirm), Python PoC for the uncertain intelligence layer, joined by the JSON interface; resolver steps are ported to Rust as they prove out. Chosen because the daily-use parts should be native and fast from the start, and the part we do not yet know how to do well is where fast iteration matters.

## Constraints

- Apple silicon macOS (currently macOS 27). Rust toolchain installed through rustup.
- Model use is local by default (Ollama); escalation may use a remote model and only sends public data.
- The legacy script `bin/kaika` keeps working until the Rust binary reaches parity for brew, cask, npm, uv and app listing, then is removed.

## Testing

- Backend adapters: tested against recorded command output, plus a small set of live read-only checks.
- Resolver: per-decision labelled corpus with accuracy gates (see above); saved page fixtures so tests run offline.
- Verification: signed, unsigned, tampered and wrong-Team-ID fixtures.
- CLI: golden tests on `--json` output and exit codes, including the no-terminal case.

## Order of work (a suggestion for the plan)

1. Repo layout, JSON contracts, recipe and manifest formats.
2. Rust core and backends reaching parity with today's script (list, install, uninstall, upgrade).
3. Verification and the `direct` pipeline with the Python resolver, proven on about ten real apps, including at least one vendor-site-only app.
4. Decision models with the labelled corpora and gates; escalation wiring.
5. `mas` backend, `search`, `outdated`, `doctor`.
6. Port resolver steps to Rust; remove the Python and the legacy script.

## Decided by me (not by the user)

- `uninstall` with `remove` as an alias; `update` refreshes indexes and `upgrade` upgrades packages, as in brew and apt.
- XDG-style paths and TOML for config, recipes and manifest.
- Ollama as the local model runtime, qwen2.5:3b as the starting model.
- Preference order among backends as listed above.
- The exit-code numbers.

## Left to the builder

- Repository layout and crate boundaries inside the Rust workspace.
- The exact recipe schema and manifest schema.
- Model confidence mechanics (log-probabilities versus self-report) and the accuracy thresholds per decision.
- Choice of HTTP, HTML-parsing and browser-automation libraries.
- Which escalation model and API to use; the interface just needs it to be swappable.
