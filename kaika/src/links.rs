//! Download links on a page: finding them, screening them, ranking them.
//!
//! The same rules serve discovery (a vendor page found online) and recipes (re-reading a page at upgrade time):
//! HTTPS only, no unresolved script placeholders, nothing whose own words say it is for another system, a
//! repository's source snapshot is not an app, and within one product the newest release wins.

use crate::asset::{asset_score, file_name, not_for_mac, product_stem, version_of};
use crate::html;
use crate::net::{USER_AGENT, ensure_https};
use crate::runner::{Runner, argv};

/// A download link on a page: (score, absolute https URL, link text).
pub type Link = (i32, String, String);

/// Pages built with scripts leave unresolved placeholders in their markup (`${version}`, `{{ version }}`).
/// Those are not downloads.
pub fn is_template(url: &str) -> bool {
    let u = url.to_lowercase();
    ["{", "}", "%7b", "%7d", "<%"].iter().any(|m| u.contains(m))
}

/// Download links on a page: (score, absolute https URL, link text), best first.
pub fn download_links(page: &str, base: &str) -> Vec<Link> {
    let mut links: Vec<Link> = html::anchors(page)
        .into_iter()
        .filter(|a| !a.href.is_empty())
        .map(|a| (resolve_url(base, &a.href), a.text))
        .filter(|(u, _)| ensure_https(u).is_ok() && !is_template(u))
        .filter(|(_, t)| !not_for_mac(t))
        .filter(|(u, _)| !u.contains("/archive/")) // a repository's source snapshot, not the app
        .filter_map(|(u, t)| asset_score(&file_name(&u)).map(|s| (s, u, t)))
        .collect();
    demote_older_versions(&mut links);
    links.dedup_by(|a, b| a.1 == b.1);
    links
}

/// A page that lists several releases of one product (a current one and "older macOS" builds) is meant to be read
/// newest first: within each product, every link below the highest version loses 20 points. Different products
/// ("Foo" and "Foo Pro") are never compared. Then the list is sorted.
pub fn demote_older_versions(links: &mut [Link]) {
    let keyed: Vec<(String, Option<Vec<u32>>)> = links
        .iter()
        .map(|l| {
            let name = file_name(&l.1);
            (product_stem(&name), version_of(&name))
        })
        .collect();
    let newest = |stem: &str| {
        keyed
            .iter()
            .filter(|k| k.0 == stem)
            .filter_map(|k| k.1.clone())
            .max()
    };
    let demote: Vec<bool> = keyed
        .iter()
        .map(|(stem, v)| {
            v.as_ref()
                .is_some_and(|v| newest(stem).is_some_and(|n| *v < n))
        })
        .collect();
    for (l, d) in links.iter_mut().zip(demote) {
        if d {
            l.0 -= 20;
        }
    }
    links.sort_by_key(|l| std::cmp::Reverse(l.0));
}

/// Links that probably lead to a download page, on the same site, for sites that keep the file one click away.
pub fn download_pages(page: &str, base: &str) -> Vec<String> {
    let host = html::host(base);
    let mut out: Vec<String> = vec![];
    for a in html::anchors(page) {
        let url = resolve_url(base, &a.href);
        let wants = a.text.to_lowercase().contains("download")
            || a.href.to_lowercase().contains("download");
        if wants
            && ensure_https(&url).is_ok()
            && html::host(&url) == host
            && url != base
            && !out.contains(&url)
            && asset_score(&file_name(&url)).is_none()
        {
            out.push(url);
        }
    }
    out.truncate(2);
    out
}

/// A link's absolute URL, resolved against the page it was found on.
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

/// Links that name no file but say they are the macOS download (`/api/download?platform=osx`,
/// `/updates/download/darwin-arm64/app`, a button "Download for macOS"): at most four, in page order.
pub fn extensionless_candidates(page: &str, base: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for a in crate::html::anchors(page) {
        let url = resolve_url(base, &a.href.replace("&amp;", "&"));
        if a.href.is_empty()
            || ensure_https(&url).is_err()
            || asset_score(&file_name(&url)).is_some()
        {
            continue;
        }
        let (u, t) = (url.to_lowercase(), a.text.to_lowercase());
        if ["ptb", "canary", "beta", "nightly", "insider", "preview"]
            .iter()
            .any(|w| u.contains(&format!("={w}")) || u.contains(&format!("/{w}")))
        {
            continue; // test channels are never the default download
        }
        let says_download = u.contains("download") || t.contains("download");
        let says_mac = [
            "darwin",
            "osx",
            "macos",
            "/mac",
            "mac-",
            "mac_",
            "=mac",
            "apple silicon",
            "for mac",
        ]
        .iter()
        .any(|w| u.contains(w) || t.contains(w));
        if says_download && says_mac && !out.iter().any(|(x, _)| *x == url) {
            out.push((url, a.text));
        }
    }
    // The Apple-silicon build first: the four slots go to the likeliest links.
    out.sort_by_key(|(u, _)| {
        !(u.contains("arm64") || u.contains("aarch64") || u.contains("universal"))
    });
    out.truncate(4);
    out
}

/// Where a download link really leads: the final URL after redirects, fetching one byte of the file at most
/// (some servers refuse HEAD requests).
pub fn final_url(r: &dyn Runner, url: &str) -> Option<String> {
    ensure_https(url).ok()?;
    let o = r
        .run(&argv(&[
            "curl",
            "-sL",
            "--proto-redir",
            "=https",
            "-r",
            "0-0",
            "-o",
            "/dev/null",
            "-w",
            "%{url_effective}",
            "--max-time",
            "20",
            "-A",
            USER_AGENT,
            url,
        ]))
        .ok()
        .filter(|o| o.ok())?;
    let last = o.stdout.trim().to_string();
    (ensure_https(&last).is_ok() && asset_score(&file_name(&last)).is_some()).then_some(last)
}

/// `extensionless_candidates` followed to the real files, scored like any other link.
pub fn redirected_links(r: &dyn Runner, page: &str, base: &str) -> Vec<Link> {
    let mut out: Vec<Link> = vec![];
    for (url, text) in extensionless_candidates(page, base) {
        let Some(real) = final_url(r, &url) else {
            continue;
        };
        if out.iter().any(|l| l.1 == real) {
            continue;
        }
        if let Some(score) = asset_score(&file_name(&real)) {
            out.push((score, real, text));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_template_links_are_never_downloads() {
        for u in [
            "https://x.example/app_${version}_mac.zip",
            "https://x.example/{{ file }}.dmg",
            "https://x.example/a%7Bv%7D.dmg",
            "https://x.example/{version}.dmg",
        ] {
            assert!(is_template(u), "{u}");
        }
        assert!(!is_template("https://x.example/App-1.2.dmg"));
        let page = r#"<a href="https://x.example/app_${version}_mac.zip">x</a><a href="/real/App-1.0.dmg">y</a>"#;
        let l = download_links(page, "https://x.example/");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].1, "https://x.example/real/App-1.0.dmg");
    }

    #[test]
    fn a_link_that_says_it_is_for_another_system_is_not_a_mac_download() {
        for t in [
            "Download for Windows",
            "Linux (x64)",
            "Windows 64-bit installer",
            "Ubuntu / Debian",
        ] {
            assert!(not_for_mac(t), "{t}");
        }
        for t in [
            "Download",
            "Download for macOS",
            "Windows, macOS and Linux",
            "Apple silicon",
            "Universal",
            "",
        ] {
            assert!(!not_for_mac(t), "{t}");
        }
        let page = r#"<a href="/putty-0.81.zip">Download for Windows</a><a href="/Ghost-1.0.zip">Download for Mac</a>"#;
        let got: Vec<String> = download_links(page, "https://x.example/")
            .into_iter()
            .map(|l| l.1)
            .collect();
        assert_eq!(got, ["https://x.example/Ghost-1.0.zip"]);
        assert!(
            download_links(
                r#"<a href="https://github.com/o/r/archive/v1.0.zip">Source</a>"#,
                "https://x.example/"
            )
            .is_empty(),
            "source snapshots are not apps"
        );
        for n in [
            "winmerge-2.16-exe.zip",
            "tool-win.zip",
            "Tool_win64.zip",
            "tool.msi.zip",
        ] {
            assert_eq!(asset_score(n), None, "{n}");
        }
        for n in ["Winamp-1.0.dmg", "Darwin-Tool.zip", "Swing-1.0.dmg"] {
            assert!(
                asset_score(n).is_some(),
                "{n} only contains the letters, not the word"
            );
        }
        assert!(
            download_links(
                r#"<a href="/putty-0.81.zip">Windows</a>"#,
                "https://x.example/"
            )
            .is_empty(),
            "a Windows-only page has no Mac download"
        );
    }

    #[test]
    fn different_products_on_one_page_are_never_ranked_by_version() {
        let page = r#"<a href="/Rectangle2.0.5.dmg">Download</a><a href="/Rectangle%20Pro%203.102.dmg">Download Pro</a>"#;
        let l = download_links(page, "https://x.example/");
        assert_eq!(product_stem("Rectangle2.0.5.dmg"), "rectangle");
        assert_eq!(product_stem("Rectangle Pro 3.102.dmg"), "rectangle-pro");
        assert_eq!(
            product_stem("Wireshark 4.4.20 Arm 64.dmg"),
            product_stem("Wireshark 4.6.10.dmg")
        );
        assert!(l.iter().all(|x| x.0 >= 20), "nobody was demoted: {l:?}");
    }

    #[test]
    fn only_links_that_say_download_and_mac_are_followed() {
        let page = r#"<a href="/download/mac">x</a><a href="/blog/macos-tips">Read more</a><a href="/dl?os=darwin-arm64&download=1">y</a><a href="/download/windows">Windows</a><a href="http://x.example/download/mac">plain</a>"#;
        let got: Vec<String> = extensionless_candidates(page, "https://ghost.example/")
            .into_iter()
            .map(|x| x.0)
            .collect();
        assert_eq!(
            got,
            [
                "https://ghost.example/dl?os=darwin-arm64&download=1",
                "https://ghost.example/download/mac"
            ],
            "the Apple-silicon link goes first"
        );
    }

    #[test]
    fn entity_escaped_addresses_test_channels_and_the_apple_silicon_build_are_handled() {
        let page = r#"<a href="/api/dl?channel=ptb&amp;platform=osx&amp;download=1">PTB</a><a href="/api/dl?channel=stable&amp;platform=osx&amp;arch=x64&amp;download=1">Intel</a><a href="/api/dl?channel=stable&amp;platform=osx&amp;arch=arm64&amp;download=1">Apple silicon</a>"#;
        let got: Vec<String> = extensionless_candidates(page, "https://ghost.example/")
            .into_iter()
            .map(|x| x.0)
            .collect();
        assert_eq!(
            got,
            [
                "https://ghost.example/api/dl?channel=stable&platform=osx&arch=arm64&download=1",
                "https://ghost.example/api/dl?channel=stable&platform=osx&arch=x64&download=1"
            ]
        );
    }

    #[test]
    fn links_resolve_against_the_page() {
        let base = "https://x.com/get/page.html";
        assert_eq!(resolve_url(base, "/dl/App.dmg"), "https://x.com/dl/App.dmg");
        assert_eq!(
            resolve_url(base, "rel/App2.zip"),
            "https://x.com/get/rel/App2.zip"
        );
        assert_eq!(
            resolve_url(base, "https://cdn.x.com/a.pkg"),
            "https://cdn.x.com/a.pkg"
        );
        assert_eq!(
            resolve_url(base, "//cdn.x.com/b.pkg"),
            "https://cdn.x.com/b.pkg"
        );
    }

    #[test]
    fn no_fetch_may_be_redirected_off_https() {
        // `curl -L` follows an https page's redirect to plain http unless told otherwise: a downgrade.
        let r = crate::runner::Fake::default();
        let _ = crate::net::fetch(&r, "https://x.example/");
        let _ = final_url(&r, "https://x.example/dl");
        let log = r.log.borrow();
        assert_eq!(log.len(), 2);
        assert!(
            log.iter().all(|c| c.contains("--proto-redir =https")),
            "{log:?}"
        );
    }
}
