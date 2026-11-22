//! The command handlers behind `kaika <command>`.
//!
//! [`dispatch`] hands a parsed command to the module that does it. Every command that changes something builds a
//! [`Plan`](crate::plan::Plan) that `run` shows first; every handler returns a [`KResult`], whose error carries the
//! process exit code.
//!
//! | module | commands |
//! |---|---|
//! | `install` | `install`: resolving a name or URL to a backend or a vendor download |
//! | `uninstall`, `upgrade` | `uninstall`, `upgrade`, `update` |
//! | `query` | `search`, `info`; `rows` (what is installed) backs `list` and the others |
//! | `outdated` | `outdated`, and its `--notify` daily check |
//! | `schedule` | `schedule`: the launchd job that runs that check |
//! | `housekeeping` | `doctor`, `cleanup`, `config` |
//! | `run` | the plan: confirm it, then carry it out |

mod housekeeping;
mod install;
mod outdated;
mod query;
mod rows;
mod run;
mod schedule;
mod uninstall;
mod upgrade;

use crate::cli::Cmd;
use crate::config::Config;
use crate::decision::Decider;
use crate::net;
use crate::paths::Dirs;
use crate::runner::Runner;

/// Everything a command needs: where kaika keeps things, how to run external tools, the decision layer, and the
/// output flags. Built once in `main`; tests substitute a stub runner and decider.
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

/// An error with the exit code it maps to: 1 failure, 2 usage, 3 verification failed, 4 needs confirmation,
/// 69 offline.
#[derive(Debug)]
pub struct KErr {
    pub code: i32,
    pub msg: String,
}

impl KErr {
    /// A failure (exit 1).
    pub fn fail(m: impl Into<String>) -> KErr {
        KErr {
            code: 1,
            msg: m.into(),
        }
    }

    /// A usage error (exit 2): the command line itself was wrong.
    pub fn usage(m: impl Into<String>) -> KErr {
        KErr {
            code: 2,
            msg: m.into(),
        }
    }
}

impl From<anyhow::Error> for KErr {
    fn from(e: anyhow::Error) -> KErr {
        KErr::fail(format!("{e:#}"))
    }
}

/// What every command handler returns.
pub type KResult<T = ()> = Result<T, KErr>;

/// Exit code for "the network is unreachable" (sysexits.h EX_UNAVAILABLE).
pub const EX_UNAVAILABLE: i32 = 69;

/// Commands that cannot work without a network fail in about two seconds, before any solver or model starts.
fn needs_network(cmd: &Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Install { .. }
            | Cmd::Upgrade { .. }
            | Cmd::Update
            | Cmd::Outdated { .. }
            | Cmd::Search { .. }
    )
}

/// Run one parsed command. Offline, the commands that need the network exit 69 before anything starts.
pub fn dispatch(ctx: &Ctx, cmd: Cmd) -> KResult {
    if needs_network(&cmd) && !net::online(ctx.r) {
        return Err(KErr {
            code: EX_UNAVAILABLE,
            msg: format!(
                "offline: could not reach {} within 2 s, so nothing was started or changed. list, info, uninstall, cleanup, doctor and config still work offline",
                net::PROBE
            ),
        });
    }
    match cmd {
        Cmd::Install { names, act } => install::install(ctx, &names, act),
        Cmd::Uninstall { names, zap, act } => uninstall::uninstall(ctx, &names, zap, act),
        Cmd::Upgrade { names, all, act } => upgrade::upgrade(ctx, &names, all, act),
        Cmd::Update => upgrade::update(ctx),
        Cmd::List => rows::list(ctx),
        Cmd::Search { query } => query::search(ctx, &query),
        Cmd::Info { name } => query::info(ctx, &name),
        Cmd::Outdated { notify } => outdated::outdated(ctx, notify),
        Cmd::Doctor => housekeeping::doctor(ctx),
        Cmd::Cleanup { dry_run } => housekeeping::cleanup(ctx, dry_run),
        Cmd::Schedule {
            action,
            at,
            dry_run,
        } => schedule::schedule(ctx, &action, &at, dry_run),
        Cmd::Config { args } => housekeeping::config(ctx, &args),
    }
}

/// Print a line of normal output (silenced by `-q`).
fn say(ctx: &Ctx, msg: &str) {
    if !ctx.quiet {
        println!("{msg}");
    }
}
