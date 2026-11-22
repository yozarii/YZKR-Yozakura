//! Command-line definition. Flags keep their established meanings from brew, apt, npm and uv.

use clap::{Args, Parser, Subcommand};

/// kaika's command line: the global flags, then one command.
#[derive(Parser)]
#[command(name = "kaika", version, about = "The universal package manager")]
pub struct Cli {
    /// Print less
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,
    /// Print more
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,
    /// Machine-readable output
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// The flags of commands that change something. `-f` alone never skips the confirmation prompt.
#[derive(Args, Clone, Copy, Debug, Default)]
pub struct Act {
    /// Print the plan and change nothing
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Skip the confirmation prompt
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// Override safety: proceed past failed verification, conflicts or an existing install
    #[arg(short = 'f', long)]
    pub force: bool,
}

/// The commands. Names and flags keep their established meanings from brew, apt, npm and uv.
#[derive(Subcommand)]
pub enum Cmd {
    /// Install packages (name, or source:name to force a backend)
    Install {
        #[arg(required = true)]
        names: Vec<String>,
        #[command(flatten)]
        act: Act,
    },
    /// Remove packages
    #[command(visible_alias = "remove")]
    Uninstall {
        #[arg(required = true)]
        names: Vec<String>,
        /// Also remove the app's support files, caches and preferences
        #[arg(long)]
        zap: bool,
        #[command(flatten)]
        act: Act,
    },
    /// Upgrade the named packages, or everything with --all
    Upgrade {
        names: Vec<String>,
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        act: Act,
    },
    /// Refresh package indexes
    Update,
    /// List installed packages from every backend
    List,
    /// Search every backend and the recipes
    Search { query: String },
    /// Show details for a package
    Info { name: String },
    /// Show packages with newer versions available
    Outdated {
        /// Post a macOS notification when the set of updates changed since the last one (for the daily check); installs nothing
        #[arg(long)]
        notify: bool,
    },
    /// Check this machine: backends, state, decision layer
    Doctor,
    /// Remove old downloads and caches
    Cleanup {
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Check for updates once a day and tell you with a notification (it never installs): `schedule on [--at HH:MM]`, `schedule off`, `schedule status`
    Schedule {
        /// on, off or status (default)
        #[arg(value_parser = ["on", "off", "status"], default_value = "status")]
        action: String,
        /// Time of day for `on`, 24-hour
        #[arg(long, default_value = "03:00")]
        at: String,
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Show or change configuration: `config`, `config get KEY`, `config set KEY VALUE`, `config path`
    Config { args: Vec<String> },
}
