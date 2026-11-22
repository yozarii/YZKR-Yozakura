//! `kaika uninstall`: remove packages through their own backend, or move a vendor download to the Trash.

use super::rows::find_installed;
use super::run::run_plan;
use super::{Ctx, KErr, KResult};
use crate::backend;
use crate::cli::Act;
use crate::manifest::Manifest;
use crate::plan::{Action, Plan};

pub(super) fn uninstall(ctx: &Ctx, names: &[String], zap: bool, act: Act) -> KResult {
    let m = Manifest::load(&ctx.dirs)?;
    let mut plan = Plan {
        title: format!("uninstall {}", names.join(", ")),
        actions: vec![],
        warnings: vec![],
    };
    for n in names {
        let row = find_installed(ctx, n)?;
        match row.source.as_str() {
            "app" => {
                return Err(KErr::fail(format!(
                    "{} is managed by hand; kaika only lists it",
                    row.name
                )));
            }
            "direct" => {
                let entry = m
                    .get("direct", &row.name)
                    .cloned()
                    .ok_or_else(|| KErr::fail("manifest entry missing"))?;
                plan.actions.push(Action::RemoveDirect { entry, zap });
            }
            src => {
                let b = backend::by_id(src).ok_or_else(|| KErr::fail("unknown backend"))?;
                plan.actions.push(Action::Cmd {
                    argv: b.uninstall_argv(&row.reference, zap),
                    admin: b.needs_admin(),
                });
                plan.actions.push(Action::Unrecord {
                    source: src.into(),
                    name: row.name.clone(),
                });
                if zap && src != "cask" {
                    plan.warnings.push(format!(
                        "--zap only applies to casks and direct installs; ignored for {src}:{}",
                        row.name
                    ));
                }
            }
        }
    }
    run_plan(ctx, plan, act)
}
