//! `Cargo.toml` and `.cargo/config.toml` read as the pinned nightly's cargo
//! (`harness/toolchain`) reads them, its `cargo-features` activated but its `-Z` flags not
//! read, so that blaze knows a workspace without running cargo: its packages, targets,
//! dependencies and features, and the configuration that applies. What cargo accepts reads
//! the same here; what it rejects is rejected with its message. `cargo metadata --no-deps` is
//! the check: [`metadata`] prints the same JSON, and `harness/resolve.py` compares the two
//! over a corpus.

pub mod config;
pub mod dependency;
pub mod diagnostic;
mod edit_distance;
pub mod features;
pub mod manifest;
pub mod metadata;
pub mod paths;
pub mod profiles;
pub mod source;
pub mod targets;
pub mod workspace;

pub use config::Config;
pub use manifest::Context;
pub use workspace::Workspace;
