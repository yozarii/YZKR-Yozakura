//! Plain-text names: `kaika install chrome`, `kaika install chome`, `kaika install vscode`.
//!
//! When a name is not an exact package, or is only an unreviewed registry's, kaika looks it up among the reviewed
//! Homebrew formulae and casks by how people actually shorten and mistype names, and takes the match only when it
//! is overwhelmingly the most-installed one (Homebrew's public install counts). Nothing here is a model: it is a
//! word, typo or acronym match plus a popularity rule, so the answer is the same every time and the plan says why.

use crate::net::fetch;
use crate::paths::Dirs;
use crate::runner::Runner;
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, SystemTime};

const CASKS: &str = "https://formulae.brew.sh/api/analytics/cask-install/365d.json";
const FORMULAE: &str = "https://formulae.brew.sh/api/analytics/install-on-request/365d.json";
const KEEP: usize = 8000;
const FRESH: Duration = Duration::from_secs(7 * 24 * 3600);
/// A package installed fewer times than this in a year is too obscure to be guessed.
pub const MIN_INSTALLS: u64 = 1000;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// `cask` or `brew`: the kaika source.
    pub source: &'static str,
    pub name: String,
    pub installs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum How {
    /// The query is a whole word of the name: `chrome` in `google-chrome`.
    Word,
    /// The query is the name's leading initials plus its last words: `vscode` is `visual-studio-code`.
    Acronym,
    /// One or two letters off a word of the name: `chome` for `chrome`.
    Typo,
}

impl How {
    pub fn describe(self) -> &'static str {
        match self {
            How::Word => "a word of its name",
            How::Acronym => "its usual abbreviation",
            How::Typo => "a likely typo",
        }
    }
    /// How many times more installed than the runner-up a match must be.
    fn dominance(self) -> u64 {
        match self {
            How::Word | How::Acronym => 5,
            How::Typo => 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub entry: Entry,
    pub how: How,
    pub runner_up: Option<Entry>,
}

impl Found {
    pub fn why(&self, query: &str) -> String {
        let ratio = self
            .runner_up
            .as_ref()
            .map(|r| {
                format!(
                    ", {}x the next match {}",
                    self.entry.installs / r.installs.max(1),
                    r.name
                )
            })
            .unwrap_or_default();
        format!(
            "'{query}' is read as {}:{} ({}; {} installs a year{ratio})",
            self.entry.source,
            self.entry.name,
            self.how.describe(),
            self.entry.installs
        )
    }
}

fn norm(s: &str) -> String {
    crate::select::norm(s)
}

fn words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(norm)
        .collect()
}

fn distance(a: &str, b: &str, limit: usize) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > limit {
        return limit + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// How `query` could be a way of writing `name`, if at all. An exact match is not a "how": it is handled elsewhere.
pub fn how(query: &str, name: &str) -> Option<How> {
    let q = norm(query);
    if q.chars().count() < 4 || q == norm(name) {
        return None;
    }
    let w = words(name);
    if w.contains(&q) {
        return Some(How::Word);
    }
    for k in 1..w.len() {
        let initials: String = w[..k].iter().filter_map(|x| x.chars().next()).collect();
        if k >= 2 && format!("{initials}{}", w[k..].concat()) == q {
            return Some(How::Acronym);
        }
    }
    let limit = if q.chars().count() >= 8 { 2 } else { 1 };
    if q.chars().count() >= 5
        && (w
            .iter()
            .any(|x| x.chars().count() >= 5 && distance(&q, x, limit) <= limit)
            || distance(&q, &norm(name), limit) <= limit)
    {
        return Some(How::Typo);
    }
    None
}

/// The one overwhelmingly most-installed match, or the best few when there is no clear winner.
pub fn best(query: &str, entries: &[Entry]) -> Result<Found, Vec<Entry>> {
    let mut m: Vec<(Entry, How)> = entries
        .iter()
        .filter(|e| e.installs >= MIN_INSTALLS)
        .filter_map(|e| how(query, &e.name).map(|h| (e.clone(), h)))
        .collect();
    m.sort_by(|a, b| {
        b.0.installs
            .cmp(&a.0.installs)
            .then(a.0.name.cmp(&b.0.name))
    });
    let Some((top, h)) = m.first().cloned() else {
        return Err(vec![]);
    };
    let runner_up = m.get(1).map(|x| x.0.clone());
    match &runner_up {
        Some(r) if top.installs < r.installs.saturating_mul(h.dominance()) => {
            Err(m.into_iter().take(5).map(|x| x.0).collect())
        }
        _ => Ok(Found {
            entry: top,
            how: h,
            runner_up,
        }),
    }
}

pub fn parse_analytics(json: &str, key: &str, source: &'static str) -> Vec<Entry> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return vec![];
    };
    v["items"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|i| {
            let name = i[key].as_str()?.to_string();
            let installs = i["count"].as_str()?.replace(',', "").parse().ok()?;
            Some(Entry {
                source,
                name,
                installs,
            })
        })
        .take(KEEP)
        .collect()
}

fn to_tsv(es: &[Entry]) -> String {
    es.iter()
        .map(|e| format!("{}\t{}\t{}\n", e.source, e.name, e.installs))
        .collect()
}

fn from_tsv(t: &str) -> Vec<Entry> {
    t.lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            let source = match p.next()? {
                "cask" => "cask",
                "brew" => "brew",
                _ => return None,
            };
            Some(Entry {
                source,
                name: p.next()?.to_string(),
                installs: p.next()?.parse().ok()?,
            })
        })
        .collect()
}

fn fresh(p: &Path) -> bool {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < FRESH)
}

/// Install counts for the reviewed packages: from the week-old cache when there is one, otherwise fetched and
/// cached (the public files are megabytes, the cache is a few hundred kilobytes). A failed fetch falls back to a
/// stale cache, then to nothing.
pub fn popular(r: &dyn Runner, dirs: &Dirs) -> Vec<Entry> {
    let file = dirs.cache.join("popularity.tsv");
    if fresh(&file) {
        if let Ok(t) = std::fs::read_to_string(&file) {
            return from_tsv(&t);
        }
    }
    let mut all = vec![];
    for (url, key, source) in [(CASKS, "cask", "cask"), (FORMULAE, "formula", "brew")] {
        if let Ok(body) = fetch(r, url) {
            all.extend(parse_analytics(&body, key, source));
        }
    }
    if all.is_empty() {
        return std::fs::read_to_string(&file)
            .map(|t| from_tsv(&t))
            .unwrap_or_default();
    }
    let _ = std::fs::create_dir_all(&dirs.cache);
    let _ = std::fs::write(&file, to_tsv(&all));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(source: &'static str, name: &str, installs: u64) -> Entry {
        Entry {
            source,
            name: name.into(),
            installs,
        }
    }

    #[test]
    fn words_acronyms_and_typos_are_recognised() {
        assert_eq!(how("chrome", "google-chrome"), Some(How::Word));
        assert_eq!(how("vscode", "visual-studio-code"), Some(How::Acronym));
        assert_eq!(how("chome", "google-chrome"), Some(How::Typo));
        assert_eq!(how("firefx", "firefox"), Some(How::Typo));
        assert_eq!(how("obsidan", "obsidian"), Some(How::Typo));
    }

    #[test]
    fn near_misses_that_are_not_the_same_word_are_not_matches() {
        assert_eq!(how("chrome", "chromium"), None);
        assert_eq!(how("chrome", "archrome"), None);
        assert_eq!(how("ghost", "ghostty"), None);
        assert_eq!(how("git", "github"), None, "too short to guess from");
        assert_eq!(how("zen", "zed"), None);
        assert_eq!(
            how("chrome", "chrome"),
            None,
            "exact names are not fuzzy matches"
        );
        assert_eq!(how("spotify", "spotty"), None);
    }

    #[test]
    fn the_overwhelmingly_most_installed_match_wins() {
        let es = [
            e("cask", "google-chrome", 1_200_000),
            e("cask", "chrome-remote-desktop-host", 20_000),
            e("brew", "chrome-cli", 9_000),
            e("cask", "chromium", 80_000),
        ];
        let f = best("chrome", &es).unwrap();
        assert_eq!((f.entry.name.as_str(), f.how), ("google-chrome", How::Word));
        assert!(
            f.why("chrome").contains("cask:google-chrome") && f.why("chrome").contains("60x"),
            "{}",
            f.why("chrome")
        );
    }

    #[test]
    fn no_clear_winner_means_no_guess() {
        let es = [
            e("cask", "foo-studio", 50_000),
            e("cask", "studio-bar", 30_000),
        ];
        let list = best("studio", &es).unwrap_err();
        assert_eq!(list.len(), 2);
        assert!(best("nothingmatches", &es).unwrap_err().is_empty());
    }

    #[test]
    fn typos_need_a_bigger_lead_than_words() {
        let es = [e("cask", "firefox", 90_000), e("cask", "firefix", 20_000)];
        assert!(
            best("firefx", &es).is_err(),
            "4.5x is not enough for a typo"
        );
        let es = [e("cask", "firefox", 900_000), e("cask", "firefix", 20_000)];
        assert_eq!(best("firefx", &es).unwrap().entry.name, "firefox");
    }

    #[test]
    fn obscure_packages_are_never_guessed() {
        assert!(best("chrome", &[e("cask", "google-chrome", 999)]).is_err());
    }

    #[test]
    fn analytics_json_is_parsed_with_thousands_separators() {
        let j = r#"{"items":[{"number":1,"cask":"google-chrome","count":"1,200,000","percent":"1"},{"number":2,"cask":"x","count":"bad"}]}"#;
        assert_eq!(
            parse_analytics(j, "cask", "cask"),
            vec![e("cask", "google-chrome", 1_200_000)]
        );
        assert!(parse_analytics("not json", "cask", "cask").is_empty());
    }

    #[test]
    fn the_cache_round_trips() {
        let es = vec![e("cask", "google-chrome", 5), e("brew", "jq", 7)];
        assert_eq!(from_tsv(&to_tsv(&es)), es);
    }
}
