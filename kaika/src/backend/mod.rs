//! Backends: each adapter shells out to the real tool and parses its output.

pub mod app;
pub mod brew;
pub mod mas;
pub mod npm;
pub mod uv;

use crate::runner::{Runner, argv};
use serde::Serialize;

/// A package as a backend reports it: installed, or outdated with its newer version.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    /// Identifier the backend wants for install, uninstall and upgrade (App Store id; otherwise the name).
    pub reference: String,
}

/// A package a backend offers for a query, before anyone has chosen it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Candidate {
    pub source: String,
    pub name: String,
    pub reference: String,
    pub desc: String,
}

/// One package source. An adapter shells out to the real tool through a `Runner` and parses its output; it never
/// decides anything about trust (that is `select` and `decision`).
pub trait Backend {
    /// Source id as used in `source:name` and in the manifest.
    fn id(&self) -> &'static str;
    /// Program that must be on PATH for this backend to be usable.
    fn tool(&self) -> &'static str;
    fn installed(&self, r: &dyn Runner) -> Vec<Pkg>;
    /// Candidates for a query; the chooser keeps only exact-name matches for bare installs.
    fn search(&self, r: &dyn Runner, query: &str) -> Vec<Candidate>;
    fn install_argv(&self, reference: &str) -> Vec<String>;
    fn uninstall_argv(&self, reference: &str, zap: bool) -> Vec<String>;
    fn upgrade_argv(&self, reference: Option<&str>) -> Vec<String>;
    fn outdated(&self, _r: &dyn Runner) -> Vec<Pkg> {
        Vec::new()
    }
    /// One line saying what the package is, fetched only when a decision needs it.
    fn describe(&self, _r: &dyn Runner, _reference: &str) -> String {
        String::new()
    }
    /// Whether installing, upgrading or uninstalling needs administrator rights (the plan says so and kaika never
    /// elevates silently).
    fn needs_admin(&self) -> bool {
        false
    }
}

/// Every backend kaika knows, in no particular order (the user's preference order is applied by `select`).
pub fn all() -> Vec<Box<dyn Backend>> {
    vec![
        Box::new(brew::Brew { cask: false }),
        Box::new(brew::Brew { cask: true }),
        Box::new(uv::Uv),
        Box::new(npm::Npm),
        Box::new(mas::Mas),
    ]
}

/// The backend with this source id (`brew`, `cask`, `uv`, `npm`, `mas`).
pub fn by_id(id: &str) -> Option<Box<dyn Backend>> {
    all().into_iter().find(|b| b.id() == id)
}

/// The backends whose tool is on PATH.
pub fn available(r: &dyn Runner) -> Vec<Box<dyn Backend>> {
    all().into_iter().filter(|b| r.exists(b.tool())).collect()
}

/// Run a command and return stdout; any failure reads as empty output.
pub(crate) fn out(r: &dyn Runner, a: &[&str]) -> String {
    r.run(&argv(a)).map(|o| o.stdout).unwrap_or_default()
}
