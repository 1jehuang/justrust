//! Unstable checked structural token utilities for the native compiler.
//!
//! These APIs validate ownership, ranges, storage and resource accounting.
//! They do not establish Rust lexical validity, hygienic name resolution,
//! semantic authority, or a production macro-expansion path. The production
//! parser still consumes its existing lexer through the immutable cursor.

pub use super::expansion_budget::{
    BudgetFailure, BudgetFailureKind, BudgetResource, BudgetSnapshot, ByteLease, Counter, Depth,
    ExpansionBudget, ExpansionLimits, InvocationBudget, StateLease, Storage,
};
pub use super::expansion_span::{
    CrateId, ExpansionId, ExpansionRecord, SpanData, SpanId, SpanOrigin, SyntheticReason,
    TraceFrame,
};
pub use super::hygiene::{ContextData, SyntaxContextId, Transparency};
pub use super::resource_accounting::ReservationAccounting;
pub use super::source::source_map::{SourceFile, SourceMap, SourceSpan};
pub use super::token::{
    Delimiter, Edition, Ident, InputToken, LiteralKind, Spacing, Token, TokenKind,
};
pub use super::token_session::{
    ExpansionTrace, OriginalSpan, SourceFileHandle, SourceLimitResource, SourceSnapshot, SymbolId,
    TokenError, TokenSession,
};
pub use super::token_tree::{GroupPolicy, Node, TokenStream, TreeId, TreeRange, group};
