//! Checked, root-context semantic names owned by one parsed program.
//!
//! This is canonical name storage, not a spelling-based lookup side table.
//! Keyword admission, typed path anchors and lexical scope belong to the parser
//! and resolver. Nonroot expansion input remains explicitly unsupported here.
use std::fmt;
use std::mem::size_of;
use std::sync::Arc;

use super::expansion_budget::{BudgetFailure, ByteLease, Counter, ExpansionBudget, Storage};
use super::token::{Edition, Token, TokenKind};
use super::token_session::{
    BudgetVec, SessionId, SpanId, SymbolId, SyntaxContextId, TokenError, TokenSession,
};

/// A checked occurrence in a single program's append-only canonical name arena.
/// It intentionally has no semantic Eq/Hash implementation. Use NameKey instead.
#[derive(Clone, Copy, Debug)]
pub struct AstIdent {
    owner: SessionId,
    index: u32,
}

/// Canonical identifier atom. ASCII spelling is already normalized. Unicode
/// admission requires bounded NFC normalization before constructing this atom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameAtomId(SymbolId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Namespace {
    Value,
    Type,
    Macro,
}

/// Closed static catalog atoms. This is descriptor binding, not a source
/// occurrence factory or a generic spelling-based resolver fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinAtom {
    Core,
    Std,
    Marker,
    CloneModule,
    Ops,
    Sized,
    Copy,
    Clone,
    Drop,
    EntryMain,
    CloneMethod,
    CloneFromMethod,
    DropMethod,
}
impl BuiltinAtom {
    fn spelling(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Std => "std",
            Self::Marker => "marker",
            Self::CloneModule => "clone",
            Self::Ops => "ops",
            Self::Sized => "Sized",
            Self::Copy => "Copy",
            Self::Clone => "Clone",
            Self::Drop => "Drop",
            Self::EntryMain => "main",
            Self::CloneMethod => "clone",
            Self::CloneFromMethod => "clone_from",
            Self::DropMethod => "drop",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookupClass {
    Local,
    Label,
    Item(Namespace),
    TypeParameter,
    LifetimeParameter,
    Member,
}

/// Root-only semantic equality. Scope/module identity is supplied separately by
/// the resolver. Nonroot contexts must never be projected to this root key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameKey {
    atom: NameAtomId,
    context: SyntaxContextId,
    class: LookupClass,
}
impl NameKey {
    pub fn class(self) -> LookupClass {
        self.class
    }
}

/// Read-only occurrence metadata, not a constructor or publication capability.
#[derive(Clone, Copy, Debug)]
pub struct NameData {
    pub atom: NameAtomId,
    pub spelling: SymbolId,
    pub context: SyntaxContextId,
    pub span: SpanId,
    pub raw: bool,
    pub edition: Edition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedName {
    NonRootContext,
    ExpandedOccurrence,
    RawIdentifier,
    UnicodeIdentifier,
}

#[derive(Debug)]
pub enum NameError {
    Token(TokenError),
    ExpectedIdentifier,
    InvalidIdentifier,
    Unsupported(UnsupportedName),
}
impl From<TokenError> for NameError {
    fn from(error: TokenError) -> Self {
        Self::Token(error)
    }
}
impl From<BudgetFailure> for NameError {
    fn from(error: BudgetFailure) -> Self {
        Self::Token(TokenError::Budget(error))
    }
}
impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(error) => write!(f, "semantic name: {error}"),
            Self::ExpectedIdentifier => f.write_str("expected identifier token"),
            Self::InvalidIdentifier => f.write_str("expected a non-wildcard ASCII identifier"),
            Self::Unsupported(reason) => write!(f, "unsupported semantic name: {reason:?}"),
        }
    }
}
impl std::error::Error for NameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Token(error) => Some(error),
            _ => None,
        }
    }
}

/// Mutable construction is separated from the frozen owner. The parser must
/// borrow independent token storage, not an input containing this same session.
#[derive(Debug)]
pub struct NameStoreBuilder {
    store: NameStore,
}

/// Shared lifetime owner for every name used by the AST, resolver and replay.
/// Records contain only flat checked handles, so destruction is nonrecursive.
pub struct NameStore {
    session: TokenSession,
    unit_edition: Edition,
    records: BudgetVec<NameData>,
    // Last field releases fixed wrapper/Arc accounting after owned payloads.
    _lease: ByteLease,
}
impl fmt::Debug for NameStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NameStore")
            .field("owner", &self.session.id())
            .field("edition", &self.unit_edition)
            .field("occurrences", &self.records.len())
            .finish()
    }
}

impl NameStoreBuilder {
    /// Closed catalog member identity, never a source occurrence or provenance.
    pub fn catalog_member_key(&mut self, atom: BuiltinAtom) -> Result<NameKey, NameError> {
        if !matches!(
            atom,
            BuiltinAtom::CloneMethod | BuiltinAtom::CloneFromMethod | BuiltinAtom::DropMethod
        ) {
            return Err(
                TokenError::InvalidInput("expected a closed catalog member descriptor").into(),
            );
        }
        let symbol = self.store.session.intern_symbol(atom.spelling())?;
        Ok(NameKey {
            atom: NameAtomId(symbol),
            context: self.session().root_context(),
            class: LookupClass::Member,
        })
    }
    /// Bind a closed catalog descriptor before freeze. No source occurrence,
    /// synthetic span, or AST name is created. The caller must still select a
    /// checked module/scope and bind this key to its canonical declaration.
    pub fn catalog_key(
        &mut self,
        atom: BuiltinAtom,
        namespace: Namespace,
    ) -> Result<NameKey, NameError> {
        let symbol = self.store.session.intern_symbol(atom.spelling())?;
        Ok(NameKey {
            atom: NameAtomId(symbol),
            context: self.session().root_context(),
            class: LookupClass::Item(namespace),
        })
    }

    pub fn new(session: TokenSession, unit_edition: Edition) -> Result<Self, NameError> {
        let budget = session.budget().clone();
        budget.check()?;
        // TokenSession already accounts its own inline size. Stable std cannot
        // fallibly allocate the small fixed Arc control block used at freeze.
        let overhead = size_of::<NameStore>()
            .checked_sub(size_of::<TokenSession>())
            .and_then(|bytes| bytes.checked_add(2 * size_of::<usize>()))
            .ok_or(TokenError::IdExhausted)?;
        let lease = budget.reserve(Storage::Retained, overhead as u64)?;
        let records = BudgetVec::new(&budget, Storage::Retained)?;
        Ok(Self {
            store: NameStore {
                session,
                unit_edition,
                records,
                _lease: lease,
            },
        })
    }

    /// Accepts an identifier leaf, never a joined textual lifetime or path.
    /// The parser separately applies raw/edition-sensitive keyword grammar.
    pub fn from_token(
        &mut self,
        token: Token,
        lexical_edition: Edition,
    ) -> Result<AstIdent, NameError> {
        self.budget().check()?;
        token.validate(self.session())?;
        let TokenKind::Ident(ident) = token.kind else {
            return Err(NameError::ExpectedIdentifier);
        };
        self.check_origin(token.span)?;
        if ident.raw {
            return Err(NameError::Unsupported(UnsupportedName::RawIdentifier));
        }
        Self::check_ascii(self.session().symbol(ident.symbol)?, self.budget())?;
        self.append(NameData {
            atom: NameAtomId(ident.symbol),
            spelling: ident.symbol,
            context: ident.context,
            span: token.span,
            raw: ident.raw,
            edition: lexical_edition,
        })
    }

    /// Structural construction for an explicit owned fixture/program builder.
    /// The origin must be issued from its real in-memory source session. This
    /// does not grant parser admission or canonical body/metadata authority.
    pub fn intern_root(&mut self, text: &str, origin: SpanId) -> Result<AstIdent, NameError> {
        self.check_origin(origin)?;
        Self::check_ascii(text, self.budget())?;
        let spelling = self.store.session.intern_symbol(text)?;
        self.append(NameData {
            atom: NameAtomId(spelling),
            spelling,
            context: self.session().root_context(),
            span: origin,
            raw: false,
            edition: self.edition(),
        })
    }

    fn check_ascii(text: &str, budget: &ExpansionBudget) -> Result<(), NameError> {
        let work = (text.len() as u64)
            .checked_mul(2)
            .and_then(|work| work.checked_add(1))
            .ok_or(TokenError::IdExhausted)?;
        budget.charge(Counter::Work, work)?;
        if !text.is_ascii() {
            return Err(NameError::Unsupported(UnsupportedName::UnicodeIdentifier));
        }
        let mut bytes = text.bytes();
        let valid_start = bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_');
        if text == "_"
            || !valid_start
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(NameError::InvalidIdentifier);
        }
        Ok(())
    }

    fn check_origin(&self, span: SpanId) -> Result<(), NameError> {
        self.budget().charge(Counter::Work, 1)?;
        let data = self.session().span(span)?;
        if data.context != self.session().root_context() {
            return Err(NameError::Unsupported(UnsupportedName::NonRootContext));
        }
        if data.expansion.is_some() {
            return Err(NameError::Unsupported(UnsupportedName::ExpandedOccurrence));
        }
        Ok(())
    }

    fn append(&mut self, data: NameData) -> Result<AstIdent, NameError> {
        // One write plus geometric backing relocation has amortized cost <=3
        // per admitted occurrence. Failed admission never refunds this work.
        self.budget().charge(Counter::Work, 3)?;
        let index = self.budget().checked_u32(self.store.records.len() as u64)?;
        self.store.records.push(data)?;
        Ok(AstIdent {
            owner: self.session().id(),
            index,
        })
    }

    pub fn freeze(self) -> Result<Arc<NameStore>, NameError> {
        self.budget().check()?;
        Ok(Arc::new(self.store))
    }

    pub fn check(&self, ident: AstIdent) -> Result<(), NameError> {
        self.store.check(ident)
    }
    pub fn key(&self, ident: AstIdent, class: LookupClass) -> Result<NameKey, NameError> {
        self.store.key(ident, class)
    }
    pub fn spelling(&self, ident: AstIdent) -> Result<&str, NameError> {
        self.store.spelling(ident)
    }
    pub fn atom_text(&self, atom: NameAtomId) -> Result<&str, NameError> {
        self.store.atom_text(atom)
    }
    pub fn span(&self, ident: AstIdent) -> Result<SpanId, NameError> {
        self.store.span(ident)
    }
    pub fn data(&self, ident: AstIdent) -> Result<NameData, NameError> {
        self.store.data(ident)
    }
    pub fn session(&self) -> &TokenSession {
        self.store.session()
    }
    pub fn budget(&self) -> &ExpansionBudget {
        self.store.budget()
    }
    pub fn edition(&self) -> Edition {
        self.store.edition()
    }
}

impl NameStore {
    fn record(&self, ident: AstIdent) -> Result<&NameData, NameError> {
        self.budget().charge(Counter::Work, 1)?;
        if ident.owner != self.session.id() {
            return Err(TokenError::ForeignHandle.into());
        }
        self.records
            .as_slice()
            .get(ident.index as usize)
            .ok_or_else(|| TokenError::InvalidHandle.into())
    }
    pub fn check(&self, ident: AstIdent) -> Result<(), NameError> {
        self.record(ident).map(|_| ())
    }
    pub fn key(&self, ident: AstIdent, class: LookupClass) -> Result<NameKey, NameError> {
        let record = self.record(ident)?;
        Ok(NameKey {
            atom: record.atom,
            context: record.context,
            class,
        })
    }
    pub fn spelling(&self, ident: AstIdent) -> Result<&str, NameError> {
        Ok(self.session.symbol(self.record(ident)?.spelling)?)
    }
    pub fn atom_text(&self, atom: NameAtomId) -> Result<&str, NameError> {
        self.budget().charge(Counter::Work, 1)?;
        Ok(self.session.symbol(atom.0)?)
    }
    pub fn span(&self, ident: AstIdent) -> Result<SpanId, NameError> {
        Ok(self.record(ident)?.span)
    }
    pub fn data(&self, ident: AstIdent) -> Result<NameData, NameError> {
        Ok(*self.record(ident)?)
    }
    pub fn session(&self) -> &TokenSession {
        &self.session
    }
    pub fn budget(&self) -> &ExpansionBudget {
        self.session.budget()
    }
    pub fn edition(&self) -> Edition {
        self.unit_edition
    }
}

#[cfg(test)]
mod tests {
    use super::super::expansion_budget::ExpansionLimits;
    use super::super::expansion_span::ExpansionRecord;
    use super::super::hygiene::Transparency;
    use super::super::token::{Ident, LiteralKind};
    use super::super::token_session::SourceSnapshot;
    use super::*;
    use std::path::Path;

    fn fixture(budget: ExpansionBudget) -> (TokenSession, Token, SpanId) {
        let source = SourceSnapshot::from_text(Path::new("fixture.rs"), "x\n", &budget).unwrap();
        let file = source.file(0).unwrap();
        let first = source.span(file, 0, 1).unwrap();
        let eof = source.span(file, 2, 2).unwrap();
        let mut session = TokenSession::new(source, budget).unwrap();
        let span = session.original_span(first).unwrap();
        let eof = session.original_span(eof).unwrap();
        let symbol = session.intern_symbol("x").unwrap();
        let token = Token {
            span,
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context: session.root_context(),
            }),
        };
        (session, token, eof)
    }

    #[test]
    fn catalog_descriptor_keys_are_owned_names_not_source_occurrences() {
        let (session, _, origin) = fixture(ExpansionBudget::default());
        let mut builder = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let count = builder.store.records.len();
        let catalog = builder
            .catalog_key(BuiltinAtom::Copy, Namespace::Type)
            .unwrap();
        assert_eq!(builder.store.records.len(), count);
        let source = builder.intern_root("Copy", origin).unwrap();
        assert_eq!(
            catalog,
            builder
                .key(source, LookupClass::Item(Namespace::Type))
                .unwrap()
        );
        assert_ne!(
            catalog,
            builder
                .catalog_key(BuiltinAtom::Copy, Namespace::Value)
                .unwrap()
        );
        let (foreign, _, _) = fixture(ExpansionBudget::default());
        let mut foreign = NameStoreBuilder::new(foreign, Edition::E2024).unwrap();
        assert_ne!(
            catalog,
            foreign
                .catalog_key(BuiltinAtom::Copy, Namespace::Type)
                .unwrap()
        );
        let frozen = builder.freeze().unwrap();
        assert_eq!(
            catalog,
            frozen
                .key(source, LookupClass::Item(Namespace::Type))
                .unwrap()
        );
    }

    #[test]
    fn compact_layout_is_measured_at_the_real_target_boundary() {
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<AstIdent>(), 16);
            assert_eq!(size_of::<NameData>(), 72);
            assert_eq!(size_of::<NameKey>(), 40);
        }
        assert!(size_of::<AstIdent>() <= size_of::<String>());
    }

    #[test]
    fn occurrence_spans_and_editions_do_not_change_semantic_keys() {
        let (session, token, eof) = fixture(ExpansionBudget::default());
        let mut builder = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let first = builder.from_token(token, Edition::E2018).unwrap();
        let second = builder.intern_root("x", eof).unwrap();
        assert_eq!(builder.spelling(first).unwrap(), "x");
        assert_eq!(builder.data(first).unwrap().edition, Edition::E2018);
        assert_eq!(builder.data(second).unwrap().edition, Edition::E2024);
        assert_ne!(builder.span(first).unwrap(), builder.span(second).unwrap());
        assert_eq!(
            builder.key(first, LookupClass::Local).unwrap(),
            builder.key(second, LookupClass::Local).unwrap()
        );
        assert_ne!(
            builder.key(first, LookupClass::Local).unwrap(),
            builder.key(first, LookupClass::Label).unwrap()
        );
        assert_ne!(
            builder
                .key(first, LookupClass::Item(Namespace::Value))
                .unwrap(),
            builder
                .key(first, LookupClass::Item(Namespace::Type))
                .unwrap()
        );
        let key = builder.key(first, LookupClass::Local).unwrap();
        let store = builder.freeze().unwrap();
        assert_eq!(store.key(first, LookupClass::Local).unwrap(), key);
        assert_eq!(
            store.atom_text(store.data(first).unwrap().atom).unwrap(),
            "x"
        );
        store.check(second).unwrap();
        assert_eq!(store.edition(), Edition::E2024);
    }

    #[test]
    fn foreign_occurrences_atoms_origins_and_tokens_are_rejected() {
        let (session_a, token_a, _) = fixture(ExpansionBudget::default());
        let (session_b, token_b, eof_b) = fixture(ExpansionBudget::default());
        let mut a = NameStoreBuilder::new(session_a, Edition::E2024).unwrap();
        let mut b = NameStoreBuilder::new(session_b, Edition::E2024).unwrap();
        let a_name = a.from_token(token_a, Edition::E2024).unwrap();
        let b_name = b.from_token(token_b, Edition::E2024).unwrap();
        assert!(matches!(
            a.check(b_name),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        assert!(matches!(
            a.atom_text(b.data(b_name).unwrap().atom),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        assert!(matches!(
            a.intern_root("x", eof_b),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        assert!(matches!(
            a.from_token(token_b, Edition::E2024),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        assert_ne!(
            a.key(a_name, LookupClass::Local).unwrap(),
            b.key(b_name, LookupClass::Local).unwrap()
        );
        let forged = AstIdent {
            owner: a.session().id(),
            index: u32::MAX,
        };
        assert!(matches!(
            a.check(forged),
            Err(NameError::Token(TokenError::InvalidHandle))
        ));
    }

    #[test]
    fn unsupported_names_remain_distinct_from_malformed_or_nonidentifier_tokens() {
        let (mut session, token, eof) = fixture(ExpansionBudget::default());
        let unicode = session.intern_symbol("α").unwrap();
        let mut builder = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let TokenKind::Ident(mut ident) = token.kind else {
            unreachable!()
        };
        ident.raw = true;
        assert!(matches!(
            builder.from_token(
                Token {
                    kind: TokenKind::Ident(ident),
                    ..token
                },
                Edition::E2024
            ),
            Err(NameError::Unsupported(UnsupportedName::RawIdentifier))
        ));
        ident.raw = false;
        ident.symbol = unicode;
        assert!(matches!(
            builder.from_token(
                Token {
                    kind: TokenKind::Ident(ident),
                    ..token
                },
                Edition::E2024
            ),
            Err(NameError::Unsupported(UnsupportedName::UnicodeIdentifier))
        ));
        assert!(matches!(
            builder.intern_root("α", eof),
            Err(NameError::Unsupported(UnsupportedName::UnicodeIdentifier))
        ));
        for text in ["", "_", "'static", "'_", "a::b", "1x", "x-y"] {
            assert!(matches!(
                builder.intern_root(text, eof),
                Err(NameError::InvalidIdentifier)
            ));
        }
        let literal = Token {
            kind: TokenKind::Literal {
                kind: LiteralKind::String,
                spelling: unicode,
            },
            ..token
        };
        assert!(matches!(
            builder.from_token(literal, Edition::E2024),
            Err(NameError::ExpectedIdentifier)
        ));
        let name = builder.from_token(token, Edition::E2024).unwrap();
        assert_eq!(builder.spelling(name).unwrap(), "x");
    }

    #[test]
    fn root_context_does_not_admit_expansion_occurrences_or_nonroot_marks() {
        let (mut session, token, _) = fixture(ExpansionBudget::default());
        let krate = session
            .register_crate_root(session.sources().file(0).unwrap())
            .unwrap();
        let expansion = session
            .add_expansion(ExpansionRecord {
                definition: token.span,
                call: token.span,
                parent: None,
                defining_crate: krate,
                edition: Edition::E2024,
            })
            .unwrap();
        let context = session
            .apply_mark(
                session.root_context(),
                expansion,
                Transparency::SemiTransparent,
            )
            .unwrap();
        let marked_span = session
            .with_context(token.span, context, Some(expansion))
            .unwrap();
        let expanded_span = session
            .with_context(token.span, session.root_context(), Some(expansion))
            .unwrap();
        let TokenKind::Ident(mut ident) = token.kind else {
            unreachable!()
        };
        ident.context = context;
        let mut builder = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        assert!(matches!(
            builder.from_token(
                Token {
                    kind: TokenKind::Ident(ident),
                    span: marked_span
                },
                Edition::E2024
            ),
            Err(NameError::Unsupported(UnsupportedName::NonRootContext))
        ));
        assert!(matches!(
            builder.from_token(
                Token {
                    span: expanded_span,
                    ..token
                },
                Edition::E2024
            ),
            Err(NameError::Unsupported(UnsupportedName::ExpandedOccurrence))
        ));
    }

    #[test]
    fn last_arc_retains_names_session_source_and_all_leases() {
        let budget = ExpansionBudget::default();
        let (session, token, _) = fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let name = builder.from_token(token, Edition::E2024).unwrap();
        let store = builder.freeze().unwrap();
        let retained = budget.snapshot().retained_bytes;
        let other = store.clone();
        drop(store);
        assert_eq!(budget.snapshot().retained_bytes, retained);
        assert_eq!(other.spelling(name).unwrap(), "x");
        let work = budget.snapshot().counters[Counter::Work as usize];
        drop(other);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().counters[Counter::Work as usize], work);
    }

    #[test]
    fn constructor_and_record_capacity_failures_release_all_payloads() {
        let healthy = ExpansionBudget::default();
        let (session, _, _) = fixture(healthy.clone());
        let session_bytes = healthy.snapshot().retained_bytes;
        let builder = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let builder_bytes = healthy.snapshot().retained_bytes;
        drop(builder);
        assert_eq!(healthy.snapshot().retained_bytes, 0);
        let budget = ExpansionBudget::new(ExpansionLimits {
            bytes: session_bytes,
            ..ExpansionLimits::default()
        });
        let (session, _, _) = fixture(budget.clone());
        assert!(matches!(
            NameStoreBuilder::new(session, Edition::E2024),
            Err(NameError::Token(TokenError::Budget(_)))
        ));
        assert_eq!(budget.snapshot().retained_bytes, 0);
        let budget = ExpansionBudget::new(ExpansionLimits {
            bytes: builder_bytes + 1,
            ..ExpansionLimits::default()
        });
        let (session, token, _) = fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let Err(NameError::Token(TokenError::Budget(first))) =
            builder.from_token(token, Edition::E2024)
        else {
            panic!("record capacity must be admitted")
        };
        let Err(NameError::Token(TokenError::Budget(again))) =
            builder.from_token(token, Edition::E2024)
        else {
            panic!("failure must latch")
        };
        assert_eq!(first, again);
        drop(builder);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn reads_and_freeze_preserve_latched_work_failure_kind() {
        let budget = ExpansionBudget::default();
        let (session, token, _) = fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let name = builder.from_token(token, Edition::E2024).unwrap();
        let remaining = budget.limits().counters[Counter::Work as usize]
            - budget.snapshot().counters[Counter::Work as usize];
        budget.charge(Counter::Work, remaining).unwrap();
        let Err(NameError::Token(TokenError::Budget(first))) = builder.check(name) else {
            panic!("work must exhaust")
        };
        let Err(NameError::Token(TokenError::Budget(again))) = builder.spelling(name) else {
            panic!("read must fail closed")
        };
        assert_eq!(first, again);
        let Err(NameError::Token(TokenError::Budget(again))) = builder.freeze() else {
            panic!("freeze must fail closed")
        };
        assert_eq!(first, again);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }
}
