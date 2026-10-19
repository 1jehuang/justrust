//! Construction-final metadata, not a new semantic admission authority.
//! Payloads are moved from the trusted source constructor. Their upstream
//! schema/type/region allocations remain paid there. These receipts separately
//! pay container capacity and the frozen wrapper, with no ledger backreference.
#[cfg(test)]
mod tests;

use super::{ir, tr};
use std::mem::size_of;
use std::sync::Arc;

type DeclTable<V> = Vec<Option<(ir::DeclId, V)>>;
type AssignmentTable = Vec<((ir::DeclId, ir::DeclId), ir::Ty)>;

#[derive(Debug)]
pub(super) struct SourceSchemas {
    snapshot: ir::SnapshotId,
    functions: DeclTable<tr::GenericSchema>,
    region_modes: DeclTable<tr::RegionContractMode>,
    source_regions: DeclTable<Arc<ir::SourceRegionEnv>>,
    associated_type_rows: Vec<tr::AssociatedTypeValue>,
    associated_type_rhs: AssignmentTable,
    authority: ir::PlanAuthority,
    receipts: [Option<ir::RetainedStorage>; 6],
}

fn check_budget(budget: &ir::PlanBudget) -> Result<(), String> {
    if budget.has_resource_failure() {
        return Err("generic planning resource limit exceeded".into());
    }
    budget.charge_work(0)
}
fn check_receipt(receipt: Option<&ir::RetainedStorage>, budget: &ir::PlanBudget) -> Result<(), String> {
    if receipt.is_some_and(|r| !r.authority().same_authority(budget.authority())) {
        return Err("source schema budget authority mismatch".into());
    }
    Ok(())
}

// Allocate into an independent temporary, then validate its *actual* public
// capacity and immutable receipt before moving even one existing element.
// Every error leaves the reusable original buffer and its receipt unchanged.
fn commit_replacement<T>(
    values: &mut Vec<T>,
    receipt: &mut Option<ir::RetainedStorage>,
    mut replacement: Vec<T>,
    lease: ir::PlanReservation,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    check_budget(budget)?;
    if !replacement.is_empty() || replacement.capacity() < values.len() {
        return Err("contract: invalid schema replacement buffer".into());
    }
    let bytes = replacement.capacity().checked_mul(size_of::<T>()).ok_or("schema capacity overflow")?;
    let paid = lease.retain()?;
    check_receipt(Some(&paid), budget)?;
    if paid.bytes() < bytes {
        return Err("infrastructure: schema capacity exceeds prepaid storage".into());
    }
    budget.charge_work(values.len())?;
    replacement.append(values);
    *values = replacement;
    *receipt = Some(paid);
    Ok(())
}
fn grow<T>(values: &mut Vec<T>, receipt: &mut Option<ir::RetainedStorage>, needed: usize, budget: &ir::PlanBudget) -> Result<(), String> {
    check_budget(budget)?;
    check_receipt(receipt.as_ref(), budget)?;
    if needed <= values.capacity() {
        return Ok(());
    }
    let capacity = needed.checked_next_power_of_two().ok_or("schema capacity overflow")?;
    let lease = budget.reserve_bytes(capacity.checked_mul(size_of::<T>()).ok_or("schema capacity overflow")?)?;
    let mut replacement = Vec::new();
    replacement.try_reserve_exact(capacity).map_err(|_| "infrastructure: schema allocation failed")?;
    commit_replacement(values, receipt, replacement, lease, budget)
}
fn lookup<V>(values: &DeclTable<V>, id: ir::DeclId, budget: &ir::PlanBudget) -> Result<Option<&V>, String> {
    budget.charge_work(1)?;
    Ok(values.get(id.index as usize).and_then(Option::as_ref).filter(|(key, _)| *key == id).map(|(_, value)| value))
}

#[derive(Debug)]
pub(super) struct PaidDeclTable<V> {
    values: DeclTable<V>,
    receipt: Option<ir::RetainedStorage>,
    snapshot: Option<ir::SnapshotId>,
}
impl<V> Default for PaidDeclTable<V> {
    fn default() -> Self { Self { values: Vec::new(), receipt: None, snapshot: None } }
}
impl<V> PaidDeclTable<V> {
    pub(super) fn insert(&mut self, id: ir::DeclId, value: V, budget: &ir::PlanBudget) -> Result<(), String> {
        check_budget(budget)?;
        check_receipt(self.receipt.as_ref(), budget)?;
        if self.snapshot.is_some_and(|snapshot| snapshot != id.snapshot) {
            return Err("foreign source schema snapshot".into());
        }
        let needed = (id.index as usize).checked_add(1).ok_or("schema index overflow")?;
        budget.charge_work(needed.saturating_sub(self.values.len()).checked_add(1).ok_or("schema work overflow")?)?;
        grow(&mut self.values, &mut self.receipt, needed, budget)?;
        if needed > self.values.len() { self.values.resize_with(needed, || None); }
        self.values[id.index as usize] = Some((id, value));
        self.snapshot = Some(id.snapshot);
        Ok(())
    }
}
fn assignment_position(values: &AssignmentTable, key: (ir::DeclId, ir::DeclId), budget: &ir::PlanBudget) -> Result<Result<usize, usize>, String> {
    let (mut lo, mut hi) = (0, values.len());
    while lo < hi {
        budget.charge_work(1)?;
        let mid = lo + (hi - lo) / 2;
        match values[mid].0.cmp(&key) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
            std::cmp::Ordering::Equal => return Ok(Ok(mid)),
        }
    }
    Ok(Err(lo))
}

#[derive(Debug, Default)]
pub(super) struct SourceSchemasBuilder {
    pub(super) functions: PaidDeclTable<tr::GenericSchema>,
    pub(super) region_modes: PaidDeclTable<tr::RegionContractMode>,
    pub(super) source_regions: PaidDeclTable<Arc<ir::SourceRegionEnv>>,
    associated_type_rows: Vec<tr::AssociatedTypeValue>,
    associated_type_rhs: AssignmentTable,
    rhs_receipt: Option<ir::RetainedStorage>,
    rows_receipt: Option<ir::RetainedStorage>,
}
impl SourceSchemasBuilder {
    pub(super) fn assignment(&mut self, implementation: ir::DeclId, associated: ir::DeclId, rhs: ir::Ty, budget: &ir::PlanBudget) -> Result<(), String> {
        check_budget(budget)?;
        check_receipt(self.rows_receipt.as_ref(), budget)?;
        check_receipt(self.rhs_receipt.as_ref(), budget)?;
        if implementation.snapshot != associated.snapshot || self.associated_type_rows.first().is_some_and(|row| row.implementation.snapshot != implementation.snapshot) {
            return Err("foreign source assignment snapshot".into());
        }
        budget.charge_work(1)?;
        let needed = self.associated_type_rows.len().checked_add(1).ok_or("source assignment capacity overflow")?;
        grow(&mut self.associated_type_rows, &mut self.rows_receipt, needed, budget)?;
        if let Err(position) = assignment_position(&self.associated_type_rhs, (implementation, associated), budget)? {
            let needed = self.associated_type_rhs.len().checked_add(1).ok_or("source assignment capacity overflow")?;
            budget.charge_work(self.associated_type_rhs.len() - position + 1)?;
            grow(&mut self.associated_type_rhs, &mut self.rhs_receipt, needed, budget)?;
            self.associated_type_rhs.insert(position, ((implementation, associated), rhs.clone()));
        }
        // Preserve duplicates for header rejection, and first-row raw authority.
        self.associated_type_rows.push(tr::AssociatedTypeValue { implementation, associated, rhs });
        Ok(())
    }
    pub(super) fn seal(self, snapshot: ir::SnapshotId, budget: &ir::PlanBudget) -> Result<Arc<SourceSchemas>, String> {
        check_budget(budget)?;
        for receipt in [&self.functions.receipt, &self.region_modes.receipt, &self.source_regions.receipt, &self.rhs_receipt, &self.rows_receipt].into_iter().flatten() {
            budget.charge_work(1)?;
            check_receipt(Some(receipt), budget)?;
        }
        fn check_table<V>(table: &PaidDeclTable<V>, snapshot: ir::SnapshotId, budget: &ir::PlanBudget) -> Result<(), String> {
            for (index, slot) in table.values.iter().enumerate() {
                budget.charge_work(1)?;
                if slot.as_ref().is_some_and(|(key, _)| key.snapshot != snapshot || key.index as usize != index) {
                    return Err("foreign source schema snapshot or slot".into());
                }
            }
            Ok(())
        }
        check_table(&self.functions, snapshot, budget)?;
        check_table(&self.region_modes, snapshot, budget)?;
        check_table(&self.source_regions, snapshot, budget)?;
        for (key, env) in self.source_regions.values.iter().flatten() {
            budget.charge_work(1)?;
            if env.declaration != *key || env.outlives.iter().any(|(a,b)| a.declaration != *key || b.declaration != *key) {
                return Err("source lifetime environment declaration mismatch".into());
            }
            budget.charge_work(env.outlives.len())?;
        }
        for row in &self.associated_type_rows {
            budget.charge_work(1)?;
            if row.implementation.snapshot != snapshot || row.associated.snapshot != snapshot {
                return Err("foreign source assignment snapshot".into());
            }
        }
        let wrapper = budget.reserve_bytes(size_of::<SourceSchemas>() + 2 * size_of::<usize>())?.retain()?;
        Ok(Arc::new(SourceSchemas {
            snapshot,
            functions: self.functions.values,
            region_modes: self.region_modes.values,
            source_regions: self.source_regions.values,
            associated_type_rows: self.associated_type_rows,
            associated_type_rhs: self.associated_type_rhs,
            authority: budget.authority().clone(),
            receipts: [self.functions.receipt, self.region_modes.receipt, self.source_regions.receipt, self.rhs_receipt, self.rows_receipt, Some(wrapper)],
        }))
    }
}
impl SourceSchemas {
    fn check(&self, id: ir::DeclId, budget: &ir::PlanBudget) -> Result<(), String> {
        check_budget(budget)?;
        if id.snapshot != self.snapshot { return Err("foreign source schema snapshot".into()); }
        if !self.authority.same_authority(budget.authority()) { return Err("source schema budget authority mismatch".into()); }
        Ok(())
    }
    pub(super) fn assignment_rows(&self, budget: &ir::PlanBudget) -> Result<&[tr::AssociatedTypeValue], String> {
        self.check(ir::DeclId { snapshot: self.snapshot, index: 0 }, budget)?;
        budget.charge_work(1)?;
        Ok(&self.associated_type_rows)
    }
    pub(super) fn signature_arity(&self, id: ir::DeclId, budget: &ir::PlanBudget) -> Result<usize, String> {
        self.check(id, budget)?;
        Ok(lookup(&self.functions, id, budget)?.map_or(0, |s| s.signature.params.len()))
    }
    pub(super) fn schema(&self, id: ir::DeclId, budget: &ir::PlanBudget) -> Result<tr::GenericSchema, String> {
        self.check(id, budget)?;
        let schema = lookup(&self.functions, id, budget)?.ok_or("unknown function declaration")?;
        budget.charge_work(schema.signature.params.len() + 1)?;
        budget.retain_bytes(schema.signature.params.len().checked_mul(4 * size_of::<ir::Ty>() + 64).ok_or("signature allocation overflow")?)?;
        Ok(schema.clone())
    }
    pub(super) fn region_mode(&self, id: ir::DeclId, budget: &ir::PlanBudget) -> Result<tr::RegionContractMode, String> {
        self.check(id, budget)?;
        Ok(lookup(&self.region_modes, id, budget)?.copied().unwrap_or(tr::RegionContractMode::Positioned))
    }
    pub(super) fn region_env(&self, id: ir::DeclId, catalog: bool, budget: &ir::PlanBudget) -> Result<Arc<ir::SourceRegionEnv>, String> {
        self.check(id, budget)?;
        if let Some(env) = lookup(&self.source_regions, id, budget)? { return Ok(env.clone()); }
        if lookup(&self.functions, id, budget)?.is_some() && !catalog { return Err("source callable has no original lifetime environment".into()); }
        budget.retain_bytes(size_of::<ir::SourceRegionEnv>() + 64)?;
        Ok(Arc::new(ir::SourceRegionEnv { declaration: id, region_count: 0, outlives: Arc::from([]), normalized: Arc::from([]) }))
    }
    pub(super) fn associated_rhs(&self, implementation: ir::DeclId, associated: ir::DeclId, budget: &ir::PlanBudget) -> Result<ir::Ty, String> {
        self.check(implementation, budget)?;
        self.check(associated, budget)?;
        budget.charge_work(1)?;
        assignment_position(&self.associated_type_rhs, (implementation, associated), budget)?
            .map(|index| self.associated_type_rhs[index].1.clone()).map_err(|_| "missing authoritative associated assignment".into())
    }
}
