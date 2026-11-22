//! The `direct` backend: download from a vendor, verify, install, and remove.
//!
//! Every download is checked before anything is installed (`verify`): a valid signature, Apple's notarization, the
//! signing Team ID (a recipe's, or the one recorded on first use) and a published checksum when the vendor has one.
//! A failed check stops the install with [`DErr::Verify`] unless the job carries `-f`. Which format is installed how:
//!
//! | download | what happens |
//! |---|---|
//! | `.zip`, `.dmg` | the app is copied out, verified, then placed in the applications folder (`app`) |
//! | `.pkg` | verified, then the system installer runs under `sudo`, announced in the plan |
//! | `.tar.gz`, `.tgz` | a command-line package: unpacked whole, executables linked into `~/.kaika/bin` (`package`) |
//! | `.gguf` | a model file: checksum required, moved into `~/.kaika/models` |

mod app;
mod package;
mod remove;

pub use remove::uninstall;

use crate::asset::file_name;
use crate::net::ensure_https;
use crate::paths::Dirs;
use crate::recipe::{Asset, Recipe};
use crate::runner::{Runner, argv};
use crate::verify::{self, Expect};
use anyhow::{Result, anyhow, bail};
use std::path::Path;

/// One download-and-install, fully decided: what to fetch, what to expect of it, and what the plan should say.
#[derive(Debug, Clone)]
pub struct Job {
    pub name: String,
    /// Empty when the source does not say (a page or a fixed URL).
    pub version: String,
    pub url: String,
    /// A published checksum of the download, when the source has one.
    pub sha256: Option<String>,
    /// The signing Team ID the download must carry; `None` is trust on first use.
    pub expect_team: Option<String>,
    /// The bundle name to install, when a download holds several apps.
    pub app_name: Option<String>,
    /// How this job came to be, for the manifest and the plan ("recipe", "found via Wikidata").
    pub resolved_by: String,
    /// Where newer versions are announced, for the manifest (`github:o/r`, `page:URL`, ...).
    pub update_source: String,
    /// `-f`: install even if verification fails (the plan and the result say so loudly).
    pub force: bool,
    /// Replace the installed copy without needing `-f`.
    pub upgrade: bool,
    /// A recipe worth saving once the install succeeds (resolved by the decision layer).
    pub learned: Option<Recipe>,
}

impl Job {
    /// The job for installing `a`, the newest asset of recipe `rec`. `recorded_team` is the Team ID the previous
    /// install was signed with: an upgrade must match it unless the recipe pins its own.
    pub fn from_asset(
        rec: &Recipe,
        a: Asset,
        force: bool,
        upgrade: bool,
        recorded_team: Option<String>,
    ) -> Job {
        let update_source = if let Some(g) = &rec.github {
            format!("github:{g}")
        } else if let Some(f) = &rec.appcast {
            format!("appcast:{f}")
        } else if let Some(p) = &rec.page {
            format!("page:{p}")
        } else {
            format!("url:{}", a.url)
        };
        Job {
            name: rec.name.clone(),
            version: a.version,
            url: a.url,
            sha256: a.sha256,
            expect_team: rec.team_id.clone().or(recorded_team),
            app_name: rec.app.clone(),
            resolved_by: "recipe".into(),
            update_source,
            force,
            upgrade,
            learned: None,
        }
    }

    /// Whether the download carries a signature kaika checks (apps and installer packages). Command-line tarballs have
    /// none: for those the published checksum is the only check, so there is no signer to compare.
    pub fn is_signed_kind(&self) -> bool {
        let n = file_name(&self.url).to_lowercase();
        !(n.ends_with(".tar.gz") || n.ends_with(".tgz"))
    }

    /// The plan line for this job: what will be downloaded from where, and how it will be trusted.
    pub fn describe(&self) -> String {
        let host = self
            .url
            .split("://")
            .nth(1)
            .and_then(|r| r.split('/').next())
            .unwrap_or("");
        let trust = match &self.expect_team {
            Some(t) => format!("expects Team ID {t}"),
            None => "Team ID shown and recorded on first use".into(),
        };
        let admin = if file_name(&self.url).to_lowercase().ends_with(".pkg") {
            "   (needs administrator rights)"
        } else {
            ""
        };
        let verb = if self.upgrade {
            "upgrade"
        } else {
            "download and install"
        };
        let what = if self.version.is_empty() {
            self.name.clone()
        } else {
            format!("{} v{}", self.name, self.version)
        };
        format!(
            "{verb} {what} from {host} ({}; verifies signature, notarization{}; {trust}){}{admin}",
            self.resolved_by,
            if self.sha256.is_some() {
                ", checksum"
            } else {
                ""
            },
            if self.force {
                "  [-f: verification failures will be overridden]"
            } else {
                ""
            },
        )
    }
}

/// What an install left behind, for the manifest.
pub struct Installed {
    /// Where it is now (empty for a `.pkg`, which the system installer places).
    pub path: String,
    /// The Team ID it was signed with, empty when it has none.
    pub team_id: String,
    /// Failures that `-f` overrode. Always shown to the user.
    pub overridden: Vec<String>,
}

/// Why an install did not happen.
#[derive(Debug)]
pub enum DErr {
    /// Verification failed and `-f` was not given: nothing was installed. One line per failed check.
    Verify(Vec<String>),
    Other(anyhow::Error),
}

impl From<anyhow::Error> for DErr {
    fn from(e: anyhow::Error) -> Self {
        DErr::Other(e)
    }
}

impl From<std::io::Error> for DErr {
    fn from(e: std::io::Error) -> Self {
        DErr::Other(e.into())
    }
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Run an external tool; its stderr is the error message when it fails.
fn run_ok(r: &dyn Runner, a: &[&str], what: &str) -> Result<()> {
    let o = r.run(&argv(a))?;
    if o.ok() {
        Ok(())
    } else {
        bail!("{what} failed: {}", o.stderr.trim())
    }
}

/// A package's name and version become folder names under `~/.kaika`, and both can come from a recipe file or a
/// vendor's release feed. Anything that is not one plain path component could climb out of that folder.
fn ensure_folder_name(what: &str, s: &str) -> Result<()> {
    if matches!(s, "" | "." | "..") || s.contains(['/', '\\', '\0']) {
        bail!("{what} '{s}' is not usable as a folder name");
    }
    Ok(())
}

/// Run the job: download, verify, install. Nothing is installed if a check fails (unless `force`).
pub fn run(r: &dyn Runner, dirs: &Dirs, job: &Job) -> Result<Installed, DErr> {
    ensure_https(&job.url)?;
    ensure_folder_name("the package name", &job.name)?;
    if !job.version.is_empty() {
        ensure_folder_name("the version", &job.version)?;
    }
    let dl = dirs.cache.join("downloads");
    std::fs::create_dir_all(&dl)?;
    let file = dl.join(file_name(&job.url));
    let o = r.run(&argv(&[
        "curl",
        "-fSL",
        "--proto-redir",
        "=https",
        "--max-time",
        "900",
        "-o",
        &path_str(&file),
        &job.url,
    ]))?;
    if !o.ok() {
        return Err(anyhow!("download failed: {}", o.stderr.trim()).into());
    }
    let mut failures = vec![];
    if let Some(f) = verify::check_checksum(&file, job.sha256.as_deref())? {
        failures.push(f);
    }
    let stage = dirs
        .cache
        .join("stage")
        .join(format!("{}-{}", job.name, std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let result = install_downloaded(r, dirs, job, &file, &stage, failures);
    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// Stop with the failures unless `force` says to go on.
fn gate(failures: &[String], force: bool) -> Result<(), DErr> {
    if !failures.is_empty() && !force {
        return Err(DErr::Verify(failures.to_vec()));
    }
    Ok(())
}

/// Install a downloaded file by its format. `failures` holds what verification has found so far.
fn install_downloaded(
    r: &dyn Runner,
    dirs: &Dirs,
    job: &Job,
    file: &Path,
    stage: &Path,
    failures: Vec<String>,
) -> Result<Installed, DErr> {
    let lower = file
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let expect = Expect {
        team_id: job.expect_team.clone(),
    };
    if lower.ends_with(".pkg") {
        install_pkg(r, job, file, &expect, failures)
    } else if lower.ends_with(".gguf") {
        install_model(dirs, job, file, failures)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        package::install_tarball(r, dirs, job, file, stage, failures)
    } else {
        app::install_app(r, dirs, job, file, stage, &expect, failures)
    }
}

/// A signed installer: verified, then the system installer runs under `sudo` (the plan said so up front).
fn install_pkg(
    r: &dyn Runner,
    job: &Job,
    file: &Path,
    expect: &Expect,
    mut failures: Vec<String>,
) -> Result<Installed, DErr> {
    // What is checked must be what is installed: the file is fingerprinted before the checks and again right before
    // the installer runs as root. A change in between is never a failure `-f` may override.
    let checked = verify::sha256_file(file)?;
    let rep = verify::check_pkg(r, file, expect);
    failures.extend(rep.failures);
    gate(&failures, job.force)?;
    if verify::sha256_file(file)? != checked {
        return Err(DErr::Verify(vec![
            "the installer package changed after it was checked; nothing was installed".into(),
        ]));
    }
    let code = r.run_live(&argv(&[
        "sudo",
        "installer",
        "-pkg",
        &path_str(file),
        "-target",
        "/",
    ]))?;
    if code != 0 {
        return Err(anyhow!("installer exited with {code}").into());
    }
    Ok(Installed {
        path: String::new(),
        team_id: rep.team_id.unwrap_or_default(),
        overridden: failures,
    })
}

/// A model file is a large opaque download that nothing can inspect, so a published checksum is the only check that
/// means anything.
fn install_model(
    dirs: &Dirs,
    job: &Job,
    file: &Path,
    mut failures: Vec<String>,
) -> Result<Installed, DErr> {
    if job.sha256.is_none() {
        failures.push("model download has no published checksum to verify".into());
    }
    gate(&failures, job.force)?;
    let dest_dir = dirs.root.join("models").join(&job.name);
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(
        file.file_name()
            .ok_or_else(|| anyhow!("the download has no name"))?,
    );
    if std::fs::rename(file, &dest).is_err() {
        std::fs::copy(file, &dest)?; // moving avoids keeping a second multi-gigabyte copy in the cache
    }
    Ok(Installed {
        path: path_str(&dest),
        team_id: String::new(),
        overridden: failures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_may_not_be_redirected_off_https() {
        let t = tempfile::tempdir().unwrap();
        let dirs = Dirs::from_lookup(|k| {
            (k == "KAIKA_HOME")
                .then(|| t.path().display().to_string())
                .or(Some("/h".into()))
        });
        let rec = Recipe {
            name: "x".into(),
            url: Some("https://x.example/a.dmg".into()),
            ..Default::default()
        };
        let a = Asset {
            name: "a.dmg".into(),
            url: "https://x.example/a.dmg".into(),
            version: String::new(),
            sha256: None,
        };
        let r = crate::runner::Fake::default();
        let _ = run(&r, &dirs, &Job::from_asset(&rec, a, false, false, None));
        let log = r.log.borrow();
        assert!(
            log[0].starts_with("curl -fSL") && log[0].contains("--proto-redir =https"),
            "{log:?}"
        );
    }

    #[test]
    fn describe_states_the_trust_model() {
        let rec = Recipe {
            name: "x".into(),
            github: Some("o/r".into()),
            ..Default::default()
        };
        let a = Asset {
            name: "X.dmg".into(),
            url: "https://github.com/o/r/X.dmg".into(),
            version: "1.0".into(),
            sha256: None,
        };
        let j = Job::from_asset(&rec, a, true, false, None);
        let d = j.describe();
        assert!(
            d.contains("github.com") && d.contains("recorded on first use") && d.contains("-f")
        );
        assert_eq!(j.update_source, "github:o/r");
    }

    #[test]
    fn recorded_team_id_becomes_the_expectation_on_upgrade() {
        let rec = Recipe {
            name: "x".into(),
            url: Some("https://x/a.dmg".into()),
            ..Default::default()
        };
        let a = Asset {
            name: "a.dmg".into(),
            url: "https://x/a.dmg".into(),
            version: String::new(),
            sha256: None,
        };
        assert_eq!(
            Job::from_asset(&rec, a, false, true, Some("TEAM123456".into()))
                .expect_team
                .as_deref(),
            Some("TEAM123456")
        );
    }
}
