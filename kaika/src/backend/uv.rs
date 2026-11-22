//! uv tools (Python command-line tools).

use super::{Backend, Candidate, Pkg, out};
use crate::runner::{Runner, argv};

pub struct Uv;

/// `uv tool list`: `name v1.2.3` lines followed by `- executable` lines.
pub fn parse_list(text: &str) -> Vec<Pkg> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('-') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?;
            let ver = it.next().unwrap_or("").trim_start_matches('v');
            Some(Pkg {
                name: name.into(),
                version: ver.into(),
                reference: name.into(),
            })
        })
        .collect()
}

impl Backend for Uv {
    fn id(&self) -> &'static str {
        "uv"
    }
    fn tool(&self) -> &'static str {
        "uv"
    }
    fn installed(&self, r: &dyn Runner) -> Vec<Pkg> {
        parse_list(&out(r, &["uv", "tool", "list"]))
    }
    fn search(&self, r: &dyn Runner, query: &str) -> Vec<Candidate> {
        // PyPI's simple index answers 200 for a project that exists, 404 otherwise.
        let url = format!("https://pypi.org/simple/{query}/");
        match r.run(&argv(&["curl", "-sfI", "--max-time", "10", &url])) {
            Ok(o) if o.ok() => {
                vec![Candidate {
                    source: "uv".into(),
                    name: query.into(),
                    reference: query.into(),
                    desc: "PyPI".into(),
                }]
            }
            _ => vec![],
        }
    }
    fn describe(&self, r: &dyn Runner, reference: &str) -> String {
        let url = format!("https://pypi.org/pypi/{reference}/json");
        let body = r
            .run(&argv(&["curl", "-sf", "--max-time", "10", &url]))
            .map(|o| o.stdout)
            .unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        v["info"]["summary"].as_str().unwrap_or("").to_string()
    }
    fn install_argv(&self, reference: &str) -> Vec<String> {
        argv(&["uv", "tool", "install", reference])
    }
    fn uninstall_argv(&self, reference: &str, _zap: bool) -> Vec<String> {
        argv(&["uv", "tool", "uninstall", reference])
    }
    fn upgrade_argv(&self, reference: Option<&str>) -> Vec<String> {
        match reference {
            Some(n) => argv(&["uv", "tool", "upgrade", n]),
            None => argv(&["uv", "tool", "upgrade", "--all"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_reads_the_pypi_summary() {
        use crate::runner::Fake;
        let r = Fake::default().with(
            "curl -sf --max-time 10 https://pypi.org/pypi/black/json",
            r#"{"info":{"summary":"The uncompromising code formatter"}}"#,
        );
        assert_eq!(
            Uv.describe(&r, "black"),
            "The uncompromising code formatter"
        );
        assert_eq!(Uv.describe(&Fake::default(), "nope"), "");
    }

    #[test]
    fn list_ignores_executable_lines_and_strips_v() {
        let p = parse_list("ruff v0.5.0\n- ruff\nhttpie v3.2\n- http\n- https\n");
        assert_eq!(
            p.iter()
                .map(|x| (x.name.as_str(), x.version.as_str()))
                .collect::<Vec<_>>(),
            vec![("ruff", "0.5.0"), ("httpie", "3.2")]
        );
    }
}
