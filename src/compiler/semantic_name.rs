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

mod literal_consume;
pub(crate) use literal_consume::{LiteralError, LiteralParts};

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
    anonymous_closure: bool,
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
    /// The session owner while building: a refundable lease. Lives only in
    /// the builder, never in the published `NameStore`. Consumed by `freeze`.
    owner: super::owner_path::BuilderOwnedPath,
}

/// Shared lifetime owner for every name used by the AST, resolver and replay.
/// Records contain only flat checked handles, so destruction is nonrecursive.
pub struct NameStore {
    session: TokenSession,
    /// Immutable published owner identity, set by the consuming freeze.
    owner: Option<super::owner_path::OwnerPath>,
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
    /// Reserve capacity for parser text escaping into the program AST. Callers
    /// charge planned capacity before allocation, then any actual-capacity delta.
    /// This conservative charge is never refunded before the last NameStore Arc
    /// drops, including parser failure. It grants no payload or name authority.
    pub(crate) fn retain_parser_text(&mut self, bytes: usize) -> Result<(), NameError> {
        self.retain_parser_bytes(bytes)
    }

    /// Reserve parser container backing retained by the published program.
    /// Callers precharge growth and reconcile actual capacity before publication.
    /// Like parser text this is conservative, not a name/session capability.
    pub(crate) fn retain_parser_storage(&mut self, bytes: usize) -> Result<(), NameError> {
        self.retain_parser_bytes(bytes)
    }

    /// Allocate one fixed-layout parser value after retained storage admission.
    /// The conservative lease stays with the final NameStore owner, like other
    /// parser containers. This grants no name or source occurrence authority.
    pub(crate) fn try_parser_box<T>(&mut self, value: T) -> Result<Box<T>, NameError> {
        let layout = std::alloc::Layout::new::<T>();
        self.retain_parser_storage(layout.size())?;
        if layout.size() == 0 {
            return Ok(Box::new(value));
        }
        // SAFETY: layout is nonzero and valid for T. The global allocator is
        // also Box<T>'s allocator, so its eventual deallocation uses this same
        // exact layout. No slice-capacity shrink or reallocation is involved.
        let allocation = unsafe { std::alloc::alloc(layout) }.cast::<T>();
        if allocation.is_null() {
            return Err(self.budget().poison_allocation().into());
        }
        // SAFETY: the fresh allocation is aligned and large enough for T and
        // has no other owner. Writing moves value exactly once. Box assumes
        // sole ownership only after initialization and drops/deallocates T.
        unsafe {
            allocation.write(value);
            Ok(Box::from_raw(allocation))
        }
    }

    fn retain_parser_bytes(&mut self, bytes: usize) -> Result<(), NameError> {
        self.budget().check()?;
        let additional = u64::try_from(bytes).map_err(|_| TokenError::IdExhausted)?;
        let total = self
            .store
            ._lease
            .bytes()
            .checked_add(additional)
            .ok_or(TokenError::IdExhausted)?;
        self.store._lease.resize(total)?;
        Ok(())
    }

    /// Compiler-only ingress after the source driver binds actual lexical sites.
    /// This is not the public root-name constructor and grants no replay proof.
    pub(crate) fn admit_expanded_token(
        &mut self,
        token: Token,
        lexical_edition: Edition,
        admission: &super::semantic_scope::ExpansionAdmission,
    ) -> Result<AstIdent, NameError> {
        self.admit_proven_token(token, lexical_edition, &(*admission).into())
    }
    pub(crate) fn admit_item_token(
        &mut self,
        token: Token,
        lexical_edition: Edition,
        admission: &super::semantic_scope::ItemExpansionAdmission,
    ) -> Result<AstIdent, NameError> {
        self.admit_proven_token(token, lexical_edition, &(*admission).into())
    }
    fn admit_proven_token(
        &mut self,
        token: Token,
        lexical_edition: Edition,
        admission: &super::semantic_scope::ExpansionTokenAdmission,
    ) -> Result<AstIdent, NameError> {
        admission
            .validate_token(self.session(), token)
            .map_err(|error| match error {
                super::semantic_scope::ScopeError::Token(error) => NameError::Token(error),
                super::semantic_scope::ScopeError::Name(error)
                | super::semantic_scope::ScopeError::Module(
                    super::module_identity::ModuleError::Name(error),
                ) => error,
                super::semantic_scope::ScopeError::Module(
                    super::module_identity::ModuleError::Token(error),
                ) => NameError::Token(error),
                super::semantic_scope::ScopeError::UnsupportedProjection => {
                    NameError::Unsupported(UnsupportedName::NonRootContext)
                }
                _ => NameError::Token(TokenError::InvalidInput(
                    "invalid lexical expansion admission",
                )),
            })?;
        let TokenKind::Ident(ident) = token.kind else {
            return Err(NameError::ExpectedIdentifier);
        };
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
            anonymous_closure: false,
        })
    }

    pub(crate) fn compile_local_rules(
        &mut self,
        body: super::macro_cursor::TokenSlice,
        site: super::macro_rules::DefinitionSite,
    ) -> Result<super::macro_rules::RulesDef, super::macro_rules::MacroError> {
        super::macro_rules::compile_rules(body, site, &mut self.store.session)
    }
    pub(crate) fn expand_local_rules(
        &mut self,
        definition: &super::macro_rules::RulesDef,
        args: super::macro_cursor::TokenSlice,
        call: super::macro_rules::CallSite,
    ) -> Result<super::macro_rules::ExpandedTokens, super::macro_rules::MacroError> {
        super::macro_rules::expand_rules(definition, args, call, &mut self.store.session)
    }
    pub(crate) fn register_crate_root(
        &mut self,
        file: super::token_session::SourceFileHandle,
    ) -> Result<super::token_session::CrateId, TokenError> {
        self.store.session.register_crate_root(file)
    }
    pub(crate) fn expansion_eof(
        &mut self,
        proof: &super::semantic_scope::ExpansionAdmission,
    ) -> Result<SpanId, super::semantic_scope::ScopeError> {
        proof.check_names(self)?;
        let expansion = proof.expansion(self)?;
        let call = self.session().expansion(expansion)?.call;
        let context = self.session().root_context();
        Ok(self.store.session.reissue_span(call, context, expansion)?)
    }
    pub(crate) fn item_expansion_eof(
        &mut self,
        proof: &super::semantic_scope::ItemExpansionAdmission,
    ) -> Result<SpanId, super::semantic_scope::ScopeError> {
        proof.check_names(self)?;
        let expansion = proof.expansion(self)?;
        let call = self.session().expansion(expansion)?.call;
        let context = self.session().root_context();
        Ok(self.store.session.reissue_span(call, context, expansion)?)
    }
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

    /// A builder over a session and the one-time capability issued with it.
    /// A cap issued for any other session is rejected, even if budgets and
    /// snapshots are equal.
    pub fn new(
        session: TokenSession,
        owner: super::token_session::SessionOwnerCap,
        unit_edition: Edition,
    ) -> Result<Self, NameError> {
        let owner = owner.bind(&session)?;
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
                owner: None,
                unit_edition,
                records,
                _lease: lease,
            },
            owner,
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
            anonymous_closure: false,
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
            anonymous_closure: false,
        })
    }

    /// Compiler-owned declaration identity anchored at the actual closure site.
    /// The `$` spelling is impossible through either source-name ingress or
    /// the public structural identifier factory. It never enters module lookup.
    pub(crate) fn anonymous_closure(
        &mut self,
        origin: SpanId,
        ordinal: usize,
    ) -> Result<AstIdent, NameError> {
        self.check_origin(origin)?;
        let spelling = self
            .store
            .session
            .intern_symbol(&format!("$closure${ordinal}"))?;
        self.append(NameData {
            atom: NameAtomId(spelling),
            spelling,
            context: self.session().root_context(),
            span: origin,
            raw: false,
            edition: self.edition(),
            anonymous_closure: true,
        })
    }

    pub(crate) fn anonymous_closure_environment(
        &mut self,
        origin: SpanId,
        ordinal: usize,
        field: usize,
    ) -> Result<AstIdent, NameError> {
        self.check_origin(origin)?;
        let spelling = self
            .store
            .session
            .intern_symbol(&format!("$closure_env${ordinal}${field}"))?;
        self.append(NameData {
            atom: NameAtomId(spelling),
            spelling,
            context: self.session().root_context(),
            span: origin,
            raw: false,
            edition: self.edition(),
            anonymous_closure: false,
        })
    }

    /// A source cannot name the anonymous generic introduced by argument
    /// `impl Trait`, even when it declares an ordinary parameter `impl_0`.
    pub(crate) fn anonymous_impl_parameter(
        &mut self,
        origin: SpanId,
        ordinal: usize,
    ) -> Result<AstIdent, NameError> {
        self.check_origin(origin)?;
        let spelling = self
            .store
            .session
            .intern_symbol(&format!("$impl_trait${ordinal}"))?;
        self.append(NameData {
            atom: NameAtomId(spelling),
            spelling,
            context: self.session().root_context(),
            span: origin,
            raw: false,
            edition: self.edition(),
            anonymous_closure: false,
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

    /// Publish the store. Consumes the builder, and converts the session's
    /// refundable owner reservation into an immutable receipt exactly once.
    pub fn freeze(self) -> Result<Arc<NameStore>, NameError> {
        self.budget().check()?;
        let Self { mut store, owner } = self;
        // The budget can be poisoned by a sibling after the precheck above:
        // keep its exact typed, latched failure rather than a generic error.
        store.owner = Some(owner.freeze()?);
        Ok(Arc::new(store))
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
    /// The immutable published owner identity of this store.
    pub fn owner_path(&self) -> Option<&super::owner_path::OwnerPath> {
        self.owner.as_ref()
    }
    pub(crate) fn is_anonymous_closure(&self, ident: AstIdent) -> Result<bool, NameError> {
        Ok(self.record(ident)?.anonymous_closure)
    }

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
        if record.context != self.session.root_context() && class == LookupClass::Label {
            return Err(NameError::Unsupported(UnsupportedName::NonRootContext));
        }
        // Only the compiler-private, driver-proof-checked ingress can append
        // nonroot records. Lifetime and type parameter names, like item names, have textual lookup
        // under semi-transparent macro marks. Their AstIdent still retains the
        // original occurrence and context, while the lexical region scope binds
        // the projected key. Labels remain explicitly unsupported above.
        // The first slice admits one semi-transparent mark;
        // item/member references use its checked invocation (root) context.
        // Local keys retain their mark and resolve through lexical frontiers.
        let context = if record.context == self.session.root_context()
            || !matches!(
                class,
                LookupClass::Item(_)
                    | LookupClass::Member
                    | LookupClass::LifetimeParameter
                    | LookupClass::TypeParameter
            ) {
            record.context
        } else {
            match *self.session.context(record.context)? {
                super::hygiene::ContextData::Mark {
                    parent,
                    transparency: super::hygiene::Transparency::SemiTransparent,
                    ..
                } if matches!(
                    self.session.context(parent)?,
                    super::hygiene::ContextData::Root
                ) =>
                {
                    parent
                }
                _ => record.context,
            }
        };
        Ok(NameKey {
            atom: record.atom,
            context,
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

/// Internal occurrence association identity, not semantic identifier equality.
pub(super) fn occurrence_index(
    names: &impl super::semantic_path::NameAccess,
    ident: AstIdent,
) -> Result<u32, NameError> {
    names.check(ident)?;
    Ok(ident.index)
}
pub(super) fn name_context(
    names: &impl super::semantic_path::NameAccess,
    ident: AstIdent,
) -> Result<SyntaxContextId, NameError> {
    Ok(names.key(ident, LookupClass::Local)?.context)
}
pub(super) fn key_at_context(
    names: &impl super::semantic_path::NameAccess,
    ident: AstIdent,
    class: LookupClass,
    context: SyntaxContextId,
) -> Result<NameKey, NameError> {
    names.session().context(context)?;
    let mut key = names.key(ident, class)?;
    key.context = context;
    Ok(key)
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

    /// The frozen store's owner receipt, checked to pay exactly the step
    /// capacity actually allocated (not an assumed logical size).
    fn exact_owner_receipt(store: &NameStore) -> u64 {
        let path = store.owner_path().expect("frozen store has an owner");
        assert_eq!(
            path.receipt_bytes(),
            super::super::owner_path::owner_receipt_bytes(path.step_capacity())
        );
        path.receipt_bytes()
    }

    /// Like `fixture`, also returning the session's one-time store capability.
    fn owned_fixture(
        budget: ExpansionBudget,
    ) -> (
        TokenSession,
        super::super::token_session::SessionOwnerCap,
        Token,
        SpanId,
    ) {
        let source = SourceSnapshot::from_text(Path::new("fixture.rs"), "x\n", &budget).unwrap();
        let file = source.file(0).unwrap();
        let first = source.span(file, 0, 1).unwrap();
        let eof = source.span(file, 2, 2).unwrap();
        let (mut session, cap) = TokenSession::standalone(source, budget).unwrap();
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
        (session, cap, token, eof)
    }

    #[test]
    fn retained_parser_text_stays_charged_until_final_name_owner() {
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let baseline = budget.snapshot().retained_bytes;
        builder.retain_parser_text(37).unwrap();
        builder.retain_parser_text(11).unwrap();
        assert_eq!(budget.snapshot().retained_bytes, baseline + 48);
        let owner = builder.freeze().unwrap();
        let receipt = exact_owner_receipt(&owner);
        let last = owner.clone();
        drop(owner);
        assert_eq!(budget.snapshot().retained_bytes, baseline + 48);
        drop(last);
        // One frozen session owner receipt is permanent by design (PARSE-SPLIT.md).
        assert_eq!(budget.snapshot().retained_bytes, receipt);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn retained_parser_text_rejects_before_capacity_admission_and_latches_budget() {
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let snapshot = budget.snapshot();
        let available = budget.limits().bytes - snapshot.retained_bytes - snapshot.temporary_bytes;
        let occupied = budget.reserve(Storage::Temporary, available).unwrap();
        let before = builder.store._lease.bytes();
        let failure = match builder.retain_parser_text(1).unwrap_err() {
            NameError::Token(TokenError::Budget(failure)) => failure,
            error => panic!("expected byte budget failure: {error}"),
        };
        assert_eq!(builder.store._lease.bytes(), before);
        assert!(matches!(builder.retain_parser_text(0),
            Err(NameError::Token(TokenError::Budget(error))) if error == failure));
        drop(occupied);
        drop(builder);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn parser_storage_zero_capacity_and_final_owner_follow_same_conservative_lease() {
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let baseline = budget.snapshot().retained_bytes;
        builder.retain_parser_storage(0).unwrap();
        assert_eq!(budget.snapshot().retained_bytes, baseline);
        builder.retain_parser_storage(19).unwrap();
        let owner = builder.freeze().unwrap();
        let receipt = exact_owner_receipt(&owner);
        let last = owner.clone();
        drop(owner);
        assert_eq!(budget.snapshot().retained_bytes, baseline + 19);
        drop(last);
        // One frozen session owner receipt is permanent by design (PARSE-SPLIT.md).
        assert_eq!(budget.snapshot().retained_bytes, receipt);
    }

    #[test]
    fn parser_storage_low_bytes_preserves_first_failure_including_zero_request() {
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let snapshot = budget.snapshot();
        let available = budget.limits().bytes - snapshot.retained_bytes - snapshot.temporary_bytes;
        let occupied = budget.reserve(Storage::Temporary, available).unwrap();
        let failure = match builder.retain_parser_storage(1).unwrap_err() {
            NameError::Token(TokenError::Budget(failure)) => failure,
            error => panic!("expected byte budget failure: {error}"),
        };
        assert!(matches!(builder.retain_parser_storage(0),
            Err(NameError::Token(TokenError::Budget(error))) if error == failure));
        drop(occupied);
        drop(builder);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn parser_box_preserves_alignment_drop_and_final_owner_storage() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        #[repr(align(64))]
        struct Aligned {
            value: u8,
            drops: Arc<AtomicUsize>,
        }
        impl Drop for Aligned {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::SeqCst);
            }
        }
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let baseline = budget.snapshot().retained_bytes;
        let drops = Arc::new(AtomicUsize::new(0));
        let value = builder
            .try_parser_box(Aligned {
                value: 42,
                drops: drops.clone(),
            })
            .unwrap();
        assert_eq!(value.value, 42);
        assert_eq!((&*value as *const Aligned as usize) % 64, 0);
        let retained = baseline + std::mem::size_of::<Aligned>() as u64;
        assert_eq!(budget.snapshot().retained_bytes, retained);
        drop(value);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(budget.snapshot().retained_bytes, retained);
        let owner = builder.freeze().unwrap();
        let receipt = exact_owner_receipt(&owner);
        let last = owner.clone();
        drop(owner);
        assert_eq!(budget.snapshot().retained_bytes, retained);
        drop(last);
        // One frozen session owner receipt is permanent by design (PARSE-SPLIT.md).
        assert_eq!(budget.snapshot().retained_bytes, receipt);
    }

    #[test]
    fn parser_box_denies_storage_before_allocation_and_drops_input_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Dropped(Arc<AtomicUsize>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let snapshot = budget.snapshot();
        let lease_before = builder.store._lease.bytes();
        let available = budget.limits().bytes - snapshot.retained_bytes - snapshot.temporary_bytes;
        let occupied = budget.reserve(Storage::Temporary, available).unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let failure = match builder.try_parser_box(Dropped(drops.clone())) {
            Err(NameError::Token(TokenError::Budget(failure))) => failure,
            _ => panic!("expected parser box storage denial before allocation"),
        };
        assert_eq!(
            failure.resource,
            super::super::expansion_budget::BudgetResource::Bytes
        );
        assert_eq!(failure.requested, std::mem::size_of::<Dropped>() as u64);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(builder.store._lease.bytes(), lease_before);
        drop(occupied);
        drop(builder);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }

    #[test]
    fn parser_box_zero_size_has_no_allocation_charge() {
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let baseline = budget.snapshot().retained_bytes;
        let value = builder.try_parser_box(()).unwrap();
        assert_eq!(std::mem::size_of_val(&*value), 0);
        assert_eq!(budget.snapshot().retained_bytes, baseline);
        drop(value);
        drop(builder);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn catalog_descriptor_keys_are_owned_names_not_source_occurrences() {
        let (session, session_cap, _, origin) = owned_fixture(ExpansionBudget::default());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
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
        let (foreign, foreign_cap, _, _) = owned_fixture(ExpansionBudget::default());
        let mut foreign = NameStoreBuilder::new(foreign, foreign_cap, Edition::E2024).unwrap();
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
        let (session, session_cap, token, eof) = owned_fixture(ExpansionBudget::default());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
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
        let (session_a, session_a_cap, token_a, _) = owned_fixture(ExpansionBudget::default());
        let (session_b, session_b_cap, token_b, eof_b) = owned_fixture(ExpansionBudget::default());
        let mut a = NameStoreBuilder::new(session_a, session_a_cap, Edition::E2024).unwrap();
        let mut b = NameStoreBuilder::new(session_b, session_b_cap, Edition::E2024).unwrap();
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
        let (mut session, session_cap, token, eof) = owned_fixture(ExpansionBudget::default());
        let unicode = session.intern_symbol("α").unwrap();
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
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
        let (mut session, session_cap, token, _) = owned_fixture(ExpansionBudget::default());
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
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
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

    /// A sibling poisons the shared budget after `freeze`'s precheck would
    /// have passed: the exact typed latched failure surfaces (not a generic
    /// InvalidInput) and no store is published.
    #[test]
    fn owner_freeze_failure_after_precheck_keeps_typed_budget_failure() {
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        budget.check().unwrap();
        let latched = budget.poison_allocation();
        // NameStoreBuilder::freeze prechecks first; drive the owner freeze
        // itself to prove the conversion after the precheck is typed.
        let NameStoreBuilder { store, owner } = builder;
        let error: NameError = owner.freeze().map_err(NameError::from).unwrap_err();
        assert!(
            matches!(&error, NameError::Token(TokenError::Budget(f)) if *f == latched),
            "{error:?}"
        );
        drop(store);
        // And through the public entry the same typed failure is reported.
        let budget = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        let builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let latched = budget.poison_allocation();
        let error = match builder.freeze() {
            Ok(_) => panic!("poisoned freeze published a store"),
            Err(error) => error,
        };
        assert!(
            matches!(&error, NameError::Token(TokenError::Budget(f)) if *f == latched),
            "{error:?}"
        );
        assert_eq!(budget.snapshot().retained_bytes, 0, "nothing published");
    }

    #[test]
    fn last_arc_retains_names_session_source_and_all_leases() {
        let budget = ExpansionBudget::default();
        let (session, session_cap, token, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let name = builder.from_token(token, Edition::E2024).unwrap();
        let store = builder.freeze().unwrap();
        let receipt = exact_owner_receipt(&store);
        let retained = budget.snapshot().retained_bytes;
        let other = store.clone();
        drop(store);
        assert_eq!(budget.snapshot().retained_bytes, retained);
        assert_eq!(other.spelling(name).unwrap(), "x");
        let work = budget.snapshot().counters[Counter::Work as usize];
        drop(other);
        // One frozen session owner receipt is permanent by design (PARSE-SPLIT.md).
        assert_eq!(budget.snapshot().retained_bytes, receipt);
        assert_eq!(budget.snapshot().counters[Counter::Work as usize], work);
    }

    #[test]
    fn constructor_and_record_capacity_failures_release_all_payloads() {
        let healthy = ExpansionBudget::default();
        let (session, session_cap, _, _) = owned_fixture(healthy.clone());
        let session_bytes = healthy.snapshot().retained_bytes;
        let builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let builder_bytes = healthy.snapshot().retained_bytes;
        drop(builder);
        assert_eq!(healthy.snapshot().retained_bytes, 0);
        let budget = ExpansionBudget::new(ExpansionLimits {
            bytes: session_bytes,
            ..ExpansionLimits::default()
        });
        let (session, session_cap, _, _) = owned_fixture(budget.clone());
        assert!(matches!(
            NameStoreBuilder::new(session, session_cap, Edition::E2024),
            Err(NameError::Token(TokenError::Budget(_)))
        ));
        assert_eq!(budget.snapshot().retained_bytes, 0);
        let budget = ExpansionBudget::new(ExpansionLimits {
            bytes: builder_bytes + 1,
            ..ExpansionLimits::default()
        });
        let (session, session_cap, token, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
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
        let (session, session_cap, token, _) = owned_fixture(budget.clone());
        let mut builder = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
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
