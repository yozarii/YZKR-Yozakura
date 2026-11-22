//! A vendor's site to its macOS download.
//!
//! The page is fetched and its links listed; if it has none, the strategies below are tried in order, each only when
//! the one before found nothing: buttons that redirect to the file, a "download" page one level deeper, the page as a
//! headless browser renders it (only if one is installed), the GitHub project the page links to. Links that talk to
//! the chooser are dropped before anything is scored or decided, so a hostile link cannot win by scoring well.

use super::{Discovered, get, github, ignored};
use crate::asset::{file_name, link_pattern};
use crate::decision::{self, Cand, Decider, Pick, Task};
use crate::html;
use crate::links::{Link, download_links, download_pages, redirected_links};
use crate::outcome::Outcome;
use crate::quarantine;
use crate::recipe::{self, Recipe};
use crate::render;
use crate::runner::Runner;

/// Said in the plan when the links only exist after the page's scripts ran.
const RENDERED: &str =
    "; the page builds its links with JavaScript, so it was rendered in a headless browser";

/// The deterministic shortcut for downloads: a lone link, or one that clearly outscores the next. `links` is
/// best first. Everything else goes to the decision step.
pub fn fast_path(links: &[Link]) -> Option<usize> {
    (!links.is_empty() && (links.len() == 1 || links[0].0 - links[1].0 >= 8)).then_some(0)
}

/// Links whose text addresses the chooser are dropped before anything is scored or decided, including by the
/// deterministic fast path: an attacker's link must not win just by scoring well. The dropped URLs are added to
/// `dropped` for the plan to report.
fn quarantined(links: Vec<Link>, dropped: &mut Vec<String>) -> Vec<Link> {
    let (clean, bad): (Vec<Link>, Vec<Link>) = links
        .into_iter()
        .partition(|(_, u, t)| !quarantine::is_hostile(&format!("{} {}", file_name(u), t)));
    dropped.extend(bad.into_iter().map(|l| l.1));
    clean
}

/// Download links on a page, plus buttons whose address names no file but redirects to it ("Download for macOS").
fn links_on(r: &dyn Runner, html: &str, url: &str, dropped: &mut Vec<String>) -> Vec<Link> {
    let links = quarantined(download_links(html, url), dropped);
    if !links.is_empty() {
        return links;
    }
    quarantined(redirected_links(r, html, url), dropped)
}

/// Many sites keep the file one click away, on a download page: the first of at most two same-site candidates
/// (read with `fetch`) that has links. Returns the links and the page they were on.
fn follow_download_pages(
    html: &str,
    site_url: &str,
    fetch: impl Fn(&str) -> Option<String>,
    dropped: &mut Vec<String>,
) -> Option<(Vec<Link>, String)> {
    for next in download_pages(html, site_url) {
        let Some(page) = fetch(&next) else { continue };
        let found = quarantined(download_links(&page, &next), dropped);
        if !found.is_empty() {
            return Some((found, next));
        }
    }
    None
}

/// Where the download links were found.
struct Scan {
    links: Vec<Link>,
    /// The page to re-read for updates.
    page_url: String,
    /// Whether a browser had to render the page.
    rendered: bool,
}

/// Find a page's download links, trying the strategies in order.
fn scan(r: &dyn Runner, site_url: &str, home: &str, dropped: &mut Vec<String>) -> Option<Scan> {
    let links = links_on(r, home, site_url, dropped);
    if !links.is_empty() {
        return Some(Scan {
            links,
            page_url: site_url.into(),
            rendered: false,
        });
    }
    if let Some((links, page_url)) = follow_download_pages(home, site_url, |u| get(r, u), dropped) {
        return Some(Scan {
            links,
            page_url,
            rendered: false,
        });
    }
    // A page that builds its links with JavaScript has none in the plain text: render it, if a browser is installed.
    let shown = render::page(r, site_url)?;
    let links = links_on(r, &shown, site_url, dropped);
    if !links.is_empty() {
        return Some(Scan {
            links,
            page_url: site_url.into(),
            rendered: true,
        });
    }
    let render_or_get = |u: &str| render::page(r, u).or_else(|| get(r, u));
    follow_download_pages(&shown, site_url, render_or_get, dropped).map(|(links, page_url)| Scan {
        links,
        page_url,
        rendered: true,
    })
}

/// An official site (or project page) to a download: its release feed if it is a GitHub project, otherwise the
/// best link on its page. `how` says how the site was chosen, for the plan.
pub(super) fn from_site(
    r: &dyn Runner,
    d: &dyn Decider,
    query: &str,
    site_url: &str,
    source: &str,
    how: &str,
) -> Outcome<Discovered> {
    if let Some(repo) = github::repo_of(site_url) {
        return from_release_feed(r, query, site_url, repo, source, how);
    }
    let Some(home) = get(r, site_url) else {
        return Outcome::Unsure(format!("could not read {site_url}"));
    };
    let mut dropped = vec![];
    let Some(found) = scan(r, site_url, &home, &mut dropped) else {
        // Many vendor pages just point at the project's GitHub releases.
        if let Some(repo_url) = github::project_linked_from(&home) {
            let how = format!("{how}; its page links to the GitHub project");
            return from_site(r, d, query, &repo_url, source, &how);
        }
        let js_hint = if render::browser().is_none() {
            "; if the page needs JavaScript, kaika can read it with a Chromium-based browser (Chrome, Brave, Edge, Chromium), none is installed"
        } else {
            ""
        };
        return Outcome::Unsure(format!(
            "no macOS download found on {site_url}{}{js_hint}",
            ignored(&dropped)
        ));
    };
    let download = match pick_download(d, query, &found.links, &mut dropped) {
        Outcome::Picked(p) => p,
        Outcome::Unsure(why) => return Outcome::Unsure(why),
    };
    let how_extra = if found.rendered { RENDERED } else { "" };
    Outcome::Picked(Discovered {
        name: query.into(),
        asset_pattern: Some(link_pattern(&file_name(&download.id))),
        url: download.id,
        version: String::new(),
        sha256: None,
        domain: html::host(site_url),
        page: Some(found.page_url),
        github: None,
        source: source.into(),
        evidence: format!(
            "official site {} ({how}{how_extra}); {}{}",
            html::host(site_url),
            download.how,
            ignored(&dropped)
        ),
    })
}

/// The best download among a page's links: the shortcut when one clearly wins, otherwise the decider.
fn pick_download(
    d: &dyn Decider,
    query: &str,
    links: &[Link],
    dropped: &mut Vec<String>,
) -> Outcome<Pick> {
    if let Some(i) = fast_path(links) {
        return Outcome::Picked(Pick {
            id: links[i].1.clone(),
            how: format!(
                "clearly the best macOS download: {}",
                file_name(&links[i].1)
            ),
        });
    }
    let cands: Vec<Cand> = links
        .iter()
        .take(decision::MAX_CANDIDATES + 4)
        .map(|(_, u, t)| Cand::new(u.clone(), format!("{} {}", file_name(u), t)))
        .collect();
    let j = decision::decide(d, Task::Download, query, cands);
    dropped.extend(j.dropped);
    match j.outcome {
        Outcome::Picked(p) => Outcome::Picked(p),
        Outcome::Unsure(why) => {
            Outcome::Unsure(format!("could not tell which download to use: {why}"))
        }
    }
}

/// A GitHub project: its latest release's best macOS asset (which also names the checksum).
fn from_release_feed(
    r: &dyn Runner,
    query: &str,
    site_url: &str,
    repo: String,
    source: &str,
    how: &str,
) -> Outcome<Discovered> {
    let rec = Recipe {
        name: query.into(),
        github: Some(repo.clone()),
        ..Default::default()
    };
    match recipe::latest(r, &rec) {
        Ok(a) => Outcome::Picked(Discovered {
            name: query.into(),
            asset_pattern: Some(link_pattern(&a.name)),
            url: a.url,
            version: a.version,
            sha256: a.sha256,
            domain: "github.com".into(),
            page: None,
            github: Some(repo),
            source: source.into(),
            evidence: format!(
                "official site github.com ({how}); the latest release's best macOS asset {}",
                a.name
            ),
        }),
        Err(e) => Outcome::Unsure(format!(
            "no macOS download in the release feed of {site_url}: {e:#}"
        )),
    }
}
