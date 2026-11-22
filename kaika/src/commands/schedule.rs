//! `kaika schedule`: the daily update check as one launchd user agent that runs `kaika outdated --notify`.
//!
//! Like macOS Software Update it only suggests: it refreshes the indexes, and when there are new updates it posts
//! one notification ("Run: kaika upgrade --all"). It never installs anything; you decide. It is a plain
//! LaunchAgent in `~/Library/LaunchAgents` (no daemon of kaika's own, nothing resident between runs); offline runs
//! exit 69 and stay quiet; everything it prints goes to `~/.kaika/logs/upgrade.log`.

use super::{Ctx, KErr, KResult, say};
use crate::runner::argv;
use std::path::PathBuf;

/// The launchd label, and the agent file's name.
pub(super) const LABEL: &str = "io.kaika.upgrade";

/// Where the agent's plist lives; also how `doctor` knows whether the check is scheduled.
pub(super) fn agent_path(ctx: &Ctx) -> PathBuf {
    ctx.dirs
        .home
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

pub(super) fn schedule(ctx: &Ctx, action: &str, at: &str, dry_run: bool) -> KResult {
    match action {
        "status" => status(ctx),
        "on" => turn_on(ctx, at, dry_run),
        _ => turn_off(ctx, dry_run),
    }
}

fn status(ctx: &Ctx) -> KResult {
    match std::fs::read_to_string(agent_path(ctx)) {
        Ok(p) => {
            let (h, m) = scheduled_time(&p).unwrap_or((0, 0));
            say(
                ctx,
                &format!(
                    "scheduled: kaika checks for updates every day at {h:02}:{m:02} and notifies you; it installs nothing (log: {})",
                    log_path(ctx).display()
                ),
            );
        }
        Err(_) => say(ctx, "not scheduled (kaika schedule on [--at HH:MM])"),
    }
    Ok(())
}

fn log_path(ctx: &Ctx) -> PathBuf {
    ctx.dirs.root.join("logs/upgrade.log")
}

fn turn_on(ctx: &Ctx, at: &str, dry_run: bool) -> KResult {
    let file = agent_path(ctx);
    let (h, m) = parse_time(at).map_err(KErr::usage)?;
    let exe = std::env::current_exe()
        .map_err(anyhow::Error::from)?
        .display()
        .to_string();
    let log = log_path(ctx);
    let path = format!(
        "{}:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        ctx.dirs.bin.display()
    );
    let body = plist(&exe, &log.display().to_string(), &path, h, m);
    say(
        ctx,
        &format!(
            "{}schedule: write {} and load it with launchctl; checks for updates daily at {h:02}:{m:02} and notifies you (it installs nothing)",
            if dry_run { "would " } else { "" },
            file.display()
        ),
    );
    if dry_run {
        return Ok(());
    }
    for dir in [file.parent(), log.parent()].into_iter().flatten() {
        std::fs::create_dir_all(dir).map_err(anyhow::Error::from)?;
    }
    // Reload cleanly: an older copy is unloaded first, and not being loaded is fine.
    let target = launchd_target(ctx);
    let _ = ctx.r.run(&argv(&[
        "launchctl",
        "bootout",
        &format!("{target}/{LABEL}"),
    ]));
    std::fs::write(&file, body).map_err(anyhow::Error::from)?;
    let code = ctx.r.run_live(&argv(&[
        "launchctl",
        "bootstrap",
        &target,
        &file.display().to_string(),
    ]))?;
    if code != 0 {
        return Err(KErr::fail(format!(
            "launchctl could not load {} (exit {code})",
            file.display()
        )));
    }
    say(ctx, &format!("scheduled daily at {h:02}:{m:02}"));
    Ok(())
}

fn turn_off(ctx: &Ctx, dry_run: bool) -> KResult {
    let file = agent_path(ctx);
    if !file.exists() {
        say(ctx, "not scheduled");
        return Ok(());
    }
    say(
        ctx,
        &format!(
            "{}schedule: unload and remove {}",
            if dry_run { "would " } else { "" },
            file.display()
        ),
    );
    if dry_run {
        return Ok(());
    }
    let _ = ctx.r.run(&argv(&[
        "launchctl",
        "bootout",
        &format!("{}/{LABEL}", launchd_target(ctx)),
    ]));
    std::fs::remove_file(&file).map_err(anyhow::Error::from)?;
    say(ctx, "schedule removed");
    Ok(())
}

/// `gui/<uid>`: the per-user launchd domain.
fn launchd_target(ctx: &Ctx) -> String {
    let uid = ctx
        .r
        .run(&argv(&["id", "-u"]))
        .map(|o| o.stdout.trim().to_string())
        .unwrap_or_default();
    format!("gui/{}", if uid.is_empty() { "501" } else { &uid })
}

/// `HH:MM`, 24-hour.
fn parse_time(s: &str) -> Result<(u8, u8), String> {
    let bad = || format!("'{s}' is not a time of day; use HH:MM, for example 03:30");
    let (h, m) = s.split_once(':').ok_or_else(bad)?;
    if h.is_empty()
        || m.is_empty()
        || h.len() > 2
        || m.len() > 2
        || !(h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()))
    {
        return Err(bad());
    }
    let (h, m): (u8, u8) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok((h, m))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The launchd job definition. `path` is the PATH the job runs with, so brew, npm, uv and mas are found.
fn plist(exe: &str, log: &str, path: &str, hour: u8, minute: u8) -> String {
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
        exe = xml_escape(exe),
        log = xml_escape(log),
        path = xml_escape(path),
    )
}

/// The time a plist is set for, read back from its text.
fn scheduled_time(plist: &str) -> Option<(u8, u8)> {
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
        for bad in [
            "24:00", "12:60", "noon", "12", "12:", ":30", "-1:00", "012:00", "12:300",
        ] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_job_only_checks_and_notifies_at_the_chosen_time() {
        let p = plist(
            "/Users/me/.cargo/bin/kaika",
            "/Users/me/.kaika/logs/upgrade.log",
            "/opt/homebrew/bin:/usr/bin",
            3,
            30,
        );
        assert!(
            !p.contains("<string>upgrade</string>") && !p.contains("--yes"),
            "the job must never install: {p}"
        );
        for want in [
            "<string>io.kaika.upgrade</string>",
            "<string>outdated</string>",
            "<string>--notify</string>",
            "<integer>3</integer>",
            "<integer>30</integer>",
            "/opt/homebrew/bin:/usr/bin",
        ] {
            assert!(p.contains(want), "{want} missing from {p}");
        }
        assert_eq!(scheduled_time(&p), Some((3, 30)));
    }

    #[test]
    fn paths_with_markup_characters_cannot_break_the_file() {
        let p = plist("/a&b/<kaika>", "/l.log", "/bin", 0, 0);
        assert!(p.contains("/a&amp;b/&lt;kaika&gt;") && !p.contains("<kaika>"));
    }
}
