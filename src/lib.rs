//! The first in-memory, std-only graph engine.
//!
//! The global ready queue is a correctness baseline, not a scalability claim.
//! See [`engine`] for the snapshot, cancellation, and execution contract.

pub mod compiler;
pub mod engine;
pub mod fixtures;
pub mod project;

pub use engine::{
    BuildRoot, CheckRoot, Context, Demand, Engine, EngineAuthority, Error, Event, Fact, Handle,
    ItemKey, Key, Outputs, OwnerRoot, OwnerTag, Provider, Report, RootKey, TagMint, TaskFuture,
};
