//! `kaika doctor`, `kaika cleanup` and `kaika config`: looking after kaika itself.

use super::schedule::agent_path;
use super::{Ctx, KErr, KResult, say};
use crate::backend;
use crate::config::KEYS;
use crate::manifest::Manifest;
use crate::model;
use crate::render;
use std::fmt::Display;
use std::path::Path;

/// One line of `doctor` output: a label in a fixed-width column, then the text.
fn line(label: &str, text: impl Display) {
    println!("{label:<8}{text}");
}

/// Check this machine: backends present, the state files readable, the decision layer ready. Only a corrupt
/// manifest makes it fail; everything else is information. Nothing is started, not even the model.
pub(super) fn doctor(ctx: &Ctx) -> KResult {
    line("kaika", env!("CARGO_PKG_VERSION"));
    print_backends(ctx);
    line("home", ctx.dirs.root.display());
    let on_path = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d == ctx.dirs.bin));
    if ctx.dirs.bin.exists() && !on_path {
        line(
            "bin",
            format!(
                "{} is not on PATH; add it to use the command-line tools kaika installs",
                ctx.dirs.bin.display()
            ),
        );
    }
    let state_ok = match Manifest::load(&ctx.dirs) {
        Ok(m) => {
            line("state", format!("ok ({} packages)", m.packages.len()));
            true
        }
        Err(e) => {
            line("state", format!("error: {e:#}"));
            false
        }
    };
    match render::browser() {
        Some(b) => line(
            "browser",
            format!(
                "{} (only for vendor pages that need JavaScript; headless, no window)",
                b.display()
            ),
        ),
        None => line(
            "browser",
            "none found: vendor pages that need JavaScript cannot be read (install Chrome, Brave, Edge or Chromium)",
        ),
    }
    line(
        "upgrade",
        if agent_path(ctx).exists() {
            "daily update check scheduled (kaika schedule status)"
        } else {
            "not scheduled (kaika schedule on)"
        },
    );
    print_models(ctx);
    if state_ok {
        Ok(())
    } else {
        Err(KErr::fail("doctor found problems"))
    }
}

fn print_backends(ctx: &Ctx) {
    for b in backend::all() {
        let ok = ctx.r.exists(b.tool());
        line(
            b.id(),
            if ok {
                format!("ok ({} found)", b.tool())
            } else {
                format!("missing ({} not on PATH)", b.tool())
            },
        );
        if b.id() == "mas" {
            line(
                "",
                if ok {
                    "uses the Apple ID signed in to the App Store app; kaika never handles it"
                } else {
                    "App Store apps need it: kaika install mas, then sign in to the App Store app"
                },
            );
        }
    }
}

/// The decision layer's state. Reports only: nothing is started, the model loads when a third-party app needs it.
fn print_models(ctx: &Ctx) {
    if !ctx.cfg.models_enabled() {
        line(
            "models",
            "off (no model decisions, no web discovery of vendor downloads)",
        );
        return;
    }
    let st = model::status(ctx.r, &ctx.dirs);
    match st.llama_build {
        Some(b) if b >= model::MIN_BUILD => line("models", format!("llama-server build {b}")),
        Some(b) => line(
            "models",
            format!(
                "llama-server build {b} is too old (need {} or newer): kaika upgrade llama.cpp",
                model::MIN_BUILD
            ),
        ),
        None => line("models", "llama-server missing: kaika install llama.cpp"),
    }
    match st.model_bytes {
        Some(n) => line(
            "",
            format!(
                "{} installed ({:.1} GB); loaded only while a third-party app is being found",
                model::MODEL_NAME,
                n as f64 / 1e9
            ),
        ),
        None => line(
            "",
            format!(
                "{} not installed: kaika install {}",
                model::MODEL_NAME,
                model::MODEL_NAME
            ),
        ),
    }
}

/// Bytes under a folder (symlinks are counted as themselves, never followed).
fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else {
        return 0;
    };
    rd.filter_map(|e| e.ok())
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Remove old downloads, staging folders and mount points from the cache. Fails (exit 1) if something stays.
pub(super) fn cleanup(ctx: &Ctx, dry_run: bool) -> KResult {
    let mut total = 0;
    let mut failed = vec![];
    for sub in ["downloads", "stage", "mnt"] {
        let p = ctx.dirs.cache.join(sub);
        let size = dir_size(&p);
        total += size;
        if size > 0 || p.exists() {
            if dry_run {
                say(ctx, &format!("would remove {} ({size} bytes)", p.display()));
            } else if let Err(e) = std::fs::remove_dir_all(&p) {
                failed.push(format!("{}: {e}", p.display()));
            }
        }
    }
    if !failed.is_empty() {
        return Err(KErr::fail(format!(
            "could not remove {}",
            failed.join("; ")
        )));
    }
    say(
        ctx,
        &format!(
            "{} {total} bytes",
            if dry_run { "would free" } else { "freed" }
        ),
    );
    Ok(())
}

/// `config`, `config get KEY`, `config set KEY VALUE`, `config path`.
pub(super) fn config(ctx: &Ctx, args: &[String]) -> KResult {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        [] => {
            for k in KEYS {
                println!("{k} = {}", ctx.cfg.get(k)?);
            }
            Ok(())
        }
        ["path"] => {
            println!("{}", ctx.dirs.root.join("kaika.toml").display());
            Ok(())
        }
        ["get", k] => {
            println!("{}", ctx.cfg.get(k)?);
            Ok(())
        }
        ["set", k, v] => {
            let mut c = ctx.cfg.clone();
            c.set(k, v)?;
            c.save(&ctx.dirs)?;
            Ok(())
        }
        _ => Err(KErr::usage(
            "usage: kaika config [get KEY | set KEY VALUE | path]",
        )),
    }
}
