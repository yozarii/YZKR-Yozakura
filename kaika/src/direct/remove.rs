//! Removing a direct install: an app goes to the Trash, a command-line package and a model are deleted.

use super::{path_str, run_ok};
use crate::manifest::{Entry, now};
use crate::paths::Dirs;
use crate::runner::{Runner, argv};
use anyhow::{Result, bail};
use std::path::{Component, Path, PathBuf};

/// Is `path` strictly inside `dir`, with no `..` in the way? The manifest names what to delete; a damaged or edited
/// entry must not be able to point deletion at `dir` itself or outside it.
fn is_inside(path: &Path, dir: &Path) -> bool {
    path.strip_prefix(dir)
        .is_ok_and(|rest| rest.components().next().is_some())
        && !path.components().any(|c| c == Component::ParentDir)
}

/// Remove a direct install; `zap` also removes an app's support files. Returns what was removed.
pub fn uninstall(r: &dyn Runner, dirs: &Dirs, e: &Entry, zap: bool) -> Result<Vec<String>> {
    if e.path.is_empty() {
        bail!(
            "{} was installed from a package file; remove it with its own uninstaller",
            e.name
        );
    }
    let path = PathBuf::from(&e.path);
    if !path.exists() {
        bail!("{} not found (already removed?)", path.display());
    }
    if is_inside(&path, &dirs.root.join("models")) {
        return remove_model(&path);
    }
    if is_inside(&path, &dirs.packages) {
        return remove_package(dirs, &path);
    }
    let bundle_id = if zap {
        bundle_identifier(r, &path)
    } else {
        None
    };
    let mut gone = vec![];
    move_to_trash(r, dirs, &path)?;
    gone.push(path_str(&path));
    if zap {
        gone.extend(remove_support_files(dirs, &path, bundle_id));
    }
    Ok(gone)
}

fn remove_model(path: &Path) -> Result<Vec<String>> {
    std::fs::remove_file(path)?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent); // the model's folder, once empty
    }
    Ok(vec![path_str(path)])
}

/// A command-line package: remove the links that point into it, then the package itself.
fn remove_package(dirs: &Dirs, path: &Path) -> Result<Vec<String>> {
    let mut gone = vec![];
    for l in std::fs::read_dir(&dirs.bin)
        .into_iter()
        .flatten()
        .filter_map(|x| x.ok())
        .map(|x| x.path())
    {
        if std::fs::read_link(&l)
            .map(|t| t.starts_with(path))
            .unwrap_or(false)
        {
            std::fs::remove_file(&l)?;
            gone.push(path_str(&l));
        }
    }
    std::fs::remove_dir_all(path)?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent); // the name folder, once empty
    }
    gone.push(path_str(path));
    Ok(gone)
}

/// The app's `CFBundleIdentifier`, which names its caches and preferences.
fn bundle_identifier(r: &dyn Runner, app: &Path) -> Option<String> {
    if !app.extension().is_some_and(|x| x == "app") {
        return None;
    }
    let info = app.join("Contents/Info");
    r.run(&argv(&[
        "defaults",
        "read",
        &path_str(&info),
        "CFBundleIdentifier",
    ]))
    .ok()
    .filter(|o| o.ok())
    .map(|o| o.stdout.trim().to_string())
}

/// Move an app (or delete a plain file) out of the applications folder; a copy-and-delete when a rename cannot cross
/// volumes.
fn move_to_trash(r: &dyn Runner, dirs: &Dirs, app: &Path) -> Result<()> {
    if !app.is_dir() {
        std::fs::remove_file(app)?;
        return Ok(());
    }
    let trash = dirs.home.join(".Trash");
    std::fs::create_dir_all(&trash)?;
    let name = app.file_name().unwrap_or_default();
    let mut dest = trash.join(name);
    if dest.exists() {
        dest = trash.join(format!("{}-{}", name.to_string_lossy(), now()));
    }
    if std::fs::rename(app, &dest).is_err() {
        run_ok(r, &["ditto", &path_str(app), &path_str(&dest)], "ditto")?;
        std::fs::remove_dir_all(app)?;
    }
    Ok(())
}

/// `--zap`: the app's Application Support folder, and its caches, preferences and saved state when its bundle id is
/// known. Returns what could be removed.
fn remove_support_files(dirs: &Dirs, app: &Path, bundle_id: Option<String>) -> Vec<String> {
    let lib = dirs.home.join("Library");
    let stem = app
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut paths = vec![lib.join("Application Support").join(&stem)];
    if let Some(b) = bundle_id.filter(|b| !b.is_empty()) {
        paths.push(lib.join("Caches").join(&b));
        paths.push(lib.join("Preferences").join(format!("{b}.plist")));
        paths.push(
            lib.join("Saved Application State")
                .join(format!("{b}.savedState")),
        );
    }
    paths
        .into_iter()
        .filter_map(|p| {
            let removed = if p.is_dir() {
                std::fs::remove_dir_all(&p).is_ok()
            } else {
                std::fs::remove_file(&p).is_ok()
            };
            removed.then(|| path_str(&p))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_strictly_inside_a_folder_may_be_deleted_from_it() {
        let packages = Path::new("/h/.kaika/packages");
        assert!(is_inside(
            Path::new("/h/.kaika/packages/tool/1.0"),
            packages
        ));
        assert!(
            !is_inside(packages, packages),
            "the folder itself is never removed"
        );
        assert!(!is_inside(Path::new("/h/.kaika/packages/../x"), packages));
        assert!(!is_inside(Path::new("/h/.kaika/other/tool"), packages));
    }
}
