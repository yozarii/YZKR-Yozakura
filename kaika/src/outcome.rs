//! The result every decision shares.

#[derive(Debug, PartialEq)]
pub enum Outcome<T> {
    Picked(T),
    /// Not decided. Never an error: the caller falls back or asks the user.
    Unsure(String),
}
