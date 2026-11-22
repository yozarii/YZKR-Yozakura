//! Apps in the applications folder that no package manager owns: listed, never changed.

use std::collections::HashSet;
use std::path::Path;

pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// `.app` bundle names (without the extension) in `dir`, minus those whose normalised name is in `owned`.
pub fn unowned(dir: &Path, owned: &HashSet<String>) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut v: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|x| x.to_str()) == Some("app"))
                .then(|| p.file_stem()?.to_str().map(String::from))?
        })
        .filter(|n| !owned.contains(&norm(n)))
        .collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_apps_not_owned_by_a_cask() {
        let t = tempfile::tempdir().unwrap();
        for n in ["Zen.app", "Safari.app", "notes.txt"] {
            std::fs::create_dir(t.path().join(n)).unwrap();
        }
        let owned: HashSet<String> = [norm("zen")].into();
        assert_eq!(unowned(t.path(), &owned), vec!["Safari"]);
    }

    #[test]
    fn missing_directory_is_empty() {
        assert!(unowned(Path::new("/definitely/not/here"), &HashSet::new()).is_empty());
    }
}
