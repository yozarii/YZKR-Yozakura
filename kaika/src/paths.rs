//! Where kaika keeps things: one dedicated home, `~/.kaika`, as cargo, rustup and bun do.
//!
//! ```text
//! ~/.kaika/
//!   kaika.toml        configuration
//!   recipes/          your recipes and the ones kaika learned
//!   manifest.toml     what kaika installed, from where
//!   bin/              command-line tools (the only folder to put on PATH)
//!   packages/<name>/<version>/   unpacked command-line packages
//!   cache/            downloads and staging
//! ```
//!
//! Apps still go to `/Applications`. `KAIKA_HOME` moves the whole tree (tests and sandboxes use it).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Dirs {
    /// The user's home directory.
    pub home: PathBuf,
    /// kaika's own home, `~/.kaika`.
    pub root: PathBuf,
    /// Where `kaika.toml` and `recipes/` live.
    pub config: PathBuf,
    /// Where `manifest.toml` lives.
    pub data: PathBuf,
    pub cache: PathBuf,
    pub bin: PathBuf,
    pub packages: PathBuf,
    pub apps: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Dirs {
        Dirs::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Dirs {
        let home = PathBuf::from(get("HOME").unwrap_or_else(|| ".".into()));
        let root = get("KAIKA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".kaika"));
        let apps = get("KAIKA_APPS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/Applications"));
        Dirs {
            config: root.clone(),
            data: root.clone(),
            cache: root.join("cache"),
            bin: root.join("bin"),
            packages: root.join("packages"),
            root,
            apps,
            home,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn dirs(pairs: &[(&str, &str)]) -> Dirs {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Dirs::from_lookup(|k| m.get(k).cloned())
    }

    #[test]
    fn everything_lives_under_one_dedicated_home() {
        let d = dirs(&[("HOME", "/h")]);
        assert_eq!(d.root, PathBuf::from("/h/.kaika"));
        assert_eq!(d.config, PathBuf::from("/h/.kaika"));
        assert_eq!(d.data, PathBuf::from("/h/.kaika"));
        assert_eq!(d.cache, PathBuf::from("/h/.kaika/cache"));
        assert_eq!(d.bin, PathBuf::from("/h/.kaika/bin"));
        assert_eq!(d.packages, PathBuf::from("/h/.kaika/packages"));
        assert_eq!(d.apps, PathBuf::from("/Applications"));
    }

    #[test]
    fn kaika_home_moves_the_whole_tree() {
        let d = dirs(&[("HOME", "/h"), ("KAIKA_HOME", "/elsewhere")]);
        assert_eq!(d.bin, PathBuf::from("/elsewhere/bin"));
        assert_eq!(d.packages, PathBuf::from("/elsewhere/packages"));
        assert_eq!(d.home, PathBuf::from("/h"));
    }

    #[test]
    fn apps_dir_is_overridable() {
        let d = dirs(&[("HOME", "/h"), ("KAIKA_APPS_DIR", "/tmp/apps")]);
        assert_eq!(d.apps, PathBuf::from("/tmp/apps"));
    }
}
