//! Finding the vendor download for an app that no backend carries.
//!
//! Web search engines answer scripts with bot challenges, so kaika does not scrape them. Candidates come from
//! Wikidata, which records an official website for most notable software (structured, keyless, meant for
//! programs), then from GitHub repository search, or from a URL the user gives. Deterministic code collects
//! candidates (sites, links, release assets). A small local model may choose among peers (which site, which
//! download) through a `Decider`; it can only name a candidate that code offered. Whatever is found still goes
//! through the plan and every verification check before anything is installed.
//!
//! | module | job |
//! |---|---|
//! | `wikidata` | the official website of a named app |
//! | `github` | an established project of that name, and the project a page links to |
//! | `site` | a vendor page (or release feed) to its download |

mod github;
mod site;
mod wikidata;

pub use site::fast_path;

use crate::asset::{asset_score, file_name, link_pattern};
use crate::decision::Decider;
use crate::html;
use crate::net::{ensure_https, fetch};
use crate::outcome::Outcome;
use crate::recipe::Recipe;
use crate::runner::Runner;
use serde_json::Value;

/// A download found online, with how it was found and what to remember about it.
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
    /// The recipe that finds this app again next time without any lookup.
    pub fn recipe(&self, name: &str) -> Recipe {
        let base = Recipe {
            name: name.into(),
            domain: Some(self.domain.clone()),
            asset_pattern: self.asset_pattern.clone(),
            ..Default::default()
        };
        match (&self.github, &self.page) {
            (Some(g), _) => Recipe {
                github: Some(g.clone()),
                ..base
            },
            (None, Some(p)) => Recipe {
                page: Some(p.clone()),
                ..base
            },
            (None, None) => Recipe {
                url: Some(self.url.clone()),
                asset_pattern: None,
                ..base
            },
        }
    }
}

/// A page or feed, as text; `None` when it cannot be read (the caller decides what that means).
fn get(r: &dyn Runner, url: &str) -> Option<String> {
    fetch(r, url).ok()
}

fn get_json(r: &dyn Runner, url: &str) -> Option<Value> {
    serde_json::from_str(&get(r, url)?).ok()
}

/// " (ignored 2 candidates that contained instructions)" for the plan, or nothing.
fn ignored(dropped: &[String]) -> String {
    if dropped.is_empty() {
        String::new()
    } else {
        format!(
            "; ignored {} candidate{} that looked hostile (instructions aimed at the chooser, or a download-farm address): {}",
            dropped.len(),
            if dropped.len() == 1 { "" } else { "s" },
            dropped.join(", ")
        )
    }
}

/// Find an app that no backend carries by name: Wikidata's official website first, then GitHub.
pub fn discover(r: &dyn Runner, d: &dyn Decider, query: &str) -> Outcome<Discovered> {
    d.preload(); // the model loads while Wikidata and the vendor page are fetched
    let why_not_wikidata = match wikidata::discover(r, d, query) {
        Outcome::Unsure(why) => why,
        found => return found,
    };
    match github::discover(r, d, query) {
        Outcome::Unsure(why) => Outcome::Unsure(format!(
            "{why_not_wikidata}; {why}; if you know the vendor's page or download, give its https URL"
        )),
        found => found,
    }
}

/// An https URL the user gave: a download itself, a GitHub project, or a page to scan.
pub fn from_url(r: &dyn Runner, d: &dyn Decider, name: &str, url: &str) -> Outcome<Discovered> {
    d.preload();
    if let Err(e) = ensure_https(url) {
        return Outcome::Unsure(e.to_string());
    }
    let file = file_name(url);
    if asset_score(&file).is_some() {
        return Outcome::Picked(Discovered {
            name: name.into(),
            asset_pattern: Some(link_pattern(&file)),
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
    site::from_site(r, d, name, url, "given URL", "the page you gave")
}

#[cfg(test)]
mod tests;
