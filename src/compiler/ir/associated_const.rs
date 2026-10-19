//! Typed symbolic associated constants in universally checked source bodies.
//! Closed scalar execution is supplied only by graph member-use value facts.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssociatedConstOperand {
    caller: DeclId,
    path: Arc<[u32]>,
    schema: super::super::traits::ConstantMemberType,
}
impl AssociatedConstOperand {
    pub(in crate::compiler) fn new(
        caller: DeclId,
        path: Arc<[u32]>,
        schema: super::super::traits::ConstantMemberType,
        facts: &super::super::traits::ValidatedFacts,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        budget.check_depth(path.len())?;
        if caller != facts.declaration() || !budget.same_authority(facts.budget()) {
            return Err("associated constant operand has foreign caller/budget authority".into());
        }
        schema
            .validate_for(facts)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        budget.retain_bytes(std::mem::size_of::<Self>())?;
        Ok(Self {
            caller,
            path,
            schema,
        })
    }
    pub fn caller(&self) -> DeclId {
        self.caller
    }
    pub fn path(&self) -> &[u32] {
        &self.path
    }
    pub fn ty(&self) -> &Ty {
        self.schema.ty()
    }
    pub fn member(&self) -> DeclId {
        self.schema.member()
    }
    pub fn application(&self) -> Option<&TraitRef> {
        self.schema.application()
    }
    pub(in crate::compiler) fn validate_context(
        &self,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        match facts {
            FactsCtx::Validated(facts) if facts.phase() == super::super::traits::FactsPhase::Universal => self.validate(facts, budget),
            _ => Err("symbolic associated constant requires exact universal source authority; closed bodies require graph CTFE".into()),
        }
    }
    pub(in crate::compiler) fn validate(
        &self,
        facts: &super::super::traits::ValidatedFacts,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if self.caller != facts.declaration() || !budget.same_authority(facts.budget()) {
            return Err("associated constant operand changed source caller/budget".into());
        }
        self.schema
            .validate_for(facts)
            .map_err(|e| e.into_legacy_diagnostic(budget))
    }
}
