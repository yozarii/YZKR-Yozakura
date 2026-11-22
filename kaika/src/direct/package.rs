//! Command-line packages from a `.tar.gz`: unpacked whole into `~/.kaika/packages/<name>/<version>`, their
//! executables linked into `~/.kaika/bin`. An upgrade replaces the old version.

use super::{DErr, Installed, Job, gate, path_str, run_ok};
use crate::paths::Dirs;
use crate::runner::Runner;
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

/// Unpack and install a command-line package. These have no notarization story, so a published checksum is the only
/// check that means anything: without one the install needs `-f`.
pub(super) fn install_tarball(
    r: &dyn Runner,
    dirs: &Dirs,
    job: &Job,
    file: &Path,
    stage: &Path,
    mut failures: Vec<String>,
) -> Result<Installed, DErr> {
    if job.sha256.is_none() {
        failures.push("command-line download has no published checksum to verify".into());
    }
    gate(&failures, job.force)?;
    run_ok(
        r,
        &["tar", "-xzf", &path_str(file), "-C", &path_str(stage)],
        "extracting archive",
    )?;
    let dest = install_package(r, dirs, job, stage)?;
    Ok(Installed {
        path: path_str(&dest),
        team_id: String::new(),
        overridden: failures,
    })
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Executables a package offers: those in its `bin/`, or else the ones at its top level.
fn package_executables(pkg: &Path) -> Vec<PathBuf> {
    let list = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| is_executable(p))
            .collect();
        v.sort();
        v
    };
    let in_bin = list(&pkg.join("bin"));
    if in_bin.is_empty() { list(pkg) } else { in_bin }
}

/// The link name for an executable: its file name.
fn link_in(bin: &Path, exe: &Path) -> Result<PathBuf> {
    let name = exe
        .file_name()
        .ok_or_else(|| anyhow!("{} has no file name", exe.display()))?;
    Ok(bin.join(name))
}

/// Unpack a command-line package into `packages/<name>/<version>` and link its executables into `bin/`.
fn install_package(r: &dyn Runner, dirs: &Dirs, job: &Job, stage: &Path) -> Result<PathBuf, DErr> {
    // Archives usually wrap everything in one folder; the package is that folder.
    let entries: Vec<PathBuf> = std::fs::read_dir(stage)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    let content = if entries.len() == 1 && entries[0].is_dir() {
        entries[0].clone()
    } else {
        stage.to_path_buf()
    };
    let version = if job.version.is_empty() {
        "latest"
    } else {
        job.version.as_str()
    };
    let pkg_dir = dirs.packages.join(&job.name);
    let dest = pkg_dir.join(version);
    let exes = package_executables(&content);
    if exes.is_empty() {
        return Err(anyhow!("no executable found in the package").into());
    }
    // Refuse to replace a file in bin/ that kaika did not put there, unless forced.
    std::fs::create_dir_all(&dirs.bin)?;
    for exe in &exes {
        let link = link_in(&dirs.bin, exe)?;
        if std::fs::symlink_metadata(&link).is_ok() && !is_ours(&link, &dirs.packages) && !job.force
        {
            return Err(anyhow!(
                "{} already exists and is not a kaika link; use -f to replace it",
                link.display()
            )
            .into());
        }
    }
    std::fs::create_dir_all(&pkg_dir)?;
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    if std::fs::rename(&content, &dest).is_err() {
        run_ok(
            r,
            &["ditto", &path_str(&content), &path_str(&dest)],
            "ditto",
        )?;
    }
    for exe in package_executables(&dest) {
        let link = link_in(&dirs.bin, &exe)?;
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
    std::fs::read_link(link)
        .map(|t| t.starts_with(packages))
        .unwrap_or(false)
}
