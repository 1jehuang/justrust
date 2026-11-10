//! Allocation ownership for the same retained and temporary topology walks.
use super::{Error, PlanBudget};
use crate::compiler::ir::PlanReservation;

pub(super) struct Allocations<'a> {
    budget: &'a PlanBudget,
    leases: Option<Leases>,
}

// Buffer values are destroyed by the caller before this owner. The guard
// inventory itself is destroyed before its metadata allocation admissions.
#[derive(Default)]
struct Leases {
    values: Vec<PlanReservation>,
    allocation: Option<PlanReservation>,
    rounding: Option<PlanReservation>,
}
impl Leases {
    fn record(&mut self, lease: PlanReservation, budget: &PlanBudget) -> Result<(), Error> {
        if self.values.len() == self.values.capacity() {
            let capacity = self
                .values
                .capacity()
                .max(2)
                .checked_mul(2)
                .ok_or_else(|| Error::new("topology lease inventory overflow"))?;
            let requested = bytes::<PlanReservation>(capacity)?;
            let allocation = budget.reserve_bytes(requested).map_err(Error::new)?;
            self.values
                .try_reserve_exact(capacity - self.values.len())
                .map_err(|_| Error::new("topology lease inventory allocation failed"))?;
            let actual = bytes::<PlanReservation>(self.values.capacity())?;
            let rounding = if actual > requested {
                Some(
                    budget
                        .reserve_bytes(actual - requested)
                        .map_err(Error::new)?,
                )
            } else {
                None
            };
            self.allocation = Some(allocation);
            self.rounding = rounding;
        }
        self.values.push(lease);
        Ok(())
    }
}
fn bytes<T>(count: usize) -> Result<usize, Error> {
    count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or_else(|| Error::new("nominal component storage overflow"))
}
impl<'a> Allocations<'a> {
    pub(super) fn retained(budget: &'a PlanBudget) -> Self {
        Self {
            budget,
            leases: None,
        }
    }
    pub(super) fn scoped(budget: &'a PlanBudget) -> Self {
        Self {
            budget,
            leases: Some(Leases::default()),
        }
    }
    pub(super) fn storage<T>(&mut self, count: usize) -> Result<Vec<T>, Error> {
        let Some(leases) = &mut self.leases else {
            return super::storage(count, self.budget);
        };
        let requested = bytes::<T>(count)?;
        let lease = self.budget.reserve_bytes(requested).map_err(Error::new)?;
        // Keep the admission in the owner before any payload allocation.
        leases.record(lease, self.budget)?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(count)
            .map_err(|_| Error::new("nominal component allocation failed"))?;
        let actual = bytes::<T>(result.capacity())?;
        if actual > requested {
            let rounding = self
                .budget
                .reserve_bytes(actual - requested)
                .map_err(Error::new)?;
            leases.record(rounding, self.budget)?;
        }
        Ok(result)
    }
    pub(super) fn grow<T>(&mut self, values: &mut Vec<T>) -> Result<(), Error> {
        let Some(leases) = &mut self.leases else {
            self.budget
                .retain_bytes(std::mem::size_of::<T>())
                .map_err(Error::new)?;
            return values
                .try_reserve_exact(1)
                .map_err(|_| Error::new("nominal topology allocation failed"));
        };
        if values.len() < values.capacity() {
            return Ok(());
        }
        let count = values
            .len()
            .checked_add(1)
            .ok_or_else(|| Error::new("nominal topology growth overflow"))?;
        let requested = bytes::<T>(count)?;
        let lease = self.budget.reserve_bytes(requested).map_err(Error::new)?;
        leases.record(lease, self.budget)?;
        // Earlier buffer admissions remain conservatively live until this
        // whole walk drops. In particular, old and new reallocations overlap.
        values
            .try_reserve_exact(1)
            .map_err(|_| Error::new("nominal topology allocation failed"))?;
        let actual = bytes::<T>(values.capacity())?;
        if actual > requested {
            let rounding = self
                .budget
                .reserve_bytes(actual - requested)
                .map_err(Error::new)?;
            leases.record(rounding, self.budget)?;
        }
        Ok(())
    }
}
