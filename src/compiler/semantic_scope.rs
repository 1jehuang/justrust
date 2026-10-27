//! Source-order lexical bindings for the explicitly admitted local-macro slice.
//! Handles describe lexical identity, never source replay or metadata authority.
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::BuildHasher;
use std::mem::size_of;
use std::num::NonZeroU64;
use std::sync::Arc;

use super::expansion_budget::{BudgetFailure, ByteLease, Counter, Depth, ExpansionBudget, Storage};
use super::expansion_span::ExpansionId;
use super::hygiene::{ContextData, Transparency};
use super::semantic_name::{self, AstIdent, LookupClass, NameError, NameKey};
use super::semantic_path::NameAccess;
use super::token::Token;
use super::token_session::{
    BudgetVec, SessionId, SpanId, SyntaxContextId, TokenError, TokenSession,
};

mod item_scope;
mod receiver;
pub(crate) use item_scope::{
    DerivedMemberAdmission, DerivedMemberKind, ExpansionTokenAdmission, ItemDerivedCopyId,
    ItemExpansionAdmission, ItemMetadataId, ModuleAccess, ModuleFrontierId, OccurrenceRole,
};

macro_rules! handle {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name {
            owner: NonZeroU64,
            index: u32,
        }
    };
}
handle!(ScopeId);
handle!(FrontierId);
handle!(BindingId);

#[derive(Debug)]
pub enum ScopeError {
    Token(TokenError),
    Name(NameError),
    Module(super::module_identity::ModuleError),
    ForeignOwner,
    InvalidHandle,
    ForeignNames,
    UnregisteredExpansion,
    ConflictingAssociation,
    UnrecordedOccurrence,
    UnsupportedProjection,
    FunctionMismatch,
}
impl From<TokenError> for ScopeError {
    fn from(value: TokenError) -> Self {
        Self::Token(value)
    }
}
impl From<NameError> for ScopeError {
    fn from(value: NameError) -> Self {
        Self::Name(value)
    }
}
impl From<super::module_identity::ModuleError> for ScopeError {
    fn from(value: super::module_identity::ModuleError) -> Self {
        Self::Module(value)
    }
}
impl From<BudgetFailure> for ScopeError {
    fn from(value: BudgetFailure) -> Self {
        Self::Token(TokenError::Budget(value))
    }
}
impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(error) => write!(f, "lexical scope: {error}"),
            Self::Name(error) => write!(f, "lexical scope: {error}"),
            Self::Module(error) => write!(f, "lexical scope: {error}"),
            other => write!(f, "lexical scope: {other:?}"),
        }
    }
}
impl std::error::Error for ScopeError {}
impl ScopeError {
    pub fn source_error(self) -> super::source::SourceError {
        use super::source::{SourceError, SourceErrorKind};
        match self {
            Self::Token(error) | Self::Name(NameError::Token(error)) => {
                SourceError::from_token(error)
            }
            Self::Module(super::module_identity::ModuleError::Token(error))
            | Self::Module(super::module_identity::ModuleError::Name(NameError::Token(error))) => {
                SourceError::from_token(error)
            }
            error => {
                let kind = match &error {
                    Self::UnsupportedProjection
                    | Self::Name(NameError::Unsupported(_))
                    | Self::Module(super::module_identity::ModuleError::Name(
                        NameError::Unsupported(_),
                    )) => SourceErrorKind::Unsupported,
                    Self::Name(NameError::ExpectedIdentifier | NameError::InvalidIdentifier)
                    | Self::Module(super::module_identity::ModuleError::Name(
                        NameError::ExpectedIdentifier | NameError::InvalidIdentifier,
                    )) => SourceErrorKind::Syntax,
                    _ => SourceErrorKind::Contract,
                };
                SourceError {
                    kind,
                    primary: None,
                    message: error.to_string(),
                    _diagnostic_lease: None,
                    location: None,
                }
            }
        }
    }
}
pub fn scope_diagnostic(error: ScopeError) -> String {
    error.source_error().to_string()
}

#[derive(Clone, Copy, Debug)]
pub struct BindingData {
    pub declaration: AstIdent,
    pub scope: ScopeId,
}
#[derive(Clone, Copy, Debug)]
pub struct ProjectedName {
    pub key: NameKey,
    pub frontier: FrontierId,
}
#[derive(Clone, Copy, Debug)]
struct ScopeRecord {
    parent: Option<FrontierId>,
    function: ScopeId,
    receiver: Option<AstIdent>,
}
#[derive(Clone, Copy, Debug)]
struct FrontierRecord {
    scope: ScopeId,
    head: Option<BindingId>,
}
#[derive(Clone, Copy, Debug)]
struct BindingRecord {
    data: BindingData,
    key: NameKey,
    previous: Option<BindingId>,
}
#[derive(Clone, Copy, Debug)]
struct Association {
    occurrence: u32,
    declaration: Option<(FrontierId, FrontierId, BindingId)>,
    reference: Option<(FrontierId, Option<BindingId>)>,
}

/// Driver-issued admission for one real expansion and its lexical points.
/// It cannot be constructed from a span/context and grants no body proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpansionAdmission {
    owner: NonZeroU64,
    session: SessionId,
    expansion: ExpansionId,
    definition: FrontierId,
    call: FrontierId,
    function: ScopeId,
}
impl ExpansionAdmission {
    pub(crate) fn validate_session(&self, session: &TokenSession) -> Result<(), ScopeError> {
        session.budget().check()?;
        // A linked crate sibling's admission stays its owner's (PARSE-SPLIT
        // step E1): the merged crate view serves, never re-owns, it.
        if !session.serves(self.session) {
            return Err(ScopeError::ForeignNames);
        }
        session.expansion(self.expansion)?;
        Ok(())
    }
    pub(crate) fn check_names(&self, names: &impl NameAccess) -> Result<(), ScopeError> {
        self.validate_session(names.session())
    }
    pub(crate) fn expansion(&self, names: &impl NameAccess) -> Result<ExpansionId, ScopeError> {
        self.check_names(names)?;
        Ok(self.expansion)
    }
    pub(crate) fn validate_span(
        &self,
        session: &TokenSession,
        span: SpanId,
    ) -> Result<(), ScopeError> {
        self.validate_session(session)?;
        session.budget().charge(Counter::Work, 1)?;
        if span.session_owner() != self.session {
            return Err(TokenError::ForeignHandle.into());
        }
        let data = session.span(span)?;
        if data.expansion != Some(self.expansion) {
            return Err(TokenError::InvalidInput(
                "expanded occurrence does not belong to admitted invocation",
            )
            .into());
        }
        self.validate_context(session, data.context)
    }
    pub(crate) fn validate_token(
        &self,
        session: &TokenSession,
        token: Token,
    ) -> Result<(), ScopeError> {
        token.validate(session)?;
        self.validate_span(session, token.span)
    }
    fn validate_context(
        &self,
        session: &TokenSession,
        context: SyntaxContextId,
    ) -> Result<(), ScopeError> {
        match *session.context(context)? {
            ContextData::Root => Ok(()),
            ContextData::Mark {
                parent,
                expansion,
                transparency: Transparency::SemiTransparent,
            } if matches!(session.context(parent)?, ContextData::Root) => {
                // Source-defined rules have root definition contexts. Nested
                // calls may capture a single mark from an active ancestor, but
                // the occurrence itself must still belong to this invocation.
                let mut current = Some(self.expansion);
                let mut depth = 0u32;
                while let Some(candidate) = current {
                    session.budget().charge(Counter::Work, 1)?;
                    depth = depth.checked_add(1).ok_or(TokenError::IdExhausted)?;
                    session.budget().check_depth(Depth::Expansion, depth)?;
                    let record = session.expansion(candidate)?;
                    if candidate == expansion {
                        return Ok(());
                    }
                    current = record.parent;
                }
                Err(ScopeError::UnsupportedProjection)
            }
            _ => Err(ScopeError::UnsupportedProjection),
        }
    }
}

// A bounded open-addressed index. Full record equality, not hash equality,
// selects entries. Rebuilds keep both allocations accounted until replacement.
struct Index {
    slots: Vec<Option<u32>>,
    _lease: ByteLease,
}
impl Index {
    fn new(budget: &ExpansionBudget, capacity: usize) -> Result<Self, ScopeError> {
        let bytes = (capacity as u64)
            .checked_mul(size_of::<Option<u32>>() as u64)
            .ok_or(TokenError::IdExhausted)?;
        budget.charge(Counter::Work, capacity as u64)?;
        let mut lease = budget.reserve(Storage::Retained, bytes)?;
        let mut slots = Vec::new();
        if slots.try_reserve_exact(capacity).is_err() {
            return Err(budget.poison_allocation().into());
        }
        lease.resize(
            (slots.capacity() as u64)
                .checked_mul(size_of::<Option<u32>>() as u64)
                .ok_or(TokenError::IdExhausted)?,
        )?;
        slots.resize(capacity, None);
        Ok(Self {
            slots,
            _lease: lease,
        })
    }
}

pub struct ScopeArena {
    owner: NonZeroU64,
    /// Frozen scope arenas of linked crate sibling files (PARSE-SPLIT.md
    /// step 1f), sorted by owner. Only their module frontier points are
    /// served (the textual macro scope a parent file had at `mod child;`);
    /// function scopes stay file-local.
    linked: Arc<[(NonZeroU64, Arc<ScopeArena>)]>,
    /// Exact file-session authority mapped to the owner-sorted linked array.
    /// Constructed and checked once, never rebuilt by occurrence lookups.
    linked_sessions: Arc<[(SessionId, usize)]>,
    names: SessionId,
    scopes: BudgetVec<ScopeRecord>,
    frontiers: BudgetVec<FrontierRecord>,
    bindings: BudgetVec<BindingRecord>,
    associations: BudgetVec<Association>,
    anchors: BudgetVec<ExpansionAdmission>,
    items: item_scope::ItemScopeState,
    association_index: Index,
    anchor_index: Index,
    hasher: RandomState,
    budget: ExpansionBudget,
    _lease: ByteLease,
}
impl fmt::Debug for ScopeArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeArena")
            .field("owner", &self.owner)
            .field("scopes", &self.scopes.len())
            .field("bindings", &self.bindings.len())
            .finish()
    }
}
#[derive(Debug)]
pub struct ScopeArenaBuilder {
    metadata_builder: item_scope::MetadataBuilder,
    arena: ScopeArena,
}

impl ScopeArenaBuilder {
    /// A scope arena owned by a child issued from the session's own issuer
    /// (the graph route: PARSE's issuer reaches here through the builder).
    pub fn for_builder(names: &mut semantic_name::NameStoreBuilder) -> Result<Self, ScopeError> {
        let owner = names.issue_owner(super::owner_path::OwnerDomain::ScopeArena)?;
        Self::with_owner(&*names, owner)
    }
    /// A scope arena for direct tooling over any name view: its owner is
    /// issued under a fresh standalone root, never a compiler counter.
    /// Direct tooling passes an owner it was issued (for example by
    /// `owner_path::standalone_issued`); this never mints one itself.
    pub fn for_names(
        names: &impl NameAccess,
        owner: super::owner_path::Issued,
    ) -> Result<Self, ScopeError> {
        Self::with_owner(names, owner)
    }
    fn with_owner(
        names: &impl NameAccess,
        owner: super::owner_path::Issued,
    ) -> Result<Self, ScopeError> {
        let budget = names.budget().clone();
        budget.check()?;
        let lease = budget.reserve(
            Storage::Retained,
            (size_of::<ScopeArena>() + 2 * size_of::<usize>()) as u64,
        )?;
        Ok(Self {
            metadata_builder: item_scope::MetadataBuilder::new(&budget)?,
            arena: ScopeArena {
                owner: owner.tag().bits(),
                linked: Arc::from([]),
                linked_sessions: Arc::from([]),
                names: names.session().id(),
                scopes: BudgetVec::new(&budget, Storage::Retained)?,
                frontiers: BudgetVec::new(&budget, Storage::Retained)?,
                bindings: BudgetVec::new(&budget, Storage::Retained)?,
                associations: BudgetVec::new(&budget, Storage::Retained)?,
                anchors: BudgetVec::new(&budget, Storage::Retained)?,
                items: item_scope::ItemScopeState::new(&budget)?,
                association_index: Index::new(&budget, 0)?,
                anchor_index: Index::new(&budget, 0)?,
                hasher: RandomState::new(),
                budget,
                _lease: lease,
            },
        })
    }
    pub fn function_root(&mut self, names: &impl NameAccess) -> Result<FrontierId, ScopeError> {
        self.receiver_function_root(names, None)
    }
    pub fn enter(
        &mut self,
        names: &impl NameAccess,
        parent: FrontierId,
    ) -> Result<FrontierId, ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        let function = self.function(parent)?;
        let scope = ScopeId {
            owner: self.arena.owner,
            index: self.arena.index(self.arena.scopes.len())?,
        };
        self.arena.scopes.push(ScopeRecord {
            parent: Some(parent),
            function,
            receiver: None,
        })?;
        self.arena
            .push_frontier(FrontierRecord { scope, head: None })
    }
    pub fn bind(
        &mut self,
        names: &impl NameAccess,
        at: FrontierId,
        declaration: AstIdent,
    ) -> Result<(FrontierId, BindingId), ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        let point = self.arena.frontier(at)?;
        self.check_occurrence(names, declaration, Some(self.function(at)?))?;
        let occurrence = semantic_name::occurrence_index(names, declaration)?;
        if let Some(association) = self.arena.association(occurrence)?
            && let Some((previous, next, binding)) = association.declaration
        {
            return if previous == at {
                Ok((next, binding))
            } else {
                Err(ScopeError::ConflictingAssociation)
            };
        }
        self.arena.check_context(
            names,
            semantic_name::name_context(names, declaration)?,
            self.function(at)?,
        )?;
        let key = names.key(declaration, LookupClass::Local)?;
        let binding = BindingId {
            owner: self.arena.owner,
            index: self.arena.index(self.arena.bindings.len())?,
        };
        self.arena.bindings.push(BindingRecord {
            data: BindingData {
                declaration,
                scope: point.scope,
            },
            key,
            previous: point.head,
        })?;
        let next = self.arena.push_frontier(FrontierRecord {
            scope: point.scope,
            head: Some(binding),
        })?;
        let mut association = self.arena.association(occurrence)?.unwrap_or(Association {
            occurrence,
            declaration: None,
            reference: None,
        });
        association.declaration = Some((at, next, binding));
        self.arena.save_association(association)?;
        Ok((next, binding))
    }
    pub fn resolve_reference(
        &mut self,
        names: &impl NameAccess,
        at: FrontierId,
        reference: AstIdent,
    ) -> Result<Option<BindingId>, ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        self.check_occurrence(names, reference, Some(self.function(at)?))?;
        let occurrence = semantic_name::occurrence_index(names, reference)?;
        if let Some(association) = self.arena.association(occurrence)?
            && let Some((previous, binding)) = association.reference
        {
            return if previous == at {
                Ok(binding)
            } else {
                Err(ScopeError::ConflictingAssociation)
            };
        }
        let result = self.arena.lookup(names, at, reference)?;
        let mut association = self.arena.association(occurrence)?.unwrap_or(Association {
            occurrence,
            declaration: None,
            reference: None,
        });
        association.reference = Some((at, result));
        self.arena.save_association(association)?;
        Ok(result)
    }
    pub(crate) fn anchor_expansion(
        &mut self,
        names: &impl NameAccess,
        expansion: ExpansionId,
        definition: FrontierId,
        call: FrontierId,
    ) -> Result<ExpansionAdmission, ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        let record = names.session().expansion(expansion)?;
        if record.parent.is_some() {
            return Err(ScopeError::UnsupportedProjection);
        }
        if !self.visible_from(definition, call)? {
            return Err(ScopeError::ConflictingAssociation);
        }
        let function = self.function(definition)?;
        if self.function(call)? != function {
            return Err(ScopeError::FunctionMismatch);
        }
        let admission = ExpansionAdmission {
            owner: self.arena.owner,
            session: self.arena.names,
            expansion,
            definition,
            call,
            function,
        };
        self.save_anchor(admission)
    }
    /// Admit a nested source-rule call only through its actual active parent
    /// proof. A session expansion ID or call span alone cannot create an anchor.
    pub(crate) fn anchor_nested_expansion(
        &mut self,
        names: &impl NameAccess,
        expansion: ExpansionId,
        definition: FrontierId,
        call: FrontierId,
        parent: &ExpansionAdmission,
    ) -> Result<ExpansionAdmission, ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        parent.check_names(names)?;
        if parent.owner != self.arena.owner {
            return Err(ScopeError::ForeignOwner);
        }
        if self.arena.anchor(parent.expansion)? != Some(*parent) {
            return Err(ScopeError::ConflictingAssociation);
        }
        let record = names.session().expansion(expansion)?;
        if record.parent != Some(parent.expansion) {
            return Err(ScopeError::ConflictingAssociation);
        }
        parent.validate_span(names.session(), record.call)?;
        let definition_span = names.session().span(record.definition)?;
        if definition_span.expansion.is_some()
            || !matches!(
                names.session().context(definition_span.context)?,
                ContextData::Root
            )
        {
            return Err(ScopeError::UnsupportedProjection);
        }
        let function = self.function(definition)?;
        if self.function(call)? != function || parent.function != function {
            return Err(ScopeError::FunctionMismatch);
        }
        if !self.visible_from(definition, call)? {
            return Err(ScopeError::ConflictingAssociation);
        }
        self.save_anchor(ExpansionAdmission {
            owner: self.arena.owner,
            session: self.arena.names,
            expansion,
            definition,
            call,
            function,
        })
    }
    fn save_anchor(
        &mut self,
        admission: ExpansionAdmission,
    ) -> Result<ExpansionAdmission, ScopeError> {
        let expansion = admission.expansion;
        if let Some(previous) = self.arena.anchor(expansion)? {
            return if previous == admission {
                Ok(previous)
            } else {
                Err(ScopeError::ConflictingAssociation)
            };
        }
        self.arena.grow_anchor_index()?;
        let index = self.arena.index(self.arena.anchors.len())?;
        self.arena.anchors.push(admission)?;
        let hash = self.arena.hasher.hash_one(expansion);
        ScopeArena::insert_index(
            &self.arena.budget,
            &mut self.arena.anchor_index,
            hash,
            index,
        )?;
        Ok(admission)
    }
    /// Link the crate's frozen sibling file scope arenas (explicit, fixed).
    pub fn link_crate_files(&mut self, arenas: Vec<Arc<ScopeArena>>) -> Result<(), ScopeError> {
        if !self.arena.linked.is_empty() {
            return Err(TokenError::InvalidInput("crate sibling scopes already linked").into());
        }
        let count = arenas.len();
        if count == 0 {
            return Ok(());
        }
        // Prepay both sorting passes, index construction and Arc relocation.
        // The temporary vectors coexist with the final Arc allocations during
        // conversion, so their full backing capacities remain charged too.
        let levels = u64::from(count.ilog2()) + 1;
        let work = (count as u64)
            .checked_mul(
                levels
                    .checked_mul(2)
                    .and_then(|n| n.checked_add(6))
                    .ok_or(TokenError::IdExhausted)?,
            )
            .ok_or(TokenError::IdExhausted)?;
        self.arena.budget.charge(Counter::Work, work)?;
        let backing_bytes = |owners: usize, sessions: usize| -> Result<u64, ScopeError> {
            owners
                .checked_mul(size_of::<(NonZeroU64, Arc<ScopeArena>)>())
                .and_then(|n| {
                    sessions
                        .checked_mul(size_of::<(SessionId, usize)>())
                        .and_then(|s| n.checked_add(s))
                })
                .map(|n| n as u64)
                .ok_or_else(|| TokenError::IdExhausted.into())
        };
        let mut scratch = self
            .arena
            .budget
            .reserve(Storage::Temporary, backing_bytes(count, count)?)?;
        let mut linked: Vec<(NonZeroU64, Arc<ScopeArena>)> = Vec::new();
        linked
            .try_reserve_exact(count)
            .map_err(|_| self.arena.budget.poison_allocation())?;
        scratch.resize(backing_bytes(linked.capacity(), count)?)?;
        let mut sessions = Vec::new();
        sessions
            .try_reserve_exact(count)
            .map_err(|_| self.arena.budget.poison_allocation())?;
        scratch.resize(backing_bytes(linked.capacity(), sessions.capacity())?)?;
        for arena in arenas {
            if arena.owner == self.arena.owner {
                return Err(TokenError::InvalidInput("a scope arena cannot link itself").into());
            }
            if arena.names == self.arena.names {
                return Err(TokenError::InvalidInput(
                    "linked scope session collides with its own session",
                )
                .into());
            }
            linked.push((arena.owner, arena));
        }
        linked.sort_unstable_by_key(|(owner, _)| *owner);
        if linked.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(TokenError::InvalidInput("duplicate linked scope owner").into());
        }
        for (position, (_, arena)) in linked.iter().enumerate() {
            sessions.push((arena.names, position));
        }
        sessions.sort_unstable_by_key(|(session, _)| session.bits());
        if sessions.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(TokenError::InvalidInput("duplicate linked scope session").into());
        }
        let retained = backing_bytes(count, count)?
            .checked_add((4 * size_of::<usize>()) as u64)
            .ok_or(TokenError::IdExhausted)?;
        self.arena._lease.resize(
            self.arena
                ._lease
                .bytes()
                .checked_add(retained)
                .ok_or(TokenError::IdExhausted)?,
        )?;
        let linked = linked.into();
        let sessions = sessions.into();
        self.arena.linked = linked;
        self.arena.linked_sessions = sessions;
        drop(scratch);
        Ok(())
    }
    pub fn freeze(mut self) -> Result<Arc<ScopeArena>, ScopeError> {
        self.arena.budget.check()?;
        self.arena.check_item_completion()?;
        self.arena.items.metadata = self.metadata_builder.freeze(&mut self.arena._lease)?;
        Ok(Arc::new(self.arena))
    }
    pub fn check_names(&self, names: &impl NameAccess) -> Result<(), ScopeError> {
        self.arena.check_names(names)
    }
    pub fn function(&self, at: FrontierId) -> Result<ScopeId, ScopeError> {
        self.arena.function(at)
    }
    pub fn check_function(&self, id: ScopeId) -> Result<(), ScopeError> {
        self.arena.check_function(id)
    }
    pub fn declaration_for(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<BindingId, ScopeError> {
        self.arena.declaration_for(names, ident)
    }
    pub fn reference_for(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<Option<BindingId>, ScopeError> {
        self.arena.reference_for(names, ident)
    }
    pub fn binding(&self, id: BindingId) -> Result<BindingData, ScopeError> {
        self.arena.binding(id)
    }
    pub fn project_name(
        &self,
        names: &impl NameAccess,
        at: FrontierId,
        ident: AstIdent,
        class: LookupClass,
    ) -> Result<ProjectedName, ScopeError> {
        self.arena.project_name(names, at, ident, class)
    }
    pub fn budget(&self) -> &ExpansionBudget {
        self.arena.budget()
    }
    pub fn visible_from(
        &self,
        definition: FrontierId,
        call: FrontierId,
    ) -> Result<bool, ScopeError> {
        self.arena.visible_from(definition, call)
    }
    pub fn binding_function(&self, id: BindingId) -> Result<ScopeId, ScopeError> {
        self.arena.binding_function(id)
    }
    pub fn reference_function(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<ScopeId, ScopeError> {
        self.arena.reference_function(names, ident)
    }
    pub fn reference_frontier(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<FrontierId, ScopeError> {
        self.arena.reference_frontier(names, ident)
    }
    pub fn project_reference(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
        class: LookupClass,
    ) -> Result<ProjectedName, ScopeError> {
        self.arena.project_reference(names, ident, class)
    }
    pub fn check_occurrence(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
        function: Option<ScopeId>,
    ) -> Result<(), ScopeError> {
        self.arena.check_occurrence(names, ident, function)
    }
}
impl ScopeArena {
    /// This arena's linked crate sibling arenas.
    pub fn linked_arenas(&self) -> Vec<Arc<ScopeArena>> {
        self.linked.iter().map(|(_, a)| a.clone()).collect()
    }
    pub fn budget(&self) -> &ExpansionBudget {
        &self.budget
    }
    fn index(&self, index: usize) -> Result<u32, ScopeError> {
        Ok(self.budget.checked_u32(index as u64)?)
    }
    fn transition(&self) -> Result<(), ScopeError> {
        self.budget.charge(Counter::ScopeTransitions, 1)?;
        self.budget.charge(Counter::Work, 1)?;
        Ok(())
    }
    pub fn check_names(&self, names: &impl NameAccess) -> Result<(), ScopeError> {
        self.budget.check()?;
        names.budget().check()?;
        if !names.session().serves(self.names) || !self.budget.same_authority(names.budget()) {
            return Err(ScopeError::ForeignNames);
        }
        Ok(())
    }
    fn owner(&self, owner: NonZeroU64) -> Result<(), ScopeError> {
        self.budget.charge(Counter::Work, 1)?;
        if owner != self.owner {
            Err(ScopeError::ForeignOwner)
        } else {
            Ok(())
        }
    }
    /// The linked crate sibling arena owning `owner`, `None` for this one.
    fn linked_arena(&self, owner: NonZeroU64) -> Result<Option<&ScopeArena>, ScopeError> {
        if owner == self.owner {
            return Ok(None);
        }
        self.budget.charge(Counter::Work, 1)?;
        self.linked
            .binary_search_by_key(&owner, |(id, _)| *id)
            .map(|index| Some(&*self.linked[index].1))
            .map_err(|_| ScopeError::ForeignOwner)
    }
    /// Association indices are local to their name session, not to the
    /// merged crate view. Select the exact file owner before reading one.
    fn occurrence_arena(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<&ScopeArena, ScopeError> {
        self.check_names(names)?;
        let owner = semantic_name::occurrence_owner(names, ident)?;
        self.session_arena(owner)?.ok_or(ScopeError::ForeignNames)
    }
    fn session_arena(&self, owner: SessionId) -> Result<Option<&ScopeArena>, ScopeError> {
        if owner == self.names {
            return Ok(Some(self));
        }
        // std's binary search may perform a final comparison after narrowing
        // a non-power-of-two slice, so floor(log2(n)) + 1 is not sufficient.
        let comparisons = self
            .linked_sessions
            .len()
            .checked_ilog2()
            .map_or(0, |n| u64::from(n) + 2);
        self.budget.charge(Counter::Work, comparisons)?;
        // The merged view has already checked caller name/budget authority.
        // A linked file keeps its distinct original task budget and identity.
        Ok(self
            .linked_sessions
            .binary_search_by_key(&owner.bits(), |(session, _)| session.bits())
            .ok()
            .map(|index| &*self.linked[self.linked_sessions[index].1].1))
    }
    fn scope(&self, id: ScopeId) -> Result<ScopeRecord, ScopeError> {
        if let Some(arena) = self.linked_arena(id.owner)? {
            return arena.scope(id);
        }
        self.owner(id.owner)?;
        self.scopes
            .as_slice()
            .get(id.index as usize)
            .copied()
            .ok_or(ScopeError::InvalidHandle)
    }
    fn frontier(&self, id: FrontierId) -> Result<FrontierRecord, ScopeError> {
        if let Some(arena) = self.linked_arena(id.owner)? {
            return arena.frontier(id);
        }
        self.owner(id.owner)?;
        self.frontiers
            .as_slice()
            .get(id.index as usize)
            .copied()
            .ok_or(ScopeError::InvalidHandle)
    }
    fn binding_record(&self, id: BindingId) -> Result<BindingRecord, ScopeError> {
        if let Some(arena) = self.linked_arena(id.owner)? {
            return arena.binding_record(id);
        }
        self.owner(id.owner)?;
        self.bindings
            .as_slice()
            .get(id.index as usize)
            .copied()
            .ok_or(ScopeError::InvalidHandle)
    }
    pub fn check_function(&self, id: ScopeId) -> Result<(), ScopeError> {
        let scope = self.scope(id)?;
        if scope.function != id || scope.parent.is_some() {
            return Err(ScopeError::InvalidHandle);
        }
        Ok(())
    }
    pub fn function(&self, at: FrontierId) -> Result<ScopeId, ScopeError> {
        let point = self.frontier(at)?;
        Ok(self.scope(point.scope)?.function)
    }
    pub fn binding(&self, id: BindingId) -> Result<BindingData, ScopeError> {
        Ok(self.binding_record(id)?.data)
    }
    pub fn binding_function(&self, id: BindingId) -> Result<ScopeId, ScopeError> {
        let data = self.binding(id)?;
        Ok(self.scope(data.scope)?.function)
    }
    pub fn reference_function(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<ScopeId, ScopeError> {
        self.check_names(names)?;
        let occurrence = semantic_name::occurrence_index(names, ident)?;
        let arena = self.occurrence_arena(names, ident)?;
        let at = arena
            .association(occurrence)?
            .and_then(|row| row.reference.map(|(at, _)| at))
            .ok_or(ScopeError::UnrecordedOccurrence)?;
        arena.function(at)
    }
    pub fn reference_frontier(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<FrontierId, ScopeError> {
        self.check_names(names)?;
        let occurrence = semantic_name::occurrence_index(names, ident)?;
        self.occurrence_arena(names, ident)?
            .association(occurrence)?
            .and_then(|row| row.reference.map(|(at, _)| at))
            .ok_or(ScopeError::UnrecordedOccurrence)
    }
    pub fn project_reference(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
        class: LookupClass,
    ) -> Result<ProjectedName, ScopeError> {
        let at = self.reference_frontier(names, ident)?;
        self.project_name(names, at, ident, class)
    }
    pub fn visible_from(
        &self,
        definition: FrontierId,
        mut call: FrontierId,
    ) -> Result<bool, ScopeError> {
        if self.function(definition)? != self.function(call)? {
            return Err(ScopeError::FunctionMismatch);
        }
        let target = self.frontier(definition)?;
        loop {
            let current = self.frontier(call)?;
            if current.scope == target.scope {
                if target.head.is_none() {
                    return Ok(true);
                }
                let mut head = current.head;
                while let Some(id) = head {
                    self.budget.charge(Counter::Work, 1)?;
                    if Some(id) == target.head {
                        return Ok(true);
                    }
                    head = self.binding_record(id)?.previous;
                }
                return Ok(false);
            }
            match self.scope(current.scope)?.parent {
                Some(parent) => call = parent,
                None => return Ok(false),
            }
        }
    }
    fn push_frontier(&mut self, point: FrontierRecord) -> Result<FrontierId, ScopeError> {
        let id = FrontierId {
            owner: self.owner,
            index: self.index(self.frontiers.len())?,
        };
        self.frontiers.push(point)?;
        Ok(id)
    }
    fn find_index(
        &self,
        index: &Index,
        hash: u64,
        mut equal: impl FnMut(u32) -> bool,
    ) -> Result<Option<usize>, ScopeError> {
        if index.slots.is_empty() {
            return Ok(None);
        }
        let mask = index.slots.len() - 1;
        let mut at = hash as usize & mask;
        loop {
            self.budget.charge(Counter::Work, 1)?;
            match index.slots[at] {
                None => return Ok(None),
                Some(entry) if equal(entry) => return Ok(Some(at)),
                _ => at = (at + 1) & mask,
            }
        }
    }
    fn insert_index(
        budget: &ExpansionBudget,
        index: &mut Index,
        hash: u64,
        entry: u32,
    ) -> Result<(), ScopeError> {
        let mask = index.slots.len() - 1;
        let mut at = hash as usize & mask;
        loop {
            budget.charge(Counter::Work, 1)?;
            if index.slots[at].is_none() {
                index.slots[at] = Some(entry);
                return Ok(());
            }
            at = (at + 1) & mask;
        }
    }
    fn association(&self, occurrence: u32) -> Result<Option<Association>, ScopeError> {
        let hash = self.hasher.hash_one(occurrence);
        Ok(self
            .find_index(&self.association_index, hash, |index| {
                self.associations.as_slice()[index as usize].occurrence == occurrence
            })?
            .and_then(|slot| self.association_index.slots[slot])
            .map(|index| self.associations.as_slice()[index as usize]))
    }
    fn save_association(&mut self, association: Association) -> Result<(), ScopeError> {
        let hash = self.hasher.hash_one(association.occurrence);
        let previous = self.find_index(&self.association_index, hash, |index| {
            self.associations.as_slice()[index as usize].occurrence == association.occurrence
        })?;
        if previous.is_none() {
            self.grow_association_index()?;
        }
        let index = self.index(self.associations.len())?;
        self.associations.push(association)?;
        if let Some(slot) = previous {
            self.association_index.slots[slot] = Some(index);
        } else {
            Self::insert_index(&self.budget, &mut self.association_index, hash, index)?;
        }
        Ok(())
    }
    fn grow_association_index(&mut self) -> Result<(), ScopeError> {
        if self
            .associations
            .len()
            .checked_add(1)
            .ok_or(TokenError::IdExhausted)?
            <= self.association_index.slots.len() / 2
        {
            return Ok(());
        }
        let capacity = self
            .association_index
            .slots
            .len()
            .max(4)
            .checked_mul(2)
            .ok_or(TokenError::IdExhausted)?;
        let mut next = Index::new(&self.budget, capacity)?;
        self.budget
            .charge(Counter::Work, self.association_index.slots.len() as u64)?;
        for entry in self.association_index.slots.iter().flatten() {
            let record = self.associations.as_slice()[*entry as usize];
            Self::insert_index(
                &self.budget,
                &mut next,
                self.hasher.hash_one(record.occurrence),
                *entry,
            )?;
        }
        self.association_index = next;
        Ok(())
    }
    fn anchor(&self, expansion: ExpansionId) -> Result<Option<ExpansionAdmission>, ScopeError> {
        let Some(arena) = self.session_arena(expansion.session_owner())? else {
            return Ok(None);
        };
        let hash = arena.hasher.hash_one(expansion);
        Ok(arena
            .find_index(&arena.anchor_index, hash, |index| {
                arena.anchors.as_slice()[index as usize].expansion == expansion
            })?
            .and_then(|slot| arena.anchor_index.slots[slot])
            .map(|index| arena.anchors.as_slice()[index as usize]))
    }
    fn grow_anchor_index(&mut self) -> Result<(), ScopeError> {
        if self
            .anchors
            .len()
            .checked_add(1)
            .ok_or(TokenError::IdExhausted)?
            <= self.anchor_index.slots.len() / 2
        {
            return Ok(());
        }
        let capacity = self
            .anchor_index
            .slots
            .len()
            .max(4)
            .checked_mul(2)
            .ok_or(TokenError::IdExhausted)?;
        let mut next = Index::new(&self.budget, capacity)?;
        self.budget
            .charge(Counter::Work, self.anchors.len() as u64)?;
        for (index, record) in self.anchors.as_slice().iter().enumerate() {
            Self::insert_index(
                &self.budget,
                &mut next,
                self.hasher.hash_one(record.expansion),
                self.index(index)?,
            )?;
        }
        self.anchor_index = next;
        Ok(())
    }
    pub fn declaration_for(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<BindingId, ScopeError> {
        self.check_names(names)?;
        let occurrence = semantic_name::occurrence_index(names, ident)?;
        self.occurrence_arena(names, ident)?
            .association(occurrence)?
            .and_then(|row| row.declaration.map(|(_, _, id)| id))
            .ok_or(ScopeError::UnrecordedOccurrence)
    }
    pub fn reference_for(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<Option<BindingId>, ScopeError> {
        self.check_names(names)?;
        let occurrence = semantic_name::occurrence_index(names, ident)?;
        self.occurrence_arena(names, ident)?
            .association(occurrence)?
            .and_then(|row| row.reference.map(|(_, id)| id))
            .ok_or(ScopeError::UnrecordedOccurrence)
    }
    fn check_context(
        &self,
        names: &impl NameAccess,
        context: SyntaxContextId,
        function: ScopeId,
    ) -> Result<(), ScopeError> {
        match *names.session().context(context)? {
            ContextData::Root => Ok(()),
            ContextData::Mark {
                parent,
                expansion,
                transparency: Transparency::SemiTransparent,
            } => {
                if !matches!(names.session().context(parent)?, ContextData::Root) {
                    return Err(ScopeError::UnsupportedProjection);
                }
                let Some(anchor) = self.anchor(expansion)? else {
                    if self.item_anchor(expansion)?.is_some() {
                        return self.check_item_context(names, expansion, function);
                    }
                    return Err(ScopeError::UnregisteredExpansion);
                };
                if anchor.owner != function.owner || anchor.function != function {
                    return Err(ScopeError::FunctionMismatch);
                }
                Ok(())
            }
            _ => Err(ScopeError::UnsupportedProjection),
        }
    }
    pub fn check_occurrence(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
        function: Option<ScopeId>,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        let context = semantic_name::name_context(names, ident)?;
        // Root-context captures still originate in an expansion. The span is
        // not a scope factory: only its already registered driver anchor may
        // establish this occurrence's existing function association.
        if let Some(expansion) = names.session().span(names.span(ident)?)?.expansion {
            if self.item_anchor(expansion)?.is_some() {
                return self.check_item_body_occurrence(
                    names,
                    ident,
                    function.ok_or(ScopeError::FunctionMismatch)?,
                );
            }
            let anchor = self
                .anchor(expansion)?
                .ok_or(ScopeError::UnregisteredExpansion)?;
            let function = function.ok_or(ScopeError::FunctionMismatch)?;
            self.check_function(function)?;
            if anchor.function != function {
                return Err(ScopeError::FunctionMismatch);
            }
        }
        if let Some(function) = function {
            self.check_function(function)?;
            self.check_context(names, context, function)
        } else if matches!(names.session().context(context)?, ContextData::Root) {
            Ok(())
        } else {
            Err(ScopeError::FunctionMismatch)
        }
    }
    fn exact_lookup(
        &self,
        mut at: FrontierId,
        key: NameKey,
    ) -> Result<Option<BindingId>, ScopeError> {
        loop {
            let point = self.frontier(at)?;
            let mut head = point.head;
            while let Some(id) = head {
                self.budget.charge(Counter::Work, 1)?;
                let binding = self.binding_record(id)?;
                if binding.key == key {
                    return Ok(Some(id));
                }
                head = binding.previous;
            }
            match self.scope(point.scope)?.parent {
                Some(parent) => at = parent,
                None => return Ok(None),
            }
        }
    }
    fn lookup(
        &self,
        names: &impl NameAccess,
        at: FrontierId,
        ident: AstIdent,
    ) -> Result<Option<BindingId>, ScopeError> {
        let function = self.function(at)?;
        let context = semantic_name::name_context(names, ident)?;
        self.check_context(names, context, function)?;
        if let Some(binding) = self.exact_lookup(at, names.key(ident, LookupClass::Local)?)? {
            return Ok(Some(binding));
        }
        match *names.session().context(context)? {
            ContextData::Root => Ok(None),
            ContextData::Mark {
                parent, expansion, ..
            } => {
                if self.item_anchor(expansion)?.is_some() {
                    // A module definition has no enclosing local environment.
                    // Exact generated/captured binders were searched above.
                    return Ok(None);
                }
                let anchor = self
                    .anchor(expansion)?
                    .ok_or(ScopeError::UnregisteredExpansion)?;
                self.exact_lookup(
                    anchor.definition,
                    semantic_name::key_at_context(names, ident, LookupClass::Local, parent)?,
                )
            }
        }
    }
    pub fn project_name(
        &self,
        names: &impl NameAccess,
        at: FrontierId,
        ident: AstIdent,
        class: LookupClass,
    ) -> Result<ProjectedName, ScopeError> {
        self.check_names(names)?;
        let function = self.function(at)?;
        let context = semantic_name::name_context(names, ident)?;
        self.check_context(names, context, function)?;
        match *names.session().context(context)? {
            ContextData::Root => Ok(ProjectedName {
                key: names.key(ident, class)?,
                frontier: at,
            }),
            ContextData::Mark {
                parent, expansion, ..
            } => {
                if class == LookupClass::Local {
                    let key = names.key(ident, class)?;
                    if self.exact_lookup(at, key)?.is_some() {
                        return Ok(ProjectedName { key, frontier: at });
                    }
                }
                if self.item_anchor(expansion)?.is_some() {
                    return match class {
                        LookupClass::Local | LookupClass::Item(_) | LookupClass::Member => {
                            Ok(ProjectedName {
                                key: names.key(ident, class)?,
                                frontier: at,
                            })
                        }
                        _ => Err(ScopeError::UnsupportedProjection),
                    };
                }
                let anchor = self
                    .anchor(expansion)?
                    .ok_or(ScopeError::UnregisteredExpansion)?;
                let frontier = match class {
                    LookupClass::Local => anchor.definition,
                    LookupClass::Item(_) | LookupClass::Member => anchor.call,
                    _ => return Err(ScopeError::UnsupportedProjection),
                };
                Ok(ProjectedName {
                    key: semantic_name::key_at_context(names, ident, class, parent)?,
                    frontier,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::expansion_span::ExpansionRecord;
    use super::super::semantic_name::{NameStoreBuilder, UnsupportedName};
    use super::super::token::{Edition, Ident, TokenKind};
    use super::super::token_session::SourceSnapshot;
    use super::*;
    use std::path::Path;

    fn fixture() -> (NameStoreBuilder, SpanId, ExpansionId, Token, Token) {
        let budget = ExpansionBudget::default();
        let sources = SourceSnapshot::from_text(
            Path::new("scope.rs"),
            "x",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let file = sources.file(0).unwrap();
        let original = sources.span(file, 0, 1).unwrap();
        let (mut session, session_cap) = TokenSession::standalone(sources, budget).unwrap();
        let span = session.original_span(original).unwrap();
        let defining_crate = session.register_crate_root(file).unwrap();
        let expansion = session
            .add_expansion(ExpansionRecord {
                definition: span,
                call: span,
                parent: None,
                defining_crate,
                edition: Edition::E2024,
            })
            .unwrap();
        let root = session.root_context();
        let marked = session
            .apply_mark(root, expansion, Transparency::SemiTransparent)
            .unwrap();
        let symbol = session.intern_symbol("x").unwrap();
        let marked_span = session.reissue_span(span, marked, expansion).unwrap();
        let captured_span = session.reissue_span(span, root, expansion).unwrap();
        let literal = Token {
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context: marked,
            }),
            span: marked_span,
        };
        let capture = Token {
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context: root,
            }),
            span: captured_span,
        };
        (
            NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap(),
            span,
            expansion,
            literal,
            capture,
        )
    }

    #[test]
    fn linked_file_occurrences_keep_exact_association_owners() {
        let mut files = Vec::new();
        let mut arenas = Vec::new();
        let mut rows = Vec::new();
        let mut expansions = Vec::new();
        for _ in 0..3 {
            let (mut names, span, expansion, literal, capture) = fixture();
            let TokenKind::Ident(mut token_ident) = literal.kind else {
                panic!("identifier")
            };
            token_ident.context = names.session().root_context();
            let token = Token {
                kind: TokenKind::Ident(token_ident),
                span,
            };
            let declaration = names.from_token(token, Edition::E2024).unwrap();
            let reference = names.from_token(token, Edition::E2024).unwrap();
            let mut scopes = ScopeArenaBuilder::for_builder(&mut names).unwrap();
            let root = scopes.function_root(&names).unwrap();
            let function = scopes.function(root).unwrap();
            let (at, binding) = scopes.bind(&names, root, declaration).unwrap();
            assert_eq!(
                scopes.resolve_reference(&names, at, reference).unwrap(),
                Some(binding)
            );
            let admission = scopes.anchor_expansion(&names, expansion, at, at).unwrap();
            let expanded = names
                .admit_expanded_token(literal, Edition::E2024, &admission)
                .unwrap();
            let captured = names
                .admit_expanded_token(capture, Edition::E2024, &admission)
                .unwrap();
            for ident in [expanded, captured] {
                assert_eq!(
                    scopes.resolve_reference(&names, at, ident).unwrap(),
                    Some(binding)
                );
            }
            expansions.push((expansion, expanded, captured, admission));
            rows.push((declaration, reference, function, at, binding));
            arenas.push(scopes.freeze().unwrap());
            files.push(names.freeze().unwrap());
        }
        let (mut names, _, _, _, _) = fixture();
        names.link_crate_files(files.clone()).unwrap();
        names.join_crate(&files[0]).unwrap();
        let mut scopes = ScopeArenaBuilder::for_builder(&mut names).unwrap();
        // The third file's names are admitted, but its scope authority is not.
        let mut reversed = arenas[..2].to_vec();
        reversed.reverse();
        scopes.link_crate_files(reversed).unwrap();
        let scopes = scopes.freeze().unwrap();
        let names = names.freeze().unwrap();
        assert_eq!(
            semantic_name::occurrence_index(&*names, rows[0].0).unwrap(),
            semantic_name::occurrence_index(&*names, rows[1].0).unwrap()
        );
        for &(declaration, reference, function, at, binding) in &rows[..2] {
            assert_eq!(
                scopes.declaration_for(&*names, declaration).unwrap(),
                binding
            );
            assert_eq!(
                scopes.reference_for(&*names, reference).unwrap(),
                Some(binding)
            );
            assert_eq!(
                scopes.reference_function(&*names, reference).unwrap(),
                function
            );
            assert_eq!(scopes.reference_frontier(&*names, reference).unwrap(), at);
            assert!(matches!(
                scopes.reference_for(&*names, declaration),
                Err(ScopeError::UnrecordedOccurrence)
            ));
        }
        assert!(matches!(
            scopes.reference_for(&*names, rows[2].1),
            Err(ScopeError::ForeignNames)
        ));
        for (index, &(expansion, expanded, captured, admission)) in
            expansions[..2].iter().enumerate()
        {
            assert_eq!(scopes.anchor(expansion).unwrap(), Some(admission));
            for ident in [expanded, captured] {
                scopes
                    .check_occurrence(&*names, ident, Some(rows[index].2))
                    .unwrap();
                assert_eq!(
                    scopes.reference_for(&*names, ident).unwrap(),
                    Some(rows[index].4)
                );
                assert!(matches!(
                    scopes.check_occurrence(&*names, ident, Some(rows[1 - index].2)),
                    Err(ScopeError::FunctionMismatch)
                ));
                admission
                    .validate_span(names.session(), names.span(ident).unwrap())
                    .unwrap();
            }
            assert!(
                admission
                    .validate_span(
                        names.session(),
                        names.span(expansions[1 - index].1).unwrap()
                    )
                    .is_err()
            );
        }
        assert_eq!(scopes.anchor(expansions[2].0).unwrap(), None);
        assert!(matches!(
            scopes.check_occurrence(&*names, expansions[2].1, Some(rows[0].2)),
            Err(ScopeError::UnregisteredExpansion)
        ));
        assert!(
            scopes
                .session_arena(files[2].session().id())
                .unwrap()
                .is_none()
        );
        let (mut foreign, span, _, _, _) = fixture();
        let forged = foreign.intern_root("x", span).unwrap();
        assert!(scopes.declaration_for(&*names, forged).is_err());
    }

    #[test]
    fn linked_file_session_index_rejects_duplicate_authorities() {
        let (mut names, _, _, _, _) = fixture();
        let first = ScopeArenaBuilder::for_builder(&mut names)
            .unwrap()
            .freeze()
            .unwrap();
        let second = ScopeArenaBuilder::for_builder(&mut names)
            .unwrap()
            .freeze()
            .unwrap();
        assert_ne!(first.owner, second.owner);
        assert_eq!(first.names, second.names);
        let (mut merged_names, _, _, _, _) = fixture();
        for files in [vec![first.clone(), second.clone()], vec![second, first]] {
            let mut merged = ScopeArenaBuilder::for_builder(&mut merged_names).unwrap();
            let before = merged.arena.budget.snapshot();
            assert!(matches!(
                merged.link_crate_files(files),
                Err(ScopeError::Token(TokenError::InvalidInput(
                    "duplicate linked scope session"
                )))
            ));
            assert!(merged.arena.linked.is_empty());
            assert!(merged.arena.linked_sessions.is_empty());
            let after = merged.arena.budget.snapshot();
            assert_eq!(after.retained_bytes, before.retained_bytes);
            assert_eq!(after.temporary_bytes, before.temporary_bytes);
        }
        let own_session = ScopeArenaBuilder::for_builder(&mut merged_names)
            .unwrap()
            .freeze()
            .unwrap();
        let mut merged = ScopeArenaBuilder::for_builder(&mut merged_names).unwrap();
        assert_ne!(merged.arena.owner, own_session.owner);
        assert!(matches!(
            merged.link_crate_files(vec![own_session]),
            Err(ScopeError::Token(TokenError::InvalidInput(
                "linked scope session collides with its own session"
            )))
        ));
        assert!(merged.arena.linked.is_empty());
        assert!(merged.arena.linked_sessions.is_empty());
    }

    #[test]
    fn linked_file_session_index_prepays_backings_and_arc_peak() {
        let (mut names, _, _, _, _) = fixture();
        let file = ScopeArenaBuilder::for_builder(&mut names)
            .unwrap()
            .freeze()
            .unwrap();
        let (mut merged_names, _, _, _, _) = fixture();
        let mut merged = ScopeArenaBuilder::for_builder(&mut merged_names).unwrap();
        let budget = merged.arena.budget.clone();
        let before = budget.snapshot();
        merged.link_crate_files(vec![file.clone()]).unwrap();
        let backing =
            (size_of::<(NonZeroU64, Arc<ScopeArena>)>() + size_of::<(SessionId, usize)>()) as u64;
        let retained = backing + (4 * size_of::<usize>()) as u64;
        let after = budget.snapshot();
        assert_eq!(after.retained_bytes - before.retained_bytes, retained);
        assert_eq!(after.temporary_bytes, before.temporary_bytes);
        assert!(
            after.peak_bytes >= before.retained_bytes + before.temporary_bytes + backing + retained
        );
        assert_eq!(merged.arena.linked_sessions.len(), 1);
        assert_eq!(
            merged
                .arena
                .session_arena(file.names)
                .unwrap()
                .unwrap()
                .owner,
            file.owner
        );
        assert!(merged.link_crate_files(vec![file]).is_err());
    }

    #[test]
    fn linked_file_session_index_lookup_work_grows_logarithmically() {
        let mut deltas = Vec::new();
        for count in [3usize, 16, 64] {
            let mut files = Vec::new();
            let mut arenas = Vec::new();
            for _ in 0..count {
                let (mut names, _, _, _, _) = fixture();
                arenas.push(
                    ScopeArenaBuilder::for_builder(&mut names)
                        .unwrap()
                        .freeze()
                        .unwrap(),
                );
                files.push(names.freeze().unwrap());
            }
            let (mut names, _, _, _, _) = fixture();
            names.link_crate_files(files).unwrap();
            let mut merged = ScopeArenaBuilder::for_builder(&mut names).unwrap();
            let expected = u64::from(count.ilog2()) + 2;
            let owners = arenas
                .iter()
                .map(|arena| (arena.names, arena.owner))
                .collect::<Vec<_>>();
            arenas.reverse();
            merged.link_crate_files(arenas).unwrap();
            let merged = merged.freeze().unwrap();
            let budget = merged.budget();
            for (session, owner) in owners {
                let before = budget.snapshot().counters[Counter::Work as usize];
                assert_eq!(merged.session_arena(session).unwrap().unwrap().owner, owner);
                let delta = budget.snapshot().counters[Counter::Work as usize] - before;
                assert_eq!(delta, expected, "{count} sibling lookup");
            }
            let (foreign, _, _, _, _) = fixture();
            let before = budget.snapshot().counters[Counter::Work as usize];
            assert!(
                merged
                    .session_arena(foreign.session().id())
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                budget.snapshot().counters[Counter::Work as usize] - before,
                expected
            );
            deltas.push(expected);
        }
        assert_eq!(deltas, [3, 6, 8]);
    }

    #[test]
    fn linked_file_session_index_rejects_resource_limits_before_publication() {
        use super::super::expansion_budget::{BudgetFailureKind, BudgetResource};
        for mode in 0..3 {
            let (mut names, _, _, _, _) = fixture();
            let file = ScopeArenaBuilder::for_builder(&mut names)
                .unwrap()
                .freeze()
                .unwrap();
            let (mut merged_names, _, _, _, _) = fixture();
            let mut merged = ScopeArenaBuilder::for_builder(&mut merged_names).unwrap();
            let budget = merged.arena.budget.clone();
            let backing = (size_of::<(NonZeroU64, Arc<ScopeArena>)>()
                + size_of::<(SessionId, usize)>()) as u64;
            let retained = backing + (4 * size_of::<usize>()) as u64;
            let blocker = if mode == 0 {
                let used = budget.snapshot().counters[Counter::Work as usize];
                budget
                    .charge(
                        Counter::Work,
                        budget.limits().counters[Counter::Work as usize] - used,
                    )
                    .unwrap();
                None
            } else {
                let before = budget.snapshot();
                let remaining = if mode == 1 {
                    backing - 1
                } else {
                    backing + retained - 1
                };
                Some(
                    budget
                        .reserve(
                            Storage::Temporary,
                            budget.limits().bytes
                                - before.retained_bytes
                                - before.temporary_bytes
                                - remaining,
                        )
                        .unwrap(),
                )
            };
            let before = budget.snapshot();
            let error = merged.link_crate_files(vec![file]).unwrap_err();
            let ScopeError::Token(TokenError::Budget(failure)) = error else {
                panic!("expected original resource failure: {error:?}")
            };
            assert_eq!(failure.kind, BudgetFailureKind::Limit);
            assert_eq!(
                failure.resource,
                if mode == 0 {
                    BudgetResource::Counter(Counter::Work)
                } else {
                    BudgetResource::Bytes
                }
            );
            if mode == 2 {
                assert_eq!(
                    failure.used,
                    before.retained_bytes + before.temporary_bytes + backing
                );
                assert_eq!(failure.requested, retained);
            }
            assert!(merged.arena.linked.is_empty());
            assert!(merged.arena.linked_sessions.is_empty());
            let after = budget.snapshot();
            assert_eq!(after.retained_bytes, before.retained_bytes);
            assert_eq!(after.temporary_bytes, before.temporary_bytes);
            assert_eq!(after.failure, Some(failure));
            drop(blocker);
        }
    }

    struct NestedFixture {
        names: NameStoreBuilder,
        original: SpanId,
        parent: (ExpansionId, Token),
        child: (ExpansionId, Token),
        sibling: (ExpansionId, Token),
        captured_parent: Token,
        captured_sibling: Token,
        stacked: Token,
        bad_call: ExpansionId,
    }
    fn nested_fixture() -> NestedFixture {
        let budget = ExpansionBudget::default();
        let sources = SourceSnapshot::from_text(
            Path::new("nested.rs"),
            "x",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let (mut session, session_cap) = TokenSession::standalone(sources, budget).unwrap();
        let file = session.sources().file(0).unwrap();
        let original = session
            .original_span(session.sources().span(file, 0, 1).unwrap())
            .unwrap();
        let defining_crate = session.register_crate_root(file).unwrap();
        let root = session.root_context();
        let symbol = session.intern_symbol("x").unwrap();
        let parent_id = session
            .add_expansion(ExpansionRecord {
                definition: original,
                call: original,
                parent: None,
                defining_crate,
                edition: Edition::E2024,
            })
            .unwrap();
        let parent_context = session
            .apply_mark(root, parent_id, Transparency::SemiTransparent)
            .unwrap();
        let parent_token = Token {
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context: parent_context,
            }),
            span: session
                .reissue_span(original, parent_context, parent_id)
                .unwrap(),
        };
        let child_id = session
            .add_expansion(ExpansionRecord {
                definition: original,
                call: parent_token.span,
                parent: Some(parent_id),
                defining_crate,
                edition: Edition::E2024,
            })
            .unwrap();
        let child_context = session
            .apply_mark(root, child_id, Transparency::SemiTransparent)
            .unwrap();
        let child_token = Token {
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context: child_context,
            }),
            span: session
                .reissue_span(original, child_context, child_id)
                .unwrap(),
        };
        let sibling_id = session
            .add_expansion(ExpansionRecord {
                definition: original,
                call: original,
                parent: None,
                defining_crate,
                edition: Edition::E2024,
            })
            .unwrap();
        let sibling_context = session
            .apply_mark(root, sibling_id, Transparency::SemiTransparent)
            .unwrap();
        let sibling_token = Token {
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context: sibling_context,
            }),
            span: session
                .reissue_span(original, sibling_context, sibling_id)
                .unwrap(),
        };
        let captured_parent = Token {
            kind: parent_token.kind,
            span: session
                .reissue_span(parent_token.span, parent_context, child_id)
                .unwrap(),
        };
        let captured_sibling = Token {
            kind: sibling_token.kind,
            span: session
                .reissue_span(sibling_token.span, sibling_context, child_id)
                .unwrap(),
        };
        let stacked_context = session
            .apply_mark(parent_context, child_id, Transparency::SemiTransparent)
            .unwrap();
        let stacked = Token {
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context: stacked_context,
            }),
            span: session
                .reissue_span(original, stacked_context, child_id)
                .unwrap(),
        };
        let bad_call = session
            .add_expansion(ExpansionRecord {
                definition: original,
                call: original,
                parent: Some(parent_id),
                defining_crate,
                edition: Edition::E2024,
            })
            .unwrap();
        let names = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        NestedFixture {
            names,
            original,
            parent: (parent_id, parent_token),
            child: (child_id, child_token),
            sibling: (sibling_id, sibling_token),
            captured_parent,
            captured_sibling,
            stacked,
            bad_call,
        }
    }

    #[test]
    fn nested_capture_keeps_outer_binder_and_inner_literal_keeps_definition_binding() {
        let mut f = nested_fixture();
        let mut scopes = ScopeArenaBuilder::for_names(
            &f.names,
            crate::compiler::owner_path::standalone_issued(
                f.names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let root = scopes.function_root(&f.names).unwrap();
        let x = f.names.intern_root("x", f.original).unwrap();
        let (definition, source_binding) = scopes.bind(&f.names, root, x).unwrap();
        let parent = scopes
            .anchor_expansion(&f.names, f.parent.0, definition, definition)
            .unwrap();
        let block = scopes.enter(&f.names, definition).unwrap();
        let outer_binder = f
            .names
            .admit_expanded_token(f.parent.1, Edition::E2024, &parent)
            .unwrap();
        let (call, outer_binding) = scopes.bind(&f.names, block, outer_binder).unwrap();
        let child = scopes
            .anchor_nested_expansion(&f.names, f.child.0, definition, call, &parent)
            .unwrap();
        let capture = f
            .names
            .admit_expanded_token(f.captured_parent, Edition::E2024, &child)
            .unwrap();
        let literal = f
            .names
            .admit_expanded_token(f.child.1, Edition::E2024, &child)
            .unwrap();
        assert_eq!(
            scopes.resolve_reference(&f.names, call, capture).unwrap(),
            Some(outer_binding)
        );
        assert_eq!(
            scopes.resolve_reference(&f.names, call, literal).unwrap(),
            Some(source_binding)
        );
        assert_eq!(
            scopes
                .anchor_nested_expansion(&f.names, f.child.0, definition, call, &parent)
                .unwrap(),
            child
        );
    }

    #[test]
    fn ancestor_context_never_admits_old_occurrence_sibling_or_stacked_context() {
        let f = nested_fixture();
        let mut scopes = ScopeArenaBuilder::for_names(
            &f.names,
            crate::compiler::owner_path::standalone_issued(
                f.names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let root = scopes.function_root(&f.names).unwrap();
        let parent = scopes
            .anchor_expansion(&f.names, f.parent.0, root, root)
            .unwrap();
        scopes
            .anchor_expansion(&f.names, f.sibling.0, root, root)
            .unwrap();
        let child = scopes
            .anchor_nested_expansion(&f.names, f.child.0, root, root, &parent)
            .unwrap();
        child
            .validate_token(f.names.session(), f.captured_parent)
            .unwrap();
        child.validate_token(f.names.session(), f.child.1).unwrap();
        assert!(matches!(
            child.validate_token(f.names.session(), f.parent.1),
            Err(ScopeError::Token(TokenError::InvalidInput(_)))
        ));
        for token in [f.captured_sibling, f.stacked] {
            assert!(matches!(
                child.validate_token(f.names.session(), token),
                Err(ScopeError::UnsupportedProjection)
            ));
        }
    }

    #[test]
    fn nested_anchor_checks_exact_parent_arena_call_and_function() {
        let f = nested_fixture();
        let mut scopes = ScopeArenaBuilder::for_names(
            &f.names,
            crate::compiler::owner_path::standalone_issued(
                f.names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let root = scopes.function_root(&f.names).unwrap();
        let parent = scopes
            .anchor_expansion(&f.names, f.parent.0, root, root)
            .unwrap();
        let sibling = scopes
            .anchor_expansion(&f.names, f.sibling.0, root, root)
            .unwrap();
        assert!(matches!(
            scopes.anchor_nested_expansion(&f.names, f.child.0, root, root, &sibling),
            Err(ScopeError::ConflictingAssociation)
        ));
        assert!(matches!(
            scopes.anchor_nested_expansion(&f.names, f.bad_call, root, root, &parent),
            Err(ScopeError::Token(TokenError::InvalidInput(_)))
        ));
        let block = scopes.enter(&f.names, root).unwrap();
        let forged = ExpansionAdmission {
            call: block,
            ..parent
        };
        assert!(matches!(
            scopes.anchor_nested_expansion(&f.names, f.child.0, root, root, &forged),
            Err(ScopeError::ConflictingAssociation)
        ));
        let mut foreign = ScopeArenaBuilder::for_names(
            &f.names,
            crate::compiler::owner_path::standalone_issued(
                f.names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let foreign_root = foreign.function_root(&f.names).unwrap();
        assert!(matches!(
            foreign.anchor_nested_expansion(
                &f.names,
                f.child.0,
                foreign_root,
                foreign_root,
                &parent
            ),
            Err(ScopeError::ForeignOwner)
        ));
        let other_function = scopes.function_root(&f.names).unwrap();
        assert!(matches!(
            scopes.anchor_nested_expansion(
                &f.names,
                f.child.0,
                other_function,
                other_function,
                &parent
            ),
            Err(ScopeError::FunctionMismatch)
        ));
        let (foreign_names, _, _, _, _) = fixture();
        assert!(
            scopes
                .anchor_nested_expansion(&foreign_names, f.child.0, root, root, &parent)
                .is_err()
        );
        let budget = f.names.budget().clone();
        let failure = budget.charge(Counter::Work, u64::MAX).unwrap_err();
        assert!(
            matches!(scopes.anchor_nested_expansion(&f.names,f.child.0,root,root,&parent),Err(ScopeError::Token(TokenError::Budget(error))) if error==failure)
        );
    }

    #[test]
    fn ancestor_validation_work_is_charged_before_chain_read() {
        let f = nested_fixture();
        let mut scopes = ScopeArenaBuilder::for_names(
            &f.names,
            crate::compiler::owner_path::standalone_issued(
                f.names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let root = scopes.function_root(&f.names).unwrap();
        let parent = scopes
            .anchor_expansion(&f.names, f.parent.0, root, root)
            .unwrap();
        let child = scopes
            .anchor_nested_expansion(&f.names, f.child.0, root, root, &parent)
            .unwrap();
        let budget = f.names.budget();
        let used = budget.snapshot().counters[Counter::Work as usize];
        let limit = budget.limits().counters[Counter::Work as usize];
        // Span validation consumes the final unit, before the first ancestry
        // visit must charge and latch its own failure instead of succeeding.
        budget.charge(Counter::Work, limit - used - 1).unwrap();
        assert!(matches!(
            child.validate_token(f.names.session(), f.captured_parent),
            Err(ScopeError::Token(TokenError::Budget(_)))
        ));
        assert!(budget.check().is_err());
    }

    #[test]
    fn nested_expansion_depth_limit_stays_at_configured_boundary() {
        use super::super::expansion_budget::{BudgetResource, ExpansionLimits};
        let budget = ExpansionBudget::new(ExpansionLimits {
            depths: [512, 256, 2],
            ..ExpansionLimits::default()
        });
        let source = SourceSnapshot::from_text(
            Path::new("depth.rs"),
            "x",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let file = source.file(0).unwrap();
        let original = source.span(file, 0, 1).unwrap();
        let mut session = TokenSession::new(source, budget.clone()).unwrap();
        let span = session.original_span(original).unwrap();
        let defining_crate = session.register_crate_root(file).unwrap();
        let record = ExpansionRecord {
            definition: span,
            call: span,
            parent: None,
            defining_crate,
            edition: Edition::E2024,
        };
        let first = session.add_expansion(record).unwrap();
        let second = session
            .add_expansion(ExpansionRecord {
                parent: Some(first),
                ..record
            })
            .unwrap();
        let failure = match session
            .add_expansion(ExpansionRecord {
                parent: Some(second),
                ..record
            })
            .unwrap_err()
        {
            TokenError::Budget(failure) => failure,
            error => panic!("expected expansion depth limit: {error}"),
        };
        assert_eq!(failure.resource, BudgetResource::Depth(Depth::Expansion));
        assert_eq!(failure.requested, 3);
        assert_eq!(failure.limit, 2);
        assert_eq!(budget.check().unwrap_err(), failure);
        drop(session);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn mixed_site_locals_select_definition_and_captures_select_current_frontier() {
        let (mut names, span, expansion, literal, capture) = fixture();
        let mut scopes = ScopeArenaBuilder::for_names(
            &names,
            crate::compiler::owner_path::standalone_issued(
                names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let root = scopes.function_root(&names).unwrap();
        let definition_x = names.intern_root("x", span).unwrap();
        let (definition, original) = scopes.bind(&names, root, definition_x).unwrap();
        let child = scopes.enter(&names, definition).unwrap();
        let caller_x = names.intern_root("x", span).unwrap();
        let (call, shadow) = scopes.bind(&names, child, caller_x).unwrap();
        let proof = scopes
            .anchor_expansion(&names, expansion, definition, call)
            .unwrap();
        let def_use = names
            .admit_expanded_token(literal, Edition::E2024, &proof)
            .unwrap();
        let captured_use = names
            .admit_expanded_token(capture, Edition::E2024, &proof)
            .unwrap();
        // Labels are fully hygienic: a macro-internal label keeps its mark
        // context and can never meet the caller's same-named label (E0426),
        // while a captured `$l:lifetime` label keys exactly as the caller's.
        let def_label = names.key(def_use, LookupClass::Label).unwrap();
        assert_ne!(def_label, names.key(caller_x, LookupClass::Label).unwrap());
        assert_ne!(
            def_label,
            names.key(captured_use, LookupClass::Label).unwrap()
        );
        // Parameter names now have the same invocation-site textual lookup as
        // items. That projection must not erase local-variable hygiene, and it
        // does not grant the unsupported label projection any authority.
        for class in [LookupClass::TypeParameter, LookupClass::LifetimeParameter] {
            assert_eq!(
                names.key(def_use, class).unwrap(),
                names.key(captured_use, class).unwrap()
            );
            assert_eq!(
                names.key(def_use, class).unwrap(),
                names.key(caller_x, class).unwrap()
            );
        }
        assert_ne!(
            names.key(def_use, LookupClass::Local).unwrap(),
            names.key(captured_use, LookupClass::Local).unwrap()
        );
        assert_eq!(
            scopes.resolve_reference(&names, call, def_use).unwrap(),
            Some(original)
        );
        assert_eq!(
            scopes
                .resolve_reference(&names, call, captured_use)
                .unwrap(),
            Some(shadow)
        );
        assert_ne!(original, shadow);
        let item = scopes
            .project_reference(
                &names,
                def_use,
                LookupClass::Item(super::super::semantic_name::Namespace::Value),
            )
            .unwrap();
        assert_eq!(item.frontier, call);
        assert_eq!(
            item.key,
            names
                .key(
                    caller_x,
                    LookupClass::Item(super::super::semantic_name::Namespace::Value)
                )
                .unwrap()
        );
        assert!(matches!(
            scopes.project_reference(&names, def_use, LookupClass::Label),
            Err(ScopeError::UnsupportedProjection)
        ));
    }

    #[test]
    fn introduced_captured_and_marked_binders_are_seen_before_anchor_projection() {
        let (mut names, span, expansion, literal, capture) = fixture();
        let mut scopes = ScopeArenaBuilder::for_names(
            &names,
            crate::compiler::owner_path::standalone_issued(
                names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let definition = scopes.function_root(&names).unwrap();
        let original_x = names.intern_root("x", span).unwrap();
        let (call, _) = scopes.bind(&names, definition, original_x).unwrap();
        let proof = scopes
            .anchor_expansion(&names, expansion, definition, call)
            .unwrap();
        let block = scopes.enter(&names, call).unwrap();
        let captured_binder = names
            .admit_expanded_token(capture, Edition::E2024, &proof)
            .unwrap();
        let (captured_frontier, captured_binding) =
            scopes.bind(&names, block, captured_binder).unwrap();
        let captured_use = names
            .admit_expanded_token(capture, Edition::E2024, &proof)
            .unwrap();
        assert_eq!(
            scopes
                .resolve_reference(&names, captured_frontier, captured_use)
                .unwrap(),
            Some(captured_binding)
        );
        let marked_binder = names
            .admit_expanded_token(literal, Edition::E2024, &proof)
            .unwrap();
        let (marked_frontier, marked_binding) = scopes
            .bind(&names, captured_frontier, marked_binder)
            .unwrap();
        let marked_use = names
            .admit_expanded_token(literal, Edition::E2024, &proof)
            .unwrap();
        assert_eq!(
            scopes
                .resolve_reference(&names, marked_frontier, marked_use)
                .unwrap(),
            Some(marked_binding)
        );
        assert_ne!(marked_binding, captured_binding);
    }

    #[test]
    fn definition_frontier_cannot_see_later_call_site_binding() {
        let (mut names, span, expansion, literal, _) = fixture();
        let mut scopes = ScopeArenaBuilder::for_names(
            &names,
            crate::compiler::owner_path::standalone_issued(
                names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let definition = scopes.function_root(&names).unwrap();
        let later = names.intern_root("x", span).unwrap();
        let (call, _) = scopes.bind(&names, definition, later).unwrap();
        let proof = scopes
            .anchor_expansion(&names, expansion, definition, call)
            .unwrap();
        let reference = names
            .admit_expanded_token(literal, Edition::E2024, &proof)
            .unwrap();
        assert_eq!(
            scopes.resolve_reference(&names, call, reference).unwrap(),
            None
        );
    }

    #[test]
    fn public_name_gate_and_unregistered_function_anchor_stay_closed() {
        let (mut names, _, expansion, literal, capture) = fixture();
        assert!(matches!(
            names.from_token(literal, Edition::E2024),
            Err(NameError::Unsupported(UnsupportedName::NonRootContext))
        ));
        let mut scopes = ScopeArenaBuilder::for_names(
            &names,
            crate::compiler::owner_path::standalone_issued(
                names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let first = scopes.function_root(&names).unwrap();
        let second = scopes.function_root(&names).unwrap();
        assert!(matches!(
            scopes.anchor_expansion(&names, expansion, first, second),
            Err(ScopeError::FunctionMismatch)
        ));
        let proof = scopes
            .anchor_expansion(&names, expansion, first, first)
            .unwrap();
        let name = names
            .admit_expanded_token(literal, Edition::E2024, &proof)
            .unwrap();
        let captured = names
            .admit_expanded_token(capture, Edition::E2024, &proof)
            .unwrap();
        let first_function = scopes.function(first).unwrap();
        let second_function = scopes.function(second).unwrap();
        scopes
            .check_occurrence(&names, captured, Some(first_function))
            .unwrap();
        assert!(matches!(
            scopes.check_occurrence(&names, captured, Some(second_function)),
            Err(ScopeError::FunctionMismatch)
        ));
        assert!(matches!(
            scopes.check_occurrence(&names, captured, None),
            Err(ScopeError::FunctionMismatch)
        ));
        assert!(matches!(
            scopes.resolve_reference(&names, second, captured),
            Err(ScopeError::FunctionMismatch)
        ));
        assert!(matches!(
            scopes.bind(&names, second, captured),
            Err(ScopeError::FunctionMismatch)
        ));
        assert!(matches!(
            scopes.resolve_reference(&names, second, name),
            Err(ScopeError::FunctionMismatch)
        ));
        let mut other = ScopeArenaBuilder::for_names(
            &names,
            crate::compiler::owner_path::standalone_issued(
                names.budget(),
                crate::compiler::owner_path::OwnerDomain::ScopeArena,
            )
            .unwrap(),
        )
        .unwrap();
        let foreign_root = other.function_root(&names).unwrap();
        assert!(matches!(
            other.resolve_reference(&names, foreign_root, name),
            Err(ScopeError::UnregisteredExpansion)
        ));
        assert!(matches!(
            scopes.check_occurrence(&names, name, None),
            Err(ScopeError::FunctionMismatch)
        ));
    }
}
