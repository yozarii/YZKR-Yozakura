//! A plan is what kaika is about to do. It is always printed before anything changes.

use crate::direct::Job;
use crate::manifest::Entry;
use serde_json::json;

pub enum Action {
    Cmd {
        argv: Vec<String>,
        admin: bool,
    },
    Direct(Box<Job>),
    /// After a backend install: look the version up and write the manifest.
    Record {
        source: String,
        name: String,
        reference: String,
        resolved_by: String,
    },
    Unrecord {
        source: String,
        name: String,
    },
    RemoveDirect {
        entry: Entry,
        zap: bool,
    },
}

impl Action {
    pub fn describe(&self) -> String {
        match self {
            Action::Cmd { argv, admin } => {
                format!(
                    "run: {}{}",
                    argv.join(" "),
                    if *admin {
                        "   (needs administrator rights)"
                    } else {
                        ""
                    }
                )
            }
            Action::Direct(j) => j.describe(),
            Action::Record { source, name, .. } => {
                format!("record {source}:{name} in the manifest")
            }
            Action::Unrecord { source, name } => {
                format!("remove {source}:{name} from the manifest")
            }
            Action::RemoveDirect { entry, zap } => format!(
                "move {} to the Trash{}",
                if entry.path.is_empty() {
                    &entry.name
                } else {
                    &entry.path
                },
                if *zap {
                    " and remove its support files"
                } else {
                    ""
                }
            ),
        }
    }
}

pub struct Plan {
    pub title: String,
    pub actions: Vec<Action>,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn render(&self) -> String {
        let mut s = format!("plan: {}\n", self.title);
        for a in &self.actions {
            s += &format!("  {}\n", a.describe());
        }
        for w in &self.warnings {
            s += &format!("  warning: {w}\n");
        }
        s
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({ "title": self.title, "actions": self.actions.iter().map(Action::describe).collect::<Vec<_>>(), "warnings": self.warnings })
    }

    /// Does the plan change anything beyond bookkeeping?
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    DryRun,
    Run,
    Cancelled,
    NeedsConfirmation,
}

/// `-n` wins; `-y` runs; with a terminal ask; without one, refuse to guess.
pub fn decide(dry_run: bool, yes: bool, tty: bool, ask: impl FnOnce() -> bool) -> Decision {
    if dry_run {
        Decision::DryRun
    } else if yes {
        Decision::Run
    } else if !tty {
        Decision::NeedsConfirmation
    } else if ask() {
        Decision::Run
    } else {
        Decision::Cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_beats_everything() {
        assert_eq!(decide(true, true, true, || true), Decision::DryRun);
    }

    #[test]
    fn yes_skips_the_prompt() {
        assert_eq!(
            decide(false, true, false, || panic!("must not ask")),
            Decision::Run
        );
    }

    #[test]
    fn no_terminal_and_no_yes_never_runs() {
        assert_eq!(
            decide(false, false, false, || true),
            Decision::NeedsConfirmation
        );
    }

    #[test]
    fn terminal_asks_and_respects_the_answer() {
        assert_eq!(decide(false, false, true, || true), Decision::Run);
        assert_eq!(decide(false, false, true, || false), Decision::Cancelled);
    }

    #[test]
    fn render_lists_actions_and_warnings() {
        let p = Plan {
            title: "install jq".into(),
            actions: vec![Action::Cmd {
                argv: vec!["brew".into(), "install".into(), "jq".into()],
                admin: false,
            }],
            warnings: vec!["w".into()],
        };
        let t = p.render();
        assert!(
            t.contains("plan: install jq")
                && t.contains("run: brew install jq")
                && t.contains("warning: w")
        );
    }
}
