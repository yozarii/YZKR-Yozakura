//! User configuration: backend preference order and the decision layer's commands.

use crate::paths::Dirs;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ORDER: [&str; 6] = ["brew", "cask", "uv", "npm", "mas", "direct"];

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Command line for the resolver (the Python PoC today).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolver: Option<String>,
    /// Command that answers escalated decisions: JSON prompt on stdin, JSON answer on stdout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<String>,
    /// Local decision model name (Ollama).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

pub const KEYS: [&str; 4] = ["order", "resolver", "escalation", "model"];

impl Config {
    pub fn load(dirs: &Dirs) -> Result<Config> {
        let path = dirs.config.join("kaika.toml");
        match std::fs::read_to_string(&path) {
            Ok(t) => toml::from_str(&t).with_context(|| format!("reading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, dirs: &Dirs) -> Result<()> {
        std::fs::create_dir_all(&dirs.config)?;
        std::fs::write(dirs.config.join("kaika.toml"), toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn order(&self) -> Vec<String> {
        if self.order.is_empty() { DEFAULT_ORDER.iter().map(|s| s.to_string()).collect() } else { self.order.clone() }
    }

    pub fn get(&self, key: &str) -> Result<String> {
        Ok(match key {
            "order" => self.order().join(","),
            "resolver" => self.resolver.clone().unwrap_or_default(),
            "escalation" => self.escalation.clone().unwrap_or_default(),
            "model" => self.model.clone().unwrap_or_default(),
            _ => bail!("unknown key '{key}' (keys: {})", KEYS.join(", ")),
        })
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let opt = |v: &str| if v.is_empty() { None } else { Some(v.to_string()) };
        match key {
            "order" => self.order = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "resolver" => self.resolver = opt(value),
            "escalation" => self.escalation = opt(value),
            "model" => self.model = opt(value),
            _ => bail!("unknown key '{key}' (keys: {})", KEYS.join(", ")),
        }
        Ok(())
    }

    /// The resolver command: `KAIKA_RESOLVER` (or "none"), then config, then the in-repo PoC.
    pub fn resolver_cmd(&self) -> Option<Vec<String>> {
        let split = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        if let Ok(v) = std::env::var("KAIKA_RESOLVER") {
            return if v.is_empty() || v == "none" { None } else { Some(split(&v)) };
        }
        if let Some(c) = &self.resolver {
            return Some(split(c));
        }
        let poc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("poc/resolver.py");
        poc.exists().then(|| vec!["python3".into(), "-I".into(), poc.display().to_string()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_order_matches_the_design() {
        assert_eq!(Config::default().order(), ["brew", "cask", "uv", "npm", "mas", "direct"]);
    }

    #[test]
    fn set_and_get_round_trip() {
        let mut c = Config::default();
        c.set("order", "cask, brew").unwrap();
        assert_eq!(c.get("order").unwrap(), "cask,brew");
        c.set("model", "qwen2.5:3b").unwrap();
        assert_eq!(c.get("model").unwrap(), "qwen2.5:3b");
        assert!(c.set("nope", "x").is_err());
    }
}
