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

/// Params is the sole owned vector. Other schema/signature children and
/// every non-inline Ty alternative clone immutable Arcs, not their payloads.
#[derive(Debug)]
pub(super) struct StoredSchema {
    pub(super) value: tr::GenericSchema,
    pub(super) params_storage: ir::RetainedStorage,
}
impl StoredSchema {
    pub(super) fn params(
        capacity: usize,
        budget: &ir::PlanBudget,
    ) -> Result<(Vec<ir::Ty>, ir::RetainedStorage), String> {
        check_budget(budget)?;
        let lease = budget.reserve_bytes(
            capacity
                .checked_mul(size_of::<ir::Ty>())
                .ok_or("schema params capacity overflow")?,
        )?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(capacity)
            .map_err(|_| "infrastructure: schema params allocation failed")?;
        let paid = lease.retain()?;
        verify_storage(&values, Some(&paid), budget)?;
        Ok((values, paid))
    }
    pub(super) fn copy(
        schema: &tr::GenericSchema,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        let (mut params, params_storage) = Self::params(schema.signature.params.len(), budget)?;
        budget.charge_work(schema.signature.params.len() + 1)?;
        params.extend(schema.signature.params.iter().cloned());
        Ok(Self {
            value: tr::GenericSchema {
                env: schema.env.clone(),
                predicates: schema.predicates.clone(),
                signature: ir::Signature {
                    params,
                    result: schema.signature.result.clone(),
                    ref_contract: schema.signature.ref_contract.clone(),
                },
            },
            params_storage,
        })
    }
}

/// Bootstrap Arc storage policy: two usize reference counts followed by aligned
/// payload. Empty slice Arcs still pay control blocks, with zero payload bytes.
fn arc_storage<T>() -> Result<usize, String> {
    std::alloc::Layout::new::<[usize; 2]>()
        .extend(std::alloc::Layout::new::<T>())
        .map(|(layout, _)| layout.pad_to_align().size())
        .map_err(|_| "schema Arc layout overflow".into())
}

/// Constructed privately from an exact registry row, never caller metadata.
#[derive(Debug)]
struct SourceDeclaration {
    info: tr::SchemaDecl,
}

#[derive(Debug)]
pub(super) struct SourceSchemas {
    snapshot: ir::SnapshotId,
    functions: DeclTable<StoredSchema>,
    region_modes: DeclTable<SourceDeclaration>,
    source_regions: DeclTable<Arc<ir::SourceRegionEnv>>,
    associated_type_rows: Vec<tr::AssociatedTypeValue>,
    associated_type_rhs: AssignmentTable,
    receipts: [Option<ir::RetainedStorage>; 6],
}

fn check_budget(budget: &ir::PlanBudget) -> Result<(), String> {
    if budget.has_resource_failure() {
        return Err("generic planning resource limit exceeded".into());
    }
    budget.charge_work(0)
}
fn check_receipt(
    receipt: Option<&ir::RetainedStorage>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
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
    let bytes = replacement
        .capacity()
        .checked_mul(size_of::<T>())
        .ok_or("schema capacity overflow")?;
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
fn verify_storage<T>(
    values: &Vec<T>,
    receipt: Option<&ir::RetainedStorage>,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    check_receipt(receipt, budget)?;
    let bytes = values
        .capacity()
        .checked_mul(size_of::<T>())
        .ok_or("schema capacity overflow")?;
    if bytes > 0 && receipt.is_none_or(|paid| paid.bytes() < bytes) {
        return Err("source schema storage is not fully prepaid".into());
    }
    Ok(())
}
fn grow<T>(
    values: &mut Vec<T>,
    receipt: &mut Option<ir::RetainedStorage>,
    needed: usize,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    check_budget(budget)?;
    check_receipt(receipt.as_ref(), budget)?;
    if needed <= values.capacity() {
        return Ok(());
    }
    let capacity = needed
        .checked_next_power_of_two()
        .ok_or("schema capacity overflow")?;
    let lease = budget.reserve_bytes(
        capacity
            .checked_mul(size_of::<T>())
            .ok_or("schema capacity overflow")?,
    )?;
    let mut replacement = Vec::new();
    replacement
        .try_reserve_exact(capacity)
        .map_err(|_| "infrastructure: schema allocation failed")?;
    commit_replacement(values, receipt, replacement, lease, budget)
}
fn lookup<'a, V>(
    values: &'a DeclTable<V>,
    id: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Option<&'a V>, String> {
    budget.charge_work(1)?;
    Ok(values
        .get(id.index as usize)
        .and_then(Option::as_ref)
        .filter(|(key, _)| *key == id)
        .map(|(_, value)| value))
}

/// Checked against the existing registry, never a standalone admission ID.
pub(super) struct DeclSlot {
    id: ir::DeclId,
    declaration_count: usize,
}
pub(super) fn decl_slot(
    id: ir::DeclId,
    registry: &super::DeclRegistry,
    budget: &ir::PlanBudget,
) -> Result<DeclSlot, String> {
    check_budget(budget)?;
    budget.charge_work(1)?;
    registry.declaration(id)?;
    Ok(DeclSlot {
        id,
        declaration_count: registry.declarations.len(),
    })
}
#[derive(Debug)]
pub(super) struct PaidDeclTable<V> {
    values: DeclTable<V>,
    receipt: Option<ir::RetainedStorage>,
    snapshot: Option<ir::SnapshotId>,
}
impl<V> Default for PaidDeclTable<V> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            receipt: None,
            snapshot: None,
        }
    }
}
impl<V> PaidDeclTable<V> {
    pub(super) fn insert(
        &mut self,
        slot: DeclSlot,
        value: V,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        check_budget(budget)?;
        let id = slot.id;
        if id.index as usize >= slot.declaration_count {
            return Err("source schema index exceeds declaration inventory".into());
        }
        check_receipt(self.receipt.as_ref(), budget)?;
        if self
            .snapshot
            .is_some_and(|snapshot| snapshot != id.snapshot)
        {
            return Err("foreign source schema snapshot".into());
        }
        let needed = (id.index as usize)
            .checked_add(1)
            .ok_or("schema index overflow")?;
        budget.charge_work(
            needed
                .saturating_sub(self.values.len())
                .checked_add(1)
                .ok_or("schema work overflow")?,
        )?;
        grow(&mut self.values, &mut self.receipt, needed, budget)?;
        if needed > self.values.len() {
            self.values.resize_with(needed, || None);
        }
        self.values[id.index as usize] = Some((id, value));
        self.snapshot = Some(id.snapshot);
        Ok(())
    }
}
fn assignment_position(
    values: &AssignmentTable,
    key: (ir::DeclId, ir::DeclId),
    budget: &ir::PlanBudget,
) -> Result<Result<usize, usize>, String> {
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
    pub(super) functions: PaidDeclTable<StoredSchema>,
    region_modes: PaidDeclTable<SourceDeclaration>,
    pub(super) source_regions: PaidDeclTable<Arc<ir::SourceRegionEnv>>,
    associated_type_rows: Vec<tr::AssociatedTypeValue>,
    associated_type_rhs: AssignmentTable,
    rhs_receipt: Option<ir::RetainedStorage>,
    rows_receipt: Option<ir::RetainedStorage>,
}
impl SourceSchemasBuilder {
    pub(super) fn set_region_mode(
        &mut self,
        id: ir::DeclId,
        mode: tr::RegionContractMode,
        registry: &super::DeclRegistry,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        check_budget(budget)?;
        budget.charge_work(1)?;
        let declaration = registry.declaration(id)?;
        budget.charge_work(1)?;
        let info = tr::SchemaDecl {
            declaration: id,
            kind: declaration.kind,
            own_env: declaration.param_env.clone(),
            env: declaration.env.clone(),
            owner: declaration.owner,
            module: declaration.path.module,
            visibility: declaration.visibility.clone(),
            receiver: super::receiver_mode(declaration.receiver),
            region_contract_mode: mode,
        };
        self.region_modes.insert(
            DeclSlot {
                id,
                declaration_count: registry.declarations.len(),
            },
            SourceDeclaration { info },
            budget,
        )
    }
    pub(super) fn seal_from_registry(
        self,
        registry: &super::DeclRegistry,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<SourceSchemas>, String> {
        check_budget(budget)?;
        let snapshot = registry
            .declarations
            .first()
            .ok_or("missing declaration inventory")?
            .id
            .snapshot;
        budget.charge_work(self.region_modes.values.len())?;
        for (key, projection) in self.region_modes.values.iter().flatten() {
            budget.charge_work(2)?;
            let original = registry.declaration(*key)?;
            let info = &projection.info;
            if info.declaration != *key
                || info.kind != original.kind
                || info.owner != original.owner
                || info.module != original.path.module
                || info.visibility != original.visibility
                || info.receiver != super::receiver_mode(original.receiver)
                || !Arc::ptr_eq(&info.own_env, &original.param_env)
                || !Arc::ptr_eq(&info.env, &original.env)
            {
                return Err(
                    "source declaration projection does not match original registry".into(),
                );
            }
        }
        budget.charge_work(self.functions.values.len())?;
        for (key, stored) in self.functions.values.iter().flatten() {
            budget.charge_work(1)?;
            let original = registry.declaration(*key)?;
            if !matches!(
                original.kind,
                ir::DeclKind::Function | ir::DeclKind::AssociatedFunction | ir::DeclKind::TraitItem
            ) || !Arc::ptr_eq(&stored.value.env, &original.env)
            {
                return Err("source callable schema does not match original registry".into());
            }
        }
        self.seal(snapshot, budget)
    }
    pub(super) fn assignment(
        &mut self,
        implementation: ir::DeclId,
        associated: ir::DeclId,
        rhs: ir::Ty,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        check_budget(budget)?;
        check_receipt(self.rows_receipt.as_ref(), budget)?;
        check_receipt(self.rhs_receipt.as_ref(), budget)?;
        if implementation.snapshot != associated.snapshot
            || self
                .associated_type_rows
                .first()
                .is_some_and(|row| row.implementation.snapshot != implementation.snapshot)
        {
            return Err("foreign source assignment snapshot".into());
        }
        budget.charge_work(1)?;
        let needed = self
            .associated_type_rows
            .len()
            .checked_add(1)
            .ok_or("source assignment capacity overflow")?;
        grow(
            &mut self.associated_type_rows,
            &mut self.rows_receipt,
            needed,
            budget,
        )?;
        if let Err(position) = assignment_position(
            &self.associated_type_rhs,
            (implementation, associated),
            budget,
        )? {
            let needed = self
                .associated_type_rhs
                .len()
                .checked_add(1)
                .ok_or("source assignment capacity overflow")?;
            budget.charge_work(self.associated_type_rhs.len() - position + 1)?;
            grow(
                &mut self.associated_type_rhs,
                &mut self.rhs_receipt,
                needed,
                budget,
            )?;
            self.associated_type_rhs
                .insert(position, ((implementation, associated), rhs.clone()));
        }
        // Preserve duplicates for header rejection, and first-row raw authority.
        self.associated_type_rows.push(tr::AssociatedTypeValue {
            implementation,
            associated,
            rhs,
        });
        Ok(())
    }
    fn seal(
        self,
        snapshot: ir::SnapshotId,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<SourceSchemas>, String> {
        check_budget(budget)?;
        for receipt in [
            &self.functions.receipt,
            &self.region_modes.receipt,
            &self.source_regions.receipt,
            &self.rhs_receipt,
            &self.rows_receipt,
        ]
        .into_iter()
        .flatten()
        {
            budget.charge_work(1)?;
            check_receipt(Some(receipt), budget)?;
        }
        fn check_table<V>(
            table: &PaidDeclTable<V>,
            snapshot: ir::SnapshotId,
            budget: &ir::PlanBudget,
        ) -> Result<(), String> {
            verify_storage(&table.values, table.receipt.as_ref(), budget)?;
            for (index, slot) in table.values.iter().enumerate() {
                budget.charge_work(1)?;
                if slot
                    .as_ref()
                    .is_some_and(|(key, _)| key.snapshot != snapshot || key.index as usize != index)
                {
                    return Err("foreign source schema snapshot or slot".into());
                }
            }
            Ok(())
        }
        check_table(&self.functions, snapshot, budget)?;
        check_table(&self.region_modes, snapshot, budget)?;
        check_table(&self.source_regions, snapshot, budget)?;
        verify_storage(
            &self.associated_type_rows,
            self.rows_receipt.as_ref(),
            budget,
        )?;
        verify_storage(&self.associated_type_rhs, self.rhs_receipt.as_ref(), budget)?;
        budget.charge_work(self.functions.values.len())?;
        for (_, schema) in self.functions.values.iter().flatten() {
            verify_storage(
                &schema.value.signature.params,
                Some(&schema.params_storage),
                budget,
            )?;
            budget.charge_work(schema.value.env.scopes().len())?;
            if schema
                .value
                .env
                .scopes()
                .iter()
                .any(|scope| scope.binder().declaration.snapshot != snapshot)
            {
                return Err("foreign source schema binder snapshot".into());
            }
        }
        budget.charge_work(self.source_regions.values.len())?;
        for (key, env) in self.source_regions.values.iter().flatten() {
            budget.charge_work(1)?;
            budget.charge_work(env.outlives.len())?;
            if env.declaration != *key
                || env
                    .outlives
                    .iter()
                    .any(|(a, b)| a.declaration != *key || b.declaration != *key)
            {
                return Err("source lifetime environment declaration mismatch".into());
            }
        }
        for row in &self.associated_type_rows {
            budget.charge_work(1)?;
            if row.implementation.snapshot != snapshot || row.associated.snapshot != snapshot {
                return Err("foreign source assignment snapshot".into());
            }
        }
        let wrapper = budget
            .reserve_bytes(arc_storage::<SourceSchemas>()?)?
            .retain()?;
        Ok(Arc::new(SourceSchemas {
            snapshot,
            functions: self.functions.values,
            region_modes: self.region_modes.values,
            source_regions: self.source_regions.values,
            associated_type_rows: self.associated_type_rows,
            associated_type_rhs: self.associated_type_rhs,
            receipts: [
                self.functions.receipt,
                self.region_modes.receipt,
                self.source_regions.receipt,
                self.rhs_receipt,
                self.rows_receipt,
                Some(wrapper),
            ],
        }))
    }
}
impl SourceSchemas {
    fn check(&self, id: ir::DeclId, budget: &ir::PlanBudget) -> Result<(), String> {
        check_budget(budget)?;
        if id.snapshot != self.snapshot {
            return Err("foreign source schema snapshot".into());
        }
        let wrapper = self.receipts[5]
            .as_ref()
            .ok_or("missing source schema storage receipt")?;
        check_receipt(Some(wrapper), budget)?;
        Ok(())
    }
    pub(super) fn assignment_rows(
        &self,
        budget: &ir::PlanBudget,
    ) -> Result<&[tr::AssociatedTypeValue], String> {
        self.check(
            ir::DeclId {
                snapshot: self.snapshot,
                index: 0,
            },
            budget,
        )?;
        budget.charge_work(1)?;
        Ok(&self.associated_type_rows)
    }
    pub(super) fn signature_arity(
        &self,
        id: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<usize, String> {
        self.check(id, budget)?;
        Ok(lookup(&self.functions, id, budget)?.map_or(0, |s| s.value.signature.params.len()))
    }
    pub(super) fn schema(
        &self,
        id: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<tr::GenericSchema, String> {
        self.check(id, budget)?;
        let schema = lookup(&self.functions, id, budget)?.ok_or("unknown function declaration")?;
        Ok(StoredSchema::copy(&schema.value, budget)?.value)
    }
    pub(super) fn declaration_info(
        &self,
        id: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<Option<tr::SchemaDecl>, String> {
        self.check(id, budget)?;
        let Some(projection) = lookup(&self.region_modes, id, budget)? else {
            return Ok(None);
        };
        budget.charge_work(1)?;
        if projection.info.declaration != id {
            return Err("source declaration projection identity mismatch".into());
        }
        Ok(Some(projection.info.clone()))
    }
    pub(super) fn region_env(
        &self,
        id: ir::DeclId,
        catalog: bool,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<ir::SourceRegionEnv>, String> {
        self.check(id, budget)?;
        if let Some(env) = lookup(&self.source_regions, id, budget)? {
            return Ok(env.clone());
        }
        if lookup(&self.functions, id, budget)?.is_some() && !catalog {
            return Err("source callable has no original lifetime environment".into());
        }
        let bytes = arc_storage::<ir::SourceRegionEnv>()?
            .checked_add(2 * arc_storage::<()>()?)
            .ok_or("source region allocation overflow")?;
        let _storage = budget.reserve_bytes(bytes)?.retain()?;
        Ok(Arc::new(ir::SourceRegionEnv {
            declaration: id,
            region_count: 0,
            outlives: Arc::from([]),
            normalized: Arc::from([]),
            carriers: Arc::from([]),
        }))
    }
    pub(super) fn associated_rhs(
        &self,
        implementation: ir::DeclId,
        associated: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        self.check(implementation, budget)?;
        self.check(associated, budget)?;
        budget.charge_work(1)?;
        assignment_position(
            &self.associated_type_rhs,
            (implementation, associated),
            budget,
        )?
        .map(|index| self.associated_type_rhs[index].1.clone())
        .map_err(|_| "missing authoritative associated assignment".into())
    }
}
