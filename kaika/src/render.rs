//! Pages that only show their download after JavaScript runs.
//!
//! A plain fetch of such a page has no links. When (and only when) that happens, a Chromium-family browser that is
//! already installed renders it headlessly and prints the resulting page, which then goes through exactly the same
//! link scan and quarantine as any other page. Headless means no window and no focus theft; a throwaway profile
//! means no cookies, sign-ins or extensions are involved; nothing is downloaded or installed for it.

use crate::direct::ensure_https;
use crate::runner::Runner;
use std::path::{Path, PathBuf};

const KNOWN: [&str; 6] = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
    "/Applications/Arc.app/Contents/MacOS/Arc",
];

/// `KAIKA_BROWSER` (a path) wins, so tests and unusual installs can choose; otherwise the first known browser.
pub fn browser() -> Option<PathBuf> {
    browser_from(std::env::var("KAIKA_BROWSER").ok().as_deref(), &KNOWN)
}

pub fn browser_from(over: Option<&str>, known: &[&str]) -> Option<PathBuf> {
    if let Some(p) = over {
        return (p != "off" && Path::new(p).is_file()).then(|| PathBuf::from(p));
    }
    known
        .iter()
        .map(Path::new)
        .find(|p| p.is_file())
        .map(Path::to_path_buf)
}

/// A throwaway profile folder, unique to this process.
pub fn profile_dir() -> PathBuf {
    std::env::temp_dir().join(format!("kaika-render-{}", std::process::id()))
}

pub fn argv(browser: &Path, profile: &Path, url: &str) -> Vec<String> {
    vec![
        browser.display().to_string(),
        "--headless=new".into(),
        "--disable-gpu".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-extensions".into(),
        "--mute-audio".into(),
        format!("--user-data-dir={}", profile.display()),
        "--virtual-time-budget=8000".into(),
        "--dump-dom".into(),
        url.into(),
    ]
}

/// The page after its scripts ran, or None when there is no browser, the URL is not https, or rendering failed.
pub fn page(r: &dyn Runner, url: &str) -> Option<String> {
    let browser = browser()?;
    ensure_https(url).ok()?;
    let profile = profile_dir();
    let out = r.run(&argv(&browser, &profile, url));
    let _ = std::fs::remove_dir_all(&profile);
    let o = out.ok().filter(|o| o.ok())?;
    (!o.stdout.trim().is_empty()).then_some(o.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_wins_and_off_disables_rendering() {
        let me = std::env::current_exe().unwrap();
        let me = me.to_str().unwrap();
        assert_eq!(browser_from(Some(me), &["/nope"]), Some(PathBuf::from(me)));
        assert_eq!(
            browser_from(Some("off"), &[me]),
            None,
            "off means no browser even if one is installed"
        );
        assert_eq!(
            browser_from(Some("/does/not/exist"), &[me]),
            None,
            "a wrong override is not silently replaced"
        );
    }

    #[test]
    fn the_first_installed_known_browser_is_used_and_none_means_none() {
        let me = std::env::current_exe().unwrap();
        let me = me.to_str().unwrap();
        assert_eq!(
            browser_from(None, &["/nope/a", me, "/nope/b"]),
            Some(PathBuf::from(me))
        );
        assert_eq!(browser_from(None, &["/nope/a"]), None);
    }

    #[test]
    fn the_browser_runs_headless_with_a_throwaway_profile_and_no_extensions() {
        let a = argv(
            Path::new("/B/chrome"),
            Path::new("/tmp/p"),
            "https://x.example/",
        );
        for want in [
            "--headless=new",
            "--disable-extensions",
            "--user-data-dir=/tmp/p",
            "--dump-dom",
        ] {
            assert!(a.iter().any(|x| x == want), "{want}");
        }
        assert_eq!(a.last().unwrap(), "https://x.example/");
        assert_eq!(a[0], "/B/chrome");
    }
}
