//! Owner-qualified expansion provenance. None of these IDs grant semantic authority.
use super::hygiene::SyntaxContextId;
use super::token::Edition;
use super::token_session::OriginalSpan;
pub use super::token_session::{CrateId, ExpansionId, SpanId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyntheticReason {
    TransportDelimiter,
    DocumentationAttribute,
    ExpansionToken,
    DerivedMember,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpanOrigin {
    Original(OriginalSpan),
    Synthetic {
        anchor: SpanId,
        reason: SyntheticReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpanData {
    pub origin: SpanOrigin,
    pub context: SyntaxContextId,
    pub expansion: Option<ExpansionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExpansionRecord {
    pub definition: SpanId,
    pub call: SpanId,
    pub parent: Option<ExpansionId>,
    pub defining_crate: CrateId,
    pub edition: Edition,
}

/// A renderer consumes this bounded, nonallocating ancestry iterator's records.
/// The session supplies validation and a maximum frame count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceFrame {
    pub expansion: ExpansionId,
    pub record: ExpansionRecord,
}
