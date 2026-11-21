//! Where kaika keeps things. XDG-style, overridable per directory (used by tests and sandboxes).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub apps: PathBuf,
    pub bin: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Dirs {
        Dirs::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Dirs {
        let home = PathBuf::from(get("HOME").unwrap_or_else(|| ".".into()));
        let pick = |own: &str, xdg: &str, rel: &str| -> PathBuf {
            if let Some(v) = get(own) {
                return PathBuf::from(v);
            }
            if let Some(v) = get(xdg) {
                return PathBuf::from(v).join("kaika");
            }
            home.join(rel).join("kaika")
        };
        let config = pick("KAIKA_CONFIG_DIR", "XDG_CONFIG_HOME", ".config");
        let data = pick("KAIKA_DATA_DIR", "XDG_DATA_HOME", ".local/share");
        let cache = pick("KAIKA_CACHE_DIR", "XDG_CACHE_HOME", ".cache");
        let apps = get("KAIKA_APPS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/Applications"));
        let bin = get("KAIKA_BIN_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".local/bin"));
        Dirs { home, config, data, cache, apps, bin }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn dirs(pairs: &[(&str, &str)]) -> Dirs {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Dirs::from_lookup(|k| m.get(k).cloned())
    }

    #[test]
    fn defaults_live_under_home() {
        let d = dirs(&[("HOME", "/h")]);
        assert_eq!(d.config, PathBuf::from("/h/.config/kaika"));
        assert_eq!(d.data, PathBuf::from("/h/.local/share/kaika"));
        assert_eq!(d.cache, PathBuf::from("/h/.cache/kaika"));
        assert_eq!(d.apps, PathBuf::from("/Applications"));
    }

    #[test]
    fn xdg_beats_home_and_own_beats_xdg() {
        let d = dirs(&[("HOME", "/h"), ("XDG_DATA_HOME", "/x"), ("KAIKA_DATA_DIR", "/own"), ("XDG_CACHE_HOME", "/c")]);
        assert_eq!(d.data, PathBuf::from("/own"));
        assert_eq!(d.cache, PathBuf::from("/c/kaika"));
    }

    #[test]
    fn apps_dir_is_overridable() {
        let d = dirs(&[("HOME", "/h"), ("KAIKA_APPS_DIR", "/tmp/apps")]);
        assert_eq!(d.apps, PathBuf::from("/tmp/apps"));
    }
}
