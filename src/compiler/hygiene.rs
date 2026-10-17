//! Immutable hygiene-context algebra, not mixed-site name resolution.
use super::expansion_span::ExpansionId;
pub use super::token_session::SyntaxContextId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transparency {
    Opaque,
    SemiTransparent,
    Transparent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContextData {
    Root,
    Mark {
        parent: SyntaxContextId,
        expansion: ExpansionId,
        transparency: Transparency,
    },
}
