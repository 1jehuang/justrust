//! Exact, closed compiler-derived member admissions. These are not source tokens.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DerivedMemberKind {
    Clone,
    Debug,
}
impl DerivedMemberKind {
    pub(crate) fn derive_name(self) -> &'static str {
        match self {
            Self::Clone => "Clone",
            Self::Debug => "Debug",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DerivedMemberAdmission {
    owner: NonZeroU64,
    index: u32,
    metadata: ItemMetadataId,
    expansion: ItemExpansionAdmission,
    derive: SpanId,
    nominal: SpanId,
    kind: DerivedMemberKind,
}
impl DerivedMemberAdmission {
    pub(crate) fn kind(&self) -> DerivedMemberKind {
        self.kind
    }
    pub(crate) fn anchor(&self, names: &impl NameAccess) -> Result<SpanId, ScopeError> {
        self.expansion.check_names(names)?;
        self.expansion.validate_span(names.session(), self.derive)?;
        Ok(self.derive)
    }
}

#[derive(Default)]
pub(super) struct State {
    admissions: Arc<[DerivedMemberAdmission]>,
}
pub(super) struct Builder {
    admissions: BudgetVec<DerivedMemberAdmission>,
}
impl std::fmt::Debug for Builder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DerivedMemberBuilder")
            .field("members", &self.admissions.len())
            .finish()
    }
}
impl Builder {
    pub(super) fn new(budget: &ExpansionBudget) -> Result<Self, ScopeError> {
        Ok(Self {
            admissions: BudgetVec::new(budget, Storage::Retained)?,
        })
    }
    pub(super) fn freeze(self, lease: &mut ByteLease) -> Result<State, ScopeError> {
        if self.admissions.len() == 0 {
            return Ok(State::default());
        }
        let bytes = self
            .admissions
            .len()
            .checked_mul(size_of::<DerivedMemberAdmission>())
            .and_then(|bytes| bytes.checked_add(2 * size_of::<usize>()))
            .ok_or(TokenError::IdExhausted)?;
        lease.resize(
            lease
                .bytes()
                .checked_add(bytes as u64)
                .ok_or(TokenError::IdExhausted)?,
        )?;
        Ok(State {
            admissions: self.admissions.as_slice().into(),
        })
    }
}

impl ScopeArenaBuilder {
    pub(crate) fn admit_derived_member(
        &mut self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        source: (ItemMetadataId, &ItemExpansionAdmission),
        derive: AstIdent,
        nominal: SpanId,
        kind: DerivedMemberKind,
    ) -> Result<DerivedMemberAdmission, ScopeError> {
        let (metadata, expansion) = source;
        self.check_names(names)?;
        expansion.check_modules(names, modules)?;
        self.arena.check_item_proof(expansion)?;
        self.arena.check_item_open(expansion)?;
        if metadata.owner != self.arena.owner || names.spelling(derive)? != kind.derive_name() {
            return Err(ScopeError::ConflictingAssociation);
        }
        let derive = names.span(derive)?;
        let state = &mut self.metadata_builder;
        let binding = state
            .bindings
            .as_slice()
            .get(metadata.index as usize)
            .ok_or(ScopeError::ConflictingAssociation)?;
        if binding.proof != *expansion {
            return Err(ScopeError::ConflictingAssociation);
        }
        let end = binding
            .start
            .checked_add(binding.count)
            .ok_or(TokenError::IdExhausted)?;
        let spans = state
            .spans
            .as_slice()
            .get(binding.start..end)
            .ok_or(ScopeError::ConflictingAssociation)?;
        names.budget().charge(Counter::Work, spans.len() as u64)?;
        if !spans.contains(&derive) {
            return Err(ScopeError::ConflictingAssociation);
        }
        expansion.validate_span(names.session(), nominal)?;
        let index =
            u32::try_from(state.members.admissions.len()).map_err(|_| TokenError::IdExhausted)?;
        let admission = DerivedMemberAdmission {
            owner: self.arena.owner,
            index,
            metadata,
            expansion: *expansion,
            derive,
            nominal,
            kind,
        };
        state.members.admissions.push(admission)?;
        Ok(admission)
    }
}
impl ScopeArena {
    pub(crate) fn validate_derived_member(
        &self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        admission: &DerivedMemberAdmission,
        metadata: ItemMetadataId,
        derive: SpanId,
        nominal: SpanId,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        admission.expansion.check_modules(names, modules)?;
        self.check_item_proof(&admission.expansion)?;
        if admission.owner != self.owner || metadata.owner != self.owner {
            return Err(ScopeError::ForeignOwner);
        }
        let original = self
            .items
            .metadata
            .members
            .admissions
            .get(admission.index as usize)
            .ok_or(ScopeError::ConflictingAssociation)?;
        if original != admission
            || original.metadata != metadata
            || original.derive != derive
            || original.nominal != nominal
        {
            return Err(ScopeError::ConflictingAssociation);
        }
        Ok(())
    }
}
