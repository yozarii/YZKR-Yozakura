//! Talking to the network: the connectivity probe, the HTTPS-only rule, and the one place kaika fetches text from the
//! web. Every fetch and download is `curl`, over HTTPS, with one User-Agent.
//!
//! The probe is asked before anything slow starts, so an offline run fails in about two seconds instead of after a
//! model has loaded or a solver has run.

use crate::runner::{Runner, argv};
use anyhow::{Result, bail};

/// What the check connects to: the same service discovery depends on.
pub const PROBE: &str = "https://www.wikidata.org/";

/// `KAIKA_OFFLINE=1` forces offline (for scripts and tests).
pub fn online(r: &dyn Runner) -> bool {
    if std::env::var("KAIKA_OFFLINE").as_deref() == Ok("1") {
        return false;
    }
    r.run(&argv(&[
        "curl",
        "-s",
        "-o",
        "/dev/null",
        "-I",
        "--connect-timeout",
        "2",
        "--max-time",
        "3",
        PROBE,
    ]))
    .map(|o| o.ok())
    .unwrap_or(false)
}

/// How kaika identifies itself to every site it reads.
pub const USER_AGENT: &str = "kaika/0.1 (package manager)";

/// Test hook: lets `file://` fixtures stand in for HTTPS so tests never touch the network.
pub fn test_mode() -> bool {
    std::env::var("KAIKA_TEST_MODE").as_deref() == Ok("1")
}

/// Refuse anything that is not an `https://` URL (or, in test mode, a `file://` fixture).
pub fn ensure_https(url: &str) -> Result<()> {
    if url.starts_with("https://") || (test_mode() && url.starts_with("file://")) {
        Ok(())
    } else {
        bail!("refusing non-HTTPS URL: {url}")
    }
}

/// Read a page or feed over HTTPS: the one place kaika fetches text from the web.
pub fn fetch(r: &dyn Runner, url: &str) -> Result<String> {
    ensure_https(url)?;
    let o = r.run(&argv(&[
        "curl",
        "-fsSL",
        "--proto-redir",
        "=https",
        "--max-time",
        "30",
        "-A",
        USER_AGENT,
        url,
    ]))?;
    if !o.ok() {
        bail!("could not fetch {url}: {}", o.stderr.trim());
    }
    Ok(o.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Fake;

    const CMD: &str =
        "curl -s -o /dev/null -I --connect-timeout 2 --max-time 3 https://www.wikidata.org/";

    #[test]
    fn online_when_the_probe_answers() {
        assert!(online(&Fake::default().with(CMD, "")));
    }

    #[test]
    fn offline_when_it_does_not() {
        assert!(!online(&Fake::default().failing(CMD)));
        assert!(
            !online(&Fake::default()),
            "a missing curl or a refused connection reads as offline"
        );
    }

    #[test]
    fn https_only_outside_test_mode() {
        assert!(ensure_https("https://x.com/a.dmg").is_ok());
        assert!(ensure_https("http://x.com/a.dmg").is_err());
        assert!(ensure_https("ftp://x.com/a.dmg").is_err());
        // file:// is refused unless KAIKA_TEST_MODE=1, which unit tests do not set.
        assert!(ensure_https("file:///tmp/a.zip").is_err());
    }
}
