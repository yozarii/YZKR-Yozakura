//! User configuration: the backend preference order, and whether models may be used at all.

use crate::paths::Dirs;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ORDER: [&str; 6] = ["brew", "cask", "uv", "npm", "mas", "direct"];
pub const KEYS: [&str; 2] = ["order", "models"];

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// "off" turns off every model call and the web lookups that feed them. Anything else (or unset) leaves them on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<String>,
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
        std::fs::write(
            dirs.config.join("kaika.toml"),
            toml::to_string_pretty(self)?,
        )?;
        Ok(())
    }

    pub fn order(&self) -> Vec<String> {
        if self.order.is_empty() {
            DEFAULT_ORDER.iter().map(|s| s.to_string()).collect()
        } else {
            self.order.clone()
        }
    }

    pub fn get(&self, key: &str) -> Result<String> {
        Ok(match key {
            "order" => self.order().join(","),
            "models" => {
                if self.models_enabled() {
                    "on".into()
                } else {
                    "off".into()
                }
            }
            _ => bail!("unknown key '{key}' (keys: {})", KEYS.join(", ")),
        })
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "order" => {
                self.order = value
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            "models" => match value {
                "on" | "off" => self.models = Some(value.into()),
                _ => bail!("models must be 'on' or 'off'"),
            },
            _ => bail!("unknown key '{key}' (keys: {})", KEYS.join(", ")),
        }
        Ok(())
    }

    /// Models, and web discovery of vendor downloads, can be switched off: `KAIKA_MODELS=off`, or `kaika config set models off`.
    pub fn models_enabled(&self) -> bool {
        std::env::var("KAIKA_MODELS").as_deref() != Ok("off")
            && self.models.as_deref() != Some("off")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_order_matches_the_design() {
        assert_eq!(
            Config::default().order(),
            ["brew", "cask", "uv", "npm", "mas", "direct"]
        );
    }

    #[test]
    fn set_and_get_round_trip() {
        let mut c = Config::default();
        c.set("order", "cask, brew").unwrap();
        assert_eq!(c.get("order").unwrap(), "cask,brew");
        c.set("models", "off").unwrap();
        assert!(!c.models_enabled());
        assert_eq!(c.get("models").unwrap(), "off");
        c.set("models", "on").unwrap();
        assert!(c.set("models", "maybe").is_err());
        assert!(c.set("nope", "x").is_err());
    }

    #[test]
    fn the_retired_settings_are_gone() {
        let mut c = Config::default();
        for k in ["model", "escalation", "resolver"] {
            assert!(
                c.set(k, "x").is_err() && c.get(k).is_err(),
                "{k} must not exist any more"
            );
        }
    }
}
