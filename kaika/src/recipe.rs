//! Recipes: how to find the latest download of an app that has no package.

use crate::direct::ensure_https;
use crate::paths::Dirs;
use crate::runner::{argv, Runner};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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

const EMBEDDED: &[&str] = &[include_str!("../recipes/rectangle.toml"), include_str!("../recipes/h2o-lightning.toml")];

pub fn embedded() -> Vec<Recipe> {
    EMBEDDED.iter().filter_map(|t| toml::from_str(t).ok()).collect()
}

fn load_dir(dir: &Path) -> Vec<Recipe> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
    files.sort();
    files.iter().filter_map(|p| std::fs::read_to_string(p).ok()).filter_map(|t| toml::from_str(&t).ok()).collect()
}

/// Embedded recipes, overridden by recipes in the user's config directory.
pub fn all(dirs: &Dirs) -> Vec<Recipe> {
    let mut v = embedded();
    for r in load_dir(&dirs.config.join("recipes")) {
        match v.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&r.name)) {
            Some(slot) => *slot = r,
            None => v.push(r),
        }
    }
    v
}

pub fn find(dirs: &Dirs, name: &str) -> Option<Recipe> {
    all(dirs).into_iter().find(|r| r.name.eq_ignore_ascii_case(name))
}

pub fn save_learned(dirs: &Dirs, r: &Recipe) -> Result<()> {
    let dir = dirs.config.join("recipes");
    std::fs::create_dir_all(&dir)?;
    let stem: String = r.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    std::fs::write(dir.join(format!("{stem}.toml")), toml::to_string_pretty(r)?)?;
    Ok(())
}

pub fn norm_version(v: &str) -> String {
    v.trim().trim_start_matches(['v', 'V']).to_string()
}

/// Simple glob: `*` matches anything; case-insensitive.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.to_lowercase(), text.to_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !t.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if i == parts.len() - 1 {
            return t.len() >= pos + part.len() && t.ends_with(part);
        } else {
            match t[pos..].find(part) {
                Some(f) => pos += f + part.len(),
                None => return false,
            }
        }
    }
    true
}

/// Score a download name for this Mac. `None` means not a macOS installable at all.
pub fn asset_score(name: &str) -> Option<i32> {
    let n = name.to_lowercase();
    let ext = if n.ends_with(".dmg") {
        30
    } else if n.ends_with(".zip") {
        20
    } else if n.ends_with(".pkg") {
        10
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        5
    } else {
        return None;
    };
    // Other operating systems, source, and files that are not the app itself (debug symbols, builds, helpers).
    if ["uninstall", "updater", "agent", "helper"].iter().any(|w| n.contains(w)) {
        return None;
    }
    if ["linux", "windows", "win32", "win64", ".exe", "freebsd", "-src", "source", "dsym", "symbols", "-debug", "_debug", ".debug", "-dbg"].iter().any(|w| n.contains(w)) {
        return None;
    }
    let mut s = ext;
    // Not the stable release: still installable when it is all there is, but never preferred.
    if ["nightly", "-beta", ".beta", "-alpha", ".alpha", "-rc", "-dev", "preview", "canary"].iter().any(|w| n.contains(w)) {
        s -= 12;
    }
    if ["arm64", "aarch64", "apple-silicon", "applesilicon"].iter().any(|w| n.contains(w)) {
        s += 8;
    }
    if ["universal", "macos", "mac", "darwin", "osx"].iter().any(|w| n.contains(w)) {
        s += 4;
    }
    if ["x86_64", "x64", "intel", "amd64", "i386"].iter().any(|w| n.contains(w)) && !n.contains("universal") {
        s -= 50;
    }
    Some(s)
}

/// Links that name no file but say they are the macOS download (`/api/download?platform=osx`,
/// `/updates/download/darwin-arm64/app`, a button "Download for macOS"): at most four, in page order.
pub fn extensionless_candidates(page: &str, base: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for a in crate::html::anchors(page) {
        let url = resolve_url(base, &a.href.replace("&amp;", "&"));
        if a.href.is_empty() || ensure_https(&url).is_err() || asset_score(&file_name(&url)).is_some() {
            continue;
        }
        let (u, t) = (url.to_lowercase(), a.text.to_lowercase());
        if ["ptb", "canary", "beta", "nightly", "insider", "preview"].iter().any(|w| u.contains(&format!("={w}")) || u.contains(&format!("/{w}"))) {
            continue; // test channels are never the default download
        }
        let says_download = u.contains("download") || t.contains("download");
        let says_mac = ["darwin", "osx", "macos", "/mac", "mac-", "mac_", "=mac", "apple silicon", "for mac"].iter().any(|w| u.contains(w) || t.contains(w));
        if says_download && says_mac && !out.iter().any(|(x, _)| *x == url) {
            out.push((url, a.text));
        }
    }
    // The Apple-silicon build first: the four slots go to the likeliest links.
    out.sort_by_key(|(u, _)| !(u.contains("arm64") || u.contains("aarch64") || u.contains("universal")));
    out.truncate(4);
    out
}

/// Where a download link really leads: the final URL after redirects, fetching one byte of the file at most
/// (some servers refuse HEAD requests).
pub fn final_url(r: &dyn Runner, url: &str) -> Option<String> {
    ensure_https(url).ok()?;
    let o = r.run(&argv(&["curl", "-sL", "-r", "0-0", "-o", "/dev/null", "-w", "%{url_effective}", "--max-time", "20", "-A", USER_AGENT, url])).ok().filter(|o| o.ok())?;
    let last = o.stdout.trim().to_string();
    (ensure_https(&last).is_ok() && asset_score(&file_name(&last)).is_some()).then_some(last)
}

/// `extensionless_candidates` followed to the real files: (final url, link text).
pub fn resolve_redirected_downloads(r: &dyn Runner, page: &str, base: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for (u, t) in extensionless_candidates(page, base) {
        if let Some(f) = final_url(r, &u) {
            if !out.iter().any(|(x, _)| *x == f) {
                out.push((f, t));
            }
        }
    }
    out
}

/// Index of the best download among `names`, honouring an optional glob. First wins ties.
pub fn pick_asset(names: &[String], pattern: Option<&str>) -> Option<usize> {
    let mut best: Option<(usize, i32)> = None;
    for (i, n) in names.iter().enumerate() {
        if pattern.is_some_and(|p| !glob_match(p, n)) {
            continue;
        }
        let Some(s) = asset_score(n) else { continue };
        if best.is_none_or(|(_, b)| s > b) {
            best = Some((i, s));
        }
    }
    best.map(|(i, _)| i)
}

/// Generalise a download's file name so the next version still matches: `Ghost-1.2-arm64.dmg` becomes `Ghost-*.dmg`.
pub fn link_pattern(filename: &str) -> String {
    let lower = filename.to_lowercase();
    let ext = [".tar.gz", ".dmg", ".zip", ".pkg", ".tgz"].into_iter().find(|e| lower.ends_with(e)).unwrap_or("");
    let stem = &filename[..filename.len() - ext.len()];
    let head: String = stem.chars().take_while(|c| !c.is_ascii_digit()).collect();
    format!("{head}*{ext}")
}

pub fn file_name(url: &str) -> String {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    no_query.rsplit('/').next().unwrap_or("download").to_string()
}

/// How kaika identifies itself to every site it reads.
pub const USER_AGENT: &str = "kaika/0.1 (package manager)";

/// Read a page or feed over HTTPS: the one place kaika fetches text from the web.
pub fn fetch(r: &dyn Runner, url: &str) -> Result<String> {
    ensure_https(url)?;
    let o = r.run(&argv(&["curl", "-fsSL", "--max-time", "30", "-A", USER_AGENT, url]))?;
    if !o.ok() {
        bail!("could not fetch {url}: {}", o.stderr.trim());
    }
    Ok(o.stdout)
}

pub fn extract_links(html: &str, base: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = html;
    while let Some(i) = rest.find("href=") {
        rest = &rest[i + 5..];
        let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let Some(end) = rest[1..].find(q) else { break };
        out.push(resolve_url(base, &rest[1..1 + end]));
        rest = &rest[1 + end..];
    }
    out
}

pub fn resolve_url(base: &str, href: &str) -> String {
    if href.starts_with("https://") || href.starts_with("http://") || href.starts_with("file://") {
        return href.into();
    }
    let (scheme, after) = base.split_once("://").unwrap_or(("https", base));
    if let Some(h) = href.strip_prefix("//") {
        return format!("{scheme}://{h}");
    }
    let host_end = after.find('/').unwrap_or(after.len());
    if let Some(p) = href.strip_prefix('/') {
        return format!("{scheme}://{}/{p}", &after[..host_end]);
    }
    let dir = match base.rfind('/') {
        Some(i) if i > scheme.len() + 2 => &base[..i],
        _ => base,
    };
    format!("{dir}/{href}")
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
            let ver = attr(tag, "sparkle:shortVersionString").or_else(|| attr(tag, "sparkle:version")).unwrap_or_default();
            v.push((url, ver));
        }
    }
    v
}

/// Work out the newest download for a recipe.
pub fn latest(r: &dyn Runner, rec: &Recipe) -> Result<Asset> {
    if let Some(repo) = &rec.github {
        let base = std::env::var("KAIKA_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into());
        let body = fetch(r, &format!("{base}/repos/{repo}/releases/latest"))?;
        let v: serde_json::Value = serde_json::from_str(&body).context("GitHub release response was not JSON")?;
        let tag = v["tag_name"].as_str().unwrap_or("").to_string();
        let assets = v["assets"].as_array().cloned().unwrap_or_default();
        let names: Vec<String> = assets.iter().map(|a| a["name"].as_str().unwrap_or("").to_string()).collect();
        let i = pick_asset(&names, rec.asset_pattern.as_deref())
            .ok_or_else(|| anyhow!("{repo} release {tag} has no macOS download matching {:?}", rec.asset_pattern))?;
        let url = assets[i]["browser_download_url"].as_str().ok_or_else(|| anyhow!("asset has no download URL"))?.to_string();
        let sha = assets[i]["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(String::from);
        return Ok(Asset { name: names[i].clone(), url, version: norm_version(&tag), sha256: sha.or(rec.sha256.clone()) });
    }
    if let Some(feed) = &rec.appcast {
        let items = parse_appcast(&fetch(r, feed)?);
        let names: Vec<String> = items.iter().map(|(u, _)| file_name(u)).collect();
        let i = pick_asset(&names, rec.asset_pattern.as_deref()).ok_or_else(|| anyhow!("appcast {feed} lists no macOS download"))?;
        return Ok(Asset { name: names[i].clone(), url: items[i].0.clone(), version: norm_version(&items[i].1), sha256: rec.sha256.clone() });
    }
    if let Some(page) = &rec.page {
        let mut links = extract_links(&fetch(r, page)?, page);
        let mut names: Vec<String> = links.iter().map(|l| file_name(l)).collect();
        let mut pick = pick_asset(&names, rec.asset_pattern.as_deref());
        if pick.is_none() {
            // Links that only reach the file through a redirect (`/download?platform=osx`).
            let page_html = fetch(r, page)?;
            let redirected: Vec<String> = resolve_redirected_downloads(r, &page_html, page).into_iter().map(|x| x.0).collect();
            if !redirected.is_empty() {
                links = redirected;
                names = links.iter().map(|l| file_name(l)).collect();
                pick = pick_asset(&names, rec.asset_pattern.as_deref());
            }
        }
        if pick.is_none() {
            // The links may only exist after the page's scripts ran (see render.rs).
            if let Some(shown) = crate::render::page(r, page) {
                links = extract_links(&shown, page);
                names = links.iter().map(|l| file_name(l)).collect();
                pick = pick_asset(&names, rec.asset_pattern.as_deref());
            }
        }
        let i = pick.ok_or_else(|| anyhow!("no download link found on {page}"))?;
        return Ok(Asset { name: names[i].clone(), url: links[i].clone(), version: String::new(), sha256: rec.sha256.clone() });
    }
    if let Some(url) = &rec.url {
        return Ok(Asset { name: file_name(url), url: url.clone(), version: String::new(), sha256: rec.sha256.clone() });
    }
    bail!("recipe '{}' has no github, appcast, page or url source", rec.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.dmg", "Zen-1.0.dmg"));
        assert!(glob_match("zen-*-mac.zip", "Zen-1.0-mac.zip"));
        assert!(!glob_match("*.dmg", "Zen.zip"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn prefers_arm64_dmg_over_intel_and_other_oses() {
        let names = s(&["app-linux-x64.tar.gz", "App-x86_64.dmg", "App-arm64.dmg", "App-arm64.zip", "App.exe", "checksums.txt"]);
        assert_eq!(pick_asset(&names, None), Some(2));
    }

    #[test]
    fn debug_symbols_and_debug_builds_are_not_the_app() {
        for n in ["Ghost-3.0-arm64-dSYM.zip", "Ghost-3.0-symbols.zip", "Ghost-3.0-arm64-debug.zip", "ghost_debug.dmg", "Ghost-3.0.debug.tar.gz"] {
            assert_eq!(asset_score(n), None, "{n}");
        }
        assert!(asset_score("Ghost-3.0-arm64.zip").is_some());
    }

    #[test]
    fn nightly_and_beta_builds_never_beat_the_stable_one_but_are_usable_alone() {
        let stable = asset_score("Ghost-3.0.2-arm64.dmg").unwrap();
        for n in ["Ghost-nightly-arm64.dmg", "Ghost-3.1.0-beta1-arm64.dmg", "Ghost-3.1-rc1-arm64.dmg", "Ghost-canary-arm64.dmg", "Ghost-preview-arm64.dmg"] {
            let s = asset_score(n).unwrap();
            assert!(stable - s >= 8, "{n}: {s} vs {stable}");
        }
        assert_eq!(pick_asset(&["Ghost-nightly-arm64.dmg".to_string()], None), Some(0));
    }

    #[test]
    fn universal_beats_intel_only() {
        let names = s(&["App-intel.dmg", "App-universal.zip"]);
        assert_eq!(pick_asset(&names, None), Some(1));
    }

    #[test]
    fn pattern_filters_and_nothing_matching_is_none() {
        let names = s(&["App-arm64.dmg", "App-arm64.zip"]);
        assert_eq!(pick_asset(&names, Some("*.zip")), Some(1));
        assert_eq!(pick_asset(&names, Some("*.pkg")), None);
        assert_eq!(pick_asset(&s(&["notes.txt"]), None), None);
    }

    #[test]
    fn links_resolve_against_the_page() {
        let html = r#"<a href="/dl/App.dmg">x</a><a href='rel/App2.zip'>y</a><a href="https://cdn.x.com/a.pkg">z</a>"#;
        let l = extract_links(html, "https://x.com/get/page.html");
        assert_eq!(l, ["https://x.com/dl/App.dmg", "https://x.com/get/rel/App2.zip", "https://cdn.x.com/a.pkg"]);
    }

    #[test]
    fn appcast_reads_enclosures() {
        let xml = r#"<item><enclosure url="https://x.com/A-2.zip" sparkle:shortVersionString="2.0" length="1"/></item>
                     <item><enclosure url="https://x.com/A-1.zip" sparkle:version="1" /></item>"#;
        assert_eq!(parse_appcast(xml), vec![("https://x.com/A-2.zip".into(), "2.0".into()), ("https://x.com/A-1.zip".into(), "1".into())]);
    }

    #[test]
    fn link_patterns_generalise_a_file_name() {
        assert_eq!(link_pattern("Ghost-1.2.3-arm64.dmg"), "Ghost-*.dmg");
        assert_eq!(link_pattern("Ghost.zip"), "Ghost*.zip");
        assert_eq!(link_pattern("tool-1.0.tar.gz"), "tool-*.tar.gz");
        assert!(glob_match(&link_pattern("Ghost-1.2.3-arm64.dmg"), "Ghost-9.9-universal.dmg"));
    }

    #[test]
    fn versions_lose_their_v() {
        assert_eq!(norm_version("v1.2.3"), "1.2.3");
    }

    #[test]
    fn user_recipes_override_embedded_ones() {
        let t = tempfile::tempdir().unwrap();
        let d = Dirs::from_lookup(|k| (k == "KAIKA_HOME").then(|| t.path().display().to_string()).or(Some("/h".into())));
        let mine = Recipe { name: "rectangle".into(), url: Some("https://x/y.dmg".into()), ..Default::default() };
        save_learned(&d, &mine).unwrap();
        assert_eq!(find(&d, "Rectangle").unwrap().url.as_deref(), Some("https://x/y.dmg"));
    }

    #[test]
    fn the_embedded_model_recipe_is_pinned_to_one_exact_file() {
        let r = embedded().into_iter().find(|r| r.name == crate::model::MODEL_NAME).expect("the model recipe is embedded");
        let url = r.url.unwrap();
        assert!(url.starts_with("https://huggingface.co/h2oai/h2o-lightning-4b-gguf/") && url.ends_with(crate::model::MODEL_FILE), "{url}");
        let sum = r.sha256.expect("a model download must carry its checksum");
        assert!(sum.len() == 64 && sum.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn embedded_recipes_parse() {
        assert!(!embedded().is_empty());
    }
}
