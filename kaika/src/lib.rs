//! kaika: the universal package manager.
//!
//! Backends do the installing; kaika picks the right one, verifies downloads, and keeps a manifest. Models choose
//! among third-party candidates; they never act, never decide trust, and never touch a package manager.
//!
//! # Layers
//!
//! | layer | modules |
//! |---|---|
//! | command line | `cli` (flags and commands), `commands` (one handler per command; `main.rs` is a thin shell) |
//! | package sources | `backend` (brew, cask, npm, uv, mas), `select` (which source carries a name), `names` (plain-text names), `config` |
//! | vendor downloads | `recipe` (how to find an app's latest download), `discovery` (finding an app no source carries), `links` and `asset` (reading pages and file names), `html`, `render` (JavaScript pages) |
//! | trust | `verify` (signature, notarization, Team ID, checksum), `direct` (download, verify, install, remove), `quarantine` (hostile candidate text) |
//! | decisions | `decision` (rules around the decider), `model` (the local model), `outcome` |
//! | state and plumbing | `manifest`, `paths`, `plan`, `net` (the only place that fetches), `runner` (every external tool) |

pub mod asset;
pub mod backend;
pub mod cli;
pub mod commands;
pub mod config;
pub mod decision;
pub mod direct;
pub mod discovery;
pub mod html;
pub mod links;
pub mod manifest;
pub mod model;
pub mod names;
pub mod net;
pub mod outcome;
pub mod paths;
pub mod plan;
pub mod quarantine;
pub mod recipe;
pub mod render;
pub mod runner;
pub mod select;
pub mod verify;
