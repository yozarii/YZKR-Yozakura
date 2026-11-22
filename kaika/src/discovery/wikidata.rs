//! Wikidata: the official website of a named app (structured, keyless, meant for programs).
//!
//! An exact label match is preferred over fuzzy ones and entities without a website are dropped, so "Obsidian" the
//! app is not "Obsidian" the mineral or the video games. One site is taken as it is; several go to the decider.

use super::{Discovered, get_json, ignored, site};
use crate::decision::{self, Cand, Decider, Pick, Task};
use crate::html;
use crate::outcome::Outcome;
use crate::runner::Runner;
use crate::select::norm;
use serde_json::Value;

const MAX_SITES: usize = 6;

/// A Wikidata search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: String,
    pub label: String,
    pub description: String,
}

/// `wbsearchentities` results.
pub(super) fn parse_entities(v: &Value) -> Vec<Entity> {
    v["search"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some(Entity {
                        id: e["id"].as_str()?.into(),
                        label: e["label"].as_str().unwrap_or("").into(),
                        description: e["description"].as_str().unwrap_or("").into(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The official website (property P856) of an entity from `wbgetentities`; deprecated claims are skipped.
pub(super) fn official_website(v: &Value, id: &str) -> Option<String> {
    v["entities"][id]["claims"]["P856"]
        .as_array()?
        .iter()
        .filter(|c| c["rank"].as_str() != Some("deprecated"))
        .find_map(|c| {
            c["mainsnak"]["datavalue"]["value"]
                .as_str()
                .map(String::from)
        })
}

/// An official website that Wikidata records for an entity.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub host: String,
    pub url: String,
    pub title: String,
}

pub(super) fn sites(r: &dyn Runner, query: &str) -> Vec<Site> {
    let search = format!(
        "https://www.wikidata.org/w/api.php?action=wbsearchentities&search={}&language=en&type=item&limit=7&format=json",
        html::url_encode(query)
    );
    let Some(found) = get_json(r, &search) else {
        return vec![];
    };
    let q = norm(query);
    let all = parse_entities(&found);
    let exact: Vec<Entity> = all
        .iter()
        .filter(|e| norm(&e.label) == q)
        .cloned()
        .collect();
    let entities = if exact.is_empty() { all } else { exact };
    if entities.is_empty() {
        return vec![];
    }
    let ids: Vec<&str> = entities.iter().map(|e| e.id.as_str()).collect();
    let details = format!(
        "https://www.wikidata.org/w/api.php?action=wbgetentities&ids={}&props=claims&format=json",
        ids.join("%7C")
    );
    let Some(d) = get_json(r, &details) else {
        return vec![];
    };
    let mut seen: Vec<String> = vec![];
    let mut out = vec![];
    for e in &entities {
        let Some(url) = official_website(&d, &e.id) else {
            continue;
        };
        let host = html::host(&url);
        if host.is_empty() || seen.contains(&host) {
            continue;
        }
        seen.push(host.clone());
        out.push(Site {
            host,
            url,
            title: format!("{}: {}", e.label, e.description),
        });
    }
    out.truncate(MAX_SITES);
    out
}

/// Which site is official? One candidate is taken as is; several go to the decision step.
fn choose_site(d: &dyn Decider, query: &str, sites: &[Site]) -> (Outcome<Pick>, Vec<String>) {
    let cands: Vec<Cand> = sites
        .iter()
        .map(|s| {
            Cand::new(
                s.host.clone(),
                format!("{}: {} ({})", s.host, s.title, s.url),
            )
        })
        .collect();
    let alone = cands.len() == 1;
    let j = decision::decide(d, Task::Site, query, cands);
    let outcome = match j.outcome {
        // The one Wikidata match for the name is the official site, unless it was dropped (then it is not alone any more).
        Outcome::Picked(p) if alone => Outcome::Picked(Pick {
            id: p.id,
            how: format!("the only Wikidata match for \"{query}\" with an official website"),
        }),
        o => o,
    };
    (outcome, j.dropped)
}

/// The app's official website, from Wikidata, and its download.
pub(super) fn discover(r: &dyn Runner, d: &dyn Decider, query: &str) -> Outcome<Discovered> {
    let sites = sites(r, query);
    if sites.is_empty() {
        return Outcome::Unsure(format!("Wikidata has no official website for \"{query}\""));
    }
    let (chosen, dropped_sites) = choose_site(d, query, &sites);
    let site = match chosen {
        Outcome::Picked(p) => p,
        Outcome::Unsure(why) => {
            return Outcome::Unsure(format!("could not tell which site is official: {why}"));
        }
    };
    let Some(url) = sites.iter().find(|s| s.host == site.id).map(|s| &s.url) else {
        return Outcome::Unsure("the chooser named a site that was not offered".into());
    };
    site::from_site(
        r,
        d,
        query,
        url,
        "Wikidata",
        &format!("{}{}", site.how, ignored(&dropped_sites)),
    )
}
