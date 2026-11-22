//! `kaika outdated`, and the daily check behind `outdated --notify`.
//!
//! Like macOS Software Update the daily check only suggests: it refreshes the indexes and, when the set of available
//! updates is new since the last notice, posts one notification ("Run: kaika upgrade --all"). It never installs.

use super::rows::{Row, live_rows};
use super::upgrade::{direct_upgrade_job, update};
use super::{Ctx, KResult, say};
use crate::backend;
use crate::manifest::Manifest;
use crate::runner::argv;
use sha2::{Digest, Sha256};

/// A package with a newer version available.
struct Update {
    /// `source:name`
    package: String,
    /// The installed version, empty when unknown.
    current: String,
    latest: String,
}

/// Every package with a newer version, and a warning for each vendor-site app whose release feed could not be read
/// (silence there would read as "up to date").
fn outdated_lines(ctx: &Ctx) -> KResult<(Vec<Update>, Vec<String>)> {
    let current: Vec<Row> = live_rows(ctx)?;
    let mut lines = vec![];
    for b in backend::available(ctx.r) {
        for p in b.outdated(ctx.r) {
            let cur = current
                .iter()
                .find(|r| r.source == b.id() && r.name == p.name)
                .map(|r| r.version.clone())
                .unwrap_or_default();
            lines.push(Update {
                package: format!("{}:{}", b.id(), p.name),
                current: cur,
                latest: p.version,
            });
        }
    }
    let m = Manifest::load(&ctx.dirs)?;
    let mut warnings = vec![];
    for e in m.packages.iter().filter(|e| e.source == "direct") {
        match direct_upgrade_job(ctx, e, false) {
            Ok(Some(j)) => lines.push(Update {
                package: format!("direct:{}", e.name),
                current: e.version.clone(),
                latest: if j.version.is_empty() {
                    "newer build".into()
                } else {
                    j.version
                },
            }),
            Ok(None) => {}
            Err(err) => warnings.push(format!(
                "{}: could not check for updates ({})",
                e.name, err.msg
            )),
        }
    }
    Ok((lines, warnings))
}

pub(super) fn outdated(ctx: &Ctx, notify: bool) -> KResult {
    if notify {
        return outdated_notify(ctx);
    }
    let (lines, warnings) = outdated_lines(ctx)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    if ctx.json {
        let v: Vec<_> = lines
            .iter()
            .map(|u| serde_json::json!({ "package": u.package, "current": u.current, "latest": u.latest }))
            .collect();
        println!("{}", serde_json::Value::Array(v));
    } else if lines.is_empty() {
        say(ctx, "everything is up to date");
    } else {
        for u in lines {
            let current = if u.current.is_empty() {
                "?"
            } else {
                &u.current
            };
            println!("{:<34}{current} -> {}", u.package, u.latest);
        }
    }
    Ok(())
}

/// The daily check: refresh the indexes, and if the set of available updates is new since the last notice, say so
/// with one macOS notification. `kaika upgrade --all` is what installs.
fn outdated_notify(ctx: &Ctx) -> KResult {
    update(ctx)?;
    let (lines, warnings) = outdated_lines(ctx)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let seen = ctx.dirs.root.join("notified");
    let now: Vec<String> = lines
        .iter()
        .map(|u| format!("{}={}", u.package, u.latest))
        .collect();
    let fingerprint = fingerprint(&now);
    if lines.is_empty() {
        let _ = std::fs::remove_file(&seen); // up to date: the next batch of updates is news again
        say(ctx, "everything is up to date");
        return Ok(());
    }
    if std::fs::read_to_string(&seen).is_ok_and(|t| t.trim() == fingerprint) {
        say(
            ctx,
            &format!("{} update(s) available, already notified", lines.len()),
        );
        return Ok(());
    }
    let names: Vec<&str> = lines.iter().map(|u| u.package.as_str()).collect();
    let script = notification_script(&names);
    let _ = ctx.r.run(&argv(&["osascript", "-e", &script]));
    std::fs::create_dir_all(&ctx.dirs.root).map_err(anyhow::Error::from)?;
    std::fs::write(&seen, &fingerprint).map_err(anyhow::Error::from)?;
    say(
        ctx,
        &format!(
            "{} update(s) available; notified. Run: kaika upgrade --all",
            lines.len()
        ),
    );
    Ok(())
}

/// A short stable fingerprint of the set of available updates, so the same set is announced only once.
fn fingerprint(items: &[String]) -> String {
    let mut v: Vec<&String> = items.iter().collect();
    v.sort();
    let mut h = Sha256::new();
    for i in v {
        h.update(i.as_bytes());
        h.update([0]);
    }
    h.finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn applescript_string(s: &str) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    format!("\"{}\"", clean.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The AppleScript that posts the banner. Package names come from outside, so they are escaped as data.
fn notification_script(names: &[&str]) -> String {
    let shown: Vec<&str> = names.iter().take(3).copied().collect();
    let more = if names.len() > 3 {
        format!(" and {} more", names.len() - 3)
    } else {
        String::new()
    };
    let body = format!(
        "{} update{} available: {}{more}. Run: kaika upgrade --all",
        names.len(),
        if names.len() == 1 { "" } else { "s" },
        shown.join(", ")
    );
    format!(
        "display notification {} with title \"kaika\"",
        applescript_string(&body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_updates_have_the_same_fingerprint_in_any_order_and_new_ones_differ() {
        let a = fingerprint(&["brew:jq=1.8".into(), "cask:zen=2".into()]);
        assert_eq!(a, fingerprint(&["cask:zen=2".into(), "brew:jq=1.8".into()]));
        assert_ne!(a, fingerprint(&["brew:jq=1.9".into(), "cask:zen=2".into()]));
    }

    #[test]
    fn the_notification_names_a_few_packages_and_cannot_be_broken_by_a_name() {
        let s = notification_script(&["brew:jq", "cask:zen", "npm:x", "uv:y"]);
        assert!(
            s.starts_with("display notification \"4 updates available: brew:jq, cask:zen, npm:x and 1 more. Run: kaika upgrade --all\" with title"),
            "{s}"
        );
        let s = notification_script(&["evil\" & (do shell script \"x\") & \""]);
        let bare = s
            .char_indices()
            .filter(|(i, c)| *c == '"' && !s[..*i].ends_with('\\'))
            .count();
        assert_eq!(
            bare, 4,
            "only the body's and the title's own quotes are bare: {s}"
        );
        assert!(notification_script(&["a"]).contains("1 update available"));
    }
}
