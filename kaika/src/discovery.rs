//! Finding the vendor download for an app that no backend carries. Ported from `poc/resolver.py`.
//!
//! Web search engines answer scripts with bot challenges, so kaika does not scrape them. Candidates come from
//! Wikidata, which records an official website for most notable software (structured, keyless, meant for
//! programs), or from a URL the user gives. Deterministic code collects candidates (sites, links, release
//! assets). A small model may choose among peers (which site, which download) through `decision::decide`; it
//! can only name a lettered candidate. Whatever is found still goes through the plan and every verification
//! check before anything is installed.

use crate::config::Config;
use crate::decision::{self, Pick};
use crate::direct::ensure_https;
use crate::html;
use crate::recipe::{self, Recipe};
use crate::resolver::Outcome;
use crate::runner::Runner;
use serde_json::Value;

const SITE_TASK: &str = "pick the official vendor site for this app";
const DOWNLOAD_TASK: &str = "pick the download for macOS on Apple silicon";
const MAX_SITES: usize = 6;

#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub name: String,
    pub url: String,
    pub version: String,
    pub sha256: Option<String>,
    pub domain: String,
    /// A page to re-read for updates, or a GitHub project whose releases to follow. At most one is set.
    pub page: Option<String>,
    pub github: Option<String>,
    pub asset_pattern: Option<String>,
    /// Where the site came from, for the manifest: "Wikidata" or "given URL".
    pub source: String,
    /// How it was found, for the plan.
    pub evidence: String,
}

impl Discovered {
    pub fn recipe(&self, name: &str) -> Recipe {
        let base = Recipe { name: name.into(), domain: Some(self.domain.clone()), asset_pattern: self.asset_pattern.clone(), ..Default::default() };
        match (&self.github, &self.page) {
            (Some(g), _) => Recipe { github: Some(g.clone()), ..base },
            (None, Some(p)) => Recipe { page: Some(p.clone()), ..base },
            (None, None) => Recipe { url: Some(self.url.clone()), asset_pattern: None, ..base },
        }
    }
}

fn norm(s: &str) -> String {
    crate::select::norm(s)
}

fn get(r: &dyn Runner, url: &str) -> Option<String> {
    recipe::fetch(r, url).ok()
}

fn get_json(r: &dyn Runner, url: &str) -> Option<Value> {
    serde_json::from_str(&get(r, url)?).ok()
}

// ---------- Wikidata ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: String,
    pub label: String,
    pub description: String,
}

/// `wbsearchentities` results.
pub fn parse_entities(v: &Value) -> Vec<Entity> {
    v["search"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| Some(Entity { id: e["id"].as_str()?.into(), label: e["label"].as_str().unwrap_or("").into(), description: e["description"].as_str().unwrap_or("").into() }))
                .collect()
        })
        .unwrap_or_default()
}

/// The official website (property P856) of an entity from `wbgetentities`; deprecated claims are skipped.
pub fn official_website(v: &Value, id: &str) -> Option<String> {
    v["entities"][id]["claims"]["P856"]
        .as_array()?
        .iter()
        .filter(|c| c["rank"].as_str() != Some("deprecated"))
        .find_map(|c| c["mainsnak"]["datavalue"]["value"].as_str().map(String::from))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub host: String,
    pub url: String,
    pub title: String,
}

fn wikidata_sites(r: &dyn Runner, query: &str) -> Vec<Site> {
    let search = format!("https://www.wikidata.org/w/api.php?action=wbsearchentities&search={}&language=en&type=item&limit=7&format=json", html::url_encode(query));
    let Some(found) = get_json(r, &search) else { return vec![] };
    let q = norm(query);
    let all = parse_entities(&found);
    let exact: Vec<Entity> = all.iter().filter(|e| norm(&e.label) == q).cloned().collect();
    let entities = if exact.is_empty() { all } else { exact };
    if entities.is_empty() {
        return vec![];
    }
    let ids: Vec<&str> = entities.iter().map(|e| e.id.as_str()).collect();
    let details = format!("https://www.wikidata.org/w/api.php?action=wbgetentities&ids={}&props=claims&format=json", ids.join("%7C"));
    let Some(d) = get_json(r, &details) else { return vec![] };
    let mut seen: Vec<String> = vec![];
    let mut out = vec![];
    for e in &entities {
        let Some(url) = official_website(&d, &e.id) else { continue };
        let host = html::host(&url);
        if host.is_empty() || seen.contains(&host) {
            continue;
        }
        seen.push(host.clone());
        out.push(Site { host, url, title: format!("{}: {}", e.label, e.description) });
    }
    out.truncate(MAX_SITES);
    out
}

fn github_repo(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    (html::host(url) == "github.com").then_some(())?;
    let mut parts = rest.split('/').skip(1);
    let (o, r) = (parts.next().filter(|x| !x.is_empty())?, parts.next().filter(|x| !x.is_empty())?);
    Some(format!("{o}/{r}"))
}

// ---------- pages ----------

type Link = (i32, String, String);

/// Pages built with scripts leave unresolved placeholders in their markup (`${version}`, `{{ version }}`).
/// Those are not downloads.
pub fn is_template(url: &str) -> bool {
    let u = url.to_lowercase();
    u.contains("${") || u.contains("{{") || u.contains('{') || u.contains('}') || u.contains("%7b") || u.contains("%7d") || u.contains("<%")
}

/// Download links on a page: (score, absolute https URL, link text), best first.
pub fn download_links(page: &str, base: &str) -> Vec<Link> {
    let mut links: Vec<Link> = html::anchors(page)
        .into_iter()
        .filter(|a| !a.href.is_empty())
        .map(|a| (recipe::resolve_url(base, &a.href), a.text))
        .filter(|(u, _)| ensure_https(u).is_ok() && !is_template(u))
        .filter_map(|(u, t)| recipe::asset_score(&recipe::file_name(&u)).map(|s| (s, u, t)))
        .collect();
    links.sort_by(|a, b| b.0.cmp(&a.0));
    links.dedup_by(|a, b| a.1 == b.1);
    links
}

/// The first link on a page that points at a GitHub project (not at GitHub's own pages).
fn github_link(page: &str) -> Option<String> {
    const NOT_PROJECTS: [&str; 10] = ["sponsors", "features", "orgs", "topics", "about", "login", "pricing", "marketplace", "settings", "apps"];
    html::anchors(page).into_iter().map(|a| a.href).filter(|h| h.starts_with("https://")).find_map(|h| {
        let repo = github_repo(&h)?;
        let owner = repo.split('/').next()?;
        (!NOT_PROJECTS.contains(&owner)).then(|| format!("https://github.com/{repo}"))
    })
}

/// Links that probably lead to a download page, on the same site, for sites that keep the file one click away.
fn download_pages(page: &str, base: &str) -> Vec<String> {
    let host = html::host(base);
    let mut out: Vec<String> = vec![];
    for a in html::anchors(page) {
        let url = recipe::resolve_url(base, &a.href);
        let wants = a.text.to_lowercase().contains("download") || a.href.to_lowercase().contains("download");
        if wants && ensure_https(&url).is_ok() && html::host(&url) == host && url != base && !out.contains(&url) && recipe::asset_score(&recipe::file_name(&url)).is_none() {
            out.push(url);
        }
    }
    out.truncate(2);
    out
}

fn from_site(r: &dyn Runner, cfg: &Config, query: &str, site_url: &str, source: &str, how: &str) -> Outcome<Discovered> {
    let domain = html::host(site_url);
    if let Some(repo) = github_repo(site_url) {
        let rec = Recipe { name: query.into(), github: Some(repo.clone()), ..Default::default() };
        return match recipe::latest(r, &rec) {
            Ok(a) => Outcome::Picked(Discovered {
                name: query.into(),
                asset_pattern: Some(recipe::link_pattern(&a.name)),
                url: a.url,
                version: a.version,
                sha256: a.sha256,
                domain: "github.com".into(),
                page: None,
                github: Some(repo),
                source: source.into(),
                evidence: format!("official site github.com ({how}); the latest release's best macOS asset {}", a.name),
            }),
            Err(e) => Outcome::Unsure(format!("no macOS download in the release feed of {site_url}: {e:#}")),
        };
    }

    let Some(home) = get(r, site_url) else { return Outcome::Unsure(format!("could not read {site_url}")) };
    let mut links = download_links(&home, site_url);
    let mut page_url = site_url.to_string();
    if links.is_empty() {
        // Many sites keep the file one click away, on a download page.
        for next in download_pages(&home, site_url) {
            if let Some(p) = get(r, &next) {
                let found = download_links(&p, &next);
                if !found.is_empty() {
                    links = found;
                    page_url = next;
                    break;
                }
            }
        }
    }
    if links.is_empty() {
        // Many vendor pages just point at the project's GitHub releases.
        if let Some(repo_url) = github_link(&home) {
            return from_site(r, cfg, query, &repo_url, source, &format!("{how}; its page links to the GitHub project"));
        }
        return Outcome::Unsure(format!("no macOS download found on {site_url}"));
    }
    let download = if links.len() == 1 || links[0].0 - links[1].0 >= 8 {
        Pick { id: links[0].1.clone(), how: format!("clearly the best macOS download: {}", recipe::file_name(&links[0].1)) }
    } else {
        let cands: Vec<(String, String)> = links.iter().take(5).map(|(_, u, t)| (u.clone(), format!("{} {}", recipe::file_name(u), t))).collect();
        match decision::decide(r, cfg, DOWNLOAD_TASK, query, &cands) {
            Outcome::Picked(p) => p,
            Outcome::Unsure(why) => return Outcome::Unsure(format!("could not tell which download to use: {why}")),
        }
    };
    Outcome::Picked(Discovered {
        name: query.into(),
        asset_pattern: Some(recipe::link_pattern(&recipe::file_name(&download.id))),
        url: download.id,
        version: String::new(),
        sha256: None,
        domain,
        page: Some(page_url),
        github: None,
        source: source.into(),
        evidence: format!("official site {} ({how}); {}", html::host(site_url), download.how),
    })
}

/// Which site is official? One candidate is taken as is; several go to the decision step.
fn choose_site(r: &dyn Runner, cfg: &Config, query: &str, sites: &[Site]) -> Outcome<Pick> {
    if sites.len() == 1 {
        return Outcome::Picked(Pick { id: sites[0].host.clone(), how: format!("the only Wikidata match for \"{query}\" with an official website") });
    }
    let cands: Vec<(String, String)> = sites.iter().map(|s| (s.host.clone(), format!("{}: {} ({})", s.host, s.title, s.url))).collect();
    decision::decide(r, cfg, SITE_TASK, query, &cands)
}

/// Find an app that no backend carries by name.
pub fn discover(r: &dyn Runner, cfg: &Config, query: &str) -> Outcome<Discovered> {
    let sites = wikidata_sites(r, query);
    if sites.is_empty() {
        return Outcome::Unsure(format!("Wikidata has no official website for \"{query}\"; if you know the vendor's page or download, give its https URL"));
    }
    let site = match choose_site(r, cfg, query, &sites) {
        Outcome::Picked(p) => p,
        Outcome::Unsure(why) => return Outcome::Unsure(format!("could not tell which site is official: {why}")),
    };
    let url = &sites.iter().find(|s| s.host == site.id).expect("picked from the list").url;
    from_site(r, cfg, query, url, "Wikidata", &site.how)
}

/// An https URL the user gave: a download itself, a GitHub project, or a page to scan.
pub fn from_url(r: &dyn Runner, cfg: &Config, name: &str, url: &str) -> Outcome<Discovered> {
    if let Err(e) = ensure_https(url) {
        return Outcome::Unsure(e.to_string());
    }
    let file = recipe::file_name(url);
    if recipe::asset_score(&file).is_some() {
        return Outcome::Picked(Discovered {
            name: name.into(),
            asset_pattern: Some(recipe::link_pattern(&file)),
            url: url.into(),
            version: String::new(),
            sha256: None,
            domain: html::host(url),
            page: None,
            github: None,
            source: "given URL".into(),
            evidence: format!("the download you gave: {file}"),
        });
    }
    from_site(r, cfg, name, url, "given URL", "the page you gave")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Fake;

    const OLLAMA: &str = "curl -s --max-time 60 -H Content-Type: application/json -d @- http://127.0.0.1:11434/api/chat";

    fn cfg() -> Config {
        Config { escalation: Some("none".into()), ..Default::default() }
    }
    fn api(url: &str) -> String {
        format!("curl -fsSL --max-time 30 -A {} {url}", recipe::USER_AGENT)
    }
    fn search_url(q: &str) -> String {
        api(&format!("https://www.wikidata.org/w/api.php?action=wbsearchentities&search={}&language=en&type=item&limit=7&format=json", html::url_encode(q)))
    }
    fn details_url(ids: &str) -> String {
        api(&format!("https://www.wikidata.org/w/api.php?action=wbgetentities&ids={ids}&props=claims&format=json"))
    }
    fn found(items: &[(&str, &str, &str)]) -> String {
        let a: Vec<String> = items.iter().map(|(id, l, d)| format!(r#"{{"id":"{id}","label":"{l}","description":"{d}"}}"#)).collect();
        format!(r#"{{"search":[{}]}}"#, a.join(","))
    }
    fn websites(items: &[(&str, Option<&str>)]) -> String {
        let a: Vec<String> = items
            .iter()
            .map(|(id, w)| match w {
                Some(u) => format!(r#""{id}":{{"claims":{{"P856":[{{"rank":"normal","mainsnak":{{"datavalue":{{"value":"{u}"}}}}}}]}}}}"#),
                None => format!(r#""{id}":{{"claims":{{}}}}"#),
            })
            .collect();
        format!(r#"{{"entities":{{{}}}}}"#, a.join(","))
    }
    fn reply(tokens: &[(&str, f64)]) -> String {
        let top: Vec<String> = tokens.iter().map(|(t, p)| format!(r#"{{"token":"{t}","logprob":{}}}"#, p.ln())).collect();
        format!(r#"{{"logprobs":[{{"top_logprobs":[{}]}}]}}"#, top.join(","))
    }
    /// Wikidata knows "ghost" as one app whose site is `site`.
    fn ghost(site: &str) -> Fake {
        Fake::default().with(&search_url("ghost"), &found(&[("Q1", "Ghost", "macOS app")])).with(&details_url("Q1"), &websites(&[("Q1", Some(site))]))
    }

    #[test]
    fn wikidata_json_is_read_carefully() {
        let v: Value = serde_json::from_str(&found(&[("Q1", "Obsidian", "notes app"), ("Q2", "Obsidian", "1996 video game")])).unwrap();
        assert_eq!(parse_entities(&v).len(), 2);
        let w: Value = serde_json::from_str(&websites(&[("Q1", Some("https://obsidian.md")), ("Q2", None)])).unwrap();
        assert_eq!(official_website(&w, "Q1").as_deref(), Some("https://obsidian.md"));
        assert_eq!(official_website(&w, "Q2"), None);
        assert_eq!(official_website(&w, "Q9"), None);
        assert!(parse_entities(&Value::Null).is_empty());
    }

    #[test]
    fn a_deprecated_website_claim_is_skipped() {
        let w: Value = serde_json::from_str(r#"{"entities":{"Q1":{"claims":{"P856":[{"rank":"deprecated","mainsnak":{"datavalue":{"value":"https://old.example"}}},{"rank":"preferred","mainsnak":{"datavalue":{"value":"https://new.example"}}}]}}}}"#).unwrap();
        assert_eq!(official_website(&w, "Q1").as_deref(), Some("https://new.example"));
    }

    #[test]
    fn exact_label_matches_win_over_fuzzy_ones_and_entities_without_a_site_are_dropped() {
        let r = Fake::default()
            .with(&search_url("Obsidian"), &found(&[("Q1", "Obsidian", "note-taking software"), ("Q2", "Obsidian", "1996 video game"), ("Q3", "Obsidian Portal", "RPG site")]))
            .with(&details_url("Q1%7CQ2"), &websites(&[("Q1", Some("https://obsidian.md")), ("Q2", None)]));
        let s = wikidata_sites(&r, "Obsidian");
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].host.as_str(), s[0].title.as_str()), ("obsidian.md", "Obsidian: note-taking software"));
    }

    #[test]
    fn github_projects_are_recognised() {
        assert_eq!(github_repo("https://github.com/o/ghost").as_deref(), Some("o/ghost"));
        assert_eq!(github_repo("https://github.com/o/ghost/releases").as_deref(), Some("o/ghost"));
        assert_eq!(github_repo("https://github.com/o"), None);
        assert_eq!(github_repo("https://notgithub.com/o/r"), None);
    }

    #[test]
    fn a_lone_wikidata_site_and_an_obvious_download_need_no_model() {
        let r = ghost("https://ghost.example/").with(&api("https://ghost.example/"), r#"<a href="/dl/Ghost-2.0-arm64.dmg">Download</a><a href="/dl/Ghost-2.0-x86_64.dmg">Intel</a><a href="/about">About</a>"#);
        match discover(&r, &cfg(), "ghost") {
            Outcome::Picked(d) => {
                assert_eq!(d.url, "https://ghost.example/dl/Ghost-2.0-arm64.dmg");
                assert_eq!((d.page.as_deref(), d.domain.as_str(), d.asset_pattern.as_deref(), d.source.as_str()), (Some("https://ghost.example/"), "ghost.example", Some("Ghost-*.dmg"), "Wikidata"));
                let rec = d.recipe("ghost");
                assert_eq!((rec.page.as_deref(), rec.github), (Some("https://ghost.example/"), None));
            }
            o => panic!("{o:?}"),
        }
        assert!(!r.log.borrow().iter().any(|c| c.contains("11434")), "no model call expected");
    }

    #[test]
    fn a_site_that_keeps_the_file_one_click_away_is_followed_one_level() {
        let r = ghost("https://ghost.example/")
            .with(&api("https://ghost.example/"), r#"<a href="/pricing">Pricing</a><a href="/download">Download Ghost</a><a href="https://other.example/download">elsewhere</a>"#)
            .with(&api("https://ghost.example/download"), r#"<a href="/files/Ghost.dmg">macOS</a>"#);
        match discover(&r, &cfg(), "ghost") {
            Outcome::Picked(d) => assert_eq!((d.url.as_str(), d.page.as_deref()), ("https://ghost.example/files/Ghost.dmg", Some("https://ghost.example/download"))),
            o => panic!("{o:?}"),
        }
        assert!(!r.log.borrow().iter().any(|c| c.contains("other.example")), "only the same site is followed");
    }

    #[test]
    fn a_page_that_only_links_to_github_releases_follows_them() {
        let r = ghost("https://ghost.example/")
            .with(&api("https://ghost.example/"), r#"<a href="https://github.com/sponsors/someone">Sponsor</a><a href="https://github.com/o/ghost/releases/latest">Download</a>"#)
            .with(&api("https://api.github.com/repos/o/ghost/releases/latest"), r#"{"tag_name":"v1.0","assets":[{"name":"Ghost.dmg","browser_download_url":"https://github.com/o/ghost/releases/download/v1.0/Ghost.dmg"}]}"#);
        match discover(&r, &cfg(), "ghost") {
            Outcome::Picked(d) => assert_eq!((d.github.as_deref(), d.url.as_str()), (Some("o/ghost"), "https://github.com/o/ghost/releases/download/v1.0/Ghost.dmg")),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn a_github_site_follows_the_release_feed() {
        let r = ghost("https://github.com/o/ghost").with(
            &api("https://api.github.com/repos/o/ghost/releases/latest"),
            r#"{"tag_name":"v3.1","assets":[{"name":"Ghost-arm64.zip","browser_download_url":"https://github.com/o/ghost/releases/download/v3.1/Ghost-arm64.zip","digest":"sha256:abc"},{"name":"Ghost-linux.tar.gz","browser_download_url":"https://github.com/x.tar.gz"}]}"#,
        );
        match discover(&r, &cfg(), "ghost") {
            Outcome::Picked(d) => {
                assert_eq!((d.github.as_deref(), d.version.as_str(), d.sha256.as_deref(), d.page.clone()), (Some("o/ghost"), "3.1", Some("abc"), None));
                assert_eq!(d.recipe("ghost").github.as_deref(), Some("o/ghost"));
            }
            o => panic!("{o:?}"),
        }
    }

    fn two_sites() -> Fake {
        Fake::default()
            .with(&search_url("ghost"), &found(&[("Q1", "Ghost", "blogging platform"), ("Q2", "Ghost", "macOS ghost writer app")]))
            .with(&details_url("Q1%7CQ2"), &websites(&[("Q1", Some("https://a.example/")), ("Q2", Some("https://b.example/"))]))
    }

    #[test]
    fn two_plausible_sites_go_to_the_model_which_must_survive_reversal() {
        let r = two_sites()
            .with(&api("https://b.example/"), r#"<a href="/Ghost.dmg">get</a>"#)
            .then(OLLAMA, &[&reply(&[("B", 0.96), ("A", 0.04)]), &reply(&[("A", 0.95), ("B", 0.05)])]);
        match discover(&r, &cfg(), "ghost") {
            Outcome::Picked(d) => assert_eq!(d.domain, "b.example"),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn two_plausible_sites_and_no_model_means_unsure() {
        assert!(matches!(discover(&two_sites(), &cfg(), "ghost"), Outcome::Unsure(m) if m.contains("official")));
    }

    #[test]
    fn unknown_to_wikidata_says_how_to_proceed() {
        let r = Fake::default().with(&search_url("ghost"), &found(&[]));
        assert!(matches!(discover(&r, &cfg(), "ghost"), Outcome::Unsure(m) if m.contains("give its https URL")));
        assert!(matches!(discover(&Fake::default(), &cfg(), "ghost"), Outcome::Unsure(_)), "a failed lookup is unsure, not a crash");
    }

    #[test]
    fn a_page_with_no_download_is_unsure() {
        let r = ghost("https://ghost.example/").with(&api("https://ghost.example/"), "<a href='/about'>About</a>");
        assert!(matches!(discover(&r, &cfg(), "ghost"), Outcome::Unsure(m) if m.contains("no macOS download")));
    }

    #[test]
    fn unresolved_template_links_are_never_downloads() {
        for u in ["https://x.example/app_${version}_mac.zip", "https://x.example/{{ file }}.dmg", "https://x.example/a%7Bv%7D.dmg", "https://x.example/{version}.dmg"] {
            assert!(is_template(u), "{u}");
        }
        assert!(!is_template("https://x.example/App-1.2.dmg"));
        let page = r#"<a href="https://x.example/app_${version}_mac.zip">x</a><a href="/real/App-1.0.dmg">y</a>"#;
        let l = download_links(page, "https://x.example/");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].1, "https://x.example/real/App-1.0.dmg");
    }

    #[test]
    fn plain_http_downloads_are_never_offered() {
        let r = ghost("https://ghost.example/").with(&api("https://ghost.example/"), r#"<a href="http://ghost.example/Ghost.dmg">get</a>"#);
        assert!(matches!(discover(&r, &cfg(), "ghost"), Outcome::Unsure(m) if m.contains("no macOS download")));
    }

    #[test]
    fn close_downloads_go_to_the_model() {
        let r = ghost("https://ghost.example/")
            .with(&api("https://ghost.example/"), r#"<a href="/a/Ghost-1.dmg">Ghost</a><a href="/b/Ghost-Lite-1.dmg">Ghost Lite</a>"#)
            .then(OLLAMA, &[&reply(&[("A", 0.95), ("B", 0.05)]), &reply(&[("B", 0.95), ("A", 0.05)])]);
        match discover(&r, &cfg(), "ghost") {
            Outcome::Picked(d) => assert_eq!(d.url, "https://ghost.example/a/Ghost-1.dmg"),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn a_url_that_is_a_download_is_taken_as_is_and_one_that_is_not_https_is_refused() {
        match from_url(&Fake::default(), &cfg(), "tool", "https://x.example/files/Tool-1.2.dmg") {
            Outcome::Picked(d) => assert_eq!((d.url.as_str(), d.asset_pattern.as_deref(), d.source.as_str()), ("https://x.example/files/Tool-1.2.dmg", Some("Tool-*.dmg"), "given URL")),
            o => panic!("{o:?}"),
        }
        assert!(matches!(from_url(&Fake::default(), &cfg(), "tool", "http://x.example/Tool.dmg"), Outcome::Unsure(_)));
    }

    #[test]
    fn a_url_that_is_a_page_is_scanned_without_any_search() {
        let r = Fake::default().with(&api("https://x.example/tool"), r#"<a href="/get/Tool.zip">macOS</a>"#);
        match from_url(&r, &cfg(), "tool", "https://x.example/tool") {
            Outcome::Picked(d) => assert_eq!((d.url.as_str(), d.page.as_deref()), ("https://x.example/get/Tool.zip", Some("https://x.example/tool"))),
            o => panic!("{o:?}"),
        }
        assert!(!r.log.borrow().iter().any(|c| c.contains("wikidata")));
    }
}
