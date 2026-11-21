//! Naming a target and choosing the backend that carries it.

use crate::backend::Candidate;

pub const SOURCES: [&str; 6] = ["brew", "cask", "uv", "npm", "mas", "direct"];

#[derive(Debug, PartialEq, Clone)]
pub struct Target {
    pub source: Option<String>,
    pub name: String,
}

/// `name`, or `source:name` to force a backend.
pub fn parse_target(s: &str) -> Result<Target, String> {
    match s.split_once(':') {
        Some((src, name)) if SOURCES.contains(&src) && !name.is_empty() => {
            Ok(Target { source: Some(src.into()), name: name.into() })
        }
        Some((src, _)) if !src.is_empty() && !src.contains('/') => {
            Err(format!("unknown source '{src}' in '{s}' (sources: {})", SOURCES.join(", ")))
        }
        _ => Ok(Target { source: None, name: s.into() }),
    }
}

#[derive(Debug, PartialEq)]
pub enum Choice {
    None,
    One(Candidate),
    Several { chosen: Candidate, others: Vec<Candidate> },
}

/// Keep exact-name hits, order them by preference, drop repeats from one source.
pub fn exact(name: &str, hits: &[Candidate], order: &[String]) -> Vec<Candidate> {
    let rank = |c: &Candidate| order.iter().position(|o| *o == c.source).unwrap_or(usize::MAX);
    let mut v: Vec<Candidate> = hits.iter().filter(|c| c.name.eq_ignore_ascii_case(name)).cloned().collect();
    v.sort_by_key(|c| rank(c));
    v.dedup_by(|a, b| a.source == b.source);
    v
}

pub fn choose(name: &str, hits: &[Candidate], order: &[String]) -> Choice {
    let mut v = exact(name, hits, order);
    match v.len() {
        0 => Choice::None,
        1 => Choice::One(v.remove(0)),
        _ => {
            let chosen = v.remove(0);
            Choice::Several { chosen, others: v }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(src: &str, name: &str) -> Candidate {
        Candidate { source: src.into(), name: name.into(), reference: name.into(), desc: String::new() }
    }
    fn order() -> Vec<String> {
        crate::config::Config::default().order()
    }

    #[test]
    fn plain_name_has_no_source() {
        assert_eq!(parse_target("jq").unwrap(), Target { source: None, name: "jq".into() });
    }

    #[test]
    fn source_prefix_forces_a_backend() {
        let t = parse_target("cask:zen").unwrap();
        assert_eq!((t.source.as_deref(), t.name.as_str()), (Some("cask"), "zen"));
        assert_eq!(parse_target("npm:@scope/pkg").unwrap().name, "@scope/pkg");
    }

    #[test]
    fn unknown_source_is_an_error() {
        assert!(parse_target("pip:requests").is_err());
    }

    #[test]
    fn nothing_matches_exactly_means_none() {
        assert_eq!(choose("ripgrep", &[c("brew", "ripgrep-all")], &order()), Choice::None);
    }

    #[test]
    fn preference_order_decides_between_sources() {
        match choose("docker", &[c("cask", "docker"), c("brew", "docker")], &order()) {
            Choice::Several { chosen, others } => {
                assert_eq!(chosen.source, "brew");
                assert_eq!(others.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn config_order_overrides_the_default() {
        let o: Vec<String> = ["cask", "brew"].iter().map(|s| s.to_string()).collect();
        match choose("docker", &[c("brew", "docker"), c("cask", "docker")], &o) {
            Choice::Several { chosen, .. } => assert_eq!(chosen.source, "cask"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn match_ignores_case() {
        assert!(matches!(choose("Zen", &[c("cask", "zen")], &order()), Choice::One(_)));
    }
}
