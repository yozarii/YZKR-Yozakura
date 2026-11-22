//! Read-only lookups: `kaika search` and `kaika info`.

use super::rows::{Row, live_rows};
use super::{Ctx, KErr, KResult, say};
use crate::backend::{self, Candidate};
use crate::manifest::{Entry, Manifest};
use crate::recipe::{self, Recipe};
use crate::select;

/// Every backend's hits for the query, and the recipes whose name or description contains it.
pub(super) fn search(ctx: &Ctx, query: &str) -> KResult {
    let mut hits: Vec<Candidate> = vec![];
    for b in backend::available(ctx.r) {
        hits.extend(b.search(ctx.r, query));
    }
    let q = query.to_lowercase();
    for rec in recipe::all(&ctx.dirs) {
        if rec.name.to_lowercase().contains(&q) || rec.description.to_lowercase().contains(&q) {
            hits.push(Candidate {
                source: "direct".into(),
                name: rec.name,
                reference: String::new(),
                desc: rec.description,
            });
        }
    }
    if hits.is_empty() {
        return Err(KErr::fail(format!("no results for '{query}'")));
    }
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string(&hits).map_err(anyhow::Error::from)?
        );
    } else {
        for h in hits {
            println!("{:<28}{}", format!("{}:{}", h.source, h.name), h.desc);
        }
    }
    Ok(())
}

/// What kaika knows about a name: the installed package(s), the manifest entry, the recipe; or, for a name that is
/// not installed, the backends that carry it.
pub(super) fn info(ctx: &Ctx, name: &str) -> KResult {
    let rows: Vec<Row> = live_rows(ctx)?
        .into_iter()
        .filter(|r| r.name.eq_ignore_ascii_case(name))
        .collect();
    let m = Manifest::load(&ctx.dirs)?;
    let rec = recipe::find(&ctx.dirs, name);
    if rows.is_empty() && rec.is_none() {
        return available_elsewhere(ctx, name);
    }
    if ctx.json {
        let entries: Vec<&Entry> = m.find_name(name);
        println!(
            "{}",
            serde_json::json!({ "installed": rows, "manifest": entries, "recipe": rec })
        );
        return Ok(());
    }
    for r in &rows {
        print_installed(r, &m);
    }
    if let Some(rec) = rec {
        print_recipe(&rec);
    }
    Ok(())
}

/// A name that is not installed and has no recipe: which backends carry it, if any.
fn available_elsewhere(ctx: &Ctx, name: &str) -> KResult {
    let mut hits = vec![];
    for b in backend::available(ctx.r) {
        hits.extend(select::exact(
            name,
            &b.search(ctx.r, name),
            &ctx.cfg.order(),
        ));
    }
    if hits.is_empty() {
        return Err(KErr::fail(format!("nothing known about '{name}'")));
    }
    for h in hits {
        say(
            ctx,
            &format!("{}:{}  available, not installed", h.source, h.name),
        );
    }
    Ok(())
}

fn print_installed(r: &Row, m: &Manifest) {
    println!(
        "{}:{}  version {}  {}",
        r.source,
        r.name,
        if r.version.is_empty() {
            "unknown"
        } else {
            &r.version
        },
        if r.managed {
            "managed by kaika"
        } else {
            "not in the manifest"
        }
    );
    let Some(e) = m.get(&r.source, &r.name) else {
        return;
    };
    for (label, value) in [
        ("path:      ", &e.path),
        ("team id:   ", &e.team_id),
        ("updates:   ", &e.update_source),
        ("found by:  ", &e.resolved_by),
    ] {
        if !value.is_empty() {
            println!("  {label}{value}");
        }
    }
}

fn print_recipe(rec: &Recipe) {
    println!(
        "recipe: {}{}",
        rec.name,
        if rec.description.is_empty() {
            String::new()
        } else {
            format!(" ({})", rec.description)
        }
    );
}
