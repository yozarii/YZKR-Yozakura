use clap::Parser;
use kaika::cli::Cli;
use kaika::commands::{self, Ctx};
use kaika::config::Config;
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
    let ctx = Ctx { dirs, r: &RealRunner, cfg, json: cli.json, quiet: cli.quiet, verbose: cli.verbose };
    match commands::dispatch(&ctx, cli.cmd) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("kaika: {}", e.msg);
            std::process::exit(e.code);
        }
    }
}
