//! Unstable checked lexical and structural token utilities for the native compiler.
//!
//! Structural constructors validate ownership, ranges, storage and resource
//! accounting, not lexical validity. `lex_prepared` performs native lexical
//! validation over a checked source projection. Neither path grants hygienic
//! name resolution, semantic authority, or a macro-expansion implementation.

pub use super::expansion_budget::{
    BudgetFailure, BudgetFailureKind, BudgetResource, BudgetSnapshot, ByteLease, Counter, Depth,
    ExpansionBudget, ExpansionLimits, InvocationBudget, StateLease, Storage,
};
pub use super::expansion_span::{
    CrateId, ExpansionId, ExpansionRecord, SpanData, SpanId, SpanOrigin, SyntheticReason,
    TraceFrame,
};
pub use super::hygiene::{ContextData, SyntaxContextId, Transparency};
pub use super::lexer::{
    DecodedString, DocComment, DocStyle, LexError, LexedEof, LexedEvent, LexedTokens, LiteralInfo,
    decode_normal_string, lex_prepared,
};
pub use super::macro_cursor::{CursorMark, LogicalToken, MacroCursor, TokenSlice};
pub use super::macro_fragments::{FragmentId, FragmentKind, FragmentPayload, LiteralCapture};
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
