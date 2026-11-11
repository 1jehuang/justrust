//! Existing cleanup occurrence correspondence, independent of new event insertion.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct CleanupMapping {
    pub(super) source_block: usize,
    pub(super) physical_block: usize,
    pub(super) origin: Option<ir::ExprId>,
    pub(super) source: Arc<cfg::ValidatedCleanupCall>,
    pub(super) physical: Arc<cfg::ValidatedCleanupCall>,
}

impl CleanupMapping {
    pub(super) fn matches_replayed(
        &self,
        other: &Self,
        budget: &ir::PlanBudget,
    ) -> Result<bool, String> {
        budget.charge_work(1)?;
        Ok(self.source_block == other.source_block
            && self.physical_block == other.physical_block
            && self.origin == other.origin
            && Arc::ptr_eq(&self.source, &other.source)
            && self.physical == other.physical)
    }

    pub(super) fn validate_candidate(
        &self,
        candidate: &cfg::Function,
        facts: &traits::ValidatedFacts,
    ) -> Result<(), String> {
        facts.budget().charge_work(1)?;
        let block = candidate
            .blocks
            .get(self.physical_block)
            .ok_or("physical cleanup mapping lost original block")?;
        let cfg::Terminator::Drop { cleanup, .. } = &block.terminator.value else {
            return Err("physical cleanup mapping lost original Drop event".into());
        };
        if block.terminator.origin != self.origin || !Arc::ptr_eq(cleanup, &self.physical) {
            return Err(
                "physical cleanup changed occurrence or exact descriptor allocation".into(),
            );
        }
        let owner = candidate.place_ty_with_budget(cleanup.owner(), Some(facts.budget()))?;
        cleanup
            .destructor()
            .validate(&owner, facts)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))
    }
}
