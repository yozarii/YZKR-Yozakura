//! User configuration: backend preference order and the decision layer's commands.

use crate::paths::Dirs;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ORDER: [&str; 6] = ["brew", "cask", "uv", "npm", "mas", "direct"];

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Command that answers escalated decisions: JSON prompt on stdin, JSON answer on stdout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<String>,
    /// Local decision model name (Ollama).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

pub const KEYS: [&str; 3] = ["order", "escalation", "model"];
pub const DEFAULT_MODEL: &str = "qwen2.5:3b-instruct-q4_K_M";

fn on_path(prog: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
}

fn poc(file: &str) -> Option<String> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("poc").join(file);
    p.exists().then(|| p.display().to_string())
}

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
            "escalation" => self.escalation.clone().unwrap_or_default(),
            "model" => self.model.clone().unwrap_or_default(),
            _ => bail!("unknown key '{key}' (keys: {})", KEYS.join(", ")),
        })
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let opt = |v: &str| if v.is_empty() { None } else { Some(v.to_string()) };
        match key {
            "order" => self.order = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "escalation" => self.escalation = opt(value),
            "model" => self.model = opt(value),
            _ => bail!("unknown key '{key}' (keys: {})", KEYS.join(", ")),
        }
        Ok(())
    }

    /// All model use, and web discovery of vendor downloads, can be switched off: `KAIKA_MODELS=off`, or `kaika config set model none`.
    pub fn models_enabled(&self) -> bool {
        std::env::var("KAIKA_MODELS").as_deref() != Ok("off") && self.model.as_deref() != Some("none")
    }

    /// The small local decision model (Ollama).
    pub fn model(&self) -> String {
        self.model.clone().filter(|m| !m.is_empty() && m != "none").unwrap_or_else(|| DEFAULT_MODEL.into())
    }

    pub fn ollama_host(&self) -> String {
        let h = std::env::var("OLLAMA_HOST").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "http://127.0.0.1:11434".into());
        let h = if h.starts_with("http") { h } else { format!("http://{h}") };
        h.trim_end_matches('/').to_string()
    }

    /// The bigger model, asked only when the small one is unsure: `KAIKA_ESCALATION` (or "none"), then config,
    /// then the Claude Code adapter when the `claude` CLI is installed.
    pub fn escalation_cmd(&self) -> Option<Vec<String>> {
        if !self.models_enabled() {
            return None;
        }
        let split = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        match std::env::var("KAIKA_ESCALATION").ok().or_else(|| self.escalation.clone()).as_deref() {
            Some("none") | Some("") => None,
            Some(c) => Some(split(c)),
            None if on_path("claude") => poc("escalate.py").map(|p| vec!["python3".into(), "-I".into(), p]),
            None => None,
        }
    }

    pub fn escalation_string(&self) -> Option<String> {
        self.escalation_cmd().map(|c| c.join(" "))
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
