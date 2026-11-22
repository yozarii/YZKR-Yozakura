//! Command handlers. Every change goes through a plan that is shown first.

use crate::backend::{self, app, Candidate};
use crate::cli::{Act, Cmd};
use crate::config::Config;
use crate::decision::{self, Decider};
use crate::discovery;
use crate::model;
use crate::names;
use crate::net;
use crate::direct::{self, DErr, Job};
use crate::manifest::{now, Entry, Manifest};
use crate::paths::Dirs;
use crate::plan::{decide, Action, Decision, Plan};
use crate::recipe::{self, Asset};
use crate::outcome::Outcome;
use crate::runner::{argv, Runner};
use crate::select::{self, Choice, Target};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{BufRead, IsTerminal, Write};

pub struct Ctx<'a> {
    pub dirs: Dirs,
    pub r: &'a dyn Runner,
    /// Decides between third-party candidates. Loads its model lazily, only when discovery asks.
    pub decider: &'a dyn Decider,
    pub cfg: Config,
    pub json: bool,
    pub quiet: bool,
    pub verbose: bool,
}

/// An error with the exit code it maps to: 1 failure, 2 usage, 3 verification failed, 4 needs confirmation.
#[derive(Debug)]
pub struct KErr {
    pub code: i32,
    pub msg: String,
}

impl KErr {
    pub fn fail(m: impl Into<String>) -> KErr {
        KErr { code: 1, msg: m.into() }
    }
    pub fn usage(m: impl Into<String>) -> KErr {
        KErr { code: 2, msg: m.into() }
    }
}

impl From<anyhow::Error> for KErr {
    fn from(e: anyhow::Error) -> KErr {
        KErr::fail(format!("{e:#}"))
    }
}

pub type KResult<T = ()> = Result<T, KErr>;

/// Exit code for "the network is unreachable" (sysexits.h EX_UNAVAILABLE).
pub const EX_UNAVAILABLE: i32 = 69;

/// Commands that cannot work without a network fail in about two seconds, before any solver or model starts.
fn needs_network(cmd: &Cmd) -> bool {
    matches!(cmd, Cmd::Install { .. } | Cmd::Upgrade { .. } | Cmd::Update | Cmd::Outdated { .. } | Cmd::Search { .. })
}

pub fn dispatch(ctx: &Ctx, cmd: Cmd) -> KResult {
    if needs_network(&cmd) && !net::online(ctx.r) {
        return Err(KErr {
            code: EX_UNAVAILABLE,
            msg: format!("offline: could not reach {} within 2 s, so nothing was started or changed. list, info, uninstall, cleanup, doctor and config still work offline", net::PROBE),
        });
    }
    match cmd {
        Cmd::Install { names, act } => install(ctx, &names, act),
        Cmd::Uninstall { names, zap, act } => uninstall(ctx, &names, zap, act),
        Cmd::Upgrade { names, all, act } => upgrade(ctx, &names, all, act),
        Cmd::Update => update(ctx),
        Cmd::List => list(ctx),
        Cmd::Search { query } => search(ctx, &query),
        Cmd::Info { name } => info(ctx, &name),
        Cmd::Outdated { notify } => outdated(ctx, notify),
        Cmd::Doctor => doctor(ctx),
        Cmd::Cleanup { dry_run } => cleanup(ctx, dry_run),
        Cmd::Schedule { action, at, dry_run } => schedule(ctx, &action, &at, dry_run),
        Cmd::Config { args } => config(ctx, &args),
    }
}

fn say(ctx: &Ctx, msg: &str) {
    if !ctx.quiet {
        println!("{msg}");
    }
}

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

// ---------- rows: what is installed, from every source ----------

#[derive(Serialize, Clone, Debug)]
pub struct Row {
    pub source: String,
    pub name: String,
    pub version: String,
    pub reference: String,
    pub managed: bool,
}

fn live_rows(ctx: &Ctx) -> KResult<Vec<Row>> {
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
        if let Some(stem) = std::path::Path::new(&e.path).file_stem() {
            owned.insert(app::norm(&stem.to_string_lossy()));
        }
        rows.push(Row { source: "direct".into(), name: e.name.clone(), version: e.version.clone(), reference: e.reference.clone(), managed: true });
    }
    for a in app::unowned(&ctx.dirs.apps, &owned) {
        rows.push(Row { source: "app".into(), name: a, version: String::new(), reference: String::new(), managed: false });
    }
    rows.sort_by(|a, b| (&a.source, a.name.to_lowercase()).cmp(&(&b.source, b.name.to_lowercase())));
    Ok(rows)
}

fn list(ctx: &Ctx) -> KResult {
    let rows = live_rows(ctx)?;
    if ctx.json {
        println!("{}", serde_json::to_string(&rows).map_err(anyhow::Error::from)?);
        return Ok(());
    }
    println!("{:<8}{:<34}{:<16}{}", "SOURCE", "NAME", "VERSION", "KAIKA");
    for r in rows {
        println!("{:<8}{:<34}{:<16}{}", r.source, r.name, r.version, if r.managed { "yes" } else { "" });
    }
    Ok(())
}

// ---------- plan, confirm, execute ----------

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
            Err(KErr { code: 4, msg: "confirmation needed but there is no terminal to ask; nothing changed. Re-run with -y to proceed, or -n to only see the plan".into() })
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

fn execute(ctx: &Ctx, plan: &Plan) -> KResult {
    let mut m = Manifest::load(&ctx.dirs)?;
    for a in &plan.actions {
        match a {
            Action::Cmd { argv, .. } => {
                let code = ctx.r.run_live(argv)?;
                if code != 0 {
                    let hint = if backend::mas::is_mas(argv) { format!("; {}", backend::mas::SIGN_IN_HINT) } else { String::new() };
                    return Err(KErr::fail(format!("`{}` failed (exit {code}){hint}", argv.join(" "))));
                }
            }
            Action::Direct(job) => {
                let inst = match direct::run(ctx.r, &ctx.dirs, job) {
                    Ok(i) => i,
                    Err(DErr::Verify(f)) => {
                        let list = f.iter().map(|x| format!("  - {x}")).collect::<Vec<_>>().join("\n");
                        return Err(KErr {
                            code: 3,
                            msg: format!("verification failed for {}; nothing was installed:\n{list}\n(-f overrides this, at your own risk)", job.name),
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
                let team = if inst.team_id.is_empty() { String::new() } else { format!(" (signed by Team ID {})", inst.team_id) };
                say(ctx, &format!("installed {} {}{team}", job.name, if inst.path.is_empty() { String::new() } else { format!("at {}", inst.path) }));
            }
            Action::Record { source, name, reference, resolved_by } => {
                // Record what the backend now reports (an App Store listing's name differs from the installed app's).
                let found = backend::by_id(source).and_then(|b| {
                    b.installed(ctx.r).into_iter().find(|p| p.name.eq_ignore_ascii_case(name) || p.reference == *reference)
                });
                let version = found.as_ref().map(|p| p.version.clone()).unwrap_or_default();
                let name = &found.map(|p| p.name).unwrap_or_else(|| name.clone());
                m.upsert(Entry {
                    name: name.clone(),
                    source: source.clone(),
                    version,
                    reference: reference.clone(),
                    resolved_by: resolved_by.clone(),
                    installed_at: now(),
                    ..Default::default()
                });
                m.save()?;
            }
            Action::Unrecord { source, name } => {
                m.remove(source, name);
                m.save()?;
            }
            Action::RemoveDirect { entry, zap } => {
                let gone = direct::uninstall(ctx.r, &ctx.dirs, entry, *zap)?;
                m.remove("direct", &entry.name);
                m.save()?;
                if ctx.verbose {
                    for g in gone {
                        say(ctx, &format!("removed {g}"));
                    }
                }
                say(ctx, &format!("removed {}", entry.name));
            }
        }
    }
    Ok(())
}

fn run_plan(ctx: &Ctx, plan: Plan, act: Act) -> KResult {
    if confirm(ctx, &plan, act)? {
        execute(ctx, &plan)?;
    }
    Ok(())
}

// ---------- install ----------

enum Resolution {
    Backend { cand: Candidate, note: Option<String> },
    Direct(Box<Job>, Option<String>),
}

fn direct_job_for(ctx: &Ctx, name: &str, act: Act) -> KResult<Job> {
    let rec = recipe::find(&ctx.dirs, name).ok_or_else(|| KErr::fail(format!("no recipe for '{name}'")))?;
    let asset = recipe::latest(ctx.r, &rec)?;
    Ok(Job::from_asset(&rec, asset, act.force, false, None))
}

fn discovered_resolution(name: &str, d: discovery::Discovered, act: Act) -> Resolution {
    let learned = d.recipe(name);
    let asset = Asset { name: recipe::file_name(&d.url), url: d.url.clone(), version: d.version.clone(), sha256: d.sha256.clone() };
    let mut job = Job::from_asset(&learned, asset, act.force, false, None);
    job.resolved_by = format!("found via {}", d.source);
    job.learned = Some(learned);
    Resolution::Direct(Box::new(job), Some(format!("found online: {}", d.evidence)))
}

fn discover(ctx: &Ctx, name: &str, act: Act) -> KResult<Resolution> {
    let not_found = |why: &str| KErr::fail(format!("no package named '{name}' found ({why}); nothing changed"));
    if !ctx.cfg.models_enabled() {
        return Err(not_found("no backend carries it, and vendor-site discovery is off"));
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
        if let Some(repo) = url.split_once("://").map(|x| x.1).and_then(|r| r.split('/').nth(2)) {
            return repo.to_string();
        }
    }
    let labels: Vec<&str> = host.split('.').collect();
    labels.get(labels.len().saturating_sub(2)).copied().unwrap_or(&host).to_string()
}

fn from_url(ctx: &Ctx, url: &str, act: Act) -> KResult<Resolution> {
    let name = name_from_url(url);
    match discovery::from_url(ctx.r, ctx.decider, &name, url) {
        Outcome::Picked(d) => Ok(discovered_resolution(&name, d, act)),
        Outcome::Unsure(why) => Err(KErr::fail(format!("could not find a download at {url}: {why}; nothing changed"))),
    }
}

fn choice_name(c: &Choice) -> String {
    match c {
        Choice::One(c) | Choice::Several { chosen: c, .. } => format!("{}:{}", c.source, c.name),
        Choice::None => String::new(),
    }
}

fn resolve_install(ctx: &Ctx, t: &Target, act: Act) -> KResult<Resolution> {
    let backends = backend::available(ctx.r);
    if let Some(src) = &t.source {
        if src == "direct" {
            return Ok(Resolution::Direct(Box::new(direct_job_for(ctx, &t.name, act)?), None));
        }
        let b = backends.iter().find(|b| b.id() == src).ok_or_else(|| KErr::fail(format!("{src} is not available on this machine")))?;
        let cand = if src == "mas" && !t.name.bytes().all(|c| c.is_ascii_digit()) {
            select::exact(&t.name, &b.search(ctx.r, &t.name), &ctx.cfg.order())
                .into_iter()
                .next()
                .ok_or_else(|| KErr::fail(format!("the App Store has no app named '{}'", t.name)))?
        } else {
            Candidate { source: src.clone(), name: t.name.clone(), reference: t.name.clone(), desc: String::new() }
        };
        return Ok(Resolution::Backend { cand, note: None });
    }

    let mut hits: Vec<Candidate> = vec![];
    // Package names are hyphenated, people type spaces: "Sublime Merge" finds `sublime-merge`.
    let term = t.name.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
    for b in &backends {
        hits.extend(b.search(ctx.r, &term));
    }
    if let Some(rec) = recipe::find(&ctx.dirs, &t.name) {
        hits.push(Candidate { source: "direct".into(), name: rec.name.clone(), reference: rec.name.clone(), desc: rec.description.clone() });
    }
    let order = ctx.cfg.order();
    let pick = |c: Candidate, note: Option<String>| -> KResult<Resolution> {
        if c.source == "direct" {
            Ok(Resolution::Direct(Box::new(direct_job_for(ctx, &c.name, act)?), note))
        } else {
            Ok(Resolution::Backend { cand: c, note })
        }
    };
    let choice = select::choose(&t.name, &hits, &order);
    // Only an open registry has this exact name, but a reviewed source has something named like it ("chrome" is
    // npm's unrelated package; the app is the cask google-chrome): never guess, show the options.
    let open_only = match &choice {
        Choice::One(c) => decision::tier(&format!("{}:{}", c.source, c.name)) == 2,
        Choice::Several { chosen, .. } => decision::tier(&format!("{}:{}", chosen.source, chosen.name)) == 2,
        Choice::None => false,
    };
    if matches!(choice, Choice::None) || open_only {
        // Plain text: "chrome", "chome" and "vscode" mean google-chrome and visual-studio-code (names.rs).
        if let Ok(f) = names::best(&t.name, &names::popular(ctx.r, &ctx.dirs)) {
            // A guess at a typo is confirmed by a person looking at the plan, never by -y alone.
            if f.how == names::How::Typo && act.yes {
                return Err(KErr::fail(format!(
                    "'{}' looks like a typo for {}:{} but -y would install it unseen; nothing was installed. Run without -y to confirm the plan, or name it: kaika install {}:{}",
                    t.name, f.entry.source, f.entry.name, f.entry.source, f.entry.name
                )));
            }
            let cand = Candidate { source: f.entry.source.into(), name: f.entry.name.clone(), reference: f.entry.name.clone(), desc: String::new() };
            return pick(cand, Some(f.why(&t.name)));
        }
    }
    if open_only {
        let mut near = select::near(&t.name, &hits);
        near.sort_by_key(|c| (c.source != "cask", c.name.len())); // apps (casks) first: what a short name usually means
        if !near.is_empty() {
            let list = near.iter().map(|c| format!("{}:{}", c.source, c.name)).collect::<Vec<_>>().join(", ");
            let first = choice_name(&choice);
            return Err(KErr::fail(format!(
                "'{}' exactly names only {first}, an unreviewed registry package, while reviewed sources have {list}; nothing was installed. Say which you mean with source:name, for example kaika install {} (or {first} for the registry package)",
                t.name,
                near[0].source.clone() + ":" + &near[0].name
            )));
        }
    }
    match choice {
        Choice::None => discover(ctx, &t.name, act),
        Choice::One(c) => pick(c, None),
        Choice::Several { chosen, others } => {
            let mut all = vec![chosen.clone()];
            all.extend(others.clone());
            let cands: Vec<(String, String)> = all.iter().map(|c| (format!("{}:{}", c.source, c.name), String::new())).collect();
            let also = others.iter().map(|o| format!("{}:{}", o.source, o.name)).collect::<Vec<_>>().join(", ");
            match decision::decide_backend(&cands) {
                Outcome::Picked(p) => {
                    let note = format!("also available as {also} (less trusted sources are never chosen over {}); {}", p.id, p.how);
                    pick(chosen, Some(note))
                }
                Outcome::Unsure(why) => {
                    // A human decision: say what the tied candidates are, so the choice can be made with open eyes.
                    let tied: Vec<&Candidate> = all.iter().filter(|c| decision::tier(&format!("{}:{}", c.source, c.name)) == decision::tier(&cands[0].0)).collect();
                    let described: Vec<String> = tied
                        .iter()
                        .map(|c| {
                            let d = backend::by_id(&c.source).map(|b| b.describe(ctx.r, &c.reference)).unwrap_or_default();
                            let d: String = d.chars().take(90).collect();
                            if d.is_empty() { format!("{}:{}", c.source, c.name) } else { format!("{}:{} ({d})", c.source, c.name) }
                        })
                        .collect();
                    let note = format!("{why}; chose {}:{} by preference order. Candidates: {}. Use source:name to choose another", chosen.source, chosen.name, described.join("; "));
                    pick(chosen, Some(note))
                }
            }
        }
    }
}

fn install(ctx: &Ctx, names: &[String], act: Act) -> KResult {
    let mut plan = Plan { title: format!("install {}", names.join(", ")), actions: vec![], warnings: vec![] };
    for n in names {
        let is_url = n.starts_with("https://") || n.starts_with("http://");
        let (resolution, explicit_source) = if is_url {
            (from_url(ctx, n, act)?, false)
        } else {
            let t = select::parse_target(n).map_err(KErr::usage)?;
            let explicit = t.source.is_some();
            (resolve_install(ctx, &t, act)?, explicit)
        };
        match resolution {
            Resolution::Backend { cand, note } => {
                let b = backend::by_id(&cand.source).ok_or_else(|| KErr::fail("unknown backend"))?;
                plan.actions.push(Action::Cmd { argv: b.install_argv(&cand.reference), admin: b.needs_admin() });
                plan.actions.push(Action::Record {
                    source: cand.source.clone(),
                    name: cand.name.clone(),
                    reference: cand.reference.clone(),
                    resolved_by: if explicit_source { "explicit source".into() } else { "backend lookup".into() },
                });
                plan.warnings.extend(note.map(|n| format!("{}: {n}", cand.name)));
            }
            Resolution::Direct(job, note) => {
                plan.warnings.extend(note.map(|n| format!("{}: {n}", job.name)));
                plan.actions.push(Action::Direct(job));
            }
        }
    }
    run_plan(ctx, plan, act)
}

// ---------- uninstall and upgrade ----------

fn find_installed(ctx: &Ctx, raw: &str) -> KResult<Row> {
    let t = select::parse_target(raw).map_err(KErr::usage)?;
    let rows = live_rows(ctx)?;
    let mut found: Vec<Row> = rows
        .into_iter()
        .filter(|r| r.name.eq_ignore_ascii_case(&t.name) && t.source.as_deref().is_none_or(|s| s == r.source))
        .collect();
    match found.len() {
        0 => Err(KErr::fail(format!("'{}' is not installed (see `kaika list`)", t.name))),
        1 => Ok(found.remove(0)),
        _ => Err(KErr::usage(format!(
            "'{}' is installed from several sources ({}); use source:name",
            t.name,
            found.iter().map(|r| r.source.clone()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

fn uninstall(ctx: &Ctx, names: &[String], zap: bool, act: Act) -> KResult {
    let m = Manifest::load(&ctx.dirs)?;
    let mut plan = Plan { title: format!("uninstall {}", names.join(", ")), actions: vec![], warnings: vec![] };
    for n in names {
        let row = find_installed(ctx, n)?;
        match row.source.as_str() {
            "app" => return Err(KErr::fail(format!("{} is managed by hand; kaika only lists it", row.name))),
            "direct" => {
                let e = m.get("direct", &row.name).cloned().ok_or_else(|| KErr::fail("manifest entry missing"))?;
                plan.actions.push(Action::RemoveDirect { entry: e, zap });
            }
            src => {
                let b = backend::by_id(src).ok_or_else(|| KErr::fail("unknown backend"))?;
                plan.actions.push(Action::Cmd { argv: b.uninstall_argv(&row.reference, zap), admin: b.needs_admin() });
                plan.actions.push(Action::Unrecord { source: src.into(), name: row.name.clone() });
                if zap && src != "cask" {
                    plan.warnings.push(format!("--zap only applies to casks and direct installs; ignored for {src}:{}", row.name));
                }
            }
        }
    }
    run_plan(ctx, plan, act)
}

fn direct_upgrade_job(ctx: &Ctx, e: &Entry, force: bool) -> KResult<Option<Job>> {
    let Some(rec) = recipe::find(&ctx.dirs, &e.name) else {
        return Err(KErr::fail(format!("no recipe for '{}', so kaika cannot check for updates", e.name)));
    };
    let asset = recipe::latest(ctx.r, &rec)?;
    let same = if asset.version.is_empty() { asset.url == e.reference } else { recipe::norm_version(&asset.version) == recipe::norm_version(&e.version) };
    if same {
        return Ok(None);
    }
    let team = (!e.team_id.is_empty()).then(|| e.team_id.clone());
    Ok(Some(Job::from_asset(&rec, asset, force, true, team)))
}

fn upgrade(ctx: &Ctx, names: &[String], all: bool, act: Act) -> KResult {
    if names.is_empty() && !all {
        return Err(KErr::usage("name the packages to upgrade, or pass --all"));
    }
    let m = Manifest::load(&ctx.dirs)?;
    let mut plan = Plan { title: if all { "upgrade everything".into() } else { format!("upgrade {}", names.join(", ")) }, actions: vec![], warnings: vec![] };
    if all {
        for b in backend::available(ctx.r) {
            plan.actions.push(Action::Cmd { argv: b.upgrade_argv(None), admin: b.needs_admin() });
        }
        for e in m.packages.iter().filter(|e| e.source == "direct") {
            match direct_upgrade_job(ctx, e, act.force) {
                Ok(Some(j)) => plan.actions.push(Action::Direct(Box::new(j))),
                Ok(None) => {}
                Err(err) => plan.warnings.push(format!("{}: {}", e.name, err.msg)),
            }
        }
    } else {
        for n in names {
            let row = find_installed(ctx, n)?;
            match row.source.as_str() {
                "app" => return Err(KErr::fail(format!("{} is managed by hand; kaika only lists it", row.name))),
                "direct" => {
                    let e = m.get("direct", &row.name).cloned().ok_or_else(|| KErr::fail("manifest entry missing"))?;
                    match direct_upgrade_job(ctx, &e, act.force)? {
                        Some(j) => plan.actions.push(Action::Direct(Box::new(j))),
                        None => say(ctx, &format!("{} is up to date", e.name)),
                    }
                }
                src => {
                    let b = backend::by_id(src).ok_or_else(|| KErr::fail("unknown backend"))?;
                    plan.actions.push(Action::Cmd { argv: b.upgrade_argv(Some(&row.reference)), admin: b.needs_admin() });
                }
            }
        }
    }
    run_plan(ctx, plan, act)
}

// ---------- read-only commands ----------

fn update(ctx: &Ctx) -> KResult {
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

fn search(ctx: &Ctx, query: &str) -> KResult {
    let mut hits: Vec<Candidate> = vec![];
    for b in backend::available(ctx.r) {
        hits.extend(b.search(ctx.r, query));
    }
    let q = query.to_lowercase();
    for rec in recipe::all(&ctx.dirs) {
        if rec.name.to_lowercase().contains(&q) || rec.description.to_lowercase().contains(&q) {
            hits.push(Candidate { source: "direct".into(), name: rec.name, reference: String::new(), desc: rec.description });
        }
    }
    if hits.is_empty() {
        return Err(KErr::fail(format!("no results for '{query}'")));
    }
    if ctx.json {
        println!("{}", serde_json::to_string(&hits).map_err(anyhow::Error::from)?);
    } else {
        for h in hits {
            println!("{:<28}{}", format!("{}:{}", h.source, h.name), h.desc);
        }
    }
    Ok(())
}

fn info(ctx: &Ctx, name: &str) -> KResult {
    let rows: Vec<Row> = live_rows(ctx)?.into_iter().filter(|r| r.name.eq_ignore_ascii_case(name)).collect();
    let m = Manifest::load(&ctx.dirs)?;
    let rec = recipe::find(&ctx.dirs, name);
    if rows.is_empty() && rec.is_none() {
        let mut hits = vec![];
        for b in backend::available(ctx.r) {
            hits.extend(select::exact(name, &b.search(ctx.r, name), &ctx.cfg.order()));
        }
        if hits.is_empty() {
            return Err(KErr::fail(format!("nothing known about '{name}'")));
        }
        for h in hits {
            say(ctx, &format!("{}:{}  available, not installed", h.source, h.name));
        }
        return Ok(());
    }
    if ctx.json {
        let entries: Vec<&Entry> = m.find_name(name);
        println!("{}", serde_json::json!({ "installed": rows, "manifest": entries, "recipe": rec }));
        return Ok(());
    }
    for r in &rows {
        println!("{}:{}  version {}  {}", r.source, r.name, if r.version.is_empty() { "unknown" } else { &r.version }, if r.managed { "managed by kaika" } else { "not in the manifest" });
        if let Some(e) = m.get(&r.source, &r.name) {
            if !e.path.is_empty() {
                println!("  path:      {}", e.path);
            }
            if !e.team_id.is_empty() {
                println!("  team id:   {}", e.team_id);
            }
            if !e.update_source.is_empty() {
                println!("  updates:   {}", e.update_source);
            }
            if !e.resolved_by.is_empty() {
                println!("  found by:  {}", e.resolved_by);
            }
        }
    }
    if let Some(rec) = rec {
        println!("recipe: {}{}", rec.name, if rec.description.is_empty() { String::new() } else { format!(" ({})", rec.description) });
    }
    Ok(())
}

/// Every package with a newer version: (source:name, current, latest), and a warning for each vendor-site app whose
/// release feed could not be read (silence there would read as "up to date").
fn outdated_lines(ctx: &Ctx) -> KResult<(Vec<(String, String, String)>, Vec<String>)> {
    let current: Vec<Row> = live_rows(ctx)?;
    let mut lines: Vec<(String, String, String)> = vec![];
    for b in backend::available(ctx.r) {
        for p in b.outdated(ctx.r) {
            let cur = current.iter().find(|r| r.source == b.id() && r.name == p.name).map(|r| r.version.clone()).unwrap_or_default();
            lines.push((format!("{}:{}", b.id(), p.name), cur, p.version));
        }
    }
    let m = Manifest::load(&ctx.dirs)?;
    let mut warnings = vec![];
    for e in m.packages.iter().filter(|e| e.source == "direct") {
        match direct_upgrade_job(ctx, e, false) {
            Ok(Some(j)) => lines.push((format!("direct:{}", e.name), e.version.clone(), if j.version.is_empty() { "newer build".into() } else { j.version })),
            Ok(None) => {}
            Err(err) => warnings.push(format!("{}: could not check for updates ({})", e.name, err.msg)),
        }
    }
    Ok((lines, warnings))
}

fn outdated(ctx: &Ctx, notify: bool) -> KResult {
    if notify {
        return outdated_notify(ctx);
    }
    let (lines, warnings) = outdated_lines(ctx)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    if ctx.json {
        let v: Vec<_> = lines.iter().map(|(n, c, l)| serde_json::json!({ "package": n, "current": c, "latest": l })).collect();
        println!("{}", serde_json::Value::Array(v));
    } else if lines.is_empty() {
        say(ctx, "everything is up to date");
    } else {
        for (n, c, l) in lines {
            println!("{:<34}{} -> {}", n, if c.is_empty() { "?" } else { &c }, l);
        }
    }
    Ok(())
}

/// The daily check: refresh the indexes, and if the set of available updates is new since the last notice, say so
/// with one macOS notification. It never installs anything; `kaika upgrade --all` is what does.
fn outdated_notify(ctx: &Ctx) -> KResult {
    update(ctx)?;
    let (lines, warnings) = outdated_lines(ctx)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let seen = ctx.dirs.root.join("notified");
    let now: Vec<String> = lines.iter().map(|(n, _, l)| format!("{n}={l}")).collect();
    let fingerprint = crate::schedule::fingerprint(&now);
    if lines.is_empty() {
        let _ = std::fs::remove_file(&seen); // up to date: the next batch of updates is news again
        say(ctx, "everything is up to date");
        return Ok(());
    }
    if std::fs::read_to_string(&seen).is_ok_and(|t| t.trim() == fingerprint) {
        say(ctx, &format!("{} update(s) available, already notified", lines.len()));
        return Ok(());
    }
    let names: Vec<&str> = lines.iter().map(|(n, _, _)| n.as_str()).collect();
    let script = crate::schedule::notification_script(&names);
    let _ = ctx.r.run(&argv(&["osascript", "-e", &script]));
    std::fs::create_dir_all(&ctx.dirs.root).map_err(anyhow::Error::from)?;
    std::fs::write(&seen, &fingerprint).map_err(anyhow::Error::from)?;
    say(ctx, &format!("{} update(s) available; notified. Run: kaika upgrade --all", lines.len()));
    Ok(())
}

fn agent_path(ctx: &Ctx) -> std::path::PathBuf {
    ctx.dirs.home.join("Library/LaunchAgents").join(format!("{}.plist", crate::schedule::LABEL))
}

fn schedule(ctx: &Ctx, action: &str, at: &str, dry_run: bool) -> KResult {
    let file = agent_path(ctx);
    match action {
        "status" => {
            match std::fs::read_to_string(&file) {
                Ok(p) => {
                    let (h, m) = crate::schedule::scheduled_time(&p).unwrap_or((0, 0));
                    say(ctx, &format!("scheduled: kaika checks for updates every day at {h:02}:{m:02} and notifies you; it installs nothing (log: {})", ctx.dirs.root.join("logs/upgrade.log").display()));
                }
                Err(_) => say(ctx, "not scheduled (kaika schedule on [--at HH:MM])"),
            }
            Ok(())
        }
        "on" => {
            let (h, m) = crate::schedule::parse_time(at).map_err(KErr::usage)?;
            let exe = std::env::current_exe().map_err(anyhow::Error::from)?.display().to_string();
            let log = ctx.dirs.root.join("logs/upgrade.log");
            let path = format!("{}:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin", ctx.dirs.bin.display());
            let body = crate::schedule::plist(&exe, &log.display().to_string(), &path, h, m);
            say(ctx, &format!("{}schedule: write {} and load it with launchctl; checks for updates daily at {h:02}:{m:02} and notifies you (it installs nothing)", if dry_run { "would " } else { "" }, file.display()));
            if dry_run {
                return Ok(());
            }
            std::fs::create_dir_all(file.parent().expect("has a parent")).map_err(anyhow::Error::from)?;
            std::fs::create_dir_all(log.parent().expect("has a parent")).map_err(anyhow::Error::from)?;
            // Reload cleanly: an older copy is unloaded first, and not being loaded is fine.
            let target = launchd_target(ctx);
            let _ = ctx.r.run(&argv(&["launchctl", "bootout", &format!("{target}/{}", crate::schedule::LABEL)]));
            std::fs::write(&file, body).map_err(anyhow::Error::from)?;
            let code = ctx.r.run_live(&argv(&["launchctl", "bootstrap", &target, &file.display().to_string()]))?;
            if code != 0 {
                return Err(KErr::fail(format!("launchctl could not load {} (exit {code})", file.display())));
            }
            say(ctx, &format!("scheduled daily at {h:02}:{m:02}"));
            Ok(())
        }
        _ => {
            if !file.exists() {
                say(ctx, "not scheduled");
                return Ok(());
            }
            say(ctx, &format!("{}schedule: unload and remove {}", if dry_run { "would " } else { "" }, file.display()));
            if dry_run {
                return Ok(());
            }
            let _ = ctx.r.run(&argv(&["launchctl", "bootout", &format!("{}/{}", launchd_target(ctx), crate::schedule::LABEL)]));
            std::fs::remove_file(&file).map_err(anyhow::Error::from)?;
            say(ctx, "schedule removed");
            Ok(())
        }
    }
}

/// `gui/<uid>`: the per-user launchd domain.
fn launchd_target(ctx: &Ctx) -> String {
    let uid = ctx.r.run(&argv(&["id", "-u"])).map(|o| o.stdout.trim().to_string()).unwrap_or_default();
    format!("gui/{}", if uid.is_empty() { "501" } else { &uid })
}

fn doctor(ctx: &Ctx) -> KResult {
    let mut bad = false;
    for b in backend::all() {
        let ok = ctx.r.exists(b.tool());
        println!("{:<8}{}", b.id(), if ok { format!("ok ({} found)", b.tool()) } else { format!("missing ({} not on PATH)", b.tool()) });
        if b.id() == "mas" {
            println!("{:<8}{}", "", if ok { "uses the Apple ID signed in to the App Store app; kaika never handles it" } else { "App Store apps need it: kaika install mas, then sign in to the App Store app" });
        }
    }
    println!("{:<8}{}", "home", ctx.dirs.root.display());
    let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == ctx.dirs.bin));
    if ctx.dirs.bin.exists() && !on_path {
        println!("{:<8}{} is not on PATH; add it to use the command-line tools kaika installs", "bin", ctx.dirs.bin.display());
    }
    match Manifest::load(&ctx.dirs) {
        Ok(m) => println!("{:<8}ok ({} packages)", "state", m.packages.len()),
        Err(e) => {
            bad = true;
            println!("{:<8}error: {e:#}", "state");
        }
    }
    match crate::render::browser() {
        Some(b) => println!("{:<8}{} (only for vendor pages that need JavaScript; headless, no window)", "browser", b.display()),
        None => println!("{:<8}none found: vendor pages that need JavaScript cannot be read (install Chrome, Brave, Edge or Chromium)", "browser"),
    }
    println!("{:<8}{}", "upgrade", if agent_path(ctx).exists() { "daily update check scheduled (kaika schedule status)" } else { "not scheduled (kaika schedule on)" });
    if !ctx.cfg.models_enabled() {
        println!("{:<8}off (no model decisions, no web discovery of vendor downloads)", "models");
    } else {
        // Reports only: nothing is started. The model loads when a third-party app needs it.
        let st = model::status(ctx.r, &ctx.dirs);
        match st.llama_build {
            Some(b) if b >= model::MIN_BUILD => println!("{:<8}llama-server build {b}", "models"),
            Some(b) => println!("{:<8}llama-server build {b} is too old (need {} or newer): kaika upgrade llama.cpp", "models", model::MIN_BUILD),
            None => println!("{:<8}llama-server missing: kaika install llama.cpp", "models"),
        }
        match st.model_bytes {
            Some(n) => println!("{:<8}{} installed ({:.1} GB); loaded only while a third-party app is being found", "", model::MODEL_NAME, n as f64 / 1e9),
            None => println!("{:<8}{} not installed: kaika install {}", "", model::MODEL_NAME, model::MODEL_NAME),
        }
    }
    if bad { Err(KErr::fail("doctor found problems")) } else { Ok(()) }
}

fn dir_size(p: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.filter_map(|e| e.ok())
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn cleanup(ctx: &Ctx, dry_run: bool) -> KResult {
    let mut total = 0;
    for sub in ["downloads", "stage", "mnt"] {
        let p = ctx.dirs.cache.join(sub);
        let size = dir_size(&p);
        total += size;
        if size > 0 || p.exists() {
            if dry_run {
                say(ctx, &format!("would remove {} ({size} bytes)", p.display()));
            } else {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
    }
    say(ctx, &format!("{} {total} bytes", if dry_run { "would free" } else { "freed" }));
    Ok(())
}

fn config(ctx: &Ctx, args: &[String]) -> KResult {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        [] => {
            for k in crate::config::KEYS {
                println!("{k} = {}", ctx.cfg.get(k)?);
            }
            Ok(())
        }
        ["path"] => {
            println!("{}", ctx.dirs.config.join("kaika.toml").display());
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
        _ => Err(KErr::usage("usage: kaika config [get KEY | set KEY VALUE | path]")),
    }
}
