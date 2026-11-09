use super::*;

/// Scratch of one source reconstruction, never retained by a compiler fact.
/// The receipt's owning seal is checked on every lookup. Retained immutable
/// allocations, not nominal IDs or structurally equal types, identify a hit.
#[derive(Default)]
pub(in crate::compiler::frontend) struct ParentLocalReplayMemo {
    entries: Vec<(Arc<ParentLocalReceipt>, ir::Ty, ir::Ty, ir::PlanReservation)>,
    metadata: tr::TraitScratch,
}

impl std::fmt::Debug for ParentLocalReplayMemo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParentLocalReplayMemo")
            .field("entries", &self.entries)
            .finish_non_exhaustive()
    }
}

fn same_allocation(left: &ir::Ty, right: &ir::Ty) -> bool {
    match (left, right) {
        (ir::Ty::Struct(left), ir::Ty::Struct(right)) => std::ptr::eq(left.def(), right.def()),
        (ir::Ty::Enum(left), ir::Ty::Enum(right)) => std::ptr::eq(left.def(), right.def()),
        _ => false,
    }
}

impl ParentLocalReplayMemo {
    pub(in crate::compiler::frontend) fn replay(
        &mut self,
        receipt: &Arc<ParentLocalReceipt>,
        definition: &ir::Ty,
        original: &ir::Ty,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        receipt.validate_context(facts, budget)?;
        if budget.has_resource_failure() {
            return Err("generic planning budget poisoned".into());
        }
        budget.charge_work(1)?;
        for (previous_receipt, previous_definition, previous_original, _) in &self.entries {
            budget.charge_work(1)?;
            if Arc::ptr_eq(previous_receipt, receipt)
                && same_allocation(previous_definition, definition)
                && same_allocation(previous_original, original)
            {
                return Ok(());
            }
        }
        receipt.replay_with_scratch(definition, original, facts, budget, &mut self.metadata)?;
        let storage = budget.reserve_bytes(
            2 * std::mem::size_of::<(Arc<ParentLocalReceipt>, ir::Ty, ir::Ty, ir::PlanReservation)>(
            ),
        )?;
        budget.charge_work(self.entries.len())?;
        self.entries
            .try_reserve_exact(1)
            .map_err(|_| "parent-local replay memo allocation failed")?;
        if self.entries.capacity() > 2 * (self.entries.len() + 1) {
            return Err("parent-local replay memo exceeded its reservation".into());
        }
        self.entries.push((
            receipt.clone(),
            definition.clone(),
            original.clone(),
            storage,
        ));
        Ok(())
    }
}
