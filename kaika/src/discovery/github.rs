//! GitHub: the second source, for apps Wikidata does not know (or whose site has no download), and the project a
//! vendor page points at.
//!
//! Only a repository whose own name equals the app's name, that is not a fork or archived and has enough stars is
//! offered. A lone one is taken, several go to the decider (which may refuse all), and descriptions pass the same
//! quarantine as any other candidate text.

use super::{Discovered, get_json, ignored, site};
use crate::decision::{self, Cand, Decider, Pick, Task};
use crate::html;
use crate::outcome::Outcome;
use crate::runner::Runner;
use crate::select::norm;
use serde_json::Value;

/// A repository must have at least this many stars to be offered as an app's home: a handful of stars proves nothing.
pub const MIN_STARS: u64 = 50;

const MAX_REPOS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
    pub full_name: String,
    pub description: String,
    pub stars: u64,
}

/// `search/repositories` results, screened deterministically: the repository's own name must be the app's name
/// (case, spaces and hyphens aside), and forks, archives and barely-starred projects are never offered.
/// Most-starred first.
pub(super) fn parse_repos(v: &Value, query: &str) -> Vec<Repo> {
    let q = norm(query);
    let mut out: Vec<Repo> = v["items"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter(|i| i["fork"].as_bool() != Some(true) && i["archived"].as_bool() != Some(true))
        .filter(|i| i["name"].as_str().is_some_and(|n| norm(n) == q))
        .filter_map(|i| {
            Some(Repo {
                full_name: i["full_name"].as_str()?.to_string(),
                description: i["description"].as_str().unwrap_or("").to_string(),
                stars: i["stargazers_count"].as_u64().unwrap_or(0),
            })
        })
        .filter(|r| r.stars >= MIN_STARS && r.full_name.matches('/').count() == 1)
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.stars));
    out.truncate(MAX_REPOS);
    out
}

fn search(r: &dyn Runner, query: &str) -> Vec<Repo> {
    let url = format!(
        "https://api.github.com/search/repositories?q={}+in:name&sort=stars&per_page=10",
        html::url_encode(query)
    );
    get_json(r, &url)
        .map(|v| parse_repos(&v, query))
        .unwrap_or_default()
}

pub(super) fn repo_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    (html::host(url) == "github.com").then_some(())?;
    let mut parts = rest.split('/').skip(1);
    let (o, r) = (
        parts.next().filter(|x| !x.is_empty())?,
        parts.next().filter(|x| !x.is_empty())?,
    );
    Some(format!("{o}/{r}"))
}

/// The first link on a page that points at a GitHub project (not at GitHub's own pages).
pub(super) fn project_linked_from(page: &str) -> Option<String> {
    const NOT_PROJECTS: [&str; 10] = [
        "sponsors",
        "features",
        "orgs",
        "topics",
        "about",
        "login",
        "pricing",
        "marketplace",
        "settings",
        "apps",
    ];
    html::anchors(page)
        .into_iter()
        .map(|a| a.href)
        .filter(|h| h.starts_with("https://"))
        .find_map(|h| {
            let repo = repo_of(&h)?;
            let owner = repo.split('/').next()?;
            (!NOT_PROJECTS.contains(&owner)).then(|| format!("https://github.com/{repo}"))
        })
}

/// Which repository is the project? Several go to the decision step, which may say none is.
fn choose_repo(d: &dyn Decider, query: &str, repos: &[Repo]) -> (Outcome<Pick>, Vec<String>) {
    let cands: Vec<Cand> = repos
        .iter()
        .map(|p| {
            Cand::new(
                p.full_name.clone(),
                format!("{}: {} ({} stars)", p.full_name, p.description, p.stars),
            )
        })
        .collect();
    let alone = cands.len() == 1;
    let j = decision::decide(d, Task::Site, query, cands);
    let outcome = match j.outcome {
        Outcome::Picked(p) if alone => {
            let stars = repos
                .iter()
                .find(|x| x.full_name == p.id)
                .map_or(0, |x| x.stars);
            Outcome::Picked(Pick {
                id: p.id,
                how: format!(
                    "the only starred, unarchived, non-fork GitHub repository named \"{query}\" ({stars} stars)"
                ),
            })
        }
        o => o,
    };
    (outcome, j.dropped)
}

/// The second source: GitHub repository search, for apps Wikidata does not know (or whose site has no download).
pub(super) fn discover(r: &dyn Runner, d: &dyn Decider, query: &str) -> Outcome<Discovered> {
    let repos = search(r, query);
    if repos.is_empty() {
        return Outcome::Unsure(format!(
            "GitHub has no established project named \"{query}\""
        ));
    }
    let (chosen, dropped) = choose_repo(d, query, &repos);
    match chosen {
        Outcome::Picked(p) => site::from_site(
            r,
            d,
            query,
            &format!("https://github.com/{}", p.id),
            "GitHub search",
            &format!("{}{}", p.how, ignored(&dropped)),
        ),
        Outcome::Unsure(why) => Outcome::Unsure(format!(
            "could not tell which GitHub project is the app: {why}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_projects_are_recognised() {
        assert_eq!(
            repo_of("https://github.com/o/ghost").as_deref(),
            Some("o/ghost")
        );
        assert_eq!(
            repo_of("https://github.com/o/ghost/releases").as_deref(),
            Some("o/ghost")
        );
        assert_eq!(repo_of("https://github.com/o"), None);
        assert_eq!(repo_of("https://notgithub.com/o/r"), None);
    }
}
