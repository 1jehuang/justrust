//! Task-local construction storage. Only immutable values and payer receipts
//! survive the one-time move into the declaring context, never a live ledger.
use super::*;
use std::mem::size_of;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct ReductionRows {
    pub(super) values: Vec<DeclaredEqualityReduction>,
    storage: Option<ir::RetainedStorage>,
}
impl ReductionRows {
    fn bytes(capacity: usize, budget: &PlanBudget) -> Result<usize> {
        capacity
            .checked_mul(size_of::<DeclaredEqualityReduction>())
            .ok_or_else(|| FactsError::resource(budget, "declared reduction capacity overflow"))
    }
    pub(super) fn check(&self, budget: &PlanBudget) -> Result<()> {
        if budget.has_resource_failure() {
            return Err(FactsError::resource(
                budget,
                "declared reduction budget already exhausted",
            ));
        }
        let bytes = Self::bytes(self.values.capacity(), budget)?;
        if self.storage.as_ref().is_some_and(|receipt| {
            !receipt.authority().same_authority(budget.authority()) || receipt.bytes() < bytes
        }) || (bytes != 0 && self.storage.is_none())
        {
            return Err(FactsError::invalid(
                budget,
                "declared reduction storage authority mismatch",
            ));
        }
        Ok(())
    }
    // Validate and pay every fallible step before moving an original row.
    // The old allocation and incoming sink remain paid while the full new
    // allocation is admitted, covering relocation/append overlap explicitly.
    fn commit_replacement(
        &mut self,
        mut replacement: Vec<DeclaredEqualityReduction>,
        lease: ir::PlanReservation,
        budget: &PlanBudget,
    ) -> Result<()> {
        self.check(budget)?;
        if !replacement.is_empty() || replacement.capacity() < self.values.len() {
            return Err(FactsError::invalid(
                budget,
                "invalid declared reduction replacement",
            ));
        }
        let bytes = Self::bytes(replacement.capacity(), budget)?;
        let receipt = charged(budget, lease.retain())?;
        if !receipt.authority().same_authority(budget.authority()) || receipt.bytes() < bytes {
            return Err(FactsError::invalid(
                budget,
                "underpaid declared reduction replacement",
            ));
        }
        charged(budget, budget.charge_work(self.values.len()))?;
        replacement.append(&mut self.values);
        self.values = replacement;
        self.storage = Some(receipt);
        Ok(())
    }
    fn grow(&mut self, needed: usize, budget: &PlanBudget) -> Result<()> {
        self.check(budget)?;
        if needed <= self.values.capacity() {
            return Ok(());
        }
        let capacity = needed
            .checked_next_power_of_two()
            .ok_or_else(|| FactsError::resource(budget, "declared reduction capacity overflow"))?;
        let lease = charged(budget, budget.reserve_bytes(Self::bytes(capacity, budget)?))?;
        let mut replacement = Vec::new();
        replacement
            .try_reserve_exact(capacity)
            .map_err(|_| FactsError::resource(budget, "declared reduction allocation failed"))?;
        self.commit_replacement(replacement, lease, budget)
    }
    pub(super) fn push(
        &mut self,
        value: DeclaredEqualityReduction,
        budget: &PlanBudget,
    ) -> Result<()> {
        self.check(budget)?;
        charged(budget, budget.charge_work(1))?;
        let needed =
            self.values.len().checked_add(1).ok_or_else(|| {
                FactsError::resource(budget, "declared reduction length overflow")
            })?;
        self.grow(needed, budget)?;
        self.values.push(value);
        Ok(())
    }
    pub(super) fn append(&mut self, mut incoming: Self, budget: &PlanBudget) -> Result<()> {
        self.check(budget)?;
        incoming.check(budget)?;
        charged(budget, budget.charge_work(1))?;
        if incoming.values.is_empty() {
            return Ok(());
        }
        if self.values.is_empty() && self.values.capacity() == 0 {
            *self = incoming;
            return Ok(());
        }
        let needed = self
            .values
            .len()
            .checked_add(incoming.values.len())
            .ok_or_else(|| FactsError::resource(budget, "declared reduction length overflow"))?;
        charged(budget, budget.charge_work(incoming.values.len()))?;
        self.grow(needed, budget)?;
        self.values.append(&mut incoming.values);
        Ok(())
    }
}
