//! kaika: the universal package manager.
//!
//! Backends do the installing; kaika picks the right one, verifies downloads, and keeps a manifest.

pub mod backend;
pub mod cli;
pub mod commands;
pub mod config;
pub mod decision;
pub mod direct;
pub mod discovery;
pub mod html;
pub mod manifest;
pub mod paths;
pub mod plan;
pub mod recipe;
pub mod resolver;
pub mod runner;
pub mod select;
pub mod verify;
