//! Existing-project support (G8): manifests, workspaces, targets, features,
//! profiles and a Cargo-compatible check/build/test/run driver.

pub mod cli;
pub mod driver;
pub mod inputs;
pub mod manifest;
pub mod target_cfg;
pub mod toml;
