//! blaze's own clippy: clippy's lints, computed from what rust-analyzer knows of the source,
//! at the levels rustc would give them, written as clippy-driver and cargo write them. No
//! clippy, rustc or cargo runs.
//!
//! [`registry`] is clippy's table of lints and groups; [`level`] decides a lint's level as
//! rustc does; [`check`] runs the lints over a crate; [`diag`] is a diagnostic as data and
//! [`emit`] writes it as rustc does, in text and in JSON.

pub mod check;
pub mod conf;
pub mod diag;
pub mod emit;
pub mod level;
mod lints;
pub mod manifest;
mod msrvs;
pub mod registry;
pub mod serve;
mod sugg;
#[cfg(test)]
mod tests;

pub use check::{Found, Setup, check};
pub use level::Level;
pub use lints::names as implemented;
