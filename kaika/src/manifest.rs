//! The manifest: what kaika installed, from where, and how to update it.

use crate::paths::Dirs;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What kaika installed: one package, where it came from, and how to update it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Entry {
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub path: String,
    /// Backend-specific identifier (for example the App Store id) or the last asset URL for direct installs.
    #[serde(default)]
    pub reference: String,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub update_source: String,
    #[serde(default)]
    pub resolved_by: String,
    #[serde(default)]
    pub installed_at: u64,
}

/// Seconds since the epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Everything kaika installed, in `~/.kaika/manifest.toml`.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Manifest {
    #[serde(default, rename = "package")]
    pub packages: Vec<Entry>,
    #[serde(skip)]
    path: PathBuf,
}

impl Manifest {
    /// Read the manifest; a missing file is empty, a corrupt one is an error (never silently empty).
    pub fn load(dirs: &Dirs) -> Result<Manifest> {
        let path = dirs.root.join("manifest.toml");
        let mut m = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<Manifest>(&text)
                .with_context(|| format!("reading {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Manifest::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        m.path = path;
        Ok(m)
    }

    /// Write the manifest atomically (a temporary file, then a rename).
    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// The entry for this source and name.
    pub fn get(&self, source: &str, name: &str) -> Option<&Entry> {
        self.packages
            .iter()
            .find(|e| e.source == source && e.name == name)
    }

    /// Every entry with this name, whatever the source.
    pub fn find_name(&self, name: &str) -> Vec<&Entry> {
        self.packages
            .iter()
            .filter(|e| e.name.eq_ignore_ascii_case(name))
            .collect()
    }

    /// Add the entry, or replace the one with the same source and name.
    pub fn upsert(&mut self, e: Entry) {
        match self
            .packages
            .iter_mut()
            .find(|x| x.source == e.source && x.name == e.name)
        {
            Some(slot) => *slot = e,
            None => self.packages.push(e),
        }
    }

    /// Forget an entry; says whether there was one.
    pub fn remove(&mut self, source: &str, name: &str) -> bool {
        let before = self.packages.len();
        self.packages
            .retain(|e| !(e.source == source && e.name == name));
        self.packages.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(tmp: &std::path::Path) -> Dirs {
        let t = tmp.to_str().unwrap().to_string();
        Dirs::from_lookup(move |k| match k {
            "HOME" => Some(t.clone()),
            "KAIKA_HOME" => Some(format!("{t}/data")),
            _ => None,
        })
    }

    fn entry(src: &str, name: &str, ver: &str) -> Entry {
        Entry {
            name: name.into(),
            source: src.into(),
            version: ver.into(),
            ..Default::default()
        }
    }

    #[test]
    fn missing_file_loads_empty() {
        let t = tempfile::tempdir().unwrap();
        assert!(Manifest::load(&dirs(t.path())).unwrap().packages.is_empty());
    }

    #[test]
    fn round_trips_through_disk() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs(t.path());
        let mut m = Manifest::load(&d).unwrap();
        m.upsert(entry("brew", "jq", "1.7"));
        m.save().unwrap();
        let back = Manifest::load(&d).unwrap();
        assert_eq!(back.get("brew", "jq").unwrap().version, "1.7");
    }

    #[test]
    fn empty_manifest_round_trips() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs(t.path());
        let m = Manifest::load(&d).unwrap();
        m.save().unwrap();
        assert!(Manifest::load(&d).unwrap().packages.is_empty());
    }

    #[test]
    fn upsert_replaces_same_source_and_name() {
        let mut m = Manifest::default();
        m.upsert(entry("npm", "x", "1"));
        m.upsert(entry("npm", "x", "2"));
        m.upsert(entry("brew", "x", "9"));
        assert_eq!(m.packages.len(), 2);
        assert_eq!(m.get("npm", "x").unwrap().version, "2");
    }

    #[test]
    fn remove_reports_whether_anything_went() {
        let mut m = Manifest::default();
        m.upsert(entry("uv", "t", "1"));
        assert!(m.remove("uv", "t"));
        assert!(!m.remove("uv", "t"));
    }

    #[test]
    fn corrupt_manifest_is_an_error_not_silently_empty() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs(t.path());
        std::fs::create_dir_all(&d.root).unwrap();
        std::fs::write(d.root.join("manifest.toml"), "package = [[[").unwrap();
        assert!(Manifest::load(&d).is_err());
    }
}
