# kaika: independent review and cleanup, 2026-11-22

Reviewed: `kaika/` at `82fd266` (branch `feature/kaika`), all of `src/`, `tests/`, the design doc and the README.
Reviewer: a separate session, read-only to start with, then fixing. Everything below is on branch
`claude/kaika-review-95c922`; nothing was merged, pushed or tagged.

**Result.** Eleven things were wrong enough to fix (four medium, the rest low), each with a test that failed before the
change and passes after. Eleven more are left on purpose, with the reason. No gate or safety rule was loosened; four
were tightened (redirects, upgrade-time link screening, names and versions, deletion paths). `cargo test` is green (unit 157, integration 124, corpus 8; the two opt-in tests stay opt-in),
`cargo build --release` has no warnings, `cargo clippy --all-targets` has no warnings, `cargo fmt --check` passes.

## How it was done

Read the design doc and README, then every source file and the integration tests, checking the code against the
design and against the live-run facts in the brief (each is covered by an existing test, listed at the end). Every
suspected defect was tried before it was believed: a failing test first, or, where a claim did not survive, dropped
(see "Checked and rejected"). Fixes are small commits; the split and the cleanup came after, so the bug fixes can be
read in the old layout.

## Findings fixed

| # | Severity | Finding | Change |
|---|---|---|---|
| 1 | Medium | **HTTPS downgrade on redirect.** Every `curl -L` (page fetches, the download itself, the redirect resolver) follows an https URL's redirect to plain `http://`. Only the first URL was checked, so the last leg of a download could be tampered with; for a first install the trust-on-first-use Team ID would then be the attacker's, and a `.tar.gz` or model has only the checksum, which comes from the same feed. Confirmed with a local server. | `--proto-redir =https` on all three. Tests assert it for fetch, download and the resolver. |
| 2 | Medium | **Replacing an app deleted it first.** On `-f` or an upgrade, the installed `.app` was removed, then the new one copied. A full disk or a permission error left no app at all. | The new copy is made next to the old one and swapped in with renames; the old one is put back if the swap fails. Tests: failed copy leaves the app and no scratch folders; successful `-f` replacement leaves none either. |
| 3 | Medium | **Path traversal from a recipe name or a feed version.** The package name and version become folders (`packages/<name>/<version>`, `cache/stage/<name>-pid`). A recipe named `../x`, or an appcast version `../../x`, installed outside `~/.kaika/packages`, and on replacing, `remove_dir_all`'d a folder outside it. Recipes can be copied from anywhere; an appcast is the vendor's text. | Both must be one plain path component, checked before anything is downloaded. Test covers the name and the appcast version. |
| 4 | Medium (functional, with a safety side) | **A page recipe was re-read with a cruder scan than discovery used.** At upgrade or `outdated` time, `recipe::latest` took any `href=`: an unresolved `${version}` template, a plain-http link or a "Download for Windows" link could be picked as the next download (and `outdated` then reported "newer build" forever). The discovery screening had been added later and never reached this path. | One scan for both (`links::download_links`); the redundant second fetch of the page is gone. Test proven failing against the old code. |
| 5 | Low | **Option injection through a name.** A name starting with `-` (`kaika install -- --registry=...`, `npm:--registry=...`) reached brew, npm, uv and mas as an option. Matters for scripts and agents, which are the stated users. | `parse_target` refuses it as a usage error (exit 2). |
| 6 | Low | **`mas upgrade` plans did not say administrator rights are needed** (install and uninstall did). | `needs_admin()` is now asked for install, uninstall and both kinds of upgrade. |
| 7 | Low-medium (functional) | **The HTML reader missed real markup:** a tag split over lines (`<a\nhref=...`), a tab after `<a`, an unquoted or spaced `href`. A page could look link-less and fall through to a browser render or an error. | Attribute scanner rewritten (quoted either way, bare, any spacing and case; `data-href` is not `href`). On the old code the test finds one anchor with an empty href. |
| 8 | Low-medium (silent failure) | **`outdated` swallowed a vendor-site feed error**, so a dead GitHub project or appcast read as "everything is up to date" (and the daily check never mentioned it). | A warning per app on stderr (and so in the daily log). |
| 9 | Low (silent failure) | **`cleanup` ignored removal errors** and printed "freed N bytes". | Exit 1 and says what stayed. |
| 10 | Low (half-state) | **`schedule on` left the plist behind when `launchctl` refused it**, so `schedule status` and `doctor` claimed a job that never loaded. | The file is removed on failure. |
| 11 | Low (hardening) | **Uninstall trusted the manifest's path:** `remove_dir_all` of whatever lies "under" `packages/`, including `packages/` itself or `packages/../x`. | The path must be strictly inside, with no `..`. Unit test. |

Also changed while there, without a separate finding: the tarball install used the real runner for its copy fallback
instead of the injected one (a test seam bypassed).

## Findings left, and why

| # | Severity | Finding | Why it stays |
|---|---|---|---|
| L1 | Low | A model that never answers its health check leaves the server in the "running" state, so each later `choose` waits the full 90 s again (up to about 180 s in a failing discovery that asks twice). | A test needs either a 90 s wait or a new environment knob, and the brief says not to invent names. Fix when convenient: mark the state `Failed` on timeout in `LlamaDecider::wait_until_ready`. |
| L2 | Low-medium | `.pkg`: `check_pkg` verifies a file in `~/.kaika/cache/downloads`, then `sudo installer` reads it. Another process of the same user could swap it between the two (privilege escalation to root). | The attacker already runs as the user, and a real fix (a copy in a root-owned place, or at least re-hashing just before `sudo`) is a design decision, not a cleanup. Re-hashing immediately before `sudo` is cheap and would shrink the window; recommended. |
| L3 | Low-medium | Upgrades check the Team ID recorded at first install. If none was recorded (installed with `-f`, or a package that had none), a later upgrade accepts any signer that is validly signed and notarized. | That is the existing trust-on-first-use design; tightening it changes behaviour users have state for. Recommend: refuse an upgrade with no recorded team unless `-f`. |
| L4 | Info | Archive path safety (`..`, absolute paths, links out of the folder) relies on `ditto -xk` and bsdtar's own defaults; kaika does not check. Symlinks are never followed when looking for the app, and a package's executables are linked by name, so a hostile archive gains nothing from them here. | Not independently verified on this machine; a post-extract containment check would be an addition, not a fix. |
| L5 | Info | `Recipe.domain` is recorded and shown but nothing enforces it: at upgrade time a page recipe accepts a matching link from any host the page names. The Team ID check is what protects that. | A schema field (kept), and enforcing it would reject legitimate CDN hosts. Now documented as informational. |
| L6 | Info | Download farms are screened as *sites* (Wikidata candidates) but not as *download hosts*: a vendor page's link to a CDN is taken. | Vendor pages legitimately use CDNs; verification is the gate. |
| L7 | Info | `config set order` accepts backend ids that do not exist; they are silently ranked last. | Changing it would turn an accepted value into an error. |
| L8 | Info | If saving the manifest fails after an app is installed, the app is installed but unrecorded. | Rare (the folder was just written to); the install itself succeeded and the error is shown. |
| L9 | Info | `KAIKA_TEST_MODE=1` allows `file://` URLs in the shipped binary. | Documented test hook; anyone who can set the environment already controls the user's session. |
| L10 | Info | The headless-browser profile folder is `$TMPDIR/kaika-render-<pid>` (predictable). | macOS's `$TMPDIR` is per user; the profile is empty and removed after each render. |
| L11 | Info | `-q` does not silence warnings, which go to stderr. | Intended: warnings are not progress chatter. |

**Resolved after the review:** L1 (a model that times out is remembered as failed and stopped), L2 (the `.pkg` is fingerprinted before the checks and again before `sudo installer`; a change is refused even with `-f`, which narrows the window but cannot close it for another process of the same user) and L3 (an upgrade of an app with no recorded Team ID needs `-f`; command-line tarballs, which carry no signature, are unaffected).

## Checked and rejected

- A suspected panic in `link_pattern` for a file name that Unicode-lowercases to an extension (U+212A, the Kelvin
  sign, lowercases to `k`, so `.pKg` reads as `.pkg`). The slice offset is measured from the end and always falls on
  the character boundary, so it cannot panic. Reverted, not reported as a bug.
- Panics reachable from external input (`unwrap`, `expect`, indexing): none found. The `expect("picked from the
  list")` after a decider's answer is now a refusal instead; the remaining `unwrap`s are on values the line above
  just checked.
- Command and AppleScript injection: every external call passes arguments as an array (no shell, except the model
  supervisor, which receives its arguments as positional parameters); the notification text is escaped as data (tested
  with a hostile package name); the launchd plist escapes `& < >`.

## What the brief's known facts rest on (all verified present and tested)

- `mas` 7 needs administrator rights (plan says so for install, uninstall and, since #6, upgrade) and asks itself.
- The manifest records the installed name, not the App Store listing's (`an_app_store_install_is_recorded_under_...`).
- A disk image's `Applications` symlink is never followed (`a_symlink_to_the_applications_folder_...`, `direct/app.rs`).
- An exact name only an open registry has does not beat a reviewed near match; `chome` needs a person, `-y` refuses
  it (`a_short_name_that_only_a_registry_has_exactly_...`, `yes_never_confirms_a_guess_at_a_typo...`).
- Extensionless buttons are followed with a one-byte ranged request (`links::final_url`); pages with no links are
  rendered once in a headless browser if one is installed (`KAIKA_BROWSER`, `off`); the daily job only runs
  `outdated --notify` (`the_job_only_checks_and_notifies...`); offline commands exit 69 (`offline_...`).
- No unverified install without `-f`, by format: app (`.zip`, `.dmg`): signature, notarization, Team ID, checksum;
  `.pkg`: the same, then `sudo installer`; `.tar.gz` and `.gguf`: the published checksum, and a missing one is itself
  a failure; anything else is refused as an unsupported format. See L2 and L3 for the two soft spots.

## The cleanup

**Layout** (the README's code-layout section now matches the code):

- `commands.rs` (891 lines, one 160-line `resolve_install`, a 120-line `execute`) became `commands/`: `mod.rs`
  (context, the exit-code error, dispatch), `install`, `uninstall`, `upgrade`, `query`, `outdated`, `schedule`,
  `housekeeping`, `rows`, `run`. Long functions are cut into named steps.
- `discovery.rs` (951 lines) became `discovery/` (`mod`, `wikidata`, `github`, `site`, `tests`); the 100-line nest in
  the vendor-page scan is a list of strategies tried in order.
- `recipe.rs` gave up what was not a recipe: `asset.rs` (arithmetic on file names), `links.rs` (page links), and
  `net.rs` gained the one HTTPS-only fetch. `direct.rs` became `direct/` (`mod`, `app`, `package`, `remove`).
  `schedule.rs` merged into the two commands that use it.
- Dependency cycles are gone (`recipe` <-> `direct` <-> `discovery`): `net`, `asset` and `links` sit under them.

**Removed because nothing used it:** `quarantine::split`, `html::percent_decode`, `Anchor.class`, `Expect.sha256`,
`Dirs.config` and `Dirs.data` (always equal to `root`), `recipe::extract_links` and the page fetched twice,
`run_ok_anyhow`, a second copy of the redirect resolver's scoring, redundant clauses of `is_template`. The scripted runner
`Fake` and `LlamaDecider::is_running` are compiled only for tests. Parsers only their own module uses are private.

**Tooling:** default `cargo fmt` (no `rustfmt.toml` needed; this is the one mechanical commit, which is why the line
count rose), `cargo clippy --all-targets` clean (nine warnings fixed, none silenced). `rustfmt` and `clippy` were
installed with `rustup component add` on request.

**Documentation:** `///` on every public item that is not a self-evident field, `//!` on every module, a layer map in
`lib.rs`, the layout in the README, the two stale file names in the design doc.

**Numbers** (non-blank, non-comment lines, the old tree formatted the same way): code 5,243 to 5,393; in-file unit
tests 2,407 to 2,698; integration tests 2,948 to 3,150; comment and doc lines 341 to 657. Roughly 190 lines of dead or
duplicate code went; about 230 lines of fixes and 80 of new tests came; the rest is the split into smaller functions
and modules. 276 tests became 289.

## What changed in tests

Existing assertions were kept. Touched: the curl command strings the tests key on gained `--proto-redir =https`
(`api()` helpers and two literals); import paths moved with the code (`kaika::net::USER_AGENT`, `kaika::asset::*`,
`kaika::links::*`); `tests/corpus.rs` has `assert!(!x)` for `assert_eq!(x, false)`; `tests/real_model.rs` only gained a
type alias (its gates, its numbers and `KNOWN_SHORTCUT_MISSES` are byte-for-byte the same values). Four exit-code
tests that only checked the code now check the message too. Assertions for deleted dead code went with it
(`class`, `percent_decode`, `quarantine::split`). `corpus/*.json`, `packaging/`, `.github/` and `bin/kaika` are
untouched.

## Left for a person

L1 to L3 above, and the question of whether an upgrade whose recorded Team ID is empty should need `-f`.
