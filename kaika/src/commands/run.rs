//! Showing a plan, getting the go-ahead, and carrying it out.

use super::{Ctx, KErr, KResult, say};
use crate::backend;
use crate::cli::Act;
use crate::direct::{self, DErr, Job};
use crate::manifest::{Entry, Manifest, now};
use crate::plan::{Action, Decision, Plan, decide};
use crate::recipe;
use std::io::{BufRead, IsTerminal, Write};

/// Is there a terminal to ask? `KAIKA_ASSUME_TTY=1` makes the prompt readable from stdin (tests).
fn tty() -> bool {
    std::env::var("KAIKA_ASSUME_TTY").as_deref() == Ok("1") || std::io::stdin().is_terminal()
}

fn prompt() -> bool {
    print!("Proceed? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Show the plan, ask for the go-ahead unless `-y`, and run it. `-n` shows it and stops; with no terminal and no
/// `-y` nothing runs and the exit code is 4.
pub(super) fn run_plan(ctx: &Ctx, plan: Plan, act: Act) -> KResult {
    if confirm(ctx, &plan, act)? {
        execute(ctx, &plan)?;
    }
    Ok(())
}

/// Whether to run the plan. Prints it when it is shown.
fn confirm(ctx: &Ctx, plan: &Plan, act: Act) -> KResult<bool> {
    if plan.is_empty() {
        say(ctx, "nothing to do");
        return Ok(false);
    }
    let show = |ctx: &Ctx| {
        if ctx.json {
            println!("{}", plan.to_json());
        } else {
            print!("{}", plan.render());
        }
    };
    let d = decide(act.dry_run, act.yes, tty(), || {
        if !ctx.json {
            show(ctx);
        }
        prompt()
    });
    match d {
        Decision::DryRun => {
            show(ctx);
            Ok(false)
        }
        Decision::NeedsConfirmation => {
            show(ctx);
            Err(KErr {
                code: 4,
                msg: "confirmation needed but there is no terminal to ask; nothing changed. Re-run with -y to proceed, or -n to only see the plan".into(),
            })
        }
        Decision::Cancelled => Err(KErr::fail("cancelled; nothing changed")),
        Decision::Run => {
            if act.yes && !ctx.quiet && !ctx.json {
                print!("{}", plan.render());
            }
            Ok(true)
        }
    }
}

/// Carry out each action in order, stopping at the first failure. The manifest is written after every change that
/// touches it, so an interrupted plan leaves a manifest that matches what was done.
fn execute(ctx: &Ctx, plan: &Plan) -> KResult {
    let mut m = Manifest::load(&ctx.dirs)?;
    for a in &plan.actions {
        match a {
            Action::Cmd { argv, .. } => run_backend_command(ctx, argv)?,
            Action::Direct(job) => install_direct(ctx, &mut m, job)?,
            Action::Record {
                source,
                name,
                reference,
                resolved_by,
            } => {
                record_backend_install(ctx, &mut m, source, name, reference, resolved_by)?;
            }
            Action::Unrecord { source, name } => {
                m.remove(source, name);
                m.save()?;
            }
            Action::RemoveDirect { entry, zap } => remove_direct(ctx, &mut m, entry, *zap)?,
        }
    }
    Ok(())
}

fn run_backend_command(ctx: &Ctx, argv: &[String]) -> KResult {
    let code = ctx.r.run_live(argv)?;
    if code == 0 {
        return Ok(());
    }
    let hint = if backend::mas::is_mas(argv) {
        format!("; {}", backend::mas::SIGN_IN_HINT)
    } else {
        String::new()
    };
    Err(KErr::fail(format!(
        "`{}` failed (exit {code}){hint}",
        argv.join(" ")
    )))
}

/// Download, verify and install a vendor download, then record it (and the recipe it was found by).
fn install_direct(ctx: &Ctx, m: &mut Manifest, job: &Job) -> KResult {
    let inst = match direct::run(ctx.r, &ctx.dirs, job) {
        Ok(i) => i,
        Err(DErr::Verify(failures)) => {
            let list = failures
                .iter()
                .map(|x| format!("  - {x}"))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(KErr {
                code: 3,
                msg: format!(
                    "verification failed for {}; nothing was installed:\n{list}\n(-f overrides this, at your own risk)",
                    job.name
                ),
            });
        }
        Err(DErr::Other(e)) => return Err(e.into()),
    };
    if !inst.overridden.is_empty() {
        eprintln!("WARNING: verification failed and was overridden with -f:");
        for f in &inst.overridden {
            eprintln!("  - {f}");
        }
    }
    m.upsert(Entry {
        name: job.name.clone(),
        source: "direct".into(),
        version: job.version.clone(),
        path: inst.path.clone(),
        reference: job.url.clone(),
        team_id: inst.team_id.clone(),
        update_source: job.update_source.clone(),
        resolved_by: job.resolved_by.clone(),
        installed_at: now(),
    });
    m.save()?;
    if let Some(rec) = &job.learned {
        recipe::save_learned(&ctx.dirs, rec)?;
    }
    let team = if inst.team_id.is_empty() {
        String::new()
    } else {
        format!(" (signed by Team ID {})", inst.team_id)
    };
    let at = if inst.path.is_empty() {
        String::new()
    } else {
        format!("at {}", inst.path)
    };
    say(ctx, &format!("installed {} {at}{team}", job.name));
    Ok(())
}

/// Record what the backend now reports: an App Store listing's name differs from the installed app's.
fn record_backend_install(
    ctx: &Ctx,
    m: &mut Manifest,
    source: &str,
    name: &str,
    reference: &str,
    resolved_by: &str,
) -> KResult {
    let found = backend::by_id(source).and_then(|b| {
        b.installed(ctx.r)
            .into_iter()
            .find(|p| p.name.eq_ignore_ascii_case(name) || p.reference == reference)
    });
    let version = found
        .as_ref()
        .map(|p| p.version.clone())
        .unwrap_or_default();
    let name = found.map_or_else(|| name.to_string(), |p| p.name);
    m.upsert(Entry {
        name,
        source: source.into(),
        version,
        reference: reference.into(),
        resolved_by: resolved_by.into(),
        installed_at: now(),
        ..Default::default()
    });
    m.save()?;
    Ok(())
}

fn remove_direct(ctx: &Ctx, m: &mut Manifest, entry: &Entry, zap: bool) -> KResult {
    let gone = direct::uninstall(ctx.r, &ctx.dirs, entry, zap)?;
    m.remove("direct", &entry.name);
    m.save()?;
    if ctx.verbose {
        for g in gone {
            say(ctx, &format!("removed {g}"));
        }
    }
    say(ctx, &format!("removed {}", entry.name));
    Ok(())
}
