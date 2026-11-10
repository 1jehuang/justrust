//! The first in-memory, std-only graph engine.
//!
//! The global ready queue is a correctness baseline, not a scalability claim.
//! See [`engine`] for the snapshot, cancellation, and execution contract.

pub mod compiler;
pub use justrust_graph as engine;
pub mod fixtures;
pub use justrust_cache as persistent_cache;
pub mod project;

pub use engine::{
    BuildRoot, CheckRoot, Context, Demand, Engine, EngineAuthority, Error, Event, Fact, Handle,
    ItemKey, Key, Outputs, OwnerRoot, OwnerTag, Provider, Report, RootKey, TagMint, TaskFuture,
};
