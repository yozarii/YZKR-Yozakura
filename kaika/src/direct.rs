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
        let verb = if self.upgrade { "upgrade" } else { "download and install" };
        let what = if self.version.is_empty() { self.name.clone() } else { format!("{} v{}", self.name, self.version) };
        format!(
            "{verb} {what} from {host} ({}; verifies signature, notarization{}; {trust}){}{admin}",
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
            // Never follow symlinks: a disk image's "Applications" shortcut leads to the user's own apps.
            let is_link = e.file_type().map(|t| t.is_symlink()).unwrap_or(true);
            if is_link {
                continue;
            }
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
    let file = dl.join(recipe::file_name(&job.url));
    let o = r.run(&argv(&["curl", "-fSL", "--proto-redir", "=https", "--max-time", "900", "-o", &path_str(&file), &job.url]))?;
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

    if lower.ends_with(".gguf") {
        // A model file is a large opaque download that nothing can inspect, so a published checksum is the only
        // check that means anything.
        if job.sha256.is_none() {
            failures.push("model download has no published checksum to verify".into());
        }
        gate(&failures, job.force)?;
        let dest_dir = dirs.root.join("models").join(&job.name);
        std::fs::create_dir_all(&dest_dir)?;
        let dest = dest_dir.join(file.file_name().unwrap());
        if std::fs::rename(file, &dest).is_err() {
            std::fs::copy(file, &dest)?; // moving avoids keeping a second multi-gigabyte copy in the cache
        }
        return Ok(Installed { path: path_str(&dest), team_id: String::new(), overridden: failures });
    }

    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        // Command-line packages have no notarization story; a checksum is the only check that means anything.
        if job.sha256.is_none() {
            failures.push("command-line download has no published checksum to verify".into());
        }
        gate(&failures, job.force)?;
        run_ok(r, &["tar", "-xzf", &path_str(file), "-C", &path_str(stage)], "extracting archive")?;
        let dest = install_package(dirs, job, stage)?;
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
    if dest.exists() && !(job.upgrade || job.force) {
        return Err(anyhow!("{} already exists; use -f to replace it", dest.display()).into());
    }
    std::fs::create_dir_all(&dirs.apps)?;
    place_app(r, &app, &dest)?;
    Ok(Installed { path: path_str(&dest), team_id: rep.team_id.unwrap_or_default(), overridden: failures })
}

/// Put `app` at `dest`, replacing what is there only once the new copy is complete: it is copied next to `dest`
/// first and swapped in with two renames, and the old one is put back if the swap fails. A full disk or a
/// permission error therefore never costs the user the app they already had.
fn place_app(r: &dyn Runner, app: &Path, dest: &Path) -> Result<(), DErr> {
    let (dir, name) = (dest.parent().unwrap_or(Path::new("/")), dest.file_name().unwrap_or_default().to_string_lossy());
    let (incoming, backup) = (dir.join(format!(".{name}.kaika-new")), dir.join(format!(".{name}.kaika-old")));
    let _ = std::fs::remove_dir_all(&incoming);
    let _ = std::fs::remove_dir_all(&backup);
    if let Err(e) = run_ok(r, &["ditto", &path_str(app), &path_str(&incoming)], "installing the app") {
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(e);
    }
    let had_old = dest.exists();
    let swapped = (!had_old || std::fs::rename(dest, &backup).is_ok()) && std::fs::rename(&incoming, dest).is_ok();
    if !swapped {
        if had_old && !dest.exists() {
            let _ = std::fs::rename(&backup, dest);
        }
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(anyhow!("could not replace {}; the installed copy was left as it was", dest.display()).into());
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// Executables a package offers: those in its `bin/`, or else the ones at its top level.
fn package_executables(pkg: &Path) -> Vec<PathBuf> {
    let list = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d).into_iter().flatten().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| is_executable(p)).collect();
        v.sort();
        v
    };
    let in_bin = list(&pkg.join("bin"));
    if in_bin.is_empty() { list(pkg) } else { in_bin }
}

/// Unpack a command-line package into `packages/<name>/<version>` and link its executables into `bin/`.
fn install_package(dirs: &Dirs, job: &Job, stage: &Path) -> Result<PathBuf, DErr> {
    // Archives usually wrap everything in one folder; the package is that folder.
    let entries: Vec<PathBuf> = std::fs::read_dir(stage)?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    let content = if entries.len() == 1 && entries[0].is_dir() { entries[0].clone() } else { stage.to_path_buf() };
    let version = if job.version.is_empty() { "latest" } else { job.version.as_str() };
    let pkg_dir = dirs.packages.join(&job.name);
    let dest = pkg_dir.join(version);
    let exes = package_executables(&content);
    if exes.is_empty() {
        return Err(anyhow!("no executable found in the package").into());
    }
    // Refuse to replace a file in bin/ that kaika did not put there, unless forced.
    std::fs::create_dir_all(&dirs.bin)?;
    for exe in &exes {
        let link = dirs.bin.join(exe.file_name().unwrap());
        if std::fs::symlink_metadata(&link).is_ok() && !is_ours(&link, &dirs.packages) && !job.force {
            return Err(anyhow!("{} already exists and is not a kaika link; use -f to replace it", link.display()).into());
        }
    }
    std::fs::create_dir_all(&pkg_dir)?;
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    if std::fs::rename(&content, &dest).is_err() {
        run_ok_anyhow(&crate::runner::RealRunner, &["ditto", &path_str(&content), &path_str(&dest)])?;
    }
    for exe in package_executables(&dest) {
        let link = dirs.bin.join(exe.file_name().unwrap());
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&exe, &link)?;
    }
    // An upgrade replaces the old version: drop every other version of this package.
    for e in std::fs::read_dir(&pkg_dir)?.filter_map(|e| e.ok()) {
        if e.path() != dest {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    Ok(dest)
}

/// Is this a symlink pointing into kaika's packages folder?
fn is_ours(link: &Path, packages: &Path) -> bool {
    std::fs::read_link(link).map(|t| t.starts_with(packages)).unwrap_or(false)
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
    if app.starts_with(dirs.root.join("models")) {
        std::fs::remove_file(&app)?;
        if let Some(parent) = app.parent() {
            let _ = std::fs::remove_dir(parent); // the model's folder, once empty
        }
        gone.push(path_str(&app));
        return Ok(gone);
    }
    if app.starts_with(&dirs.packages) {
        // A command-line package: remove the links that point into it, then the package itself.
        for l in std::fs::read_dir(&dirs.bin).into_iter().flatten().filter_map(|x| x.ok()).map(|x| x.path()) {
            if std::fs::read_link(&l).map(|t| t.starts_with(&app)).unwrap_or(false) {
                std::fs::remove_file(&l)?;
                gone.push(path_str(&l));
            }
        }
        std::fs::remove_dir_all(&app)?;
        if let Some(parent) = app.parent() {
            let _ = std::fs::remove_dir(parent); // the name folder, once empty
        }
        gone.push(path_str(&app));
        return Ok(gone);
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
    #[test]
    fn a_symlink_to_the_applications_folder_is_never_searched() {
        let t = tempfile::tempdir().unwrap();
        let (disk, apps) = (t.path().join("disk"), t.path().join("Applications"));
        std::fs::create_dir_all(apps.join("Aardvark.app")).unwrap();
        std::fs::create_dir_all(disk.join("Obsidian.app")).unwrap();
        std::os::unix::fs::symlink(&apps, disk.join("Applications")).unwrap();
        std::os::unix::fs::symlink(apps.join("Aardvark.app"), disk.join("Shortcut.app")).unwrap();
        let found = find_app(&disk, None).unwrap();
        assert_eq!(found.file_name().unwrap(), "Obsidian.app", "found {found:?}");
        assert!(find_app(&disk, Some("Aardvark.app")).is_none());
    }

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
    fn a_download_may_not_be_redirected_off_https() {
        let t = tempfile::tempdir().unwrap();
        let dirs = Dirs::from_lookup(|k| (k == "KAIKA_HOME").then(|| t.path().display().to_string()).or(Some("/h".into())));
        let rec = Recipe { name: "x".into(), url: Some("https://x.example/a.dmg".into()), ..Default::default() };
        let a = Asset { name: "a.dmg".into(), url: "https://x.example/a.dmg".into(), version: String::new(), sha256: None };
        let r = crate::runner::Fake::default();
        let _ = run(&r, &dirs, &Job::from_asset(&rec, a, false, false, None));
        let log = r.log.borrow();
        assert!(log[0].starts_with("curl -fSL") && log[0].contains("--proto-redir =https"), "{log:?}");
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
