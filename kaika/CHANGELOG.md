# Changelog

## 0.1.0 (unreleased)

The first release. kaika is one command-line tool for installing, upgrading and removing software on an Apple-silicon
Mac, whatever carries it.

### What it does

- **One front door.** `kaika install | uninstall | upgrade | update | list | search | info | outdated | doctor |
  cleanup | schedule | config`. It picks the backend (Homebrew formula or cask, npm, uv, the App Store through `mas`),
  always preferring curated sources over open registries, and shows the plan before it changes anything.
- **Plain-text names.** `chrome`, `vscode`, `"Sublime Merge"`, a typo: matched against reviewed Homebrew formulae and
  casks; a guess at a typo is never confirmed by `-y`. `source:name` forces a backend.
- **Apps no backend carries.** The vendor's site is found through Wikidata (or an https URL you give, or a GitHub
  project as a fallback), its download is picked by a deterministic score, and a small local model
  (H2O-Lightning-4B through `llama-server`) decides only when that is ambiguous. The model is installed by you, runs
  only for these decisions, stops when the command ends, and is turned off by `KAIKA_MODELS=off`.
- **Verification.** Every vendor download is checked for signature, notarization, the signing Team ID (a recipe's, or
  the one recorded on first use) and a published checksum. A failure installs nothing (exit 3); only `-f` overrides it.
  Redirects may not leave https. Text from the web that tries to talk to the chooser is dropped before the model sees it.
- **Update suggestions.** `kaika schedule on [--at HH:MM]` installs a launchd job that checks once a day and posts a
  notification. It never installs anything; you run `kaika upgrade --all`.
- **State** lives in one folder, `~/.kaika` (`KAIKA_HOME`); apps go to `/Applications`.
- **Exit codes** 0 ok, 1 failure, 2 usage, 3 verification failed, 4 needs confirmation but no terminal, 69 offline.

### Known limits

- **Vendor-page accuracy is measured, not gated.** On 32 real vendor download pages (`corpus/real.json`) the
  deterministic choice is right for 21. The misses are bare archive names with no system in them, an unversioned file
  against a versioned older-macOS one (Itsycal), and arm64 against the universal build (Wireshark, Audacity). A wrong
  pick fails verification and installs nothing, but kaika then asks you or stops; it does not always find the right file.
- **The model gate is synthetic.** The 95% / 1% release gates apply to 626 labelled cases written for this project
  (`corpus/discovery.json`), not to the open web. The real-model test is opt-in (`cargo test --test real_model --
  --ignored`) and is not part of `cargo test` or CI.
- **The daily check is opt-in.** Nothing is scheduled until you run `kaika schedule on`.
- **A `.pkg` can be raced.** The file is fingerprinted before the checks and again just before `sudo installer`, and a
  change is refused even with `-f`. That narrows the window; it cannot close it against another process running as the
  same user.
- Archive path safety relies on `ditto` and `tar`'s own defaults; kaika does not re-check what they extract.
- App Store installs go through `mas`, need an Apple ID already signed in to the App Store app, and need administrator
  rights that `mas` asks for itself.
- Apple silicon is the tested target. The formula builds from source, so an Intel Mac may build it, but it has not
  been tried.
- Unsigned: the binary is not notarized (installed through Homebrew it is not quarantined).

The design and the review that preceded this release are in `docs/superpowers/specs/2026-11-21-kaika-design.md` and
`docs/superpowers/reviews/2026-11-22-kaika-review.md` in the repository.
