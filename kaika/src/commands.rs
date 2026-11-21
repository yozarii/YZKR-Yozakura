//! Command handlers. Every change goes through a plan that is shown first.

use crate::backend::{self, app, Candidate};
use crate::cli::{Act, Cmd};
use crate::config::Config;
use crate::direct::{self, DErr, Job};
use crate::manifest::{now, Entry, Manifest};
use crate::paths::Dirs;
use crate::plan::{decide, Action, Decision, Plan};
use crate::recipe::{self, Asset, Recipe};
use crate::resolver::{self, Outcome};
use crate::runner::{argv, Runner};
use crate::select::{self, Choice, Target};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{BufRead, IsTerminal, Write};

pub struct Ctx<'a> {
    pub dirs: Dirs,
    pub r: &'a dyn Runner,
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

pub fn dispatch(ctx: &Ctx, cmd: Cmd) -> KResult {
    match cmd {
        Cmd::Install { names, act } => install(ctx, &names, act),
        Cmd::Uninstall { names, zap, act } => uninstall(ctx, &names, zap, act),
        Cmd::Upgrade { names, all, act } => upgrade(ctx, &names, all, act),
        Cmd::Update => update(ctx),
        Cmd::List => list(ctx),
        Cmd::Search { query } => search(ctx, &query),
        Cmd::Info { name } => info(ctx, &name),
        Cmd::Outdated => outdated(ctx),
        Cmd::Doctor => doctor(ctx),
        Cmd::Cleanup { dry_run } => cleanup(ctx, dry_run),
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
                    return Err(KErr::fail(format!("`{}` failed (exit {code})", argv.join(" "))));
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
                let version = backend::by_id(source)
                    .and_then(|b| b.installed(ctx.r).into_iter().find(|p| p.name.eq_ignore_ascii_case(name) || p.reference == *reference))
                    .map(|p| p.version)
                    .unwrap_or_default();
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

fn discover(ctx: &Ctx, name: &str, act: Act) -> KResult<Resolution> {
    let not_found = |why: &str| KErr::fail(format!("no package named '{name}' found ({why}); nothing changed"));
    let Some(cmd) = ctx.cfg.resolver_cmd() else { return Err(not_found("no backend carries it and the decision layer is off")) };
    match resolver::resolve(ctx.r, &cmd, &ctx.cfg, name) {
        Outcome::Unsure(why) => Err(not_found(&format!("vendor-site discovery unsure: {why}"))),
        Outcome::Picked(res) => {
            let asset = Asset { name: recipe::file_name(&res.url), url: res.url.clone(), version: res.version.clone().unwrap_or_default(), sha256: res.sha256.clone() };
            let base = Recipe { name: name.into(), domain: res.domain.clone(), app: res.app.clone(), asset_pattern: res.link_pattern.clone(), ..Default::default() };
            let learned = match (&res.github, &res.page) {
                (Some(g), _) => Recipe { github: Some(g.clone()), ..base },
                (None, Some(p)) => Recipe { page: Some(p.clone()), ..base },
                (None, None) => Recipe { url: Some(res.url.clone()), asset_pattern: None, ..base },
            };
            let mut job = Job::from_asset(&learned, asset, act.force, false, None);
            job.resolved_by = format!("decision layer: {} ({:.2})", if res.model.is_empty() { "model" } else { &res.model }, res.confidence);
            job.sha256 = res.sha256.clone();
            job.learned = Some(learned);
            Ok(Resolution::Direct(Box::new(job), Some(format!("found by the decision layer: {}", res.evidence))))
        }
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
    for b in &backends {
        hits.extend(b.search(ctx.r, &t.name));
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
    match select::choose(&t.name, &hits, &order) {
        Choice::None => discover(ctx, &t.name, act),
        Choice::One(c) => pick(c, None),
        Choice::Several { chosen, others } => {
            let mut all = vec![chosen.clone()];
            all.extend(others.clone());
            let also = others.iter().map(|o| format!("{}:{}", o.source, o.name)).collect::<Vec<_>>().join(", ");
            let mut note = format!("also available as {also}; chose {}:{} by preference order", chosen.source, chosen.name);
            let mut winner = chosen;
            if let Some(cmd) = ctx.cfg.resolver_cmd() {
                let cands: Vec<(String, String)> = all.iter().map(|c| (format!("{}:{}", c.source, c.name), c.desc.clone())).collect();
                match resolver::decide(ctx.r, &cmd, &ctx.cfg, "backend", &t.name, &cands) {
                    Outcome::Picked(id) => {
                        if let Some(c) = all.iter().find(|c| format!("{}:{}", c.source, c.name) == id) {
                            note = format!("also available as {also}; the decision layer chose {id}");
                            winner = c.clone();
                        }
                    }
                    Outcome::Unsure(why) => note = format!("{note} (decision layer unsure: {why})"),
                }
            }
            pick(winner, Some(note))
        }
    }
}

fn install(ctx: &Ctx, names: &[String], act: Act) -> KResult {
    let mut plan = Plan { title: format!("install {}", names.join(", ")), actions: vec![], warnings: vec![] };
    for n in names {
        let t = select::parse_target(n).map_err(KErr::usage)?;
        match resolve_install(ctx, &t, act)? {
            Resolution::Backend { cand, note } => {
                let b = backend::by_id(&cand.source).ok_or_else(|| KErr::fail("unknown backend"))?;
                plan.actions.push(Action::Cmd { argv: b.install_argv(&cand.reference), admin: b.needs_admin(false) });
                plan.actions.push(Action::Record {
                    source: cand.source.clone(),
                    name: cand.name.clone(),
                    reference: cand.reference.clone(),
                    resolved_by: if t.source.is_some() { "explicit source".into() } else { "backend lookup".into() },
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
                plan.actions.push(Action::Cmd { argv: b.uninstall_argv(&row.reference, zap), admin: b.needs_admin(true) });
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
            plan.actions.push(Action::Cmd { argv: b.upgrade_argv(None), admin: false });
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
                    plan.actions.push(Action::Cmd { argv: b.upgrade_argv(Some(&row.reference)), admin: false });
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

fn outdated(ctx: &Ctx) -> KResult {
    let current: Vec<Row> = live_rows(ctx)?;
    let mut lines: Vec<(String, String, String)> = vec![];
    for b in backend::available(ctx.r) {
        for p in b.outdated(ctx.r) {
            let cur = current.iter().find(|r| r.source == b.id() && r.name == p.name).map(|r| r.version.clone()).unwrap_or_default();
            lines.push((format!("{}:{}", b.id(), p.name), cur, p.version));
        }
    }
    let m = Manifest::load(&ctx.dirs)?;
    for e in m.packages.iter().filter(|e| e.source == "direct") {
        if let Ok(Some(j)) = direct_upgrade_job(ctx, e, false) {
            lines.push((format!("direct:{}", e.name), e.version.clone(), if j.version.is_empty() { "newer build".into() } else { j.version }));
        }
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

fn doctor(ctx: &Ctx) -> KResult {
    let mut bad = false;
    for b in backend::all() {
        let ok = ctx.r.exists(b.tool());
        println!("{:<8}{}", b.id(), if ok { format!("ok ({} found)", b.tool()) } else { format!("missing ({} not on PATH)", b.tool()) });
    }
    match Manifest::load(&ctx.dirs) {
        Ok(m) => println!("{:<8}ok ({} packages)", "state", m.packages.len()),
        Err(e) => {
            bad = true;
            println!("{:<8}error: {e:#}", "state");
        }
    }
    match ctx.cfg.resolver_cmd() {
        None => println!("{:<8}off (vendor-site discovery and ambiguity decisions disabled)", "models"),
        Some(c) => {
            println!("{:<8}resolver: {}", "models", c.join(" "));
            let up = ctx.r.run(&argv(&["curl", "-sf", "--max-time", "2", "http://localhost:11434/api/tags"])).map(|o| o.ok()).unwrap_or(false);
            println!("{:<8}ollama {}", "", if up { "reachable" } else { "not reachable (falls back to heuristics, then escalation)" });
            println!("{:<8}escalation: {}", "", ctx.cfg.escalation.as_deref().unwrap_or("not configured (kaika asks you instead)"));
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
