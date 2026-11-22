//! What is installed, from every source: the rows behind `list`, `info`, `uninstall` and `upgrade`.

use super::{Ctx, KErr, KResult};
use crate::backend::{self, app};
use crate::manifest::Manifest;
use crate::select;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// One installed package, as `kaika list` shows it.
#[derive(Serialize, Clone, Debug)]
pub(super) struct Row {
    /// Backend id (`brew`, `cask`, `npm`, `uv`, `mas`), `direct` for vendor downloads, or `app` for an app nobody manages.
    pub source: String,
    pub name: String,
    pub version: String,
    /// What the backend wants to be told to act on it (an App Store id, otherwise the name).
    pub reference: String,
    /// Whether kaika's manifest knows it.
    pub managed: bool,
}

/// Every backend's installed packages, kaika's vendor downloads, and the apps in the applications folder that
/// nothing above owns, sorted by source and name.
pub(super) fn live_rows(ctx: &Ctx) -> KResult<Vec<Row>> {
    let m = Manifest::load(&ctx.dirs)?;
    let mut rows = vec![];
    let mut owned: HashSet<String> = HashSet::new();
    for b in backend::available(ctx.r) {
        for p in b.installed(ctx.r) {
            if b.id() == "cask" {
                owned.insert(app::norm(&p.name));
            }
            rows.push(Row {
                source: b.id().into(),
                managed: m.get(b.id(), &p.name).is_some(),
                name: p.name,
                version: p.version,
                reference: p.reference,
            });
        }
    }
    for e in m.packages.iter().filter(|e| e.source == "direct") {
        owned.insert(app::norm(&e.name));
        if let Some(stem) = Path::new(&e.path).file_stem() {
            owned.insert(app::norm(&stem.to_string_lossy()));
        }
        rows.push(Row {
            source: "direct".into(),
            name: e.name.clone(),
            version: e.version.clone(),
            reference: e.reference.clone(),
            managed: true,
        });
    }
    for a in app::unowned(&ctx.dirs.apps, &owned) {
        rows.push(Row {
            source: "app".into(),
            name: a,
            version: String::new(),
            reference: String::new(),
            managed: false,
        });
    }
    rows.sort_by(|a, b| {
        (&a.source, a.name.to_lowercase()).cmp(&(&b.source, b.name.to_lowercase()))
    });
    Ok(rows)
}

pub(super) fn list(ctx: &Ctx) -> KResult {
    let rows = live_rows(ctx)?;
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string(&rows).map_err(anyhow::Error::from)?
        );
        return Ok(());
    }
    println!("{:<8}{:<34}{:<16}KAIKA", "SOURCE", "NAME", "VERSION");
    for r in rows {
        println!(
            "{:<8}{:<34}{:<16}{}",
            r.source,
            r.name,
            r.version,
            if r.managed { "yes" } else { "" }
        );
    }
    Ok(())
}

/// The one installed package that `raw` (`name` or `source:name`) names; an error when none or several do.
pub(super) fn find_installed(ctx: &Ctx, raw: &str) -> KResult<Row> {
    let t = select::parse_target(raw).map_err(KErr::usage)?;
    let mut found: Vec<Row> = live_rows(ctx)?
        .into_iter()
        .filter(|r| {
            r.name.eq_ignore_ascii_case(&t.name)
                && t.source.as_deref().is_none_or(|s| s == r.source)
        })
        .collect();
    match found.len() {
        0 => Err(KErr::fail(format!(
            "'{}' is not installed (see `kaika list`)",
            t.name
        ))),
        1 => Ok(found.remove(0)),
        _ => Err(KErr::usage(format!(
            "'{}' is installed from several sources ({}); use source:name",
            t.name,
            found
                .iter()
                .map(|r| r.source.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}
