//! `kaika install`: turn each name (or URL) into backend commands or a verified vendor download, show the plan, run it.
//!
//! Which source carries a name is decided by rules in code, never by a model: curated sources before open
//! registries, ties shown to the person (`decision::decide_backend`), plain-text guesses by `names`. Discovery of
//! vendor downloads (`discovery`) is only for names no backend carries.

use super::run::run_plan;
use super::{Ctx, KErr, KResult};
use crate::backend::{self, Candidate};
use crate::cli::Act;
use crate::decision;
use crate::direct::Job;
use crate::discovery;
use crate::names;
use crate::outcome::Outcome;
use crate::plan::{Action, Plan};
use crate::recipe::{self, Asset};
use crate::select::{self, Choice, Target};

/// What a name resolved to: a backend package, or a vendor download. The note, if any, goes into the plan.
enum Resolution {
    Backend {
        cand: Candidate,
        note: Option<String>,
    },
    Direct(Box<Job>, Option<String>),
}

pub(super) fn install(ctx: &Ctx, names: &[String], act: Act) -> KResult {
    let mut plan = Plan {
        title: format!("install {}", names.join(", ")),
        actions: vec![],
        warnings: vec![],
    };
    for n in names {
        let is_url = n.starts_with("https://") || n.starts_with("http://");
        let (resolution, explicit_source) = if is_url {
            (from_url(ctx, n, act)?, false)
        } else {
            let t = select::parse_target(n).map_err(KErr::usage)?;
            let explicit = t.source.is_some();
            (resolve_install(ctx, &t, act)?, explicit)
        };
        add_to_plan(&mut plan, resolution, explicit_source)?;
    }
    run_plan(ctx, plan, act)
}

fn add_to_plan(plan: &mut Plan, resolution: Resolution, explicit_source: bool) -> KResult {
    match resolution {
        Resolution::Backend { cand, note } => {
            let b = backend::by_id(&cand.source).ok_or_else(|| KErr::fail("unknown backend"))?;
            plan.actions.push(Action::Cmd {
                argv: b.install_argv(&cand.reference),
                admin: b.needs_admin(),
            });
            plan.actions.push(Action::Record {
                source: cand.source.clone(),
                name: cand.name.clone(),
                reference: cand.reference.clone(),
                resolved_by: if explicit_source {
                    "explicit source".into()
                } else {
                    "backend lookup".into()
                },
            });
            plan.warnings
                .extend(note.map(|n| format!("{}: {n}", cand.name)));
        }
        Resolution::Direct(job, note) => {
            plan.warnings
                .extend(note.map(|n| format!("{}: {n}", job.name)));
            plan.actions.push(Action::Direct(job));
        }
    }
    Ok(())
}

// ---------- resolving a name ----------

fn resolve_install(ctx: &Ctx, t: &Target, act: Act) -> KResult<Resolution> {
    let backends = backend::available(ctx.r);
    if let Some(src) = &t.source {
        return resolve_explicit(ctx, &backends, t, src, act);
    }
    let hits = lookup(ctx, &backends, &t.name);
    let choice = select::choose(&t.name, &hits, &ctx.cfg.order());
    // Only an open registry has this exact name, but a reviewed source has something named like it ("chrome" is
    // npm's unrelated package; the app is the cask google-chrome): never guess, show the options.
    let open_only = match &choice {
        Choice::One(c) | Choice::Several { chosen: c, .. } => tier_of(c) == 2,
        Choice::None => false,
    };
    if matches!(choice, Choice::None) || open_only {
        if let Some(found) = plain_text_guess(ctx, t, act)? {
            return Ok(found);
        }
    }
    if open_only {
        refuse_open_registry(t, &hits, &choice)?;
    }
    match choice {
        Choice::None => discover(ctx, &t.name, act),
        Choice::One(c) => resolution_for(ctx, c, None, act),
        Choice::Several { chosen, others } => resolve_tie(ctx, chosen, others, act),
    }
}

/// `source:name`: the person chose the backend. An App Store name is looked up to its id.
fn resolve_explicit(
    ctx: &Ctx,
    backends: &[Box<dyn backend::Backend>],
    t: &Target,
    src: &str,
    act: Act,
) -> KResult<Resolution> {
    if src == "direct" {
        return Ok(Resolution::Direct(
            Box::new(direct_job_for(ctx, &t.name, act)?),
            None,
        ));
    }
    let b = backends
        .iter()
        .find(|b| b.id() == src)
        .ok_or_else(|| KErr::fail(format!("{src} is not available on this machine")))?;
    let cand = if src == "mas" && !t.name.bytes().all(|c| c.is_ascii_digit()) {
        select::exact(&t.name, &b.search(ctx.r, &t.name), &ctx.cfg.order())
            .into_iter()
            .next()
            .ok_or_else(|| KErr::fail(format!("the App Store has no app named '{}'", t.name)))?
    } else {
        Candidate {
            source: src.to_string(),
            name: t.name.clone(),
            reference: t.name.clone(),
            desc: String::new(),
        }
    };
    Ok(Resolution::Backend { cand, note: None })
}

/// Every backend's hits for the name, plus a recipe of that name.
fn lookup(ctx: &Ctx, backends: &[Box<dyn backend::Backend>], name: &str) -> Vec<Candidate> {
    // Package names are hyphenated, people type spaces: "Sublime Merge" finds `sublime-merge`.
    let term = name
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    let mut hits: Vec<Candidate> = backends
        .iter()
        .flat_map(|b| b.search(ctx.r, &term))
        .collect();
    if let Some(rec) = recipe::find(&ctx.dirs, name) {
        hits.push(Candidate {
            source: "direct".into(),
            name: rec.name.clone(),
            reference: rec.name.clone(),
            desc: rec.description.clone(),
        });
    }
    hits
}

fn tier_of(c: &Candidate) -> u8 {
    decision::tier(&format!("{}:{}", c.source, c.name))
}

/// Plain text: "chrome", "chome" and "vscode" mean google-chrome and visual-studio-code (`names`). A guess at a
/// typo is confirmed by a person looking at the plan, never by `-y` alone.
fn plain_text_guess(ctx: &Ctx, t: &Target, act: Act) -> KResult<Option<Resolution>> {
    let Ok(f) = names::best(&t.name, &names::popular(ctx.r, &ctx.dirs)) else {
        return Ok(None);
    };
    if f.how == names::How::Typo && act.yes {
        return Err(KErr::fail(format!(
            "'{}' looks like a typo for {}:{} but -y would install it unseen; nothing was installed. Run without -y to confirm the plan, or name it: kaika install {}:{}",
            t.name, f.entry.source, f.entry.name, f.entry.source, f.entry.name
        )));
    }
    let cand = Candidate {
        source: f.entry.source.into(),
        name: f.entry.name.clone(),
        reference: f.entry.name.clone(),
        desc: String::new(),
    };
    resolution_for(ctx, cand, Some(f.why(&t.name)), act).map(Some)
}

/// An exact name that only an open registry carries, next to a reviewed source's near match: say so, install nothing.
fn refuse_open_registry(t: &Target, hits: &[Candidate], choice: &Choice) -> KResult {
    let mut near = select::near(&t.name, hits);
    near.sort_by_key(|c| (c.source != "cask", c.name.len())); // apps (casks) first: what a short name usually means
    let Some(first_near) = near.first() else {
        return Ok(());
    };
    let list = near
        .iter()
        .map(|c| format!("{}:{}", c.source, c.name))
        .collect::<Vec<_>>()
        .join(", ");
    let exact = match choice {
        Choice::One(c) | Choice::Several { chosen: c, .. } => format!("{}:{}", c.source, c.name),
        Choice::None => String::new(),
    };
    Err(KErr::fail(format!(
        "'{}' exactly names only {exact}, an unreviewed registry package, while reviewed sources have {list}; nothing was installed. Say which you mean with source:name, for example kaika install {}:{} (or {exact} for the registry package)",
        t.name, first_near.source, first_near.name
    )))
}

/// The name exists in several sources. Trust tiers settle what they can; a true tie keeps the preference order and
/// the plan describes every tied candidate so a person can choose another with `source:name`.
fn resolve_tie(
    ctx: &Ctx,
    chosen: Candidate,
    others: Vec<Candidate>,
    act: Act,
) -> KResult<Resolution> {
    let mut all = vec![chosen.clone()];
    all.extend(others.iter().cloned());
    let ids: Vec<(String, String)> = all
        .iter()
        .map(|c| (format!("{}:{}", c.source, c.name), String::new()))
        .collect();
    let note = match decision::decide_backend(&ids) {
        Outcome::Picked(p) => {
            let also = others
                .iter()
                .map(|o| format!("{}:{}", o.source, o.name))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "also available as {also} (less trusted sources are never chosen over {}); {}",
                p.id, p.how
            )
        }
        Outcome::Unsure(why) => {
            let top = decision::tier(&ids[0].0);
            let described: Vec<String> = all
                .iter()
                .filter(|c| tier_of(c) == top)
                .map(|c| describe_candidate(ctx, c))
                .collect();
            format!(
                "{why}; chose {}:{} by preference order. Candidates: {}. Use source:name to choose another",
                chosen.source,
                chosen.name,
                described.join("; ")
            )
        }
    };
    resolution_for(ctx, chosen, Some(note), act)
}

/// `source:name (what it is)`, the description clipped to 90 characters.
fn describe_candidate(ctx: &Ctx, c: &Candidate) -> String {
    let d = backend::by_id(&c.source)
        .map(|b| b.describe(ctx.r, &c.reference))
        .unwrap_or_default();
    let d: String = d.chars().take(90).collect();
    if d.is_empty() {
        format!("{}:{}", c.source, c.name)
    } else {
        format!("{}:{} ({d})", c.source, c.name)
    }
}

/// A chosen candidate as something to put in the plan.
fn resolution_for(ctx: &Ctx, c: Candidate, note: Option<String>, act: Act) -> KResult<Resolution> {
    if c.source == "direct" {
        Ok(Resolution::Direct(
            Box::new(direct_job_for(ctx, &c.name, act)?),
            note,
        ))
    } else {
        Ok(Resolution::Backend { cand: c, note })
    }
}

// ---------- vendor downloads ----------

/// The download a recipe names, at its latest version.
fn direct_job_for(ctx: &Ctx, name: &str, act: Act) -> KResult<Job> {
    let rec = recipe::find(&ctx.dirs, name)
        .ok_or_else(|| KErr::fail(format!("no recipe for '{name}'")))?;
    let asset = recipe::latest(ctx.r, &rec)?;
    Ok(Job::from_asset(&rec, asset, act.force, false, None))
}

/// A download found online: a job plus the recipe to save once it installs.
fn discovered_resolution(name: &str, d: discovery::Discovered, act: Act) -> Resolution {
    let learned = d.recipe(name);
    let asset = Asset {
        name: recipe::file_name(&d.url),
        url: d.url.clone(),
        version: d.version.clone(),
        sha256: d.sha256.clone(),
    };
    let mut job = Job::from_asset(&learned, asset, act.force, false, None);
    job.resolved_by = format!("found via {}", d.source);
    job.learned = Some(learned);
    Resolution::Direct(Box::new(job), Some(format!("found online: {}", d.evidence)))
}

/// Look an app up online: no backend carries `name`.
fn discover(ctx: &Ctx, name: &str, act: Act) -> KResult<Resolution> {
    let not_found = |why: &str| {
        KErr::fail(format!(
            "no package named '{name}' found ({why}); nothing changed"
        ))
    };
    if !ctx.cfg.models_enabled() {
        return Err(not_found(
            "no backend carries it, and vendor-site discovery is off",
        ));
    }
    match discovery::discover(ctx.r, ctx.decider, name) {
        Outcome::Unsure(why) => Err(not_found(&format!("vendor-site discovery unsure: {why}"))),
        Outcome::Picked(d) => Ok(discovered_resolution(name, d, act)),
    }
}

/// A short name for an app given only as a URL: the site's own name, or the GitHub project's.
fn name_from_url(url: &str) -> String {
    let host = crate::html::host(url);
    if host == "github.com" {
        if let Some(repo) = url
            .split_once("://")
            .map(|x| x.1)
            .and_then(|r| r.split('/').nth(2))
        {
            return repo.to_string();
        }
    }
    let labels: Vec<&str> = host.split('.').collect();
    labels
        .get(labels.len().saturating_sub(2))
        .copied()
        .unwrap_or(&host)
        .to_string()
}

fn from_url(ctx: &Ctx, url: &str, act: Act) -> KResult<Resolution> {
    let name = name_from_url(url);
    match discovery::from_url(ctx.r, ctx.decider, &name, url) {
        Outcome::Picked(d) => Ok(discovered_resolution(&name, d, act)),
        Outcome::Unsure(why) => Err(KErr::fail(format!(
            "could not find a download at {url}: {why}; nothing changed"
        ))),
    }
}
