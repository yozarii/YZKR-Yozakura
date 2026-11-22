//! Recipes: how to find the latest download of an app that has no package.
//!
//! A recipe names an app and one place its releases are announced: a GitHub project, a Sparkle appcast, a page whose
//! links include the download, or a fixed URL. [`latest`] reads that place and says which download is newest.
//! Curated recipes are embedded; user recipes in `~/.kaika/recipes` override them, and so do the ones kaika learns
//! when it finds an app online. A recipe is deterministic: it never involves a model.

use crate::asset::{file_name, glob_match, norm_version, pick_asset};
use crate::links::{Link, download_links, redirected_links};
use crate::net::fetch;
use crate::paths::Dirs;
use crate::render;
use crate::runner::Runner;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One vendor app. Every field but the name is optional: which of `github`, `appcast`, `page` or `url` is set says
/// where releases are found, and an absent `team_id` means trust on first use.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Recipe {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Expected signing Team ID. Absent means trust-on-first-use: shown, then recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// Bundle name inside the download, for example `Rectangle.app`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// GitHub `owner/repo` whose latest release carries the download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// Sparkle appcast feed URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appcast: Option<String>,
    /// A page whose links include the download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// A fixed download URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Glob over asset names, for example `*.dmg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub version: String,
    pub sha256: Option<String>,
}

const EMBEDDED: &[&str] = &[
    include_str!("../recipes/rectangle.toml"),
    include_str!("../recipes/h2o-lightning.toml"),
];

pub fn embedded() -> Vec<Recipe> {
    EMBEDDED
        .iter()
        .filter_map(|t| toml::from_str(t).ok())
        .collect()
}

fn load_dir(dir: &Path) -> Vec<Recipe> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut files: Vec<_> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|t| toml::from_str(&t).ok())
        .collect()
}

/// Embedded recipes, overridden by recipes in the user's config directory.
pub fn all(dirs: &Dirs) -> Vec<Recipe> {
    let mut v = embedded();
    for r in load_dir(&dirs.root.join("recipes")) {
        match v.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&r.name)) {
            Some(slot) => *slot = r,
            None => v.push(r),
        }
    }
    v
}

pub fn find(dirs: &Dirs, name: &str) -> Option<Recipe> {
    all(dirs)
        .into_iter()
        .find(|r| r.name.eq_ignore_ascii_case(name))
}

pub fn save_learned(dirs: &Dirs, r: &Recipe) -> Result<()> {
    let dir = dirs.root.join("recipes");
    std::fs::create_dir_all(&dir)?;
    let stem: String = r
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    std::fs::write(dir.join(format!("{stem}.toml")), toml::to_string_pretty(r)?)?;
    Ok(())
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)? + key.len();
    Some(tag[i..i + tag[i..].find('"')?].to_string())
}

pub fn parse_appcast(xml: &str) -> Vec<(String, String)> {
    let mut v = vec![];
    for chunk in xml.split("<enclosure").skip(1) {
        let tag = chunk.split('>').next().unwrap_or("");
        if let Some(url) = attr(tag, "url") {
            let ver = attr(tag, "sparkle:shortVersionString")
                .or_else(|| attr(tag, "sparkle:version"))
                .unwrap_or_default();
            v.push((url, ver));
        }
    }
    v
}

/// Work out the newest download for a recipe, from whichever source it names.
pub fn latest(r: &dyn Runner, rec: &Recipe) -> Result<Asset> {
    if let Some(repo) = &rec.github {
        return latest_on_github(r, rec, repo);
    }
    if let Some(feed) = &rec.appcast {
        return latest_in_appcast(r, rec, feed);
    }
    if let Some(page) = &rec.page {
        return latest_on_page(r, rec, page);
    }
    if let Some(url) = &rec.url {
        return Ok(Asset {
            name: file_name(url),
            url: url.clone(),
            version: String::new(),
            sha256: rec.sha256.clone(),
        });
    }
    bail!(
        "recipe '{}' has no github, appcast, page or url source",
        rec.name
    )
}

/// The latest GitHub release's best macOS asset, with the checksum GitHub publishes for it.
fn latest_on_github(r: &dyn Runner, rec: &Recipe, repo: &str) -> Result<Asset> {
    let base =
        std::env::var("KAIKA_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into());
    let body = fetch(r, &format!("{base}/repos/{repo}/releases/latest"))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).context("GitHub release response was not JSON")?;
    let tag = v["tag_name"].as_str().unwrap_or("").to_string();
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let names: Vec<String> = assets
        .iter()
        .map(|a| a["name"].as_str().unwrap_or("").to_string())
        .collect();
    let i = pick_asset(&names, rec.asset_pattern.as_deref()).ok_or_else(|| {
        anyhow!(
            "{repo} release {tag} has no macOS download matching {:?}",
            rec.asset_pattern
        )
    })?;
    let url = assets[i]["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow!("asset has no download URL"))?
        .to_string();
    let sha = assets[i]["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .map(String::from);
    Ok(Asset {
        name: names[i].clone(),
        url,
        version: norm_version(&tag),
        sha256: sha.or(rec.sha256.clone()),
    })
}

fn latest_in_appcast(r: &dyn Runner, rec: &Recipe, feed: &str) -> Result<Asset> {
    let items = parse_appcast(&fetch(r, feed)?);
    let names: Vec<String> = items.iter().map(|(u, _)| file_name(u)).collect();
    let i = pick_asset(&names, rec.asset_pattern.as_deref())
        .ok_or_else(|| anyhow!("appcast {feed} lists no macOS download"))?;
    Ok(Asset {
        name: names[i].clone(),
        url: items[i].0.clone(),
        version: norm_version(&items[i].1),
        sha256: rec.sha256.clone(),
    })
}

/// The best matching link on the recipe's page: its plain links, then buttons that only reach the file through a
/// redirect (`/download?platform=osx`), then the page as a browser shows it once its scripts ran (see `render`).
/// The links are screened exactly as in discovery: HTTPS only, no unresolved templates, nothing that says it is for
/// another system, the newest release of each product.
fn latest_on_page(r: &dyn Runner, rec: &Recipe, page: &str) -> Result<Asset> {
    let pattern = rec.asset_pattern.as_deref();
    let best = |links: Vec<Link>| {
        links
            .into_iter()
            .find(|l| pattern.is_none_or(|p| glob_match(p, &file_name(&l.1))))
            .map(|l| l.1)
    };
    let html = fetch(r, page)?;
    let url = best(download_links(&html, page))
        .or_else(|| best(redirected_links(r, &html, page)))
        .or_else(|| render::page(r, page).and_then(|shown| best(download_links(&shown, page))))
        .ok_or_else(|| anyhow!("no download link found on {page}"))?;
    Ok(Asset {
        name: file_name(&url),
        url,
        version: String::new(),
        sha256: rec.sha256.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::USER_AGENT;
    use crate::runner::Fake;

    #[test]
    fn appcast_reads_enclosures() {
        let xml = r#"<item><enclosure url="https://x.com/A-2.zip" sparkle:shortVersionString="2.0" length="1"/></item>
                     <item><enclosure url="https://x.com/A-1.zip" sparkle:version="1" /></item>"#;
        assert_eq!(
            parse_appcast(xml),
            vec![
                ("https://x.com/A-2.zip".into(), "2.0".into()),
                ("https://x.com/A-1.zip".into(), "1".into())
            ]
        );
    }

    #[test]
    fn user_recipes_override_embedded_ones() {
        let t = tempfile::tempdir().unwrap();
        let d = Dirs::from_lookup(|k| {
            (k == "KAIKA_HOME")
                .then(|| t.path().display().to_string())
                .or(Some("/h".into()))
        });
        let mine = Recipe {
            name: "rectangle".into(),
            url: Some("https://x/y.dmg".into()),
            ..Default::default()
        };
        save_learned(&d, &mine).unwrap();
        assert_eq!(
            find(&d, "Rectangle").unwrap().url.as_deref(),
            Some("https://x/y.dmg")
        );
    }

    #[test]
    fn the_embedded_model_recipe_is_pinned_to_one_exact_file() {
        let r = embedded()
            .into_iter()
            .find(|r| r.name == crate::model::MODEL_NAME)
            .expect("the model recipe is embedded");
        let url = r.url.unwrap();
        assert!(
            url.starts_with("https://huggingface.co/h2oai/h2o-lightning-4b-gguf/")
                && url.ends_with(crate::model::MODEL_FILE),
            "{url}"
        );
        let sum = r.sha256.expect("a model download must carry its checksum");
        assert!(sum.len() == 64 && sum.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn embedded_recipes_parse() {
        assert!(!embedded().is_empty());
    }

    fn fetch_cmd(url: &str) -> String {
        format!("curl -fsSL --proto-redir =https --max-time 30 -A {USER_AGENT} {url}")
    }

    #[test]
    fn a_page_recipe_screens_links_as_discovery_does_when_it_looks_for_the_next_version() {
        // Raw markup holds a script placeholder, a plain-http link and a Windows-only link ahead of the real one.
        let page = r#"<a href="https://x.example/Tool_${version}_mac.dmg">t</a>
            <a href="http://x.example/Tool-9.dmg">plain http</a>
            <a href="https://x.example/Tool-8.zip">Download for Windows</a>
            <a href="https://x.example/Tool-1.2.dmg">Download</a>"#;
        let r = Fake::default().with(&fetch_cmd("https://x.example/dl"), page);
        let rec = Recipe {
            name: "tool".into(),
            page: Some("https://x.example/dl".into()),
            asset_pattern: Some("Tool*".into()),
            ..Default::default()
        };
        assert_eq!(
            latest(&r, &rec).unwrap().url,
            "https://x.example/Tool-1.2.dmg"
        );
    }
}
