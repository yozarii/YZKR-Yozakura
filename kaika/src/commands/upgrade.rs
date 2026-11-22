//! `kaika upgrade` and `kaika update`: refresh the indexes, and upgrade packages through their backend or, for vendor
//! downloads, through the recipe's release feed.

use super::rows::find_installed;
use super::run::run_plan;
use super::{Ctx, KErr, KResult, say};
use crate::asset::norm_version;
use crate::backend;
use crate::cli::Act;
use crate::direct::Job;
use crate::manifest::{Entry, Manifest};
use crate::plan::{Action, Plan};
use crate::recipe;
use crate::runner::argv;

/// Refresh Homebrew's indexes (the only backend with a separate index).
pub(super) fn update(ctx: &Ctx) -> KResult {
    if !ctx.r.exists("brew") {
        say(ctx, "nothing to refresh (no brew)");
        return Ok(());
    }
    let code = ctx.r.run_live(&argv(&["brew", "update"]))?;
    if code != 0 {
        return Err(KErr::fail(format!("`brew update` failed (exit {code})")));
    }
    Ok(())
}

/// The newer download for a vendor-installed app, or `None` when the installed one is current. A team ID recorded
/// at install is carried over, so the upgrade must be signed by the same vendor.
pub(super) fn direct_upgrade_job(ctx: &Ctx, e: &Entry, force: bool) -> KResult<Option<Job>> {
    let Some(rec) = recipe::find(&ctx.dirs, &e.name) else {
        return Err(KErr::fail(format!(
            "no recipe for '{}', so kaika cannot check for updates",
            e.name
        )));
    };
    let asset = recipe::latest(ctx.r, &rec)?;
    let same = if asset.version.is_empty() {
        asset.url == e.reference
    } else {
        norm_version(&asset.version) == norm_version(&e.version)
    };
    if same {
        return Ok(None);
    }
    let team = (!e.team_id.is_empty()).then(|| e.team_id.clone());
    Ok(Some(Job::from_asset(&rec, asset, force, true, team)))
}

pub(super) fn upgrade(ctx: &Ctx, names: &[String], all: bool, act: Act) -> KResult {
    if names.is_empty() && !all {
        return Err(KErr::usage("name the packages to upgrade, or pass --all"));
    }
    let mut plan = Plan {
        title: if all {
            "upgrade everything".into()
        } else {
            format!("upgrade {}", names.join(", "))
        },
        actions: vec![],
        warnings: vec![],
    };
    if all {
        plan_everything(ctx, &mut plan, act)?;
    } else {
        for n in names {
            plan_one(ctx, &mut plan, n, act)?;
        }
    }
    run_plan(ctx, plan, act)
}

/// Every backend's own upgrade-all, then every vendor download whose feed has something newer.
fn plan_everything(ctx: &Ctx, plan: &mut Plan, act: Act) -> KResult {
    let m = Manifest::load(&ctx.dirs)?;
    for b in backend::available(ctx.r) {
        plan.actions.push(Action::Cmd {
            argv: b.upgrade_argv(None),
            admin: b.needs_admin(),
        });
    }
    for e in m.packages.iter().filter(|e| e.source == "direct") {
        match direct_upgrade_job(ctx, e, act.force) {
            Ok(Some(j)) => plan.actions.push(Action::Direct(Box::new(j))),
            Ok(None) => {}
            Err(err) => plan.warnings.push(format!("{}: {}", e.name, err.msg)),
        }
    }
    Ok(())
}

fn plan_one(ctx: &Ctx, plan: &mut Plan, raw: &str, act: Act) -> KResult {
    let row = find_installed(ctx, raw)?;
    match row.source.as_str() {
        "app" => Err(KErr::fail(format!(
            "{} is managed by hand; kaika only lists it",
            row.name
        ))),
        "direct" => {
            let m = Manifest::load(&ctx.dirs)?;
            let e = m
                .get("direct", &row.name)
                .cloned()
                .ok_or_else(|| KErr::fail("manifest entry missing"))?;
            match direct_upgrade_job(ctx, &e, act.force)? {
                Some(j) => plan.actions.push(Action::Direct(Box::new(j))),
                None => say(ctx, &format!("{} is up to date", e.name)),
            }
            Ok(())
        }
        src => {
            let b = backend::by_id(src).ok_or_else(|| KErr::fail("unknown backend"))?;
            plan.actions.push(Action::Cmd {
                argv: b.upgrade_argv(Some(&row.reference)),
                admin: b.needs_admin(),
            });
            Ok(())
        }
    }
}
