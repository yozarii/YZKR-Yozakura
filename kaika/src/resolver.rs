//! The outcome type every decision shares, and the confidence bar.
//!
//! Vendor-site discovery and single decisions now run natively (`discovery.rs`, `decision.rs`); the Python in
//! `poc/` is the reference implementation they were ported from and the place to experiment.

/// A decision must reach this probability to count.
pub const THRESHOLD: f64 = 0.6;

#[derive(Debug, PartialEq)]
pub enum Outcome<T> {
    Picked(T),
    /// Not decided. Never an error: the caller falls back, escalates, or asks the user.
    Unsure(String),
}
