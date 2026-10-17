//! Source-order lexical bindings for the explicitly admitted local-macro slice.
//! Handles describe lexical identity, never source replay or metadata authority.
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::BuildHasher;
use std::mem::size_of;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use super::expansion_budget::{BudgetFailure, ByteLease, Counter, ExpansionBudget, Storage};
use super::expansion_span::ExpansionId;
use super::hygiene::{ContextData, Transparency};
use super::semantic_name::{self, AstIdent, LookupClass, NameError, NameKey};
use super::semantic_path::NameAccess;
use super::token::Token;
use super::token_session::{
    BudgetVec, SessionId, SpanId, SyntaxContextId, TokenError, TokenSession, allocate_identity,
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
            error => {
                let kind = match &error {
                    Self::UnsupportedProjection | Self::Name(NameError::Unsupported(_)) => {
                        SourceErrorKind::Unsupported
                    }
                    Self::Name(NameError::ExpectedIdentifier | NameError::InvalidIdentifier) => {
                        SourceErrorKind::Syntax
                    }
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
#[derive(Clone, Copy, Debug)]
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
        if session.id() != self.session {
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
        session.budget().charge(Counter::Work, 1)?;
        if session.id() != self.session {
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
            } if expansion == self.expansion
                && matches!(session.context(parent)?, ContextData::Root) =>
            {
                Ok(())
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
    names: SessionId,
    scopes: BudgetVec<ScopeRecord>,
    frontiers: BudgetVec<FrontierRecord>,
    bindings: BudgetVec<BindingRecord>,
    associations: BudgetVec<Association>,
    anchors: BudgetVec<ExpansionAdmission>,
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
    arena: ScopeArena,
}

impl ScopeArenaBuilder {
    pub fn for_names(names: &impl NameAccess) -> Result<Self, ScopeError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let budget = names.budget().clone();
        budget.check()?;
        let lease = budget.reserve(
            Storage::Retained,
            (size_of::<ScopeArena>() + 2 * size_of::<usize>()) as u64,
        )?;
        Ok(Self {
            arena: ScopeArena {
                owner: allocate_identity(&NEXT)?,
                names: names.session().id(),
                scopes: BudgetVec::new(&budget, Storage::Retained)?,
                frontiers: BudgetVec::new(&budget, Storage::Retained)?,
                bindings: BudgetVec::new(&budget, Storage::Retained)?,
                associations: BudgetVec::new(&budget, Storage::Retained)?,
                anchors: BudgetVec::new(&budget, Storage::Retained)?,
                association_index: Index::new(&budget, 0)?,
                anchor_index: Index::new(&budget, 0)?,
                hasher: RandomState::new(),
                budget,
                _lease: lease,
            },
        })
    }
    pub fn function_root(&mut self, names: &impl NameAccess) -> Result<FrontierId, ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        let scope = ScopeId {
            owner: self.arena.owner,
            index: self.arena.index(self.arena.scopes.len())?,
        };
        self.arena.scopes.push(ScopeRecord {
            parent: None,
            function: scope,
        })?;
        self.arena
            .push_frontier(FrontierRecord { scope, head: None })
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
        if let Some(previous) = self.arena.anchor(expansion)? {
            return if previous.definition == definition && previous.call == call {
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
    pub fn freeze(self) -> Result<Arc<ScopeArena>, ScopeError> {
        self.arena.budget.check()?;
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
        if names.session().id() != self.names || !self.budget.same_authority(names.budget()) {
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
    fn scope(&self, id: ScopeId) -> Result<ScopeRecord, ScopeError> {
        self.owner(id.owner)?;
        self.scopes
            .as_slice()
            .get(id.index as usize)
            .copied()
            .ok_or(ScopeError::InvalidHandle)
    }
    fn frontier(&self, id: FrontierId) -> Result<FrontierRecord, ScopeError> {
        self.owner(id.owner)?;
        self.frontiers
            .as_slice()
            .get(id.index as usize)
            .copied()
            .ok_or(ScopeError::InvalidHandle)
    }
    fn binding_record(&self, id: BindingId) -> Result<BindingRecord, ScopeError> {
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
        let at = self
            .association(occurrence)?
            .and_then(|row| row.reference.map(|(at, _)| at))
            .ok_or(ScopeError::UnrecordedOccurrence)?;
        self.function(at)
    }
    pub fn reference_frontier(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
    ) -> Result<FrontierId, ScopeError> {
        self.check_names(names)?;
        let occurrence = semantic_name::occurrence_index(names, ident)?;
        self.association(occurrence)?
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
        let hash = self.hasher.hash_one(expansion);
        Ok(self
            .find_index(&self.anchor_index, hash, |index| {
                self.anchors.as_slice()[index as usize].expansion == expansion
            })?
            .and_then(|slot| self.anchor_index.slots[slot])
            .map(|index| self.anchors.as_slice()[index as usize]))
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
        self.association(occurrence)?
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
        self.association(occurrence)?
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
                let anchor = self
                    .anchor(expansion)?
                    .ok_or(ScopeError::UnregisteredExpansion)?;
                if anchor.owner != self.owner || anchor.function != function {
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
        let sources = SourceSnapshot::from_text(Path::new("scope.rs"), "x", &budget).unwrap();
        let file = sources.file(0).unwrap();
        let original = sources.span(file, 0, 1).unwrap();
        let mut session = TokenSession::new(sources, budget).unwrap();
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
            NameStoreBuilder::new(session, Edition::E2024).unwrap(),
            span,
            expansion,
            literal,
            capture,
        )
    }

    #[test]
    fn mixed_site_locals_select_definition_and_captures_select_current_frontier() {
        let (mut names, span, expansion, literal, capture) = fixture();
        let mut scopes = ScopeArenaBuilder::for_names(&names).unwrap();
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
        for class in [
            LookupClass::Label,
            LookupClass::TypeParameter,
            LookupClass::LifetimeParameter,
        ] {
            assert!(matches!(
                names.key(def_use, class),
                Err(NameError::Unsupported(UnsupportedName::NonRootContext))
            ));
        }
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
        let mut scopes = ScopeArenaBuilder::for_names(&names).unwrap();
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
        let mut scopes = ScopeArenaBuilder::for_names(&names).unwrap();
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
        let mut scopes = ScopeArenaBuilder::for_names(&names).unwrap();
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
        let mut other = ScopeArenaBuilder::for_names(&names).unwrap();
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
