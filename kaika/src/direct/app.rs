//! Apps from a `.zip` or `.dmg`: extract, verify the app, then put it in the applications folder.

use super::{DErr, Installed, Job, gate, path_str, run_ok};
use crate::paths::Dirs;
use crate::runner::{Runner, argv};
use crate::verify::{self, Expect};
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

/// Install the app inside a `.zip` or `.dmg` download. The app is copied into `stage` first, so what is verified
/// is exactly what gets installed.
pub(super) fn install_app(
    r: &dyn Runner,
    dirs: &Dirs,
    job: &Job,
    file: &Path,
    stage: &Path,
    expect: &Expect,
    mut failures: Vec<String>,
) -> Result<Installed, DErr> {
    let lower = file
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if lower.ends_with(".zip") {
        run_ok(
            r,
            &["ditto", "-xk", &path_str(file), &path_str(stage)],
            "extracting archive",
        )?;
    } else if lower.ends_with(".dmg") {
        copy_app_out_of_image(r, dirs, job, file, stage)?;
    } else {
        return Err(anyhow!("unsupported download format: {lower}").into());
    }

    let app = find_app(stage, job.app_name.as_deref())
        .ok_or_else(|| anyhow!("no app found in the download"))?;
    let rep = verify::check_app(r, &app, expect);
    failures.extend(rep.failures.clone());
    gate(&failures, job.force)?;

    let name = app
        .file_name()
        .ok_or_else(|| anyhow!("the app has no name"))?;
    let dest = dirs.apps.join(name);
    if dest.exists() && !(job.upgrade || job.force) {
        return Err(anyhow!("{} already exists; use -f to replace it", dest.display()).into());
    }
    std::fs::create_dir_all(&dirs.apps)?;
    place_app(r, &app, &dest)?;
    Ok(Installed {
        path: path_str(&dest),
        team_id: rep.team_id.unwrap_or_default(),
        overridden: failures,
    })
}

/// Mount the image read-only, copy the app into `stage`, and always detach again.
fn copy_app_out_of_image(
    r: &dyn Runner,
    dirs: &Dirs,
    job: &Job,
    file: &Path,
    stage: &Path,
) -> Result<(), DErr> {
    let mnt = dirs
        .cache
        .join("mnt")
        .join(format!("{}-{}", job.name, std::process::id()));
    std::fs::create_dir_all(&mnt)?;
    let attach = r.run(&argv(&[
        "hdiutil",
        "attach",
        "-nobrowse",
        "-noverify",
        "-readonly",
        "-quiet",
        "-mountpoint",
        &path_str(&mnt),
        &path_str(file),
    ]))?;
    let copied = if attach.ok() {
        match find_app(&mnt, job.app_name.as_deref()) {
            Some(app) => copy_into(r, &app, stage),
            None => Err(anyhow!("no app found in the disk image")),
        }
    } else {
        Err(anyhow!(
            "could not open the disk image: {}",
            attach.stderr.trim()
        ))
    };
    if attach.ok() {
        let _ = r.run(&argv(&["hdiutil", "detach", "-quiet", &path_str(&mnt)]));
    }
    let _ = std::fs::remove_dir_all(&mnt);
    Ok(copied?)
}

fn copy_into(r: &dyn Runner, app: &Path, stage: &Path) -> Result<()> {
    let name = app
        .file_name()
        .ok_or_else(|| anyhow!("the app has no name"))?;
    run_ok(
        r,
        &["ditto", &path_str(app), &path_str(&stage.join(name))],
        "copying app out of the disk image",
    )
}

/// The `.app` bundle in `dir` (or one level down): the one named `wanted`, else the first by name. Symlinks are
/// never followed: a disk image's "Applications" shortcut leads to the user's own apps.
pub(super) fn find_app(dir: &Path, wanted: Option<&str>) -> Option<PathBuf> {
    let mut found = vec![];
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if e.file_type().map(|t| t.is_symlink()).unwrap_or(true) {
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
        Some(w) => found.into_iter().find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(w))
        }),
        None => found.into_iter().next(),
    }
}

/// Put `app` at `dest`, replacing what is there only once the new copy is complete: it is copied next to `dest`
/// first and swapped in with two renames, and the old one is put back if the swap fails. A full disk or a
/// permission error therefore never costs the user the app they already had.
fn place_app(r: &dyn Runner, app: &Path, dest: &Path) -> Result<()> {
    let (dir, name) = (
        dest.parent().unwrap_or(Path::new("/")),
        dest.file_name().unwrap_or_default().to_string_lossy(),
    );
    let (incoming, backup) = (
        dir.join(format!(".{name}.kaika-new")),
        dir.join(format!(".{name}.kaika-old")),
    );
    let _ = std::fs::remove_dir_all(&incoming);
    let _ = std::fs::remove_dir_all(&backup);
    if let Err(e) = run_ok(
        r,
        &["ditto", &path_str(app), &path_str(&incoming)],
        "installing the app",
    ) {
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(e);
    }
    let had_old = dest.exists();
    let swapped = (!had_old || std::fs::rename(dest, &backup).is_ok())
        && std::fs::rename(&incoming, dest).is_ok();
    if !swapped {
        if had_old && !dest.exists() {
            let _ = std::fs::rename(&backup, dest);
        }
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(anyhow!(
            "could not replace {}; the installed copy was left as it was",
            dest.display()
        ));
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_symlink_to_the_applications_folder_is_never_searched() {
        let t = tempfile::tempdir().unwrap();
        let (disk, apps) = (t.path().join("disk"), t.path().join("Applications"));
        std::fs::create_dir_all(apps.join("Aardvark.app")).unwrap();
        std::fs::create_dir_all(disk.join("Obsidian.app")).unwrap();
        std::os::unix::fs::symlink(&apps, disk.join("Applications")).unwrap();
        std::os::unix::fs::symlink(apps.join("Aardvark.app"), disk.join("Shortcut.app")).unwrap();
        let found = find_app(&disk, None).unwrap();
        assert_eq!(
            found.file_name().unwrap(),
            "Obsidian.app",
            "found {found:?}"
        );
        assert!(find_app(&disk, Some("Aardvark.app")).is_none());
    }
}
