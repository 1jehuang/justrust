//! Where a package comes from, as cargo 1.99.0 (5f94df4789) names it: [`SourceId`], the
//! interned names it and the resolver compare, and the names of the directories cargo keeps a
//! source in under `$CARGO_HOME`. Pure: no file, network or environment is read, so that
//! blaze-resolve and the crates over it build for wasm (#458), and blaze-registry, which
//! fetches, names its directories from the same type.

pub mod interning;
pub mod source_id;

pub use interning::InternedString;
pub use source_id::{
    CRATES_IO_DOMAIN, CRATES_IO_HTTP_INDEX, CRATES_IO_INDEX, CRATES_IO_REGISTRY, CanonicalUrl,
    GitReference, Precise, PrettyRef, SourceId, SourceIdAsUrl, SourceKind, short_hash,
};
