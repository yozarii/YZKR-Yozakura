//! The `kaika` binary: parse the command line, build the context, run one command, exit with its code.

use clap::Parser;
use kaika::cli::Cli;
use kaika::commands::{self, Ctx};
use kaika::config::Config;
use kaika::model::LlamaDecider;
use kaika::paths::Dirs;
use kaika::runner::RealRunner;

fn main() {
    let cli = Cli::parse();
    let dirs = Dirs::from_env();
    let cfg = match Config::load(&dirs) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kaika: {e:#}");
            std::process::exit(1);
        }
    };
    let runner = RealRunner;
    let decider = LlamaDecider::new(&runner, dirs.clone(), cfg.models_enabled());
    let ctx = Ctx {
        dirs,
        r: &runner,
        decider: &decider,
        cfg,
        json: cli.json,
        quiet: cli.quiet,
        verbose: cli.verbose,
    };
    let result = commands::dispatch(&ctx, cli.cmd);
    decider.shutdown(); // std::process::exit skips destructors, so stop the model explicitly
    if let Err(e) = result {
        eprintln!("kaika: {}", e.msg);
        std::process::exit(e.code);
    }
}
