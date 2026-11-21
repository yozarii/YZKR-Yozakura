//! The App Store through `mas`. Purchases are out of scope; it only handles apps the Apple ID can get.

use super::{out, Backend, Candidate, Pkg};
use crate::runner::{argv, Runner};

pub struct Mas;

/// `12345  Name  (1.2.3)`: id, name, version. Anything after the version group (a price) is ignored.
pub fn parse_line(line: &str) -> Option<(String, String, String)> {
    let line = line.trim();
    let (id, rest) = line.split_once(char::is_whitespace)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let rest = rest.trim();
    match (rest.rfind('('), rest.rfind(')')) {
        (Some(o), Some(c)) if o < c => Some((id.into(), rest[..o].trim().into(), rest[o + 1..c].trim().into())),
        _ => Some((id.into(), rest.into(), String::new())),
    }
}

impl Backend for Mas {
    fn id(&self) -> &'static str {
        "mas"
    }
    fn tool(&self) -> &'static str {
        "mas"
    }
    fn installed(&self, r: &dyn Runner) -> Vec<Pkg> {
        out(r, &["mas", "list"])
            .lines()
            .filter_map(parse_line)
            .map(|(id, name, version)| Pkg { name, version, reference: id })
            .collect()
    }
    fn search(&self, r: &dyn Runner, query: &str) -> Vec<Candidate> {
        out(r, &["mas", "search", query])
            .lines()
            .filter_map(parse_line)
            .map(|(id, name, _)| Candidate { source: "mas".into(), name, reference: id, desc: "App Store".into() })
            .collect()
    }
    fn install_argv(&self, reference: &str) -> Vec<String> {
        argv(&["mas", "install", reference])
    }
    fn uninstall_argv(&self, reference: &str, _zap: bool) -> Vec<String> {
        argv(&["sudo", "mas", "uninstall", reference])
    }
    fn upgrade_argv(&self, reference: Option<&str>) -> Vec<String> {
        let mut a = argv(&["mas", "upgrade"]);
        if let Some(n) = reference {
            a.push(n.into());
        }
        a
    }
    fn outdated(&self, r: &dyn Runner) -> Vec<Pkg> {
        // `id  Name  (old -> new)`
        out(r, &["mas", "outdated"])
            .lines()
            .filter_map(parse_line)
            .map(|(id, name, v)| Pkg { name, version: v.rsplit("->").next().unwrap_or("").trim().into(), reference: id })
            .collect()
    }
    fn needs_admin(&self, uninstall: bool) -> bool {
        uninstall
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_and_search_lines() {
        assert_eq!(parse_line("497799835   Xcode  (15.0)"), Some(("497799835".into(), "Xcode".into(), "15.0".into())));
        assert_eq!(parse_line("1444383602  GoodNotes 5  (5.9.13)  $9.99").unwrap().1, "GoodNotes 5");
        assert_eq!(parse_line("not a line"), None);
        assert_eq!(parse_line("123  NoVersion").unwrap().2, "");
    }

    #[test]
    fn outdated_takes_the_new_version() {
        use crate::runner::Fake;
        let r = Fake::default().with("mas outdated", "497799835 Xcode (15.0 -> 15.1)\n");
        assert_eq!(Mas.outdated(&r)[0].version, "15.1");
    }
}
