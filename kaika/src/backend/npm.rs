//! npm global packages.

use super::{Backend, Candidate, Pkg, out};
use crate::runner::{Runner, argv};

/// npm global packages.
pub struct Npm;

fn parse_ls(json: &str) -> Vec<Pkg> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let Some(deps) = v.get("dependencies").and_then(|d| d.as_object()) else {
        return vec![];
    };
    deps.iter()
        .filter(|(n, _)| !matches!(n.as_str(), "npm" | "corepack"))
        .map(|(n, d)| Pkg {
            name: n.clone(),
            version: d
                .get("version")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .into(),
            reference: n.clone(),
        })
        .collect()
}

fn parse_outdated(json: &str) -> Vec<Pkg> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let Some(m) = v.as_object() else {
        return vec![];
    };
    m.iter()
        .map(|(n, d)| Pkg {
            name: n.clone(),
            version: d
                .get("latest")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .into(),
            reference: n.clone(),
        })
        .collect()
}

impl Backend for Npm {
    fn id(&self) -> &'static str {
        "npm"
    }
    fn tool(&self) -> &'static str {
        "npm"
    }
    fn installed(&self, r: &dyn Runner) -> Vec<Pkg> {
        parse_ls(&out(r, &["npm", "ls", "-g", "--depth=0", "--json"]))
    }
    fn search(&self, r: &dyn Runner, query: &str) -> Vec<Candidate> {
        // `npm view <name> name` prints the name when the package exists and nothing otherwise.
        let found = out(r, &["npm", "view", query, "name"]);
        let found = found.trim().trim_matches('"');
        if found.is_empty() {
            return vec![];
        }
        vec![Candidate {
            source: "npm".into(),
            name: found.into(),
            reference: found.into(),
            desc: String::new(),
        }]
    }
    fn describe(&self, r: &dyn Runner, reference: &str) -> String {
        out(r, &["npm", "view", reference, "description"])
            .trim()
            .to_string()
    }
    fn install_argv(&self, reference: &str) -> Vec<String> {
        argv(&["npm", "install", "-g", reference])
    }
    fn uninstall_argv(&self, reference: &str, _zap: bool) -> Vec<String> {
        argv(&["npm", "uninstall", "-g", reference])
    }
    fn upgrade_argv(&self, reference: Option<&str>) -> Vec<String> {
        let mut a = argv(&["npm", "update", "-g"]);
        if let Some(n) = reference {
            a.push(n.into());
        }
        a
    }
    fn outdated(&self, r: &dyn Runner) -> Vec<Pkg> {
        parse_outdated(&out(r, &["npm", "outdated", "-g", "--json"]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_skips_npm_and_corepack() {
        let j = r#"{"dependencies":{"npm":{"version":"10"},"corepack":{"version":"1"},"typescript":{"version":"5.4.0"}}}"#;
        let p = parse_ls(j);
        assert_eq!(p.len(), 1);
        assert_eq!(
            (p[0].name.as_str(), p[0].version.as_str()),
            ("typescript", "5.4.0")
        );
    }

    #[test]
    fn garbage_json_is_empty_not_a_panic() {
        assert!(parse_ls("not json").is_empty());
        assert!(parse_outdated("").is_empty());
    }

    #[test]
    fn outdated_reads_latest() {
        let p = parse_outdated(r#"{"typescript":{"current":"5.4.0","latest":"5.5.0"}}"#);
        assert_eq!(p[0].version, "5.5.0");
    }
}
