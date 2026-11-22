//! Trust checks for downloads. Deterministic: no model is ever consulted here.

use crate::runner::{Runner, argv};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// What a download must be: signed by this Team ID, when one is expected.
#[derive(Debug, Default, Clone)]
pub struct Expect {
    pub team_id: Option<String>,
}

/// What the checks found.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub signature: bool,
    pub notarized: bool,
    pub team_id: Option<String>,
    pub failures: Vec<String>,
}

impl Report {
    /// Did every check pass?
    pub fn ok(&self) -> bool {
        self.failures.is_empty()
    }
}

/// The signing Team ID from `codesign -dv` output.
pub fn parse_team_id(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "not set")
        .map(String::from)
}

/// `Developer ID Installer: Name (TEAMID)` to `TEAMID`.
pub fn parse_pkg_team(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.contains("Developer ID Installer"))
        .and_then(|l| {
            let (o, c) = (l.rfind('(')?, l.rfind(')')?);
            (o < c).then(|| l[o + 1..c].to_string())
        })
}

/// The SHA-256 of a file, as lowercase hex.
pub fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `Some(reason)` when a published checksum exists and does not match.
pub fn check_checksum(file: &Path, expected: Option<&str>) -> Result<Option<String>> {
    let Some(want) = expected else {
        return Ok(None);
    };
    let got = sha256_file(file)?;
    Ok((!got.eq_ignore_ascii_case(want.trim()))
        .then(|| format!("checksum mismatch: expected {want}, got {got}")))
}

fn combined(o: &crate::runner::Out) -> String {
    format!("{}\n{}", o.stdout, o.stderr)
}

fn team_check(report: &mut Report, expect: &Expect) {
    match (&expect.team_id, &report.team_id) {
        (Some(e), Some(t)) if e != t => report.failures.push(format!(
            "signing Team ID {t} does not match the expected {e}"
        )),
        (Some(e), None) => report
            .failures
            .push(format!("no signing Team ID found; expected {e}")),
        _ => {}
    }
}

/// Verify an app: valid signature, notarized, and the expected Team ID.
pub fn check_app(r: &dyn Runner, app: &Path, expect: &Expect) -> Report {
    let p = app.to_string_lossy().to_string();
    let mut rep = Report {
        signature: r
            .run(&argv(&["codesign", "--verify", "--deep", "--strict", &p]))
            .map(|o| o.ok())
            .unwrap_or(false),
        ..Default::default()
    };
    if !rep.signature {
        rep.failures
            .push("code signature is missing or invalid".into());
    }
    if let Ok(o) = r.run(&argv(&["codesign", "-dv", "--verbose=4", &p])) {
        rep.team_id = parse_team_id(&combined(&o));
    }
    if let Ok(o) = r.run(&argv(&["spctl", "-a", "-vv", &p])) {
        rep.notarized = o.ok() && combined(&o).contains("source=Notarized Developer ID");
    }
    if !rep.notarized {
        rep.failures.push("not notarized by Apple".into());
    }
    team_check(&mut rep, expect);
    rep
}

/// Verify an installer package: valid signature, notarized, and the expected Team ID.
pub fn check_pkg(r: &dyn Runner, pkg: &Path, expect: &Expect) -> Report {
    let p = pkg.to_string_lossy().to_string();
    let mut rep = Report::default();
    if let Ok(o) = r.run(&argv(&["pkgutil", "--check-signature", &p])) {
        let t = combined(&o);
        rep.signature = o.ok() && t.contains("signed by a certificate trusted by macOS");
        rep.team_id = parse_pkg_team(&t);
    }
    if !rep.signature {
        rep.failures
            .push("installer signature is missing or invalid".into());
    }
    if let Ok(o) = r.run(&argv(&["spctl", "-a", "-vv", "-t", "install", &p])) {
        rep.notarized = o.ok() && combined(&o).contains("source=Notarized Developer ID");
    }
    if !rep.notarized {
        rep.failures.push("not notarized by Apple".into());
    }
    team_check(&mut rep, expect);
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Fake;

    const GOOD_DV: &str = "Identifier=com.x.app\nTeamIdentifier=ABCDE12345\n";
    const GOOD_SPCTL: &str = "/x.app: accepted\nsource=Notarized Developer ID\norigin=Developer ID Application: X (ABCDE12345)\n";

    fn fake(dv: &str, spctl: &str, verify_ok: bool) -> Fake {
        let mut f = Fake::default()
            .with("codesign -dv --verbose=4 /x.app", dv)
            .with("spctl -a -vv /x.app", spctl);
        f = if verify_ok {
            f.with("codesign --verify --deep --strict /x.app", "")
        } else {
            f.failing("codesign --verify --deep --strict /x.app")
        };
        f
    }

    #[test]
    fn team_id_parsing() {
        assert_eq!(parse_team_id(GOOD_DV).as_deref(), Some("ABCDE12345"));
        assert_eq!(parse_team_id("TeamIdentifier=not set\n"), None);
        assert_eq!(
            parse_pkg_team("   1. Developer ID Installer: Acme Inc (Z9Y8X7W6V5)\n").as_deref(),
            Some("Z9Y8X7W6V5")
        );
    }

    #[test]
    fn a_good_app_passes() {
        let rep = check_app(
            &fake(GOOD_DV, GOOD_SPCTL, true),
            Path::new("/x.app"),
            &Expect {
                team_id: Some("ABCDE12345".into()),
            },
        );
        assert!(rep.ok(), "{:?}", rep.failures);
        assert_eq!(rep.team_id.as_deref(), Some("ABCDE12345"));
    }

    #[test]
    fn wrong_team_id_fails() {
        let rep = check_app(
            &fake(GOOD_DV, GOOD_SPCTL, true),
            Path::new("/x.app"),
            &Expect {
                team_id: Some("ZZZZZZZZZZ".into()),
            },
        );
        assert!(!rep.ok() && rep.failures[0].contains("does not match"));
    }

    #[test]
    fn unknown_team_id_is_reported_not_failed() {
        let rep = check_app(
            &fake(GOOD_DV, GOOD_SPCTL, true),
            Path::new("/x.app"),
            &Expect::default(),
        );
        assert!(rep.ok());
    }

    #[test]
    fn unsigned_and_unnotarized_both_reported() {
        let rep = check_app(
            &fake("TeamIdentifier=not set\n", "rejected\n", false),
            Path::new("/x.app"),
            &Expect::default(),
        );
        assert_eq!(rep.failures.len(), 2);
    }

    #[test]
    fn checksum_matches_and_mismatches() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a");
        std::fs::write(&f, "abc").unwrap();
        let sum = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_file(&f).unwrap(), sum);
        assert!(check_checksum(&f, Some(sum)).unwrap().is_none());
        assert!(
            check_checksum(&f, Some("00"))
                .unwrap()
                .unwrap()
                .contains("mismatch")
        );
        assert!(check_checksum(&f, None).unwrap().is_none());
    }
}
