//! Exact parser-issued metadata-to-item associations in the frozen scope arena.
use super::*;
#[path = "derived_members.rs"]
mod members;
pub(crate) use members::{DerivedMemberAdmission, DerivedMemberKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ItemMetadataId {
    owner: NonZeroU64,
    index: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ItemDerivedCopyId {
    owner: NonZeroU64,
    index: u32,
}
#[derive(Clone, Debug)]
struct CopyBinding {
    metadata: ItemMetadataId,
    derive: SpanId,
    nominal: SpanId,
}

#[derive(Clone, Debug)]
struct Binding {
    proof: ItemExpansionAdmission,
    scope: ModuleId,
    item: SpanId,
    start: usize,
    count: usize,
}

pub(in crate::compiler::semantic_scope) struct State {
    bindings: Arc<[Binding]>,
    spans: Arc<[SpanId]>,
    copies: Arc<[CopyBinding]>,
    members: members::State,
}
impl Default for State {
    fn default() -> Self {
        Self {
            bindings: Arc::from([]),
            spans: Arc::from([]),
            copies: Arc::from([]),
            members: members::State::default(),
        }
    }
}

pub(in crate::compiler::semantic_scope) struct Builder {
    bindings: BudgetVec<Binding>,
    spans: BudgetVec<SpanId>,
    copies: BudgetVec<CopyBinding>,
    members: members::Builder,
}
impl std::fmt::Debug for Builder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataBuilder")
            .field("bindings", &self.bindings.len())
            .field("spans", &self.spans.len())
            .field("copies", &self.copies.len())
            .field("members", &self.members)
            .finish()
    }
}
impl Builder {
    pub(in crate::compiler::semantic_scope) fn new(
        budget: &ExpansionBudget,
    ) -> Result<Self, ScopeError> {
        Ok(Self {
            bindings: BudgetVec::new(budget, Storage::Retained)?,
            spans: BudgetVec::new(budget, Storage::Retained)?,
            copies: BudgetVec::new(budget, Storage::Retained)?,
            members: members::Builder::new(budget)?,
        })
    }
    pub(in crate::compiler::semantic_scope) fn freeze(
        self,
        lease: &mut ByteLease,
    ) -> Result<State, ScopeError> {
        if self.bindings.len() == 0 {
            return Ok(State::default());
        }
        let bytes = self
            .bindings
            .len()
            .checked_mul(size_of::<Binding>())
            .and_then(|n| {
                self.spans
                    .len()
                    .checked_mul(size_of::<SpanId>())
                    .and_then(|s| n.checked_add(s))
            })
            .and_then(|n| {
                self.copies
                    .len()
                    .checked_mul(size_of::<CopyBinding>())
                    .and_then(|s| n.checked_add(s))
            })
            .and_then(|n| n.checked_add(6 * size_of::<usize>()))
            .ok_or(TokenError::IdExhausted)?;
        lease.resize(
            lease
                .bytes()
                .checked_add(bytes as u64)
                .ok_or(TokenError::IdExhausted)?,
        )?;
        Ok(State {
            bindings: self.bindings.as_slice().into(),
            spans: self.spans.as_slice().into(),
            copies: self.copies.as_slice().into(),
            members: self.members.freeze(lease)?,
        })
    }
}

impl ScopeArenaBuilder {
    pub(crate) fn seal_item_copy(
        &mut self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        metadata: ItemMetadataId,
        proof: &ItemExpansionAdmission,
        derive: SpanId,
        nominal: SpanId,
    ) -> Result<ItemDerivedCopyId, ScopeError> {
        self.check_names(names)?;
        proof.check_modules(names, modules)?;
        self.arena.check_item_proof(proof)?;
        self.arena.check_item_open(proof)?;
        if metadata.owner != self.arena.owner {
            return Err(ScopeError::ForeignOwner);
        }
        let state = &mut self.metadata_builder;
        let binding = state
            .bindings
            .as_slice()
            .get(metadata.index as usize)
            .ok_or(ScopeError::ConflictingAssociation)?;
        if binding.proof != *proof {
            return Err(ScopeError::ConflictingAssociation);
        }
        let end = binding
            .start
            .checked_add(binding.count)
            .ok_or(TokenError::IdExhausted)?;
        let attributes = state
            .spans
            .as_slice()
            .get(binding.start..end)
            .ok_or(ScopeError::ConflictingAssociation)?;
        names
            .budget()
            .charge(Counter::Work, attributes.len() as u64)?;
        if !attributes.contains(&derive) {
            return Err(ScopeError::ConflictingAssociation);
        }
        proof.validate_span(names.session(), nominal)?;
        let index = u32::try_from(state.copies.len()).map_err(|_| TokenError::IdExhausted)?;
        state.copies.push(CopyBinding {
            metadata,
            derive,
            nominal,
        })?;
        Ok(ItemDerivedCopyId {
            owner: self.arena.owner,
            index,
        })
    }
    pub(crate) fn seal_item_metadata(
        &mut self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        proof: &ItemExpansionAdmission,
        scope: ModuleId,
        item: SpanId,
        attributes: &[SpanId],
    ) -> Result<ItemMetadataId, ScopeError> {
        self.check_names(names)?;
        proof.check_modules(names, modules)?;
        self.arena.check_item_proof(proof)?;
        self.arena.check_item_open(proof)?;
        if scope != proof.target_module || attributes.is_empty() {
            return Err(ScopeError::ConflictingAssociation);
        }
        proof.validate_span(names.session(), item)?;
        for span in attributes {
            proof.validate_span(names.session(), *span)?;
        }
        let state = &mut self.metadata_builder;
        let index = u32::try_from(state.bindings.len()).map_err(|_| TokenError::IdExhausted)?;
        let start = state.spans.len();
        for span in attributes {
            state.spans.push(*span)?;
        }
        state.bindings.push(Binding {
            proof: *proof,
            scope,
            item,
            start,
            count: attributes.len(),
        })?;
        Ok(ItemMetadataId {
            owner: self.arena.owner,
            index,
        })
    }
}

impl ScopeArena {
    pub(crate) fn validate_item_copy(
        &self,
        names: &impl NameAccess,
        id: ItemDerivedCopyId,
        metadata: ItemMetadataId,
        derive: SpanId,
        nominal: SpanId,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        if id.owner != self.owner || metadata.owner != self.owner {
            return Err(ScopeError::ForeignOwner);
        }
        let binding = self
            .items
            .metadata
            .copies
            .get(id.index as usize)
            .ok_or(ScopeError::ConflictingAssociation)?;
        if binding.metadata != metadata || binding.derive != derive || binding.nominal != nominal {
            return Err(ScopeError::ConflictingAssociation);
        }
        Ok(())
    }
    pub(crate) fn validate_item_metadata(
        &self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        id: ItemMetadataId,
        proof: &ItemExpansionAdmission,
        scope: ModuleId,
        item: SpanId,
        attributes: &[SpanId],
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        proof.check_modules(names, modules)?;
        self.check_item_proof(proof)?;
        if id.owner != self.owner {
            return Err(ScopeError::ForeignOwner);
        }
        let state = &self.items.metadata;
        let binding = state
            .bindings
            .get(id.index as usize)
            .ok_or(ScopeError::ConflictingAssociation)?;
        if binding.proof != *proof
            || binding.scope != scope
            || binding.item != item
            || binding.count != attributes.len()
        {
            return Err(ScopeError::ConflictingAssociation);
        }
        let end = binding
            .start
            .checked_add(binding.count)
            .ok_or(TokenError::IdExhausted)?;
        let original = state
            .spans
            .get(binding.start..end)
            .ok_or(ScopeError::ConflictingAssociation)?;
        for (original, supplied) in original.iter().zip(attributes) {
            self.budget.charge(Counter::Work, 1)?;
            if original != supplied {
                return Err(ScopeError::ConflictingAssociation);
            }
        }
        Ok(())
    }
}
