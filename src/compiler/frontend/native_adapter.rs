//! Checked, bounded lowering from native lexical events to the current parser.
//! This adapter does not admit macro expansion, hygienic names or new literals.
use super::{ParseError, ParserTokenKind, Token, TokenPart};
use crate::compiler::expansion_budget::Storage;
use crate::compiler::expansion_span::{SpanId, SyntheticReason};
use crate::compiler::lexer::{DocComment, LexError, LexedEvent, LexedTokens};
use crate::compiler::semantic_scope::ExpansionTokenAdmission;
use crate::compiler::source::PreparedLexInput;
use crate::compiler::source::{SourceError, SourceErrorKind};
use crate::compiler::token::{InputToken, Spacing, TokenKind};
use crate::compiler::token_session::{BudgetVec, TokenError, TokenSession};
use std::ops::{Deref, Range};

/// Every token has one original constituent. Only joined logical tokens need
/// additional original occurrences, whose immutable backing stays metered.
#[derive(Clone, Debug)]
pub(super) struct TokenConstituents {
    first: SpanId,
    tail: Option<std::sync::Arc<ConstituentTail>>,
}
#[derive(Debug)]
struct ConstituentTail {
    spans: [Option<SpanId>; 2],
    _lease: crate::compiler::expansion_budget::ByteLease,
}
impl TokenConstituents {
    fn new(
        budget: &crate::compiler::expansion_budget::ExpansionBudget,
        parts: &[TokenPart],
        fallback: SpanId,
    ) -> Result<Self, ParseError> {
        budget
            .check()
            .map_err(|error| transport_error(error.into()))?;
        if parts.len() > 3 {
            return Err(transport_error(TokenError::InvalidRange));
        }
        let first = parts.first().map_or(fallback, |part| part.span);
        let tail = if parts.len() > 1 {
            let fixed = std::mem::size_of::<ConstituentTail>()
                .checked_add(2 * std::mem::size_of::<usize>())
                .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
            let lease = budget
                .reserve(Storage::Retained, fixed as u64)
                .map_err(|error| transport_error(error.into()))?;
            Some(std::sync::Arc::new(ConstituentTail {
                spans: [
                    parts.get(1).map(|part| part.span),
                    parts.get(2).map(|part| part.span),
                ],
                _lease: lease,
            }))
        } else {
            None
        };
        Ok(Self { first, tail })
    }
    pub(super) fn get(&self, index: usize) -> Option<SpanId> {
        match index {
            0 => Some(self.first),
            1 | 2 => self.tail.as_ref()?.spans[index - 1],
            _ => None,
        }
    }
    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = &SpanId> {
        std::iter::once(&self.first).chain(
            self.tail
                .iter()
                .flat_map(|tail| tail.spans.iter().flatten()),
        )
    }
}

pub(super) struct NativeParserInput {
    edition: crate::compiler::token::Edition,
    tokens: BudgetVec<Token>,
    session: TokenSession,
    /// Local parser admission only: consumed by `NameStoreBuilder::new`,
    /// never retained in a published fact.
    owner: crate::compiler::token_session::SessionOwnerCap,
    text_leases: BudgetVec<crate::compiler::expansion_budget::ByteLease>,
    names: BudgetVec<AdmittedRootName>,
    macro_source: Option<crate::compiler::token_tree::TokenStream>,
}
/// Issued only after the native identifier's context, rawness and spelling have
/// passed the bounded root-only ingress gate. This is not a hygiene projection.
struct AdmittedRootName {
    symbol: crate::compiler::token_session::SymbolId,
    raw: bool,
    span: SpanId,
    edition: crate::compiler::token::Edition,
    logical: u32,
}
/// Independently owned lexical views. These strings never borrow the session,
/// so parsing may mint semantic identifiers while holding token views.
pub(super) struct NativeTokenBuffer {
    tokens: BudgetVec<Token>,
    names: AdmittedNames,
    _text_leases: BudgetVec<crate::compiler::expansion_budget::ByteLease>,
    macro_source: Option<crate::compiler::token_tree::TokenStream>,
}
enum AdmittedNames {
    Root(BudgetVec<AdmittedRootName>),
    Expanded {
        names: BudgetVec<AdmittedExpandedName>,
        literals: BudgetVec<AdmittedOpaqueLiteral>,
        macro_slice: Option<crate::compiler::macro_cursor::TokenSlice>,
        admission: ExpansionProof,
    },
}
// The item/expression union is deliberately not inline in every buffer enum.
// Its one immutable owner is fallibly allocated and fully capacity-accounted.
struct ExpansionProof {
    values: BudgetVec<ExpansionTokenAdmission>,
}
impl ExpansionProof {
    fn checked(&self) -> Result<&ExpansionTokenAdmission, ParseError> {
        let [proof] = self.values.as_slice() else {
            return Err(transport_error(TokenError::InvalidHandle));
        };
        Ok(proof)
    }
}
struct AdmittedExpandedName {
    token: crate::compiler::token::Token,
    edition: crate::compiler::token::Edition,
    logical: u32,
}
struct AdmittedOpaqueLiteral {
    token: crate::compiler::token::Token,
    logical: u32,
}
impl Deref for NativeTokenBuffer {
    type Target = [Token];
    fn deref(&self) -> &Self::Target {
        self.tokens.as_slice()
    }
}
impl std::fmt::Debug for NativeTokenBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.tokens.as_slice()).finish()
    }
}
impl NativeTokenBuffer {
    /// Actual engine output only. Isolated literal-consumption buffers never
    /// expose their semantic leaves as fresh macro matcher input.
    pub(super) fn current_macro_slice(&self) -> Option<&crate::compiler::macro_cursor::TokenSlice> {
        match &self.names {
            AdmittedNames::Expanded { macro_slice, .. } => macro_slice.as_ref(),
            AdmittedNames::Root(_) => None,
        }
    }

    pub(super) fn retain_macro_slice(
        &mut self,
        session: &TokenSession,
        slice: crate::compiler::macro_cursor::TokenSlice,
    ) -> Result<(), ParseError> {
        if !self.tokens.as_slice().is_empty() {
            return Err(transport_error(TokenError::InvalidHandle));
        }
        let AdmittedNames::Expanded {
            macro_slice,
            admission,
            ..
        } = &mut self.names
        else {
            return Err(transport_error(TokenError::InvalidHandle));
        };
        admission
            .checked()?
            .validate_session(session)
            .map_err(scope_error)?;
        super::expanded_adapter::validate_slice(session, &slice, admission.checked()?)?;
        if macro_slice.is_some() {
            return Err(transport_error(TokenError::InvalidHandle));
        }
        *macro_slice = Some(slice);
        Ok(())
    }

    pub(super) fn macro_source(&self) -> Option<&crate::compiler::token_tree::TokenStream> {
        self.macro_source.as_ref()
    }
    pub(super) fn expanded(
        session: &TokenSession,
        admission: &crate::compiler::semantic_scope::ExpansionAdmission,
    ) -> Result<Self, ParseError> {
        Self::expanded_in(session, &ExpansionTokenAdmission::Expression(*admission))
    }

    pub(super) fn expanded_in(
        session: &TokenSession,
        admission: &ExpansionTokenAdmission,
    ) -> Result<Self, ParseError> {
        admission.validate_session(session).map_err(scope_error)?;
        let budget = session.budget();
        let mut proof = BudgetVec::new(budget, Storage::Retained).map_err(transport_error)?;
        proof.push(*admission).map_err(transport_error)?;
        Ok(Self {
            tokens: BudgetVec::new(budget, Storage::Retained).map_err(transport_error)?,
            names: AdmittedNames::Expanded {
                names: BudgetVec::new(budget, Storage::Retained).map_err(transport_error)?,
                literals: BudgetVec::new(budget, Storage::Retained).map_err(transport_error)?,
                macro_slice: None,
                admission: ExpansionProof { values: proof },
            },
            _text_leases: BudgetVec::new(budget, Storage::Retained).map_err(transport_error)?,
            macro_source: None,
        })
    }

    pub(super) fn push_opaque_literal(
        &mut self,
        session: &TokenSession,
        token: crate::compiler::token::Token,
        part: TokenPart,
    ) -> Result<(), ParseError> {
        let AdmittedNames::Expanded {
            literals,
            admission,
            ..
        } = &mut self.names
        else {
            return Err(transport_error(TokenError::InvalidHandle));
        };
        admission
            .checked()?
            .validate_token(session, token)
            .map_err(scope_error)?;
        if !matches!(token.kind, TokenKind::Opaque(_)) || part.span != token.span {
            return Err(transport_error(TokenError::InvalidHandle));
        }
        let index = literals
            .len()
            .checked_add(1)
            .and_then(|index| u32::try_from(index).ok())
            .and_then(std::num::NonZeroU32::new)
            .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
        let logical = u32::try_from(self.tokens.len())
            .map_err(|_| transport_error(TokenError::IdExhausted))?;
        literals
            .push(AdmittedOpaqueLiteral { token, logical })
            .map_err(transport_error)?;
        // A nonempty private sentinel makes buffer-identity pointer checks
        // meaningful. It is never matched or parsed as source spelling.
        self.push_expanded(
            session,
            "<opaque>",
            ParserTokenKind::OpaqueLiteral,
            &[part],
            None,
        )?;
        let mut logical = self
            .tokens
            .pop()
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        logical.name = Some(index);
        self.tokens.push(logical).map_err(transport_error)
    }

    /// Resolve only this buffer's opaque occurrence into a private, isolated
    /// semantic parser input. The matcher stream and FragmentId stay opaque.
    pub(super) fn opaque_literal_buffer(
        &self,
        view: &super::TokenView<'_>,
        names: &mut crate::compiler::semantic_name::NameStoreBuilder,
    ) -> Result<NativeTokenBuffer, ParseError> {
        let AdmittedNames::Expanded {
            literals,
            admission,
            ..
        } = &self.names
        else {
            return Err(transport_error(TokenError::InvalidHandle));
        };
        if !view.is_opaque_literal() {
            return Err(transport_error(TokenError::InvalidHandle));
        }
        let index = view
            .name
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let record = literals
            .as_slice()
            .get(index.get() as usize - 1)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let logical = self
            .tokens
            .as_slice()
            .get(record.logical as usize)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        if logical.kind != ParserTokenKind::OpaqueLiteral
            || !std::ptr::eq(logical.text.as_str(), view.text)
            || logical.name != Some(index)
            || view.span != record.token.span
            || view.span != logical.whole.span
            || view.bytes != logical.whole.bytes()
            || view.provenance != super::TokenProvenance::Expanded
            || logical.whole.provenance() != super::TokenProvenance::Expanded
        {
            return Err(transport_error(TokenError::ForeignHandle));
        }
        super::expanded_adapter::literal_buffer(names, record.token, admission.checked()?)
    }

    pub(super) fn push_expanded(
        &mut self,
        session: &TokenSession,
        text: &str,
        kind: ParserTokenKind,
        parts: &[TokenPart],
        name: Option<(
            crate::compiler::token::Token,
            crate::compiler::token::Edition,
        )>,
    ) -> Result<(), ParseError> {
        use crate::compiler::expansion_budget::Counter;
        let AdmittedNames::Expanded { admission, .. } = &self.names else {
            return Err(transport_error(TokenError::InvalidHandle));
        };
        let admission = admission.checked()?;
        admission.validate_session(session).map_err(scope_error)?;
        let whole = parts
            .first()
            .ok_or_else(|| transport_error(TokenError::InvalidRange))?
            .clone();
        if parts.len() > 3 {
            return Err(transport_error(TokenError::InvalidRange));
        }
        for part in parts {
            session.span(part.span).map_err(transport_error)?;
            if part.provenance() != super::TokenProvenance::Expanded {
                return Err(transport_error(TokenError::InvalidHandle));
            }
            admission
                .validate_span(session, part.span)
                .map_err(scope_error)?;
        }
        let budget = session.budget();
        let raw = kind == ParserTokenKind::Ident
            && name.is_some_and(
                |(token, _)| matches!(token.kind, TokenKind::Ident(ident) if ident.raw),
            );
        let text_len = text
            .len()
            .checked_add(if raw { 2 } else { 0 })
            .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
        budget
            .charge(Counter::Work, text_len as u64 + 1)
            .map_err(|error| transport_error(error.into()))?;
        let mut lease = budget
            .reserve(Storage::Retained, text_len as u64)
            .map_err(|error| transport_error(error.into()))?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(text_len)
            .map_err(|_| transport_error(TokenError::Budget(budget.poison_allocation())))?;
        lease
            .resize(owned.capacity() as u64)
            .map_err(|error| transport_error(error.into()))?;
        if raw {
            owned.push_str("r#");
        }
        owned.push_str(text);
        let constituents = TokenConstituents::new(budget, parts, whole.span)?;
        let mut suffixes = std::array::from_fn(|_| None);
        if kind == ParserTokenKind::Punct
            && matches!(text, ">>" | ">=" | ">>=" | "&&" | "<<" | "<<=")
        {
            for prefix in 1..text.len() {
                suffixes[prefix - 1] = Some(
                    parts
                        .get(prefix)
                        .ok_or_else(|| transport_error(TokenError::InvalidRange))?
                        .clone(),
                );
            }
        }
        let (line, column) = whole.position(1, 1, 0);
        let (line, column) = compact_position(line, column)?;
        let index = if let Some((token, edition)) = name {
            admission
                .validate_token(session, token)
                .map_err(scope_error)?;
            let AdmittedNames::Expanded { names, .. } = &mut self.names else {
                return Err(transport_error(TokenError::InvalidHandle));
            };
            let index = names
                .len()
                .checked_add(1)
                .and_then(|value| u32::try_from(value).ok())
                .and_then(std::num::NonZeroU32::new)
                .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
            let logical = u32::try_from(self.tokens.len())
                .map_err(|_| transport_error(TokenError::IdExhausted))?;
            names
                .push(AdmittedExpandedName {
                    token,
                    edition,
                    logical,
                })
                .map_err(transport_error)?;
            Some(index)
        } else {
            None
        };
        self._text_leases.push(lease).map_err(transport_error)?;
        self.tokens
            .push(Token {
                text: owned,
                line,
                column,
                string: kind == ParserTokenKind::String,
                kind,
                whole,
                constituents,
                suffixes,
                name: index,
            })
            .map_err(transport_error)
    }

    /// Read the edition of this exact admitted occurrence, not the receiving
    /// crate. Macro tokens can originate in a different edition.
    pub(super) fn name_edition(
        &self,
        view: &super::TokenView<'_>,
    ) -> Result<crate::compiler::token::Edition, ParseError> {
        let index = view
            .name
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let (edition, logical) = match &self.names {
            AdmittedNames::Root(names) => {
                let record = names
                    .as_slice()
                    .get(index.get() as usize - 1)
                    .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
                (record.edition, record.logical)
            }
            AdmittedNames::Expanded { names, .. } => {
                let record = names
                    .as_slice()
                    .get(index.get() as usize - 1)
                    .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
                (record.edition, record.logical)
            }
        };
        let logical = self
            .tokens
            .as_slice()
            .get(logical as usize)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        if !std::ptr::eq(logical.text.as_str(), view.text)
            || logical.name != Some(index)
            || view.span != logical.whole.span
            || view.bytes != logical.whole.bytes()
            || view.provenance != logical.whole.provenance()
        {
            return Err(transport_error(TokenError::ForeignHandle));
        }
        Ok(edition)
    }

    pub(super) fn edition_keyword_is_reserved(
        &self,
        view: &super::TokenView<'_>,
    ) -> Result<bool, ParseError> {
        use crate::compiler::token::Edition;
        let edition = self.name_edition(view)?;
        let spelling = view.text.strip_prefix('\'').unwrap_or(view.text);
        Ok(match spelling {
            "gen" => edition == Edition::E2024,
            "async" | "await" | "dyn" | "try" => edition != Edition::E2015,
            _ => false,
        })
    }

    pub(super) fn mint_name(
        &self,
        token: &super::TokenView<'_>,
        names: &mut crate::compiler::semantic_name::NameStoreBuilder,
    ) -> Result<crate::compiler::semantic_name::AstIdent, ParseError> {
        if token.is_opaque_literal() {
            return Err(transport_error(TokenError::InvalidHandle));
        }
        let index = token
            .name
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let AdmittedNames::Root(root_names) = &self.names else {
            return self.mint_expanded_name(token, names);
        };
        let admitted = root_names
            .as_slice()
            .get(index.get() as usize - 1)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let logical = self
            .tokens
            .as_slice()
            .get(admitted.logical as usize)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        if !std::ptr::eq(logical.text.as_str(), token.text)
            || logical.name != Some(index)
            || token.span != logical.whole.span
            || token.bytes != logical.whole.bytes()
            || token.provenance != logical.whole.provenance()
        {
            return Err(transport_error(TokenError::ForeignHandle));
        }
        let token = crate::compiler::token::Token {
            kind: TokenKind::Ident(crate::compiler::token::Ident {
                symbol: admitted.symbol,
                context: names.session().root_context(),
                raw: admitted.raw,
            }),
            span: admitted.span,
        };
        names
            .from_token(token, admitted.edition)
            .map_err(name_error)
    }

    fn mint_expanded_name(
        &self,
        view: &super::TokenView<'_>,
        names: &mut crate::compiler::semantic_name::NameStoreBuilder,
    ) -> Result<crate::compiler::semantic_name::AstIdent, ParseError> {
        let AdmittedNames::Expanded {
            names: admitted,
            admission,
            ..
        } = &self.names
        else {
            return Err(transport_error(TokenError::InvalidHandle));
        };
        let index = view
            .name
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let record = admitted
            .as_slice()
            .get(index.get() as usize - 1)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        let logical = self
            .tokens
            .as_slice()
            .get(record.logical as usize)
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        if !std::ptr::eq(logical.text.as_str(), view.text)
            || logical.name != Some(index)
            || view.span != logical.whole.span
            || view.provenance != super::TokenProvenance::Expanded
            || logical.whole.provenance() != super::TokenProvenance::Expanded
            || view.bytes != logical.whole.bytes()
        {
            return Err(transport_error(TokenError::ForeignHandle));
        }
        match admission.checked()? {
            ExpansionTokenAdmission::Expression(proof) => {
                names.admit_expanded_token(record.token, record.edition, proof)
            }
            ExpansionTokenAdmission::Item(proof) => {
                names.admit_item_token(record.token, record.edition, proof)
            }
        }
        .map_err(name_error)
    }
}

pub(super) fn name_error(error: crate::compiler::semantic_name::NameError) -> ParseError {
    use crate::compiler::semantic_name::NameError;
    match error {
        NameError::Token(error) => transport_error(error),
        error @ NameError::Unsupported(_) => {
            simple_error(SourceErrorKind::Unsupported, &error.to_string())
        }
        error @ (NameError::ExpectedIdentifier | NameError::InvalidIdentifier) => {
            simple_error(SourceErrorKind::Syntax, &error.to_string())
        }
    }
}
pub(super) fn name_diagnostic(error: crate::compiler::semantic_name::NameError) -> String {
    let error = name_error(error);
    let mut diagnostic = SourceError::syntax(error.primary, error.message);
    diagnostic._diagnostic_lease = error._diagnostic_lease;
    diagnostic.kind = error.kind;
    diagnostic.to_string()
}
impl std::fmt::Debug for NativeParserInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.tokens.as_slice()).finish()
    }
}
impl Deref for NativeParserInput {
    type Target = [Token];
    fn deref(&self) -> &Self::Target {
        self.tokens.as_slice()
    }
}
impl NativeParserInput {
    pub(super) fn edition(&self) -> crate::compiler::token::Edition {
        self.edition
    }
    pub(super) fn into_parts(
        self,
    ) -> (
        NativeTokenBuffer,
        TokenSession,
        crate::compiler::token_session::SessionOwnerCap,
    ) {
        (
            NativeTokenBuffer {
                tokens: self.tokens,
                names: AdmittedNames::Root(self.names),
                _text_leases: self.text_leases,
                macro_source: self.macro_source,
            },
            self.session,
            self.owner,
        )
    }
    pub(super) fn new(
        lexed: LexedTokens,
        session: TokenSession,
        owner: crate::compiler::token_session::SessionOwnerCap,
        projection: &PreparedLexInput<'_>,
    ) -> Result<Self, ParseError> {
        Self::new_with_purpose(lexed, session, owner, projection, false, false)
    }

    /// A mounted child file may invoke textual macros inherited from its
    /// parent without defining any itself (PARSE-SPLIT.md step C2), so it
    /// always retains its macro token source.
    pub(super) fn new_macro_bearing(
        lexed: LexedTokens,
        session: TokenSession,
        owner: crate::compiler::token_session::SessionOwnerCap,
        projection: &PreparedLexInput<'_>,
    ) -> Result<Self, ParseError> {
        Self::new_with_purpose(lexed, session, owner, projection, false, true)
    }
    pub(super) fn new_inactive(
        lexed: LexedTokens,
        session: TokenSession,
        owner: crate::compiler::token_session::SessionOwnerCap,
        projection: &PreparedLexInput<'_>,
    ) -> Result<Self, ParseError> {
        Self::new_with_purpose(lexed, session, owner, projection, true, false)
    }

    fn new_with_purpose(
        lexed: LexedTokens,
        mut session: TokenSession,
        owner: crate::compiler::token_session::SessionOwnerCap,
        projection: &PreparedLexInput<'_>,
        inactive: bool,
        inherits_macros: bool,
    ) -> Result<Self, ParseError> {
        lexed
            .validate_input(&session, projection)
            .map_err(lex_error)?;
        if projection.text().starts_with('\u{feff}') {
            let mut error = simple_error(
                SourceErrorKind::Unsupported,
                "byte-order-mark input is unsupported",
            );
            error.has_location = true;
            error.bytes = 0..3;
            return Err(error);
        }
        if projection.text().starts_with("#!") && !projection.text().starts_with("#![") {
            let mut error =
                simple_error(SourceErrorKind::Unsupported, "shebang input is unsupported");
            error.has_location = true;
            error.bytes = 0..2;
            return Err(error);
        }
        let budget = session.budget().clone();
        let edition = lexed.edition();
        // Native ordinary sources pay only the bounded audit scan. No second
        // token arena is retained unless an actual macro_rules! pair occurs.
        budget
            .charge(
                crate::compiler::expansion_budget::Counter::Work,
                lexed.events().len() as u64,
            )
            .map_err(|error| transport_error(error.into()))?;
        let mut macro_bearing = inactive || inherits_macros;
        for pair in lexed.events().windows(2) {
            if let [
                LexedEvent {
                    token:
                        InputToken::Leaf(crate::compiler::token::Token {
                            kind: TokenKind::Ident(ident),
                            ..
                        }),
                    ..
                },
                LexedEvent {
                    token:
                        InputToken::Leaf(crate::compiler::token::Token {
                            kind: TokenKind::Punct { ch: '!', .. },
                            ..
                        }),
                    ..
                },
            ] = pair
                && !ident.raw
                && session.symbol(ident.symbol).map_err(transport_error)? == "macro_rules"
            {
                macro_bearing = true;
                break;
            }
        }
        let macro_source = if macro_bearing {
            let mut events =
                BudgetVec::new(&budget, Storage::Temporary).map_err(transport_error)?;
            let mut docs = lexed.docs().iter().peekable();
            for event in lexed.events() {
                while let Some(doc) = docs.next_if(|doc| doc.prepared.start <= event.prepared.start)
                {
                    lower_doc_comment(&mut session, projection, doc, &mut events)?;
                }
                events.push(event.token).map_err(transport_error)?;
            }
            for doc in docs {
                lower_doc_comment(&mut session, projection, doc, &mut events)?;
            }
            Some(
                crate::compiler::token_tree::group(
                    &mut session,
                    events.as_slice(),
                    crate::compiler::token_tree::GroupPolicy::OriginalSource,
                )
                .map_err(transport_error)?,
            )
        } else {
            None
        };
        let mut input = Self {
            edition,
            tokens: BudgetVec::new(&budget, Storage::Retained).map_err(transport_error)?,
            text_leases: BudgetVec::new(&budget, Storage::Retained).map_err(transport_error)?,
            session,
            owner,
            names: BudgetVec::new(&budget, Storage::Retained).map_err(transport_error)?,
            macro_source,
        };
        let events = lexed.events();
        let mut at = 0;
        while at < events.len() {
            let event = &events[at];
            input.check_event(event)?;
            if inactive {
                let deferred = match event.token {
                    InputToken::Leaf(crate::compiler::token::Token {
                        kind: TokenKind::Ident(ident),
                        ..
                    }) => {
                        ident.raw
                            || !input
                                .session
                                .symbol(ident.symbol)
                                .map_err(transport_error)?
                                .is_ascii()
                    }
                    InputToken::Leaf(crate::compiler::token::Token {
                        kind: TokenKind::Literal { kind, .. },
                        ..
                    }) => !matches!(
                        kind,
                        crate::compiler::token::LiteralKind::Integer
                            | crate::compiler::token::LiteralKind::Float
                            | crate::compiler::token::LiteralKind::String
                            | crate::compiler::token::LiteralKind::RawString
                            | crate::compiler::token::LiteralKind::ByteString
                            | crate::compiler::token::LiteralKind::RawByteString
                            | crate::compiler::token::LiteralKind::Char
                            | crate::compiler::token::LiteralKind::Byte
                    ),
                    _ => false,
                };
                if deferred {
                    input.push_range(
                        projection,
                        event.prepared.clone(),
                        event.line,
                        event.column,
                        ParserTokenKind::DeferredUnsupported,
                        None,
                        &[native_part(event)?],
                    )?;
                    at += 1;
                    continue;
                }
            }
            match event.token {
                InputToken::Open { .. } | InputToken::Close { .. } => {
                    input.push_range(
                        projection,
                        event.prepared.clone(),
                        event.line,
                        event.column,
                        ParserTokenKind::Punct,
                        None,
                        &[native_part(event)?],
                    )?;
                    at += 1;
                }
                InputToken::Leaf(token) => match token.kind {
                    TokenKind::Ident(ident) => {
                        let text = input
                            .session
                            .symbol(ident.symbol)
                            .map_err(transport_error)?;
                        if !text.is_ascii() || ident.context != input.session.root_context() {
                            return Err(event_error(
                                event,
                                SourceErrorKind::Unsupported,
                                "non-ASCII and non-root identifiers are unsupported",
                            ));
                        }
                        input.push_range(
                            projection,
                            event.prepared.clone(),
                            event.line,
                            event.column,
                            ParserTokenKind::Ident,
                            None,
                            &[native_part(event)?],
                        )?;
                        input.attach_name(token, edition)?;
                        at += 1;
                    }
                    TokenKind::Literal { kind, spelling } => {
                        use crate::compiler::token::LiteralKind;
                        match kind {
                            LiteralKind::String | LiteralKind::RawString => {
                                let info = event.literal.as_ref().ok_or_else(|| {
                                    event_error(
                                        event,
                                        SourceErrorKind::Contract,
                                        "literal metadata missing",
                                    )
                                })?;
                                if !info.suffix.is_empty() {
                                    if input.macro_source.is_some() {
                                        // The canonical inventory retains the literal.
                                        // This unparsed, non-string view can only be
                                        // skipped, never consumed as a valid string.
                                        input.push_range(
                                            projection,
                                            event.prepared.clone(),
                                            event.line,
                                            event.column,
                                            ParserTokenKind::DeferredLiteral,
                                            None,
                                            &[native_part(event)?],
                                        )?;
                                        at += 1;
                                        continue;
                                    }
                                    return Err(event_error(
                                        event,
                                        SourceErrorKind::Syntax,
                                        "string literal suffix is invalid",
                                    ));
                                }
                                let spelling =
                                    input.session.symbol(spelling).map_err(transport_error)?;
                                let decoded =
                                    crate::compiler::lexer::decode_text_string(spelling, &budget)
                                        .map_err(|e| {
                                        let mut error = lex_error(e);
                                        if error.kind == SourceErrorKind::Syntax {
                                            error.kind = SourceErrorKind::Contract;
                                            error.message =
                                                "validated string decoding failed".into();
                                        }
                                        error
                                    })?;
                                input.push_range(
                                    projection,
                                    event.prepared.clone(),
                                    event.line,
                                    event.column,
                                    ParserTokenKind::String,
                                    Some(decoded.as_str()),
                                    &[native_part(event)?],
                                )?;
                            }
                            LiteralKind::ByteString | LiteralKind::RawByteString => {
                                input.push_range(
                                    projection,
                                    event.prepared.clone(),
                                    event.line,
                                    event.column,
                                    ParserTokenKind::ByteString,
                                    None,
                                    &[native_part(event)?],
                                )?;
                            }
                            LiteralKind::Char | LiteralKind::Byte => {
                                input.push_range(
                                    projection,
                                    event.prepared.clone(),
                                    event.line,
                                    event.column,
                                    if kind == LiteralKind::Byte {
                                        ParserTokenKind::Byte
                                    } else {
                                        ParserTokenKind::Char
                                    },
                                    None,
                                    &[native_part(event)?],
                                )?;
                            }
                            LiteralKind::Integer | LiteralKind::Float => {
                                input.push_number(projection, event)?;
                            }
                            _ => {
                                return Err(event_error(
                                    event,
                                    SourceErrorKind::Unsupported,
                                    "C string literals are unsupported",
                                ));
                            }
                        }
                        at += 1;
                    }
                    TokenKind::Punct {
                        ch: '\'',
                        spacing: Spacing::Joint,
                    } if events.get(at + 1).is_some_and(|next| {
                        matches!(
                            next.token,
                            InputToken::Leaf(crate::compiler::token::Token {
                                kind: TokenKind::Ident(_),
                                ..
                            })
                        )
                    }) =>
                    {
                        let next = &events[at + 1];
                        input.check_event(next)?;
                        let InputToken::Leaf(crate::compiler::token::Token {
                            kind: TokenKind::Ident(ident),
                            ..
                        }) = next.token
                        else {
                            unreachable!()
                        };
                        let name = input
                            .session
                            .symbol(ident.symbol)
                            .map_err(transport_error)?;
                        if inactive && !name.is_ascii() {
                            input.push_range(
                                projection,
                                event.prepared.start..next.prepared.end,
                                event.line,
                                event.column,
                                ParserTokenKind::DeferredUnsupported,
                                None,
                                &[native_part(event)?, native_part(next)?],
                            )?;
                            at += 2;
                            continue;
                        }
                        if !name.is_ascii() || ident.context != input.session.root_context() {
                            return Err(event_error(
                                next,
                                SourceErrorKind::Unsupported,
                                "non-ASCII and non-root lifetimes are unsupported",
                            ));
                        }
                        if !inactive && ident.raw && name == "static" {
                            return Err(event_error(
                                next,
                                SourceErrorKind::Unsupported,
                                "static lifetimes are unsupported",
                            ));
                        }
                        if event.prepared.end != next.prepared.start {
                            return Err(event_error(
                                event,
                                SourceErrorKind::Contract,
                                "joint lifetime is not adjacent",
                            ));
                        }
                        input.push_range(
                            projection,
                            event.prepared.start..next.prepared.end,
                            event.line,
                            event.column,
                            ParserTokenKind::Lifetime,
                            None,
                            &[native_part(event)?, native_part(next)?],
                        )?;
                        let InputToken::Leaf(name_token) = next.token else {
                            unreachable!("validated lifetime identifier leaf")
                        };
                        input.attach_name(name_token, edition)?;
                        at += 2;
                    }
                    TokenKind::Punct { .. } => {
                        let mut count = 1;
                        let mut end = event.prepared.end;
                        for n in 1..3 {
                            let Some(next) = events.get(at + n) else {
                                break;
                            };
                            let previous = &events[at + n - 1];
                            if !matches!(
                                previous.token,
                                InputToken::Leaf(crate::compiler::token::Token {
                                    kind: TokenKind::Punct {
                                        spacing: Spacing::Joint,
                                        ..
                                    },
                                    ..
                                })
                            ) || !matches!(
                                next.token,
                                InputToken::Leaf(crate::compiler::token::Token {
                                    kind: TokenKind::Punct { .. },
                                    ..
                                })
                            ) || end != next.prepared.start
                            {
                                break;
                            }
                            input.check_event(next)?;
                            let candidate = projection
                                .text()
                                .get(event.prepared.start..next.prepared.end)
                                .ok_or_else(|| {
                                    event_error(
                                        event,
                                        SourceErrorKind::Contract,
                                        "invalid punctuation view range",
                                    )
                                })?;
                            if !compound(candidate) {
                                break;
                            }
                            count = n + 1;
                            end = next.prepared.end;
                        }
                        let range = event.prepared.start..end;
                        let text = projection.text().get(range.clone()).ok_or_else(|| {
                            event_error(
                                event,
                                SourceErrorKind::Contract,
                                "invalid punctuation range",
                            )
                        })?;
                        let attribute = text == "#"
                            && events.get(at + 1).is_some_and(|next| {
                                matches!(next.token, InputToken::Open { .. })
                                    && projection.text().get(next.prepared.clone()) == Some("[")
                            });
                        let admitted = punctuation(text) || attribute;
                        let deferred = !admitted && input.macro_source.is_some();
                        if !admitted && !deferred {
                            return Err(event_error(
                                event,
                                if matches!(text, "#" | "@" | "?" | "$" | "~") {
                                    SourceErrorKind::Unsupported
                                } else {
                                    SourceErrorKind::Syntax
                                },
                                &format!(
                                    "{} token {text:?}",
                                    if matches!(text, "#" | "@" | "?" | "$" | "~") {
                                        "unsupported"
                                    } else {
                                        "invalid"
                                    }
                                ),
                            ));
                        }
                        let first = native_part(event)?;
                        let mut parts: [TokenPart; 3] = std::array::from_fn(|_| first.clone());
                        for n in 0..count {
                            parts[n] = native_part(&events[at + n])?;
                        }
                        input.push_range(
                            projection,
                            range,
                            event.line,
                            event.column,
                            if deferred {
                                ParserTokenKind::DeferredUnsupported
                            } else {
                                ParserTokenKind::Punct
                            },
                            None,
                            &parts[..count],
                        )?;
                        at += count;
                    }
                    TokenKind::DollarCrate(_) => {
                        return Err(event_error(
                            event,
                            SourceErrorKind::Unsupported,
                            "macro crate identities are unsupported",
                        ));
                    }
                    TokenKind::Opaque(_) => {
                        return Err(event_error(
                            event,
                            SourceErrorKind::Unsupported,
                            "opaque macro fragments are unsupported by this parser",
                        ));
                    }
                },
            }
        }
        let eof = lexed.eof();
        if eof.prepared != (projection.text().len()..projection.text().len()) {
            return Err(simple_error(
                SourceErrorKind::Contract,
                "invalid native EOF range",
            ));
        }
        input.push_range(
            projection,
            eof.prepared.clone(),
            eof.line,
            eof.column,
            ParserTokenKind::Eof,
            None,
            &[TokenPart::new(eof.span, eof.prepared.clone())?],
        )?;
        Ok(input)
    }

    fn check_event(&self, event: &LexedEvent) -> Result<(), ParseError> {
        let span = self
            .session
            .span(event.token.span())
            .map_err(transport_error)?;
        if span.context != self.session.root_context() || span.expansion.is_some() {
            return Err(event_error(
                event,
                SourceErrorKind::Unsupported,
                "expanded or hygienic tokens are unsupported by this parser",
            ));
        }
        if let InputToken::Leaf(token) = event.token {
            token.validate(&self.session).map_err(transport_error)?;
        }
        Ok(())
    }

    fn part(
        &mut self,
        projection: &PreparedLexInput<'_>,
        bytes: Range<usize>,
    ) -> Result<TokenPart, ParseError> {
        let projected = projection.projected(bytes.clone()).map_err(source_error)?;
        let anchor = self
            .session
            .original_span(projected.original)
            .map_err(transport_error)?;
        let span = if projected.synthetic {
            self.session
                .synthetic_span(anchor, SyntheticReason::TransportDelimiter)
                .map_err(transport_error)?
        } else {
            anchor
        };
        TokenPart::new(span, bytes)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_range(
        &mut self,
        projection: &PreparedLexInput<'_>,
        bytes: Range<usize>,
        line: usize,
        column: usize,
        kind: ParserTokenKind,
        decoded: Option<&str>,
        parts: &[TokenPart],
    ) -> Result<(), ParseError> {
        let (line, column) = compact_position(line, column)?;
        // Validate the whole legacy logical token, not merely its lexical atoms.
        let whole = if let [part] = parts
            && part.bytes() == bytes
        {
            projection.projected(bytes.clone()).map_err(source_error)?;
            part.clone()
        } else {
            self.part(projection, bytes.clone())?
        };
        let text = decoded.unwrap_or(projection.text().get(bytes.clone()).ok_or_else(|| {
            simple_error(SourceErrorKind::Contract, "invalid logical token range")
        })?);
        let budget = self.session.budget();
        let mut lease = budget
            .reserve(Storage::Retained, text.len() as u64)
            .map_err(|e| transport_error(TokenError::Budget(e)))?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(text.len())
            .map_err(|_| transport_error(TokenError::Budget(budget.poison_allocation())))?;
        lease
            .resize(owned.capacity() as u64)
            .map_err(|e| transport_error(TokenError::Budget(e)))?;
        owned.push_str(text);
        let constituents = TokenConstituents::new(budget, parts, whole.span)?;
        let mut suffixes = std::array::from_fn(|_| None);
        if kind == ParserTokenKind::Punct
            && matches!(text, ">>" | ">=" | ">>=" | "&&" | "<<" | "<<=")
        {
            for prefix in 1..text.len() {
                suffixes[prefix - 1] =
                    Some(self.part(projection, bytes.start + prefix..bytes.end)?);
            }
        }
        self.text_leases.push(lease).map_err(transport_error)?;
        self.tokens
            .push(Token {
                text: owned,
                line,
                column,
                string: kind == ParserTokenKind::String,
                kind,
                whole,
                constituents,
                suffixes,
                name: None,
            })
            .map_err(transport_error)
    }

    fn attach_name(
        &mut self,
        token: crate::compiler::token::Token,
        edition: crate::compiler::token::Edition,
    ) -> Result<(), ParseError> {
        let TokenKind::Ident(ident) = token.kind else {
            return Err(transport_error(TokenError::InvalidInput(
                "invalid native parser name payload",
            )));
        };
        token.validate(&self.session).map_err(transport_error)?;
        if ident.context != self.session.root_context()
            || !self
                .session
                .symbol(ident.symbol)
                .map_err(transport_error)?
                .is_ascii()
        {
            return Err(transport_error(TokenError::InvalidInput(
                "invalid native parser name payload",
            )));
        }
        let index = u32::try_from(self.names.len())
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(std::num::NonZeroU32::new)
            .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
        let logical = self
            .tokens
            .len()
            .checked_sub(1)
            .and_then(|index| u32::try_from(index).ok())
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        self.names
            .push(AdmittedRootName {
                symbol: ident.symbol,
                raw: ident.raw,
                span: token.span,
                edition,
                logical,
            })
            .map_err(transport_error)?;
        let mut logical = self
            .tokens
            .pop()
            .ok_or_else(|| transport_error(TokenError::InvalidHandle))?;
        if logical.name.is_some()
            || !matches!(
                logical.kind,
                ParserTokenKind::Ident | ParserTokenKind::Lifetime
            )
        {
            return Err(transport_error(TokenError::InvalidInput(
                "invalid native parser name payload",
            )));
        }
        logical.name = Some(index);
        self.tokens.push(logical).map_err(transport_error)
    }

    fn push_number(
        &mut self,
        projection: &PreparedLexInput<'_>,
        event: &LexedEvent,
    ) -> Result<(), ParseError> {
        let after_dot = self
            .tokens
            .last()
            .is_some_and(|t| t.kind == ParserTokenKind::Punct && t.text == ".");
        let info = event.literal.as_ref().ok_or_else(|| {
            event_error(event, SourceErrorKind::Contract, "number metadata missing")
        })?;
        if !after_dot {
            return self.push_range(
                projection,
                event.prepared.clone(),
                event.line,
                event.column,
                ParserTokenKind::Number,
                None,
                &[native_part(event)?],
            );
        }
        // The old parser's tuple-number view divides the already lexed literal
        // at its known decimal point and signed exponent. This is not relexing.
        let mut start = event.prepared.start;
        if let Some(dot) = info.decimal_point {
            self.push_range(
                projection,
                start..dot,
                event.line,
                event.column,
                ParserTokenKind::Number,
                None,
                &[],
            )?;
            self.push_range(
                projection,
                dot..dot + 1,
                event.line,
                event.column + dot - event.prepared.start,
                ParserTokenKind::Punct,
                None,
                &[],
            )?;
            start = dot + 1;
        }
        if let Some(exponent) = info.exponent {
            let sign = exponent + 1;
            if matches!(projection.text().as_bytes().get(sign), Some(b'+' | b'-')) {
                self.push_range(
                    projection,
                    start..sign,
                    event.line,
                    event.column + start - event.prepared.start,
                    ParserTokenKind::Number,
                    None,
                    &[],
                )?;
                self.push_range(
                    projection,
                    sign..sign + 1,
                    event.line,
                    event.column + sign - event.prepared.start,
                    ParserTokenKind::Punct,
                    None,
                    &[],
                )?;
                start = sign + 1;
            }
        }
        if start < event.prepared.end {
            self.push_range(
                projection,
                start..event.prepared.end,
                event.line,
                event.column + start - event.prepared.start,
                ParserTokenKind::Number,
                None,
                &[],
            )?;
        }
        Ok(())
    }
}

fn lower_doc_comment(
    session: &mut TokenSession,
    projection: &PreparedLexInput<'_>,
    doc: &DocComment,
    out: &mut BudgetVec<InputToken>,
) -> Result<(), ParseError> {
    let spelling = projection
        .text()
        .get(doc.body.clone())
        .ok_or_else(|| simple_error(SourceErrorKind::Contract, "invalid doc comment body range"))?;
    let literal = doc_string_literal(spelling);
    let hash = session
        .synthetic_span(doc.span, SyntheticReason::DocumentationAttribute)
        .map_err(transport_error)?;
    let bang = session
        .synthetic_span(doc.span, SyntheticReason::DocumentationAttribute)
        .map_err(transport_error)?;
    let open = session
        .synthetic_span(doc.span, SyntheticReason::DocumentationAttribute)
        .map_err(transport_error)?;
    let name = session
        .synthetic_span(doc.span, SyntheticReason::DocumentationAttribute)
        .map_err(transport_error)?;
    let equals = session
        .synthetic_span(doc.span, SyntheticReason::DocumentationAttribute)
        .map_err(transport_error)?;
    let value = session
        .synthetic_span(doc.span, SyntheticReason::DocumentationAttribute)
        .map_err(transport_error)?;
    let close = session
        .synthetic_span(doc.span, SyntheticReason::DocumentationAttribute)
        .map_err(transport_error)?;
    let doc_symbol = session.intern_symbol("doc").map_err(transport_error)?;
    let literal_symbol = session.intern_symbol(&literal).map_err(transport_error)?;
    out.push(InputToken::Leaf(crate::compiler::token::Token {
        kind: TokenKind::Punct {
            ch: '#',
            spacing: Spacing::Joint,
        },
        span: hash,
    }))
    .map_err(transport_error)?;
    if doc.inner {
        out.push(InputToken::Leaf(crate::compiler::token::Token {
            kind: TokenKind::Punct {
                ch: '!',
                spacing: Spacing::Joint,
            },
            span: bang,
        }))
        .map_err(transport_error)?;
    }
    out.push(InputToken::Open {
        kind: crate::compiler::token::Delimiter::Bracket,
        span: open,
    })
    .map_err(transport_error)?;
    out.push(InputToken::Leaf(crate::compiler::token::Token {
        kind: TokenKind::Ident(crate::compiler::token::Ident {
            symbol: doc_symbol,
            raw: false,
            context: session.root_context(),
        }),
        span: name,
    }))
    .map_err(transport_error)?;
    out.push(InputToken::Leaf(crate::compiler::token::Token {
        kind: TokenKind::Punct {
            ch: '=',
            spacing: Spacing::Alone,
        },
        span: equals,
    }))
    .map_err(transport_error)?;
    out.push(InputToken::Leaf(crate::compiler::token::Token {
        kind: TokenKind::Literal {
            kind: crate::compiler::token::LiteralKind::String,
            spelling: literal_symbol,
        },
        span: value,
    }))
    .map_err(transport_error)?;
    out.push(InputToken::Close {
        kind: crate::compiler::token::Delimiter::Bracket,
        span: close,
    })
    .map_err(transport_error)
}

fn doc_string_literal(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('"');
    for ch in text.chars() {
        literal.extend(ch.escape_default());
    }
    literal.push('"');
    literal
}

pub(super) fn original(
    session: &TokenSession,
    span: SpanId,
) -> Result<crate::compiler::source::SourceSpan, ParseError> {
    use crate::compiler::expansion_span::SpanOrigin;
    let mut current = span;
    for _ in 0..512 {
        match session.span(current).map_err(transport_error)?.origin {
            SpanOrigin::Original(original) => {
                return session
                    .sources()
                    .render_span(original)
                    .map_err(transport_error);
            }
            SpanOrigin::Synthetic { anchor, .. } => current = anchor,
        }
    }
    Err(simple_error(
        SourceErrorKind::Contract,
        "parser token provenance is cyclic or too deep",
    ))
}
fn native_part(event: &LexedEvent) -> Result<TokenPart, ParseError> {
    TokenPart::new(event.token.span(), event.prepared.clone())
}
fn compound(s: &str) -> bool {
    matches!(
        s,
        "->" | "<-"
            | "::"
            | "=="
            | "!="
            | "<="
            | ">="
            | "<<"
            | ">>"
            | "&&"
            | "||"
            | "=>"
            | "%="
            | "+="
            | "-="
            | "*="
            | "/="
            | "&="
            | "|="
            | "^="
            | ".."
            | "..="
            | "<<="
            | ">>="
    )
}
pub(super) fn punctuation(s: &str) -> bool {
    compound(s)
        || matches!(
            s,
            "(" | ")"
                | "{"
                | "}"
                | "["
                | "]"
                | ":"
                | ";"
                | ","
                | "."
                | "+"
                | "-"
                | "*"
                | "/"
                | "%"
                | "="
                | "!"
                | "<"
                | ">"
                | "&"
                | "|"
                | "^"
                | "?"
                | "@"
        )
}
/// Expanded coordinates are rendered only from checked occurrence provenance.
/// The zero byte range below is deliberately not a prepared-source location.
pub(super) fn expanded_error(
    session: &TokenSession,
    span: SpanId,
    kind: SourceErrorKind,
    message: &str,
) -> ParseError {
    match expanded_diagnostic(session, span, message) {
        Ok((primary, line, column, message, lease)) => ParseError {
            kind,
            has_location: false,
            bytes: 0..0,
            primary: Some(primary),
            line,
            column,
            message,
            _diagnostic_lease: Some(lease),
        },
        Err(error) => error,
    }
}

fn expanded_diagnostic(
    session: &TokenSession,
    span: SpanId,
    message: &str,
) -> Result<
    (
        crate::compiler::source::SourceSpan,
        usize,
        usize,
        String,
        crate::compiler::expansion_budget::ByteLease,
    ),
    ParseError,
> {
    use std::fmt::Write;
    let source = session.original(span, 512).map_err(transport_error)?;
    let primary = session
        .sources()
        .render_span(source)
        .map_err(transport_error)?;
    let (_, line, column) = session
        .sources()
        .location(source)
        .map_err(transport_error)?;
    let budget = session.budget();
    budget
        .charge(
            crate::compiler::expansion_budget::Counter::Work,
            (message.len() as u64)
                .checked_add(1)
                .ok_or_else(|| transport_error(TokenError::IdExhausted))?,
        )
        .map_err(|error| transport_error(error.into()))?;
    let mut lease = budget
        .reserve(Storage::Retained, message.len() as u64)
        .map_err(|error| transport_error(error.into()))?;
    let mut text = String::new();
    text.try_reserve_exact(message.len())
        .map_err(|_| transport_error(TokenError::Budget(budget.poison_allocation())))?;
    lease
        .resize(text.capacity() as u64)
        .map_err(|error| transport_error(error.into()))?;
    text.push_str(message);
    let mut trace = session
        .trace(
            session.span(span).map_err(transport_error)?.expansion,
            budget.limits().backtrace_frames,
        )
        .map_err(transport_error)?;
    for frame in trace.by_ref() {
        let frame = frame.map_err(transport_error)?;
        budget
            .charge(crate::compiler::expansion_budget::Counter::Work, 192)
            .map_err(|error| transport_error(error.into()))?;
        let call = original(session, frame.record.call)?;
        let definition = original(session, frame.record.definition)?;
        let required = text
            .len()
            .checked_add(192)
            .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
        lease
            .resize(required.max(text.capacity()) as u64)
            .map_err(|error| transport_error(error.into()))?;
        text.try_reserve_exact(192)
            .map_err(|_| transport_error(TokenError::Budget(budget.poison_allocation())))?;
        lease
            .resize(text.capacity() as u64)
            .map_err(|error| transport_error(error.into()))?;
        write!(
            text,
            "\n  expanded at file {}:{}..{}, defined at file {}:{}..{}",
            call.file.0, call.lo, call.hi, definition.file.0, definition.lo, definition.hi
        )
        .map_err(|_| transport_error(TokenError::InvalidHandle))?;
    }
    if trace.truncated() {
        const NOTE: &str = "\n  expansion backtrace truncated";
        let required = text
            .len()
            .checked_add(NOTE.len())
            .ok_or_else(|| transport_error(TokenError::IdExhausted))?;
        lease
            .resize(required.max(text.capacity()) as u64)
            .map_err(|error| transport_error(error.into()))?;
        text.try_reserve_exact(NOTE.len())
            .map_err(|_| transport_error(TokenError::Budget(budget.poison_allocation())))?;
        lease
            .resize(text.capacity() as u64)
            .map_err(|error| transport_error(error.into()))?;
        text.push_str(NOTE);
    }
    budget
        .charge(
            crate::compiler::expansion_budget::Counter::DiagnosticBytes,
            text.capacity() as u64,
        )
        .map_err(|error| transport_error(error.into()))?;
    Ok((primary, line, column, text, lease))
}
fn compact_position(line: usize, column: usize) -> Result<(u32, u32), ParseError> {
    let out_of_range = |_| {
        simple_error(
            SourceErrorKind::Limit,
            "parser diagnostic coordinate exceeds u32 range",
        )
    };
    Ok((
        u32::try_from(line).map_err(out_of_range)?,
        u32::try_from(column).map_err(out_of_range)?,
    ))
}
fn simple_error(kind: SourceErrorKind, message: &str) -> ParseError {
    ParseError {
        kind,
        has_location: false,
        bytes: 0..0,
        line: 1,
        column: 1,
        message: message.into(),
        _diagnostic_lease: None,
        primary: None,
    }
}
fn event_error(event: &LexedEvent, kind: SourceErrorKind, message: &str) -> ParseError {
    ParseError {
        kind,
        has_location: true,
        bytes: event.prepared.clone(),
        line: event.line,
        column: event.column,
        message: message.into(),
        _diagnostic_lease: None,
        primary: None,
    }
}
pub(super) fn source_error(error: SourceError) -> ParseError {
    ParseError {
        kind: error.kind,
        has_location: error.primary.is_some(),
        bytes: 0..0,
        line: 1,
        column: 1,
        message: error.message,
        _diagnostic_lease: error._diagnostic_lease,
        primary: error.primary,
    }
}
pub(super) fn scope_error(error: crate::compiler::semantic_scope::ScopeError) -> ParseError {
    source_error(error.source_error())
}

pub(super) fn transport_error(error: TokenError) -> ParseError {
    source_error(SourceError::from_token(error))
}
pub(super) fn lex_error(error: LexError) -> ParseError {
    match error {
        LexError::Source {
            kind,
            prepared,
            line,
            column,
            message,
        } => ParseError {
            kind,
            has_location: true,
            bytes: prepared,
            line,
            column,
            message: message.into(),
            _diagnostic_lease: None,
            primary: None,
        },
        LexError::Projection(error) => source_error(error),
        LexError::Transport(error) => transport_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionBudget;
    use crate::compiler::source::{OriginSegment, SourceOptions, SourceSpan, prepare_text};
    use std::path::Path;

    fn constituent_fixture() -> (NativeParserInput, [TokenPart; 3]) {
        let input = super::super::lex(">>=").unwrap();
        let spans = &input[0].constituents;
        let parts = [
            TokenPart::new(spans.get(0).unwrap(), 0..1).unwrap(),
            TokenPart::new(spans.get(1).unwrap(), 1..2).unwrap(),
            TokenPart::new(spans.get(2).unwrap(), 2..3).unwrap(),
        ];
        (input, parts)
    }

    #[test]
    fn compact_constituents_keep_every_original_and_share_metered_tail() {
        let (input, parts) = constituent_fixture();
        let budget = input.session.budget();
        let before = budget.snapshot().retained_bytes;
        let one = TokenConstituents::new(budget, &parts[..1], parts[0].span).unwrap();
        assert_eq!(one.get(0), Some(parts[0].span));
        assert_eq!(one.get(1), None);
        assert_eq!(budget.snapshot().retained_bytes, before);
        let all = TokenConstituents::new(budget, &parts, parts[0].span).unwrap();
        let fixed =
            (std::mem::size_of::<ConstituentTail>() + 2 * std::mem::size_of::<usize>()) as u64;
        assert_eq!(budget.snapshot().retained_bytes - before, fixed);
        for (index, part) in parts.iter().enumerate() {
            assert_eq!(all.get(index), Some(part.span));
        }
        assert_eq!(all.get(3), None);
        let shared = all.clone();
        assert!(std::sync::Arc::ptr_eq(
            all.tail.as_ref().unwrap(),
            shared.tail.as_ref().unwrap()
        ));
        assert_eq!(budget.snapshot().retained_bytes - before, fixed);
        drop(all);
        assert_eq!(budget.snapshot().retained_bytes - before, fixed);
        drop(shared);
        assert_eq!(budget.snapshot().retained_bytes, before);
    }

    #[test]
    fn compact_constituent_tail_denies_before_allocation_and_latches() {
        let (_input, parts) = constituent_fixture();
        let fixed =
            (std::mem::size_of::<ConstituentTail>() + 2 * std::mem::size_of::<usize>()) as u64;
        let budget = ExpansionBudget::new(crate::compiler::expansion_budget::ExpansionLimits {
            bytes: fixed - 1,
            ..crate::compiler::expansion_budget::ExpansionLimits::default()
        });
        let first = TokenConstituents::new(&budget, &parts, parts[0].span).unwrap_err();
        assert_eq!(first.kind, SourceErrorKind::Limit);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        let again = TokenConstituents::new(&budget, &parts, parts[0].span).unwrap_err();
        assert_eq!(first.message, again.message);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn compact_parser_coordinates_are_checked_and_lossless() {
        let maximum = u32::MAX as usize;
        for (line, column) in [(1, 1), (65537, 40001), (maximum, maximum)] {
            let (retained_line, retained_column) = compact_position(line, column).unwrap();
            assert_eq!(
                (retained_line as usize, retained_column as usize),
                (line, column)
            );
        }
        if let Some(overflow) = maximum.checked_add(1) {
            for (line, column) in [(overflow, 1), (1, overflow)] {
                let error = compact_position(line, column).unwrap_err();
                assert_eq!(error.kind, SourceErrorKind::Limit);
                assert!(error.message.contains("diagnostic coordinate exceeds u32"));
            }
        }
    }

    fn inactive_input(text: &str) -> Result<NativeParserInput, ParseError> {
        let budget = ExpansionBudget::default();
        let projection = PreparedLexInput::identity(
            Path::new("inactive.rs"),
            text,
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .map_err(source_error)?;
        let (mut session, session_cap) =
            TokenSession::standalone(projection.snapshot().clone(), budget)
                .map_err(transport_error)?;
        let lexed = crate::compiler::lexer::lex_prepared(
            &projection,
            crate::compiler::token::Edition::E2024,
            &mut session,
        )
        .map_err(lex_error)?;
        NativeParserInput::new_inactive(lexed, session, session_cap, &projection)
    }

    #[test]
    fn inactive_input_retains_undefined_macro_tokens_without_semantic_literal_or_name_admission() {
        let text =
            "unknown!(r#raw, 水, 'x', b'x', b\"bytes\", r\"raw\", c\"C\", @, $, \"a\\\nb\");";
        assert_eq!(
            super::super::lex(text).unwrap_err().kind,
            SourceErrorKind::Unsupported
        );
        let input = inactive_input(text).unwrap();
        assert!(input.macro_source.is_some());
        let (buffer, session, _session_cap) = input.into_parts();
        assert!(buffer.current_macro_slice().is_none());
        let stream = buffer.macro_source().unwrap();
        let roots = stream.roots(&session).unwrap();
        assert!(!stream.children(&session, &roots).unwrap().is_empty());
        let mut count = 0;
        let mut cursor = super::super::TokenCursor::new(&buffer);
        while !cursor.current().matches("") {
            let view = cursor.current();
            if view.is_deferred_unsupported() {
                count += 1;
                assert!(!view.matches(view.text));
                assert!(!view.is_ident());
                assert!(!view.string);
                assert!(view.name.is_none());
            }
            cursor.bump();
        }
        // Char and byte now have native semantic tokens rather than deferred syntax.
        assert!(count >= 3);
        assert!(
            buffer
                .iter()
                .any(|token| token.kind == ParserTokenKind::Char)
        );
        assert!(
            buffer
                .iter()
                .any(|token| token.kind == ParserTokenKind::Byte)
        );
    }

    #[test]
    fn inactive_transport_still_checks_complete_lexical_input_and_doc_audit() {
        assert_eq!(
            inactive_input("unknown!(\"bad\\q\");").unwrap_err().kind,
            SourceErrorKind::Syntax
        );
        assert_eq!(
            inactive_input("unknown!(\"unterminated);")
                .unwrap_err()
                .kind,
            SourceErrorKind::Syntax
        );
        let input = inactive_input("/// doc\nunknown!();").unwrap();
        let (buffer, session, _session_cap) = input.into_parts();
        let stream = buffer
            .macro_source()
            .expect("doc transport keeps actual macro source");
        let roots = stream.roots(&session).unwrap();
        let mut seen = Vec::new();
        tree_texts(&session, stream, &roots, &mut seen);
        assert!(seen.iter().any(|token| token == "doc"), "{seen:?}");
        assert!(seen.iter().any(|token| token.contains(" doc")), "{seen:?}");
        assert_eq!(
            inactive_input("/// doc\nunknown!(\"bad\\q\");")
                .unwrap_err()
                .kind,
            SourceErrorKind::Syntax
        );
    }

    #[test]
    fn inactive_character_is_not_a_lifetime_and_keeps_checked_original_span() {
        let text = "fn unused() { let value = 'x'; }";
        let start = text.find("'x'").unwrap();
        let input = inactive_input(text).unwrap();
        let (buffer, session, _session_cap) = input.into_parts();
        let mut cursor = super::super::TokenCursor::new(&buffer);
        while !cursor.current().is_char() {
            assert!(!cursor.current().matches(""));
            cursor.bump();
        }
        let character = cursor.current();
        assert_eq!(character.text, "'x'");
        assert!(!character.is_lifetime());
        assert!(character.name.is_none());
        assert_eq!(character.bytes, start..start + 3);
        let checked = original(&session, character.span).unwrap();
        assert_eq!(checked.lo as usize, start);
        assert_eq!(checked.hi as usize, start + 3);
        super::super::validate_inactive_syntax(text).unwrap();
        let lifetime = super::super::lex("'label").unwrap();
        assert!(
            super::super::TokenCursor::new(&lifetime)
                .current()
                .is_lifetime()
        );
    }

    #[test]
    fn empty_expansion_proof_storage_cannot_issue_buffer_authority() {
        let budget = ExpansionBudget::default();
        let proof = ExpansionProof {
            values: BudgetVec::new(&budget, Storage::Retained).unwrap(),
        };
        assert_eq!(proof.checked().unwrap_err().kind, SourceErrorKind::Contract);
    }

    #[test]
    fn discarded_string_suffix_is_transport_only_and_consumption_still_fails() {
        let discarded =
            "fn main(){macro_rules! discard{($x:literal)=>{0};}let _=discard!(\"ok\"wat);}";
        let input = super::super::lex(discarded).unwrap();
        let deferred = input
            .tokens
            .as_slice()
            .iter()
            .find(|token| token.kind == ParserTokenKind::DeferredLiteral)
            .unwrap();
        assert_eq!(deferred.text, "\"ok\"wat");
        assert!(!deferred.string);
        super::super::parse_syntax_unprepared(discarded).unwrap();
        for source in [
            "fn main(){println!(\"bad\"wat);}",
            "fn main(){macro_rules! discard{($x:literal)=>{0};}println!(\"bad\"wat);}",
            "fn main(){macro_rules! forward{($x:literal)=>{$x};}let _=forward!(\"bad\"wat);}",
            "fn main(){macro_rules! emit{()=>{\"bad\"wat};}let _=emit!();}",
        ] {
            let error = super::super::parse_syntax_unprepared(source).err().unwrap();
            assert_eq!(error.kind, SourceErrorKind::Syntax, "{source}: {error}");
        }
    }

    #[test]
    fn matcher_only_punctuation_is_deferred_without_semantic_admission() {
        let countdown = "fn main(){macro_rules! walk{()=>{40};(@ $($rest:tt)*)=>{walk!($($rest)*)+1};}if walk!(@ @ @)==43{println!(\"ok\");}}";
        let input = super::super::lex(countdown).unwrap();
        assert!(
            input
                .tokens
                .as_slice()
                .iter()
                .any(|token| token.kind == ParserTokenKind::Punct && token.text == "@")
        );
        super::super::parse_syntax_unprepared(countdown).unwrap();
        for source in [
            countdown,
            "/// documented\nfn value()->i32{42}fn main(){println!(\"{}\",value());}",
            "fn main(){let x=Some(42);match x{whole @ Some(_)=>{match whole{Some(n)=>println!(\"{}\",n),None=>{}}},None=>{}}}",
            "fn value()->Result<i32,i32>{let x:Result<i32,i32>=Ok(42);Ok(x?)}fn main(){match value(){Ok(n)=>println!(\"{}\",n),Err(_)=>{}}}",
        ] {
            super::super::parser_tests::stock_native_run(source);
        }
        // Labels are hygienic: fragment-passed labels keep the caller's
        // context, while labels written in a macro body stay expansion-local.
        for source in [
            "fn main(){macro_rules! stop{($label:lifetime)=>{break $label};}'done:loop{stop!('done);}println!(\"{}\",42);}",
            "fn main(){macro_rules! next{($l:lifetime)=>{continue $l};}let mut n=0;'outer:for i in 0..4{loop{n+=i;next!('outer);}}println!(\"{}\",n);}",
            "fn main(){macro_rules! inner{()=>{{let mut n=0;'l:loop{n+=1;if n==3{break 'l;}}n}};}let v=inner!();'l:loop{break 'l;}println!(\"{}\",v);}",
            "fn main(){macro_rules! a{($l:lifetime)=>{b!($l)};}macro_rules! b{($m:lifetime)=>{break $m 7};}let v='x:loop{a!('x);};println!(\"{}\",v);}",
        ] {
            super::super::parser_tests::stock_native_run(source);
        }
        for source in [
            "macro_rules! m{()=>{'l:loop{break 'l;}};}fn unused(){m!();break 'l;}fn main(){}",
            "macro_rules! m{()=>{break 'l};}fn unused(){'l:loop{m!();}}fn main(){}",
        ] {
            super::super::parser_tests::stock_native_reject_e0426(source);
        }
        for punctuation in ["@", "?"] {
            let input = super::super::lex(punctuation).unwrap();
            assert_eq!(input.tokens.as_slice()[0].kind, ParserTokenKind::Punct);
            assert_eq!(input.tokens.as_slice()[0].text, punctuation);
        }
        for punctuation in ["#", "~", "$"] {
            assert_eq!(
                super::super::lex(punctuation).unwrap_err().kind,
                SourceErrorKind::Unsupported
            );
        }
        for source in [
            "fn main(){let _=@;}",
            "fn main(){macro_rules! discard{($x:tt)=>{0};}let _=@;}",
            "fn main(){macro_rules! emit{()=>{@};}let _=emit!();}",
        ] {
            let stock = super::super::parser_tests::stock_compile(
                source,
                std::path::Path::new("unused"),
                true,
            );
            assert!(!stock.status.success(), "stock accepted {source}");
            assert!(String::from_utf8_lossy(&stock.stderr).contains("error:"));
            let error = super::super::parse_syntax_unprepared(source).err().unwrap();
            assert_eq!(error.kind, SourceErrorKind::Syntax, "{source}: {error}");
        }
    }

    #[test]
    fn joint_less_minus_is_transportable_but_not_an_expression() {
        let direct = "fn main(){let x=0;let _=x<-1;}";
        let input = super::super::lex(direct).unwrap();
        assert!(
            input
                .tokens
                .as_slice()
                .iter()
                .any(|token| token.text == "<-")
        );
        let error = super::super::parse_syntax_unprepared(direct).err().unwrap();
        assert_eq!(error.kind, SourceErrorKind::Syntax);
        super::super::parse_syntax_unprepared("fn main(){let x=0;let _=x < -1;}").unwrap();

        // A joint operator is still a legal token-tree matcher token.
        let input = super::super::lex("fn main(){macro_rules! fixed{(<-)=>{0};}let _=fixed!(<-);}")
            .unwrap();
        assert!(input.macro_source.is_some());
        assert!(
            input
                .tokens
                .as_slice()
                .iter()
                .any(|token| token.text == "<-")
        );
    }

    fn split_source(text: &str, boundary: usize) -> crate::compiler::source::PreparedSource {
        let mut source = prepare_text("split.rs", text, &SourceOptions::default()).unwrap();
        let origin = source.origins[0].origin;
        source.origins = vec![
            OriginSegment {
                prepared: 0..boundary,
                origin: SourceSpan {
                    hi: boundary as u32,
                    ..origin
                },
                synthetic: false,
            },
            OriginSegment {
                prepared: boundary..text.len(),
                origin: SourceSpan {
                    lo: boundary as u32,
                    ..origin
                },
                synthetic: false,
            },
        ];
        source
    }

    #[test]
    fn logical_operators_and_lifetimes_reject_atom_only_mapping() {
        for (text, boundary) in [(">>=", 1), ("&&", 1), ("'region", 1)] {
            let source = split_source(text, boundary);
            let budget = ExpansionBudget::default();
            let projection = PreparedLexInput::prepared(
                &source,
                &budget,
                crate::compiler::owner_path::standalone_issued(
                    &budget,
                    crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
                )
                .unwrap(),
            )
            .unwrap();
            let (mut session, session_cap) =
                TokenSession::standalone(projection.snapshot().clone(), budget).unwrap();
            let lexed = crate::compiler::lexer::lex_prepared(
                &projection,
                crate::compiler::token::Edition::E2024,
                &mut session,
            )
            .unwrap();
            assert!(
                lexed
                    .events()
                    .iter()
                    .all(|event| projection.project(event.prepared.clone()).is_ok())
            );
            let error =
                NativeParserInput::new(lexed, session, session_cap, &projection).unwrap_err();
            assert_eq!(error.kind, SourceErrorKind::Contract, "{text}: {error}");
            assert!(error.message.contains("segment boundary"));
        }
    }

    #[test]
    fn canonical_float_requires_whole_mapping_before_tuple_projection() {
        let source = split_source("t.0.1", 3);
        let budget = ExpansionBudget::default();
        let projection = PreparedLexInput::prepared(
            &source,
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let error = super::super::native_input(
            &projection,
            budget.clone(),
            super::super::super::owner_path::standalone_issued(
                &budget,
                super::super::super::owner_path::OwnerDomain::TokenSession,
            )
            .unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Contract);
        for text in ["t.0.1", "t. 0.1", "t./* comment */0.1"] {
            let input = super::super::lex(text).unwrap();
            assert_eq!(
                input
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["t", ".", "0", ".", "1", ""]
            );
        }
    }

    #[test]
    fn native_input_rejects_foreign_session_and_view_without_adoption() {
        let budget = ExpansionBudget::default();
        let projection = PreparedLexInput::identity(
            Path::new("a.rs"),
            "name",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let (mut session, session_cap) =
            TokenSession::standalone(projection.snapshot().clone(), budget.clone()).unwrap();
        let lexed = crate::compiler::lexer::lex_prepared(
            &projection,
            crate::compiler::token::Edition::E2024,
            &mut session,
        )
        .unwrap();
        let (wrong, wrong_cap) =
            TokenSession::standalone(projection.snapshot().clone(), budget.clone()).unwrap();
        assert_eq!(
            NativeParserInput::new(lexed, wrong, wrong_cap, &projection)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        let lexed = crate::compiler::lexer::lex_prepared(
            &projection,
            crate::compiler::token::Edition::E2024,
            &mut session,
        )
        .unwrap();
        let other = PreparedLexInput::identity(
            Path::new("a.rs"),
            "name",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            NativeParserInput::new(lexed, session, session_cap, &other)
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
    }

    #[test]
    fn constituent_and_suffix_spans_retain_original_unicode_crlf_coordinates() {
        let input = {
            let text = String::from("/* 水 */\r\n  >>= &&");
            super::super::lex(&text).unwrap()
        };
        let token = &input[0];
        for (part, expected) in token.constituents.iter().zip([13..14, 14..15, 15..16]) {
            let original = original(&input.session, *part).unwrap();
            assert_eq!((original.lo as usize..original.hi as usize), expected);
        }
        let mut cursor = super::super::TokenCursor::new(&input);
        assert!(cursor.consume_type_gt());
        let suffix = cursor.current();
        assert_eq!(suffix.bytes, 14..16);
        let projected = original(&input.session, suffix.span).unwrap();
        assert_eq!((projected.lo, projected.hi), (14, 16));
        assert!(cursor.consume_type_gt());
        assert_eq!(
            original(&input.session, cursor.current().span).unwrap().lo,
            15
        );
        cursor.bump();
        assert!(cursor.consume_type_amp_prefix());
        assert_eq!(
            original(&input.session, cursor.current().span).unwrap().lo,
            18
        );
    }

    #[test]
    fn native_leaf_identity_is_retained_and_compact_ranges_remain_checked() {
        let budget = ExpansionBudget::default();
        let projection = PreparedLexInput::identity(
            Path::new("single.rs"),
            "name",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let (mut session, session_cap) =
            TokenSession::standalone(projection.snapshot().clone(), budget).unwrap();
        let lexed = crate::compiler::lexer::lex_prepared(
            &projection,
            crate::compiler::token::Edition::E2024,
            &mut session,
        )
        .unwrap();
        let span = lexed.events()[0].token.span();
        let eof = lexed.eof().span;
        let input = NativeParserInput::new(lexed, session, session_cap, &projection).unwrap();
        assert_eq!(input[0].whole.span, span);
        assert_eq!(input[0].constituents.get(0).unwrap(), span);
        assert_eq!(input.last().unwrap().whole.span, eof);
        assert_eq!(
            TokenPart::new(span, Range { start: 1, end: 0 })
                .unwrap_err()
                .kind,
            SourceErrorKind::Contract
        );
        assert_eq!(
            TokenPart::new(span, 0..usize::MAX).unwrap_err().kind,
            SourceErrorKind::Contract
        );
    }

    #[test]
    fn nonroot_identifiers_literals_and_punctuation_are_not_semantic_input() {
        use crate::compiler::expansion_span::ExpansionRecord;
        use crate::compiler::hygiene::Transparency;
        use crate::compiler::token::{Edition, Ident, LiteralKind};
        let mut input = super::super::lex("name").unwrap();
        let original = input[0].whole.span;
        let file = input.session.sources().file(0).unwrap();
        let krate = input.session.register_crate_root(file).unwrap();
        let expansion = input
            .session
            .add_expansion(ExpansionRecord {
                definition: original,
                call: original,
                parent: None,
                defining_crate: krate,
                edition: Edition::E2024,
            })
            .unwrap();
        let context = input
            .session
            .apply_mark(
                input.session.root_context(),
                expansion,
                Transparency::Opaque,
            )
            .unwrap();
        let span = input
            .session
            .with_context(original, context, Some(expansion))
            .unwrap();
        let symbol = input.session.intern_symbol("name").unwrap();
        for kind in [
            TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context,
            }),
            TokenKind::Literal {
                kind: LiteralKind::String,
                spelling: symbol,
            },
            TokenKind::Punct {
                ch: '+',
                spacing: Spacing::Alone,
            },
        ] {
            let event = LexedEvent {
                token: InputToken::Leaf(crate::compiler::token::Token { kind, span }),
                prepared: 0..4,
                line: 1,
                column: 1,
                literal: None,
            };
            assert_eq!(
                input.check_event(&event).unwrap_err().kind,
                SourceErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn richer_lexical_classes_are_rejected_before_string_named_ast_lowering() {
        let input = super::super::lex("b'x'").unwrap();
        let view = super::super::TokenCursor::new(&input).current();
        assert!(view.is_byte());
        assert_eq!(view.text, "b'x'");
        assert!(!view.is_char());
        assert!(!view.matches("b'x'"));
        assert!(view.name.is_none());
        let raw = super::super::lex("r#name").unwrap();
        let raw_view = super::super::TokenCursor::new(&raw).current();
        assert!(raw_view.is_ident());
        assert_eq!(raw_view.text, "r#name");
        assert!(raw_view.name.is_some());
        for text in [
            "\u{feff}fn f(){}",
            "#!/bin/native\nfn f(){}",
            "水",
            "c\"x\"",
        ] {
            assert_eq!(
                super::super::lex(text).unwrap_err().kind,
                SourceErrorKind::Unsupported,
                "{text:?}"
            );
        }
        let input = super::super::lex("\"\" \"fn\" \"&&\"").unwrap();
        let mut cursor = super::super::TokenCursor::new(&input);
        assert!(!cursor.current().matches(""));
        cursor.bump();
        assert!(!cursor.current().matches("fn"));
        cursor.bump();
        assert!(!cursor.consume_type_amp_prefix());
        cursor.bump();
        assert!(cursor.current().matches(""));
    }

    #[test]
    fn macro_inventory_is_typed_optional_and_dollar_transport_is_conditional() {
        let ordinary = super::super::lex("fn main(){}").unwrap();
        assert!(ordinary.macro_source.is_none());
        let quoted = super::super::lex("\"macro_rules!\"").unwrap();
        assert!(quoted.macro_source.is_none());
        assert_eq!(
            super::super::lex("$").unwrap_err().kind,
            SourceErrorKind::Unsupported
        );
        let input = super::super::lex("macro_rules! m { ($x:ident)=>{$x}; }").unwrap();
        assert!(input.macro_source.is_some());
        assert!(input.iter().any(|token| token.text == "$"));
        let (buffer, session, _session_cap) = input.into_parts();
        let stream = buffer.macro_source().unwrap();
        let roots = stream.roots(&session).unwrap();
        assert!(!stream.children(&session, &roots).unwrap().is_empty());
    }

    fn tree_texts(
        session: &crate::compiler::token_session::TokenSession,
        stream: &crate::compiler::token_tree::TokenStream,
        range: &crate::compiler::token_tree::TreeRange,
        out: &mut Vec<String>,
    ) {
        for child in stream.children(session, range).unwrap() {
            match stream.node(session, *child).unwrap() {
                crate::compiler::token_tree::Node::Leaf(token) => match token.kind {
                    crate::compiler::token::TokenKind::Ident(ident) => {
                        out.push(session.symbol(ident.symbol).unwrap().to_string())
                    }
                    crate::compiler::token::TokenKind::Literal { spelling, .. } => {
                        out.push(session.symbol(spelling).unwrap().to_string())
                    }
                    crate::compiler::token::TokenKind::Punct { ch, .. } => out.push(ch.to_string()),
                    crate::compiler::token::TokenKind::Opaque(_) => out.push("<opaque>".into()),
                    crate::compiler::token::TokenKind::DollarCrate(_) => out.push("$crate".into()),
                },
                crate::compiler::token_tree::Node::Delimited { children, .. } => {
                    tree_texts(session, stream, children, out)
                }
            }
        }
    }

    #[test]
    fn macro_inventory_lowers_doc_comments_into_macro_source() {
        for text in [
            "/// audit\nmacro_rules! m {()=>{1};}",
            "macro_rules! m {()=>{/** audit */1};}",
        ] {
            let input = super::super::lex(text).unwrap();
            let (buffer, session, _session_cap) = input.into_parts();
            let stream = buffer.macro_source().unwrap();
            let roots = stream.roots(&session).unwrap();
            let mut seen = Vec::new();
            tree_texts(&session, stream, &roots, &mut seen);
            assert!(seen.iter().any(|token| token == "doc"), "{seen:?}");
            assert!(seen.iter().any(|token| token.contains("audit")), "{seen:?}");
        }
        assert!(super::super::lex("/// audit\nfn main(){}").is_ok());
        // Complete lexical validation precedes the legal unsupported doc gate.
        assert_eq!(
            super::super::lex("macro_rules! m {()=>{1};} /// audit\n\"unterminated")
                .unwrap_err()
                .kind,
            SourceErrorKind::Syntax
        );
    }
}
