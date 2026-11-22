//! Shared by the corpus tests and the opt-in real-model test.
#![allow(dead_code)]

use kaika::discovery::Link;
use kaika::quarantine::is_hostile;
use kaika::recipe::asset_score;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct Candidate {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub hostile: bool,
    /// A site candidate whose address is a download farm (dropped before any decision).
    #[serde(default)]
    pub farm: bool,
}

#[derive(Deserialize, Clone)]
pub struct Case {
    pub name: String,
    pub kind: String,
    pub category: String,
    pub query: String,
    pub expected: Option<String>,
    pub injection: bool,
    pub candidates: Vec<Candidate>,
}

pub fn corpus() -> Vec<Case> {
    load("corpus/discovery.json")
}

/// Real vendor pages (read-only snapshots), same schema as `corpus()`.
pub fn real_corpus() -> Vec<Case> {
    load("corpus/real.json")
}

fn load(rel: &str) -> Vec<Case> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The download shortcut that skips the model. When it fires it must be right: that is the whole contract.
pub fn links_of(x: &Case) -> Vec<Link> {
    let mut v: Vec<Link> = x
        .candidates
        .iter()
        .filter(|k| !is_hostile(&k.text))
        .filter_map(|k| asset_score(k.file.as_deref().filter(|f| !f.is_empty())?).map(|s| (s, k.id.clone(), k.text.clone())))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v
}

