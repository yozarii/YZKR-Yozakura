//! Homebrew: formulae (`brew`) and casks (`cask`).

use super::{Backend, Candidate, Pkg, out};
use crate::runner::{Runner, argv};
use std::collections::HashMap;

pub struct Brew {
    pub cask: bool,
}

/// `name v1 v2` lines to (name, last version).
pub fn parse_versions(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (!p.is_empty()).then(|| {
                (
                    p[0].to_string(),
                    p.last().filter(|_| p.len() > 1).unwrap_or(&"").to_string(),
                )
            })
        })
        .collect()
}

/// `brew search` output: names, skipping `==>` section headers.
pub fn parse_search(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("==>"))
        .map(String::from)
        .collect()
}

/// `brew desc` prints `name: description`; casks add the display name in parentheses.
pub fn parse_desc(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    line.split_once(": ")
        .map(|(_, d)| d)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// `brew outdated --verbose` lines: `name (1.0) < 2.0`.
pub fn parse_outdated(text: &str) -> Vec<Pkg> {
    text.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (p.len() >= 2).then(|| Pkg {
                name: p[0].into(),
                version: p.last().unwrap().to_string(),
                reference: p[0].into(),
            })
        })
        .collect()
}

impl Backend for Brew {
    fn id(&self) -> &'static str {
        if self.cask { "cask" } else { "brew" }
    }
    fn tool(&self) -> &'static str {
        "brew"
    }

    fn installed(&self, r: &dyn Runner) -> Vec<Pkg> {
        let pkg = |n: String, v: String| Pkg {
            reference: n.clone(),
            name: n,
            version: v,
        };
        if self.cask {
            parse_versions(&out(r, &["brew", "list", "--cask", "--versions"]))
                .into_iter()
                .map(|(n, v)| pkg(n, v))
                .collect()
        } else {
            let ver: HashMap<String, String> =
                parse_versions(&out(r, &["brew", "list", "--formula", "--versions"]))
                    .into_iter()
                    .collect();
            out(r, &["brew", "leaves"])
                .split_whitespace()
                .map(|n| pkg(n.to_string(), ver.get(n).cloned().unwrap_or_default()))
                .collect()
        }
    }

    fn search(&self, r: &dyn Runner, query: &str) -> Vec<Candidate> {
        let kind = if self.cask { "--cask" } else { "--formula" };
        parse_search(&out(r, &["brew", "search", kind, query]))
            .into_iter()
            .map(|n| Candidate {
                source: self.id().into(),
                reference: n.clone(),
                name: n,
                desc: String::new(),
            })
            .collect()
    }

    fn install_argv(&self, reference: &str) -> Vec<String> {
        if self.cask {
            argv(&["brew", "install", "--cask", reference])
        } else {
            argv(&["brew", "install", reference])
        }
    }

    fn uninstall_argv(&self, reference: &str, zap: bool) -> Vec<String> {
        match (self.cask, zap) {
            (true, true) => argv(&["brew", "uninstall", "--cask", "--zap", reference]),
            (true, false) => argv(&["brew", "uninstall", "--cask", reference]),
            (false, _) => argv(&["brew", "uninstall", reference]),
        }
    }

    fn upgrade_argv(&self, reference: Option<&str>) -> Vec<String> {
        let mut a = argv(&[
            "brew",
            "upgrade",
            if self.cask { "--cask" } else { "--formula" },
        ]);
        if let Some(n) = reference {
            a.push(n.into());
        }
        a
    }

    fn describe(&self, r: &dyn Runner, reference: &str) -> String {
        let kind = if self.cask { "--cask" } else { "--formula" };
        parse_desc(&out(r, &["brew", "desc", kind, reference]))
    }

    fn outdated(&self, r: &dyn Runner) -> Vec<Pkg> {
        let kind = if self.cask { "--cask" } else { "--formula" };
        parse_outdated(&out(r, &["brew", "outdated", kind, "--verbose"]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Fake;

    #[test]
    fn versions_take_the_last_column() {
        assert_eq!(
            parse_versions("jq 1.7\nnode 22.1 23.0\n"),
            vec![("jq".into(), "1.7".into()), ("node".into(), "23.0".into())]
        );
        assert_eq!(
            parse_versions("lonely\n"),
            vec![("lonely".into(), "".into())]
        );
    }

    #[test]
    fn search_skips_headers_and_blanks() {
        assert_eq!(
            parse_search("==> Formulae\nripgrep\n\n==> Casks\nrectangle\n"),
            vec!["ripgrep", "rectangle"]
        );
    }

    #[test]
    fn outdated_reads_name_and_target_version() {
        let o = parse_outdated("zen (1.0) < 1.2\n");
        assert_eq!(
            o,
            vec![Pkg {
                name: "zen".into(),
                version: "1.2".into(),
                reference: "zen".into()
            }]
        );
    }

    #[test]
    fn formula_installed_uses_leaves_with_versions() {
        let r = Fake::default().with("brew leaves", "jq\nfzf\n").with(
            "brew list --formula --versions",
            "jq 1.7\nfzf 0.5\noniguruma 6\n",
        );
        let got = Brew { cask: false }.installed(&r);
        assert_eq!(
            got.iter()
                .map(|p| (p.name.as_str(), p.version.as_str()))
                .collect::<Vec<_>>(),
            vec![("jq", "1.7"), ("fzf", "0.5")]
        );
    }

    #[test]
    fn cask_uninstall_zap_uses_brews_own_flag() {
        let b = Brew { cask: true };
        assert_eq!(
            b.uninstall_argv("zen", true),
            ["brew", "uninstall", "--cask", "--zap", "zen"]
        );
        assert_eq!(
            b.uninstall_argv("zen", false),
            ["brew", "uninstall", "--cask", "zen"]
        );
        assert_eq!(
            Brew { cask: false }.uninstall_argv("jq", true),
            ["brew", "uninstall", "jq"]
        );
    }

    #[test]
    fn desc_is_the_text_after_the_name() {
        assert_eq!(
            parse_desc("ripgrep: Search tool like grep\n"),
            "Search tool like grep"
        );
        assert_eq!(parse_desc(""), "");
        let r = Fake::default().with(
            "brew desc --cask zen",
            "zen: (Zen Browser) Firefox-based browser\n",
        );
        assert_eq!(
            Brew { cask: true }.describe(&r, "zen"),
            "(Zen Browser) Firefox-based browser"
        );
    }

    #[test]
    fn upgrade_all_or_one() {
        assert_eq!(
            Brew { cask: true }.upgrade_argv(None),
            ["brew", "upgrade", "--cask"]
        );
        assert_eq!(
            Brew { cask: false }.upgrade_argv(Some("jq")),
            ["brew", "upgrade", "--formula", "jq"]
        );
    }
}
