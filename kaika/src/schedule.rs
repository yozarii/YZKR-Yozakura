//! The daily update check: one launchd user agent that runs `kaika outdated --notify` once a day.
//!
//! Like macOS Software Update it only suggests: it refreshes the indexes, and when there are new updates it posts
//! one notification ("Run: kaika upgrade --all"). It never installs anything; you decide. It is a plain
//! LaunchAgent in `~/Library/LaunchAgents` (no daemon of kaika's own, nothing resident between runs); offline runs
//! exit 69 and stay quiet; everything it prints goes to `~/.kaika/logs/upgrade.log`.

pub const LABEL: &str = "io.kaika.upgrade";

/// `HH:MM`, 24-hour.
pub fn parse_time(s: &str) -> Result<(u8, u8), String> {
    let bad = || format!("'{s}' is not a time of day; use HH:MM, for example 03:30");
    let (h, m) = s.split_once(':').ok_or_else(bad)?;
    if h.is_empty() || m.is_empty() || h.len() > 2 || m.len() > 2 || !(h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit())) {
        return Err(bad());
    }
    let (h, m): (u8, u8) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok((h, m))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The launchd job definition. `path` is the PATH the job runs with, so brew, npm, uv and mas are found.
pub fn plist(exe: &str, log: &str, path: &str, hour: u8, minute: u8) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>outdated</string>
    <string>--notify</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>{path}</string></dict>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>{hour}</integer><key>Minute</key><integer>{minute}</integer></dict>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
  <key>ProcessType</key><string>Background</string>
  <key>LowPriorityIO</key><true/>
</dict>
</plist>
"#,
        exe = esc(exe),
        log = esc(log),
        path = esc(path),
    )
}

/// A short stable fingerprint of the set of available updates, so the same set is announced only once.
pub fn fingerprint(items: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut v: Vec<&String> = items.iter().collect();
    v.sort();
    let mut h = Sha256::new();
    for i in v {
        h.update(i.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn applescript_string(s: &str) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    format!("\"{}\"", clean.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The AppleScript that posts the banner. Package names come from outside, so they are escaped as data.
pub fn notification_script(names: &[&str]) -> String {
    let shown: Vec<&str> = names.iter().take(3).copied().collect();
    let more = if names.len() > 3 { format!(" and {} more", names.len() - 3) } else { String::new() };
    let body = format!("{} update{} available: {}{more}. Run: kaika upgrade --all", names.len(), if names.len() == 1 { "" } else { "s" }, shown.join(", "));
    format!("display notification {} with title \"kaika\"", applescript_string(&body))
}

/// The time a plist is set for, read back from its text.
pub fn scheduled_time(plist: &str) -> Option<(u8, u8)> {
    let num = |key: &str| -> Option<u8> {
        let after = plist.split_once(&format!("<key>{key}</key><integer>"))?.1;
        after.split_once("</integer>")?.0.parse().ok()
    };
    Some((num("Hour")?, num("Minute")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_hh_mm_on_a_24_hour_clock() {
        assert_eq!(parse_time("03:30"), Ok((3, 30)));
        assert_eq!(parse_time("3:05"), Ok((3, 5)));
        assert_eq!(parse_time("23:59"), Ok((23, 59)));
        for bad in ["24:00", "12:60", "noon", "12", "12:", ":30", "-1:00", "012:00", "12:300"] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_job_only_checks_and_notifies_at_the_chosen_time() {
        let p = plist("/Users/me/.cargo/bin/kaika", "/Users/me/.kaika/logs/upgrade.log", "/opt/homebrew/bin:/usr/bin", 3, 30);
        assert!(!p.contains("<string>upgrade</string>") && !p.contains("--yes"), "the job must never install: {p}");
        for want in ["<string>io.kaika.upgrade</string>", "<string>outdated</string>", "<string>--notify</string>", "<integer>3</integer>", "<integer>30</integer>", "/opt/homebrew/bin:/usr/bin"] {
            assert!(p.contains(want), "{want} missing from {p}");
        }
        assert_eq!(scheduled_time(&p), Some((3, 30)));
    }

    #[test]
    fn the_same_updates_have_the_same_fingerprint_in_any_order_and_new_ones_differ() {
        let a = fingerprint(&["brew:jq=1.8".into(), "cask:zen=2".into()]);
        assert_eq!(a, fingerprint(&["cask:zen=2".into(), "brew:jq=1.8".into()]));
        assert_ne!(a, fingerprint(&["brew:jq=1.9".into(), "cask:zen=2".into()]));
    }

    #[test]
    fn the_notification_names_a_few_packages_and_cannot_be_broken_by_a_name() {
        let s = notification_script(&["brew:jq", "cask:zen", "npm:x", "uv:y"]);
        assert!(s.starts_with("display notification \"4 updates available: brew:jq, cask:zen, npm:x and 1 more. Run: kaika upgrade --all\" with title"), "{s}");
        let s = notification_script(&["evil\" & (do shell script \"x\") & \""]);
        let bare = s.char_indices().filter(|(i, c)| *c == '"' && !s[..*i].ends_with('\\')).count();
        assert_eq!(bare, 4, "only the body's and the title's own quotes are bare: {s}");
        assert!(notification_script(&["a"]).contains("1 update available"));
    }

    #[test]
    fn paths_with_markup_characters_cannot_break_the_file() {
        let p = plist("/a&b/<kaika>", "/l.log", "/bin", 0, 0);
        assert!(p.contains("/a&amp;b/&lt;kaika&gt;") && !p.contains("<kaika>"));
    }
}
