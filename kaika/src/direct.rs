//! The `direct` backend: download from a vendor, verify, install, and remove.

use crate::manifest::Entry;
use crate::paths::Dirs;
use crate::recipe::{self, Asset, Recipe};
use crate::runner::{argv, Runner};
use crate::verify::{self, Expect};
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

/// Test hook: lets `file://` fixtures stand in for HTTPS so tests never touch the network.
pub fn test_mode() -> bool {
    std::env::var("KAIKA_TEST_MODE").as_deref() == Ok("1")
}

pub fn ensure_https(url: &str) -> Result<()> {
    if url.starts_with("https://") || (test_mode() && url.starts_with("file://")) {
        Ok(())
    } else {
        bail!("refusing non-HTTPS URL: {url}")
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub name: String,
    pub version: String,
    pub url: String,
    pub sha256: Option<String>,
    pub expect_team: Option<String>,
    pub app_name: Option<String>,
    pub resolved_by: String,
    pub update_source: String,
    pub force: bool,
    pub upgrade: bool,
    /// A recipe worth saving once the install succeeds (resolved by the decision layer).
    pub learned: Option<Recipe>,
}

impl Job {
    pub fn from_asset(rec: &Recipe, a: Asset, force: bool, upgrade: bool, recorded_team: Option<String>) -> Job {
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

    pub fn describe(&self) -> String {
        let host = self.url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("");
        let trust = match &self.expect_team {
            Some(t) => format!("expects Team ID {t}"),
            None => "Team ID shown and recorded on first use".into(),
        };
        let admin = if recipe::file_name(&self.url).to_lowercase().ends_with(".pkg") { "   (needs administrator rights)" } else { "" };
        format!(
            "{} {} {} from {host} ({}; verifies signature, notarization{}; {trust}){}{admin}",
            if self.upgrade { "upgrade" } else { "download and install" },
            self.name,
            if self.version.is_empty() { String::new() } else { format!("v{}", self.version) },
            self.resolved_by,
            if self.sha256.is_some() { ", checksum" } else { "" },
            if self.force { "  [-f: verification failures will be overridden]" } else { "" },
        )
    }
}

pub struct Installed {
    pub path: String,
    pub team_id: String,
    /// Failures that `-f` overrode. Always shown to the user.
    pub overridden: Vec<String>,
}

#[derive(Debug)]
pub enum DErr {
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

fn find_app(dir: &Path, wanted: Option<&str>) -> Option<PathBuf> {
    let mut found = vec![];
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "app") {
                found.push(p);
            } else if p.is_dir() && depth < 1 {
                stack.push((p, depth + 1));
            }
        }
    }
    found.sort();
    match wanted {
        Some(w) => found.into_iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(w))),
        None => found.into_iter().next(),
    }
}

fn run_ok(r: &dyn Runner, a: &[&str], what: &str) -> Result<(), DErr> {
    let o = r.run(&argv(a))?;
    if o.ok() {
        Ok(())
    } else {
        Err(anyhow!("{what} failed: {}", o.stderr.trim()).into())
    }
}

/// Run the job: download, verify, install. Nothing is installed if a check fails (unless `force`).
pub fn run(r: &dyn Runner, dirs: &Dirs, job: &Job) -> Result<Installed, DErr> {
    ensure_https(&job.url)?;
    let dl = dirs.cache.join("downloads");
    std::fs::create_dir_all(&dl)?;
    let file = dl.join(recipe::file_name(&job.url));
    let o = r.run(&argv(&["curl", "-fSL", "--max-time", "900", "-o", &path_str(&file), &job.url]))?;
    if !o.ok() {
        return Err(anyhow!("download failed: {}", o.stderr.trim()).into());
    }
    let mut failures = vec![];
    if let Some(f) = verify::check_checksum(&file, job.sha256.as_deref())? {
        failures.push(f);
    }
    let stage = dirs.cache.join("stage").join(format!("{}-{}", job.name, std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let result = install_downloaded(r, dirs, job, &file, &stage, failures);
    let _ = std::fs::remove_dir_all(&stage);
    result
}

fn gate(failures: &[String], force: bool) -> Result<(), DErr> {
    if !failures.is_empty() && !force {
        return Err(DErr::Verify(failures.to_vec()));
    }
    Ok(())
}

fn install_downloaded(r: &dyn Runner, dirs: &Dirs, job: &Job, file: &Path, stage: &Path, mut failures: Vec<String>) -> Result<Installed, DErr> {
    let lower = file.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let expect = Expect { team_id: job.expect_team.clone(), sha256: job.sha256.clone() };

    if lower.ends_with(".pkg") {
        let rep = verify::check_pkg(r, file, &expect);
        failures.extend(rep.failures.clone());
        gate(&failures, job.force)?;
        let code = r.run_live(&argv(&["sudo", "installer", "-pkg", &path_str(file), "-target", "/"]))?;
        if code != 0 {
            return Err(anyhow!("installer exited with {code}").into());
        }
        return Ok(Installed { path: String::new(), team_id: rep.team_id.unwrap_or_default(), overridden: failures });
    }

    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        // Command-line binaries have no notarization story; a checksum is the only check that means anything.
        if job.sha256.is_none() {
            failures.push("command-line download has no published checksum to verify".into());
        }
        gate(&failures, job.force)?;
        run_ok(r, &["tar", "-xzf", &path_str(file), "-C", &path_str(stage)], "extracting archive")?;
        let bin = first_executable(stage).ok_or_else(|| anyhow!("no executable found in the archive"))?;
        std::fs::create_dir_all(&dirs.bin)?;
        let dest = dirs.bin.join(bin.file_name().unwrap());
        std::fs::copy(&bin, &dest)?;
        return Ok(Installed { path: path_str(&dest), team_id: String::new(), overridden: failures });
    }

    if lower.ends_with(".zip") {
        run_ok(r, &["ditto", "-xk", &path_str(file), &path_str(stage)], "extracting archive")?;
    } else if lower.ends_with(".dmg") {
        let mnt = dirs.cache.join("mnt").join(format!("{}-{}", job.name, std::process::id()));
        std::fs::create_dir_all(&mnt)?;
        let attach = r.run(&argv(&["hdiutil", "attach", "-nobrowse", "-noverify", "-readonly", "-quiet", "-mountpoint", &path_str(&mnt), &path_str(file)]))?;
        let copied = if attach.ok() {
            match find_app(&mnt, job.app_name.as_deref()) {
                Some(app) => run_ok(r, &["ditto", &path_str(&app), &path_str(&stage.join(app.file_name().unwrap()))], "copying app out of the disk image"),
                None => Err(anyhow!("no app found in the disk image").into()),
            }
        } else {
            Err(anyhow!("could not open the disk image: {}", attach.stderr.trim()).into())
        };
        if attach.ok() {
            let _ = r.run(&argv(&["hdiutil", "detach", "-quiet", &path_str(&mnt)]));
        }
        let _ = std::fs::remove_dir_all(&mnt);
        copied?;
    } else {
        return Err(anyhow!("unsupported download format: {lower}").into());
    }

    let app = find_app(stage, job.app_name.as_deref()).ok_or_else(|| anyhow!("no app found in the download"))?;
    let rep = verify::check_app(r, &app, &expect);
    failures.extend(rep.failures.clone());
    gate(&failures, job.force)?;

    let dest = dirs.apps.join(app.file_name().unwrap());
    if dest.exists() {
        if job.upgrade || job.force {
            std::fs::remove_dir_all(&dest)?;
        } else {
            return Err(anyhow!("{} already exists; use -f to replace it", dest.display()).into());
        }
    }
    std::fs::create_dir_all(&dirs.apps)?;
    run_ok(r, &["ditto", &path_str(&app), &path_str(&dest)], "installing the app")?;
    Ok(Installed { path: path_str(&dest), team_id: rep.team_id.unwrap_or_default(), overridden: failures })
}

fn first_executable(dir: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                stack.push(p);
            } else if std::fs::metadata(&p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false) {
                return Some(p);
            }
        }
    }
    None
}

/// Remove a direct install: the app goes to the Trash; `zap` also removes its support files.
pub fn uninstall(r: &dyn Runner, dirs: &Dirs, e: &Entry, zap: bool) -> Result<Vec<String>> {
    let mut gone = vec![];
    if e.path.is_empty() {
        bail!("{} was installed from a package file; remove it with its own uninstaller", e.name);
    }
    let app = PathBuf::from(&e.path);
    if !app.exists() {
        bail!("{} not found (already removed?)", app.display());
    }
    let bundle_id = if zap && app.extension().is_some_and(|x| x == "app") {
        let info = app.join("Contents/Info");
        r.run(&argv(&["defaults", "read", &path_str(&info), "CFBundleIdentifier"])).ok().filter(|o| o.ok()).map(|o| o.stdout.trim().to_string())
    } else {
        None
    };
    if app.is_dir() {
        let trash = dirs.home.join(".Trash");
        std::fs::create_dir_all(&trash)?;
        let mut dest = trash.join(app.file_name().unwrap());
        if dest.exists() {
            dest = trash.join(format!("{}-{}", app.file_name().unwrap().to_string_lossy(), crate::manifest::now()));
        }
        if std::fs::rename(&app, &dest).is_err() {
            run_ok_anyhow(r, &["ditto", &path_str(&app), &path_str(&dest)])?;
            std::fs::remove_dir_all(&app)?;
        }
        gone.push(path_str(&app));
    } else {
        std::fs::remove_file(&app)?;
        gone.push(path_str(&app));
    }
    if zap {
        let lib = dirs.home.join("Library");
        let stem = app.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut paths = vec![lib.join("Application Support").join(&stem)];
        if let Some(b) = bundle_id.filter(|b| !b.is_empty()) {
            paths.push(lib.join("Caches").join(&b));
            paths.push(lib.join("Preferences").join(format!("{b}.plist")));
            paths.push(lib.join("Saved Application State").join(format!("{b}.savedState")));
        }
        for p in paths {
            let removed = if p.is_dir() { std::fs::remove_dir_all(&p).is_ok() } else { std::fs::remove_file(&p).is_ok() };
            if removed {
                gone.push(path_str(&p));
            }
        }
    }
    Ok(gone)
}

fn run_ok_anyhow(r: &dyn Runner, a: &[&str]) -> Result<()> {
    let o = r.run(&argv(a))?;
    if o.ok() { Ok(()) } else { bail!("{} failed: {}", a[0], o.stderr.trim()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_only_outside_test_mode() {
        assert!(ensure_https("https://x.com/a.dmg").is_ok());
        assert!(ensure_https("http://x.com/a.dmg").is_err());
        assert!(ensure_https("ftp://x.com/a.dmg").is_err());
        // file:// is refused unless KAIKA_TEST_MODE=1, which unit tests do not set.
        assert!(ensure_https("file:///tmp/a.zip").is_err());
    }

    #[test]
    fn describe_states_the_trust_model() {
        let rec = Recipe { name: "x".into(), github: Some("o/r".into()), ..Default::default() };
        let a = Asset { name: "X.dmg".into(), url: "https://github.com/o/r/X.dmg".into(), version: "1.0".into(), sha256: None };
        let j = Job::from_asset(&rec, a, true, false, None);
        let d = j.describe();
        assert!(d.contains("github.com") && d.contains("recorded on first use") && d.contains("-f"));
        assert_eq!(j.update_source, "github:o/r");
    }

    #[test]
    fn recorded_team_id_becomes_the_expectation_on_upgrade() {
        let rec = Recipe { name: "x".into(), url: Some("https://x/a.dmg".into()), ..Default::default() };
        let a = Asset { name: "a.dmg".into(), url: "https://x/a.dmg".into(), version: String::new(), sha256: None };
        assert_eq!(Job::from_asset(&rec, a, false, true, Some("TEAM123456".into())).expect_team.as_deref(), Some("TEAM123456"));
    }
}
