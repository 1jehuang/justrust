//! Bounded, nonescaping B1 reference provenance and last-use loan analysis.
//! Runtime pointers are never used as provenance. Canonical targets and parent
//! authority are frozen at issuance, independently of later carrier rebinding.
use super::cfg::{self, BlockId, LoanId, Operand, Rvalue, Statement, Terminator};
use super::ir::{
    self, BorrowKind, CarrierChannel, CarrierLocation, CarrierPath, CarrierStep, FactsCtx,
    InputCarrier, InputReference, LocalId, Place, Projection, ReferencePosition, ReturnContract,
    Ty,
};
use super::ownership::{Budget, Exhausted};
use std::collections::{HashMap, VecDeque};
use std::mem::size_of;
use std::sync::Arc;

mod aggregate_reborrow;
mod aggregate_result;
mod cell_access;
mod correlation;
mod indexed_write;
mod linear_family;
mod native_exchange;
mod native_storage;
mod singleton_heap;
use singleton_heap::SingletonHeapSlot;
mod renewal;
use native_exchange::NativeStorageExchange;
use native_storage::NativeStorageTransfer;

#[cfg(test)]
#[path = "borrow/projection_family_tests.rs"]
mod projection_family_tests;
#[cfg(test)]
#[path = "borrow/projection_world_tests.rs"]
mod projection_world_tests;

/// Distinct indices prevent an opaque family from indexing a loan target table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReferenceOriginId(usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FamilyOriginId(usize);
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ReferenceOriginSet(Set);
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FamilyOriginSet(Set);
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CarrierOrigins {
    references: ReferenceOriginSet,
    families: FamilyOriginSet,
}
impl CarrierOrigins {
    fn new(references: usize, families: usize) -> Self {
        Self {
            references: ReferenceOriginSet(Set::new(references)),
            families: FamilyOriginSet(Set::new(families)),
        }
    }
    fn clear(&mut self) {
        self.references.0.clear();
        self.families.0.clear();
    }
    fn union(&mut self, other: &Self, budget: &mut Budget) -> Result<bool, Exhausted> {
        let a = self.references.0.union(&other.references.0, budget)?;
        let b = self.families.0.union(&other.families.0, budget)?;
        Ok(a || b)
    }
    fn count(&self) -> usize {
        self.references.0.count() + self.families.0.count()
    }
}
#[derive(Clone, Debug)]
struct FamilyOrigin {
    input: super::ir::InputCarrier,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CellChannel {
    ReferenceValue,
    Family(ir::CarrierFamily),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CellKey {
    storage: CanonicalPlace,
    channel: CellChannel,
}
/// Only successful universal analysis may publish these refinements. OLD is
/// implicit in the header envelope; must-overwrite is intersected at joins.
#[derive(Clone, Debug, Default)]
pub(super) struct CarrierEffects {
    writes: Arc<[super::ir::CarrierWriteEnvelope]>,
    must_overwrite: Arc<[super::ir::InputCarrier]>,
    /// Strictly ordered by block and occurrence-validated before publication.
    bindings: Arc<[CallCarrierBinding]>,
    /// Exact source-slot operations, never ordinary write/refinement rows.
    _native_storage: Arc<[NativeStorageTransfer]>,
    _native_exchanges: Arc<[NativeStorageExchange]>,
}

/// Count storage that escapes with the effect summary, not temporary analysis
/// reservations or their bookkeeping. Repeated shared paths are deliberately
/// counted per occurrence, so no pointer identity or deduplication is authority.
fn carrier_effects_payload_bytes(
    writes: &[ir::CarrierWriteEnvelope],
    must_overwrite: &[InputCarrier],
    bindings: &[CallCarrierBinding],
    budget: &mut Budget,
) -> Result<usize, Exhausted> {
    struct Payload<'a> {
        bytes: usize,
        budget: &'a mut Budget,
    }
    impl Payload<'_> {
        fn allocation<T>(&mut self, capacity: usize) -> Result<(), Exhausted> {
            self.budget.spend(1)?;
            self.bytes = add(self.bytes, mul(capacity, size_of::<T>())?)?;
            Ok(())
        }
        fn arc<T>(&mut self, length: usize) -> Result<(), Exhausted> {
            self.allocation::<T>(length)?;
            self.bytes = add(self.bytes, 2 * size_of::<usize>())?;
            Ok(())
        }
        fn location(&mut self, location: &CarrierLocation) -> Result<(), Exhausted> {
            // Family projection/opaque Arcs are cloned from paid canonical
            // source/contract facts, never allocated by the carrier analysis.
            self.arc::<CarrierStep>(location.path.0.len())
        }
        fn origins(&mut self, origins: &CarrierOrigins) -> Result<(), Exhausted> {
            self.allocation::<u64>(origins.references.0.0.capacity())?;
            self.allocation::<u64>(origins.families.0.0.capacity())
        }
        fn place(&mut self, place: &CanonicalPlace) -> Result<(), Exhausted> {
            self.allocation::<Projection>(place.projection.capacity())?;
            if let CanonicalRoot::ParameterReferent(_, location) = &place.root {
                self.location(location)?;
            }
            Ok(())
        }
        fn targets(&mut self, targets: &Vec<Target>) -> Result<(), Exhausted> {
            self.allocation::<Target>(targets.capacity())?;
            for target in targets {
                self.place(&target.place)?;
                self.allocation::<usize>(target.authority.capacity())?;
            }
            Ok(())
        }
        fn region_key(&mut self, key: &RegionKey) -> Result<(), Exhausted> {
            self.budget.spend(1)?;
            match key {
                RegionKey::Family { component, .. } => self.location(component)?,
                RegionKey::Occurrence {
                    component: Some(component),
                    ..
                } => {
                    self.location(component)?;
                }
                _ => {}
            }
            Ok(())
        }
        fn binding(&mut self, binding: &CallCarrierBinding) -> Result<(), Exhausted> {
            // source/function/facts/contract/evidence only clone independently
            // paid immutable graph allocations. Their Arc handles are already
            // included in the outer binding array, with no new pointee storage.
            self.allocation::<CarrierEntrySnapshot>(binding.entry.capacity())?;
            for entry in &binding.entry {
                self.location(&entry.input.location)?;
                self.origins(&entry.origins)?;
            }
            self.allocation::<CarrierDestinationBinding>(binding.destinations.capacity())?;
            for destination in &binding.destinations {
                self.location(&destination.destination.location)?;
                self.targets(&destination.targets)?;
                self.allocation::<(CellKey, CarrierOrigins)>(destination.old.capacity())?;
                for (cell, origins) in &destination.old {
                    self.place(&cell.storage)?;
                    self.origins(origins)?;
                }
            }
            self.allocation::<CallRegionBinding>(binding.regions.capacity())?;
            for region in &binding.regions {
                self.budget.spend(1)?;
                if let Some(component) = &region.component {
                    self.location(component)?;
                }
            }
            self.allocation::<RegionKey>(binding.relation.keys.capacity())?;
            for key in &binding.relation.keys {
                self.region_key(key)?;
            }
            self.allocation::<Set>(binding.relation.closure.capacity())?;
            for row in &binding.relation.closure {
                self.allocation::<u64>(row.0.capacity())?;
            }
            self.targets(&binding.actual_writes)
        }
    }
    let mut payload = Payload { bytes: 0, budget };
    payload.arc::<CarrierEffects>(1)?;
    payload.arc::<ir::CarrierWriteEnvelope>(writes.len())?;
    for write in writes {
        payload.location(&write.destination.location)?;
        payload.arc::<InputCarrier>(write.sources.len())?;
        for input in write.sources.iter() {
            payload.location(&input.location)?;
        }
    }
    payload.arc::<InputCarrier>(must_overwrite.len())?;
    for input in must_overwrite {
        payload.location(&input.location)?;
    }
    payload.arc::<CallCarrierBinding>(bindings.len())?;
    for binding in bindings {
        payload.binding(binding)?;
    }
    Ok(payload.bytes)
}

fn retain_carrier_effects_payload(
    writes: &[ir::CarrierWriteEnvelope],
    must_overwrite: &[InputCarrier],
    bindings: &[CallCarrierBinding],
    budget: &mut Budget,
    ledger: &ir::PlanBudget,
) -> Result<usize, Exhausted> {
    let bytes = carrier_effects_payload_bytes(writes, must_overwrite, bindings, budget)?;
    ledger.retain_bytes(bytes).map_err(|_| Exhausted)?;
    Ok(bytes)
}

fn call_binding_subject_matches(
    binding: &CallCarrierBinding,
    f: &cfg::Function,
    function: Option<&Arc<cfg::Function>>,
    facts: Option<&Arc<super::traits::ValidatedFacts>>,
) -> bool {
    Arc::ptr_eq(&binding.source, binding.evidence.source())
        && binding.call == binding.evidence.call()
        && f.blocks
            .get(binding.block.0)
            .and_then(|block| block.terminator.origin)
            == Some(binding.call)
        && function.is_some_and(|function| Arc::ptr_eq(function, &binding.function))
        && facts.is_some_and(|facts| Arc::ptr_eq(facts, &binding.facts))
}

fn ordered_call_bindings(
    bindings: &[CallCarrierBinding],
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    for pair in bindings.windows(2) {
        budget.spend(1)?;
        if pair[0].block.0 >= pair[1].block.0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn call_binding_view_matches(
    binding: &CallCarrierBinding,
    calls: &[Option<Arc<super::traits::CarrierContractView>>],
) -> bool {
    calls
        .get(binding.block.0)
        .and_then(Option::as_ref)
        .is_some_and(|validated| Arc::ptr_eq(validated, &binding.contract))
}

fn source_region_input(
    view: &super::traits::CarrierContractView,
    row: &ir::SourceCarrierRegion,
    budget: &mut Budget,
) -> Result<Option<InputCarrier>, Exhausted> {
    budget.spend(view.source_region_env().carriers.len() + view.contract().inputs.len() + 1)?;
    if !view.source_region_env().carriers.contains(row) {
        return Ok(None);
    }
    let Some(param) = row.parameter else {
        return Ok(None);
    };
    let input = InputCarrier {
        param,
        location: row.location.clone(),
    };
    Ok(view.contract().inputs.contains(&input).then_some(input))
}

// Family source rows may name only a component already issued by the original
// input contract and its canonical substitution. A family term alone does not
// associate every component region with every substituted source location.
fn source_family_region_is_issued(
    issued: &[(InputCarrier, AnalysisRegionId)],
    input: &InputCarrier,
    regions: &[(Option<CarrierLocation>, AnalysisRegionId)],
) -> bool {
    regions.iter().any(|(component, region)| {
        component.is_some()
            && issued
                .iter()
                .any(|(candidate, actual)| candidate == input && actual == region)
    })
}

/// Read an authored writable referent region only from the exact sealed call
/// binding. A type-equal or source-equivalent foreign call is not authority.
fn call_result_slot_region(
    binding: &CallCarrierBinding,
    f: &cfg::Function,
    function: Option<&Arc<cfg::Function>>,
    facts: Option<&Arc<super::traits::ValidatedFacts>>,
    calls: &[Option<Arc<super::traits::CarrierContractView>>],
    location: &CarrierLocation,
    budget: &mut Budget,
) -> Result<Result<Option<ir::CarrierRegionTerm>, Problem>, Exhausted> {
    if !call_binding_subject_matches(binding, f, function, facts)
        || !call_binding_view_matches(binding, calls)
    {
        return Ok(Err(Problem::Invalid(
            "carrier write changed its exact authored call-result authority".into(),
        )));
    }
    let CarrierChannel::Fixed(position) = location.channel else {
        return Ok(Ok(None));
    };
    let depth = position.depth.checked_add(1).ok_or(Exhausted)?;
    let rows = &binding.contract.source_region_env().carriers;
    budget.spend(rows.len() + location.path.0.len())?;
    Ok(Ok(rows
        .iter()
        .find(|row| {
            row.parameter.is_none()
                && row.location.path == location.path
                && row.location.channel == CarrierChannel::Fixed(ReferencePosition { depth })
        })
        .map(|row| row.region.clone())))
}

/// The diagnostic pass visits blocks in ascending order, and publication
/// rejects duplicates or reordered rows. Search only that immutable table,
/// charging before every comparison, including failed/absent-key probes.
fn call_binding_index(
    bindings: &[CallCarrierBinding],
    block: BlockId,
    budget: &mut Budget,
) -> Result<Option<usize>, Exhausted> {
    let (mut low, mut high) = (0, bindings.len());
    while low < high {
        let middle = low + (high - low) / 2;
        budget.spend(1)?;
        match bindings[middle].block.0.cmp(&block.0) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Equal => return Ok(Some(middle)),
            std::cmp::Ordering::Greater => high = middle,
        }
    }
    Ok(None)
}

pub(super) fn fixed_location(position: ReferencePosition) -> CarrierLocation {
    CarrierLocation {
        path: CarrierPath(Arc::from([])),
        channel: CarrierChannel::Fixed(position),
    }
}
fn checked_vec_growth<T>(values: &mut Vec<T>, budget: &mut Budget) -> Result<(), Exhausted> {
    if values.len() < values.capacity() {
        return Ok(());
    }
    let capacity = values.capacity().checked_mul(2).ok_or(Exhausted)?.max(4);
    let peak = mul(add(capacity, values.capacity())?, size_of::<T>())?;
    budget.reserve_bytes(peak)?;
    budget.spend(add(values.len(), 1)?)?;
    budget.nodes(1)?;
    values
        .try_reserve_exact(capacity.checked_sub(values.len()).ok_or(Exhausted)?)
        .map_err(|_| Exhausted)
}
fn checked_arc_transfer<T>(length: usize, budget: &mut Budget) -> Result<(), Exhausted> {
    budget.reserve_bytes(add(mul(length, size_of::<T>())?, 2 * size_of::<usize>())?)?;
    budget.spend(add(length, 1)?)?;
    budget.nodes(1)
}

fn family_region_term(family: &ir::CarrierFamily) -> ir::CarrierRegionTerm {
    match family {
        ir::CarrierFamily::Parameter(parameter) => ir::CarrierRegionTerm::Family {
            parameter: *parameter,
        },
        ir::CarrierFamily::Projection(projection) => ir::CarrierRegionTerm::ProjectionFamily {
            projection: projection.clone(),
        },
        ir::CarrierFamily::Array(array) => ir::CarrierRegionTerm::ArrayFamily {
            array: array.clone(),
        },
        ir::CarrierFamily::Opaque(opaque) => ir::CarrierRegionTerm::OpaqueFamily {
            opaque: opaque.clone(),
        },
    }
}
fn family_channel(family: &ir::CarrierFamily, prefix: u8) -> CarrierChannel {
    match family {
        ir::CarrierFamily::Parameter(parameter) => CarrierChannel::ParamTail {
            parameter: *parameter,
            prefix,
        },
        ir::CarrierFamily::Projection(projection) => CarrierChannel::ProjectionTail {
            projection: projection.clone(),
            prefix,
        },
        ir::CarrierFamily::Array(array) => CarrierChannel::ArrayTail {
            array: array.clone(),
            prefix,
        },
        ir::CarrierFamily::Opaque(opaque) => CarrierChannel::OpaqueTail {
            opaque: opaque.clone(),
            prefix,
        },
    }
}
fn value_projection(location: &CarrierLocation) -> Vec<Projection> {
    let mut path = Vec::new();
    for step in location.path.0.iter() {
        match step {
            CarrierStep::Referent => path.push(Projection::Deref),
            CarrierStep::Field(index) => path.push(Projection::Field(*index)),
            CarrierStep::ManuallyDropPayload => path.push(Projection::ManuallyDropPayload),
            CarrierStep::ConstantIndex(index) => path.push(Projection::ConstantIndex(*index)),
            CarrierStep::SliceElement => path.push(Projection::DynamicIndex(LocalId(usize::MAX))),
            CarrierStep::VariantField { variant, field } => {
                path.push(Projection::Downcast(*variant));
                path.push(Projection::Field(*field));
            }
        }
    }
    let depth = match location.channel {
        CarrierChannel::Fixed(position) => position.depth,
        CarrierChannel::ParamTail { prefix, .. }
        | CarrierChannel::ProjectionTail { prefix, .. }
        | CarrierChannel::ArrayTail { prefix, .. }
        | CarrierChannel::OpaqueTail { prefix, .. } => prefix,
    };
    path.extend(std::iter::repeat_n(Projection::Deref, usize::from(depth)));
    path
}

#[derive(Clone, Debug)]
struct CellSlot {
    key: CellKey,
    ty: Ty,
    input: Option<InputCarrier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AnalysisRegionId(usize);
#[derive(Clone, Debug)]
struct CarrierEntrySnapshot {
    input: InputCarrier,
    origins: CarrierOrigins,
}
#[derive(Clone, Debug)]
struct CarrierDestinationBinding {
    destination: InputCarrier,
    targets: Vec<Target>,
    old: Vec<(CellKey, CarrierOrigins)>,
}
#[derive(Clone, Debug)]
struct CallRegionBinding {
    schema: ir::CarrierRegionTerm,
    component: Option<CarrierLocation>,
    region: AnalysisRegionId,
}
#[derive(Clone, Debug)]
struct CallCarrierBinding {
    source: Arc<ir::CheckedFunction>,
    function: Arc<cfg::Function>,
    call: ir::ExprId,
    block: BlockId,
    facts: Arc<super::traits::ValidatedFacts>,
    contract: Arc<super::traits::CarrierContractView>,
    evidence: Arc<ir::ValidatedCallEvidence>,
    entry: Vec<CarrierEntrySnapshot>,
    destinations: Vec<CarrierDestinationBinding>,
    regions: Vec<CallRegionBinding>,
    relation: RegionRelations,
    actual_writes: Vec<Target>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RegionKey {
    Fixed(ir::RegionParamId),
    Named(ir::SourceRegionId),
    Family {
        family: ir::CarrierFamily,
        component: CarrierLocation,
    },
    Local(ReferenceOriginId),
    Occurrence {
        block: BlockId,
        term: ir::CarrierRegionTerm,
        component: Option<CarrierLocation>,
    },
}
#[derive(Clone, Debug, Default)]
struct RegionRelations {
    keys: Vec<RegionKey>,
    closure: Vec<Set>,
}
impl RegionRelations {
    fn node(&mut self, key: RegionKey, budget: &mut Budget) -> Result<AnalysisRegionId, Exhausted> {
        budget.spend(self.keys.len() + 1)?;
        if let Some(index) = self.keys.iter().position(|row| row == &key) {
            return Ok(AnalysisRegionId(index));
        }
        budget.nodes(1)?;
        let index = self.keys.len();
        let count = index.checked_add(1).ok_or(Exhausted)?;
        let words = count.div_ceil(64);
        let bytes = region_storage_bytes(count)?
            .checked_sub(region_storage_bytes(index)?)
            .ok_or(Exhausted)?;
        budget.reserve_bytes(bytes)?;
        budget.spend(index.checked_add(words).ok_or(Exhausted)?)?;
        for row in &mut self.closure {
            row.0.resize(words, 0);
        }
        let mut row = Set::new(count);
        row.insert(index);
        self.keys.push(key);
        self.closure.push(row);
        Ok(AnalysisRegionId(index))
    }
    fn edge(
        &mut self,
        longer: AnalysisRegionId,
        shorter: AnalysisRegionId,
        budget: &mut Budget,
    ) -> Result<(), Exhausted> {
        budget.spend(1)?;
        self.closure[longer.0].insert(shorter.0);
        Ok(())
    }
    fn close(&mut self, budget: &mut Budget) -> Result<(), Exhausted> {
        let mut memberships = self
            .closure
            .iter()
            .try_fold(0usize, |total, row| add(total, row.count()))?;
        if memberships > MAX_MEMBERSHIPS {
            return Err(Exhausted);
        }
        let count = self.keys.len();
        budget.spend(mul(count, count)?)?;
        for via in 0..count {
            for from in 0..count {
                if self.closure[from].has(via) {
                    budget.spend(self.closure[via].0.len())?;
                    for word in 0..self.closure[via].0.len() {
                        let bits = self.closure[via].0[word];
                        let new = bits & !self.closure[from].0[word];
                        memberships = add(memberships, new.count_ones() as usize)?;
                        if memberships > MAX_MEMBERSHIPS {
                            return Err(Exhausted);
                        }
                        self.closure[from].0[word] |= bits;
                    }
                }
            }
        }
        Ok(())
    }
    fn copied(&self, budget: &mut Budget) -> Result<Self, Exhausted> {
        let words = self.closure.iter().try_fold(0usize, |total, row| {
            total.checked_add(row.0.len()).ok_or(Exhausted)
        })?;
        budget.spend(self.keys.len().checked_add(words).ok_or(Exhausted)?)?;
        budget.reserve_bytes(region_storage_bytes(self.keys.len())?)?;
        Ok(self.clone())
    }
    fn entails(&self, longer: AnalysisRegionId, shorter: AnalysisRegionId) -> bool {
        self.closure[longer.0].has(shorter.0)
    }
}

/// A read cover names existing physical cells, not more precise loan targets.
/// Completeness comes from the original layout, never from matching closed types
/// or from finding a nonempty subset of the cell table.
fn read_cell_cover(
    original_locals: &[Ty],
    cell_index: &HashMap<CellKey, usize>,
    target: &Target,
    channel: &CellChannel,
    budget: &mut Budget,
) -> Result<Option<Vec<usize>>, Exhausted> {
    let ledger = budget.plan_budget().cloned();
    read_cell_cover_with_ledger(
        original_locals,
        cell_index,
        target,
        channel,
        ledger.as_ref(),
        budget,
    )
}

fn read_cell_cover_with_ledger(
    original_locals: &[Ty],
    cell_index: &HashMap<CellKey, usize>,
    target: &Target,
    channel: &CellChannel,
    ledger: Option<&ir::PlanBudget>,
    budget: &mut Budget,
) -> Result<Option<Vec<usize>>, Exhausted> {
    fn walk(
        ty: &Ty,
        storage: &mut CanonicalPlace,
        cell_index: &HashMap<CellKey, usize>,
        output: &mut Vec<usize>,
        depth: usize,
        ledger: Option<&ir::PlanBudget>,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        if depth > ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err(Exhausted);
        }
        budget.nodes(1)?;
        budget.spend(add(storage.projection.len(), 1)?)?;
        match ty {
            Ty::Ref { .. } => {
                budget.reserve_bytes(mul(storage.projection.len(), size_of::<Projection>())?)?;
                let key = CellKey {
                    storage: storage.clone(),
                    channel: CellChannel::ReferenceValue,
                };
                let Some(&index) = cell_index.get(&key) else {
                    return Ok(false);
                };
                if output.len() >= MAX_MEMBERSHIPS {
                    return Err(Exhausted);
                }
                checked_vec_growth(output, budget)?;
                output.push(index);
                // A pointer's referent is a different canonical object.
                Ok(true)
            }
            Ty::DependentArray { .. } | Ty::Param(_) => Ok(false),
            Ty::Int(_)
            | Ty::Float(_)
            | Ty::Char
            | Ty::Str
            | Ty::Bool
            | Ty::Unit
            | Ty::Never
            | Ty::Cell(_)
            | Ty::RawPtr { .. } => Ok(true),
            _ => {
                if let Ty::Enum(value) = ty {
                    budget.spend(value.def().variants.len())?;
                }
                // Materialization uses the original proof ledger, independently
                // of whether the ownership scratch counter carries a ledger.
                let field_budget = ledger;
                let mut child = |steps: &[Projection], ty: &Ty| -> Result<bool, Exhausted> {
                    let saved = storage.projection.len();
                    let length = add(saved, steps.len())?;
                    if length > ir::MAX_PATTERN_PROJECTION_DEPTH {
                        return Err(Exhausted);
                    }
                    budget.reserve_bytes(mul(length, size_of::<Projection>())?)?;
                    budget.spend(length)?;
                    storage
                        .projection
                        .try_reserve_exact(steps.len())
                        .map_err(|_| Exhausted)?;
                    storage.projection.extend_from_slice(steps);
                    let result = walk(ty, storage, cell_index, output, depth + 1, ledger, budget);
                    storage.projection.truncate(saved);
                    result
                };
                match ty {
                    Ty::ManuallyDrop(wrapper) => {
                        if !child(&[Projection::ManuallyDropPayload], wrapper.argument())? {
                            return Ok(false);
                        }
                    }
                    Ty::Tuple(fields) => {
                        for (index, field) in fields.iter().enumerate() {
                            if !child(&[Projection::Field(index)], field)? {
                                return Ok(false);
                            }
                        }
                    }
                    // Plain scalar elements hold no reference cells.
                    Ty::Array { element, .. }
                        if matches!(
                            element.as_ref(),
                            Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool
                        ) => {}
                    Ty::Array { element, len } => {
                        if *len > ir::MAX_ARRAY_LEN {
                            return Err(Exhausted);
                        }
                        for index in 0..*len {
                            if !child(&[Projection::ConstantIndex(index)], element)? {
                                return Ok(false);
                            }
                        }
                    }
                    Ty::Slice { element } => {
                        if !child(&[Projection::DynamicIndex(LocalId(usize::MAX))], element)? {
                            return Ok(false);
                        }
                    }
                    Ty::Struct(value) => {
                        if let Some(element) = ty.owned_sequence_element()
                            && !child(&[Projection::DynamicIndex(LocalId(usize::MAX))], element)?
                        {
                            return Ok(false);
                        }
                        for (index, field) in value
                            .def()
                            .fields
                            .read_optional(field_budget)
                            .map_err(|_| Exhausted)?
                            .iter()
                            .enumerate()
                        {
                            if !child(&[Projection::Field(index)], &field.ty)? {
                                return Ok(false);
                            }
                        }
                    }
                    Ty::Enum(value) => {
                        for (variant, definition) in value.def().variants.iter().enumerate() {
                            for (field, ty) in definition
                                .fields
                                .read_optional(field_budget)
                                .map_err(|_| Exhausted)?
                                .iter()
                                .enumerate()
                            {
                                let variant = ir::VariantId {
                                    owner: value.def().id,
                                    index: variant,
                                };
                                if !child(
                                    &[Projection::Downcast(variant), Projection::Field(field)],
                                    &ty.ty,
                                )? {
                                    return Ok(false);
                                }
                            }
                        }
                    }
                    _ => unreachable!("physical leaf handled above"),
                }
                Ok(true)
            }
        }
    }
    budget.spend(add(target.place.projection.len(), 1)?)?;
    if target.precision == TargetPrecision::Exact {
        budget.reserve_bytes(mul(target.place.projection.len(), size_of::<Projection>())?)?;
        let key = CellKey {
            storage: target.place.clone(),
            channel: channel.clone(),
        };
        if let Some(&index) = cell_index.get(&key) {
            let mut output = Vec::new();
            checked_vec_growth(&mut output, budget)?;
            output.push(index);
            return Ok(Some(output));
        }
    }
    if *channel != CellChannel::ReferenceValue {
        return Ok(None);
    }
    let (local, location) = match &target.place.root {
        CanonicalRoot::Owned(local) => (*local, None),
        CanonicalRoot::ParameterReferent(local, location) => (*local, Some(location)),
    };
    let Some(ty) = original_locals.get(local.0) else {
        return Ok(None);
    };
    let mut ty = ty.clone();
    if let Some(location) = location {
        let depth = match location.channel {
            CarrierChannel::Fixed(position) => position.depth,
            CarrierChannel::ParamTail { prefix, .. }
            | CarrierChannel::ProjectionTail { prefix, .. }
            | CarrierChannel::ArrayTail { prefix, .. }
            | CarrierChannel::OpaqueTail { prefix, .. } => prefix,
        };
        let length = add(mul(location.path.0.len(), 2)?, usize::from(depth))?;
        budget.reserve_bytes(mul(length, size_of::<Projection>())?)?;
        budget.spend(add(length, 1)?)?;
        let path = value_projection(location);
        let Ty::Ref { referent, .. } =
            ir::projected_ty_optional(&ty, &path, ledger).map_err(|_| Exhausted)?
        else {
            return Ok(None);
        };
        ty = referent.as_ref().clone();
    }
    // A canonical object's physical coordinates never traverse another pointer.
    if target.place.projection.contains(&Projection::Deref) {
        return Ok(None);
    }
    let ty =
        ir::projected_ty_optional(&ty, &target.place.projection, ledger).map_err(|_| Exhausted)?;
    // A slice root can conservatively designate any of its elements. Only
    // its issued summary cell supplies a cover, never a matching layout or
    // a fabricated element origin. Other exact targets remain exact.
    if target.precision == TargetPrecision::Exact && !matches!(ty, Ty::Slice { .. }) {
        return Ok(None);
    }
    budget.reserve_bytes(mul(target.place.projection.len(), size_of::<Projection>())?)?;
    let mut storage = target.place.clone();
    let mut output = Vec::new();
    if !walk(
        &ty,
        &mut storage,
        cell_index,
        &mut output,
        0,
        ledger,
        budget,
    )? || (target.precision == TargetPrecision::Exact && output.is_empty())
    {
        return Ok(None);
    }
    Ok(Some(output))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolutionFailure {
    EmptyOrigins,
    IncompleteCover,
}
impl ResolutionFailure {
    fn problem(self, unsupported: &str) -> Problem {
        match self {
            Self::EmptyOrigins => {
                Problem::Invalid("reference carrier has no initialized issued provenance".into())
            }
            Self::IncompleteCover => Problem::Unsupported(unsupported.into()),
        }
    }
}

type CellResolution = (
    Vec<Target>,
    Vec<ReferenceOriginId>,
    Option<ResolutionFailure>,
);

fn probe_projected_shape(
    root: &Ty,
    path: &[Projection],
    budget: &mut Budget,
) -> Result<Option<Ty>, Exhausted> {
    match ir::projected_ty_optional(root, path, budget.plan_budget()) {
        Ok(ty) => Ok(Some(ty)),
        Err(error) => {
            let resource = budget.plan_budget().map_or_else(
                || error.contains("limit") || error.contains("exhaust"),
                |plan| super::traits::FactsError::from_message(plan, &error).is_resource(),
            );
            if resource { Err(Exhausted) } else { Ok(None) }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_cells<'a>(
    original_locals: &[Ty],
    cover_locals: &[Ty],
    cells: &[CellSlot],
    cell_index: &HashMap<CellKey, usize>,
    origins_targets: &[Vec<Target>],
    origins: impl Fn(usize) -> &'a Set,
    place: &Place,
    budget: &mut Budget,
) -> Result<CellResolution, Exhausted> {
    budget.spend(place.projection.len() + 1)?;
    // Storage behind a raw pointer has no canonical object or loan cells.
    if let Some(local) = original_locals.get(place.local.0)
        && ir::raw_deref_index_optional(local, &place.projection, budget.plan_budget())
            .map_err(|_| Exhausted)?
            .is_some()
    {
        return Ok((vec![], vec![], None));
    }
    budget.reserve_bytes(target_payload_bytes_with_growth(
        1,
        place.projection.len(),
        0,
        3,
        0,
    )?)?;
    let mut initial_projection = Vec::new();
    initial_projection
        .try_reserve_exact(place.projection.len())
        .map_err(|_| Exhausted)?;
    let mut candidates = vec![Target {
        place: CanonicalPlace {
            root: CanonicalRoot::Owned(place.local),
            projection: initial_projection,
        },
        precision: TargetPrecision::Exact,
        authority: vec![],
    }];
    let mut trace = Vec::new();
    let mut failure = None;
    let mut static_referent = false;
    for (step_index, step) in place.projection.iter().enumerate() {
        if matches!(step, Projection::DynamicIndex(_)) {
            // A runtime index cannot select a more precise provenance row.
            // Expand a known array's complete domain, or select the single
            // summary cell of an abstract shared slice. Never use the index
            // local as an alias-disjointness certificate.
            let mut expanded = Vec::new();
            for candidate in candidates {
                if candidate.precision == TargetPrecision::Subtree {
                    checked_vec_growth(&mut expanded, budget)?;
                    expanded.push(candidate);
                    continue;
                }
                let (local, location) = match &candidate.place.root {
                    CanonicalRoot::Owned(local) => (*local, None),
                    CanonicalRoot::ParameterReferent(local, location) => (*local, Some(location)),
                };
                // Only the compiler-issued slice summary may use the exact
                // template substitution's additional aggregate cover. Runtime
                // indices still require the original source shape and cannot
                // gain disjointness from a concrete instantiation.
                let locals = if *step == Projection::DynamicIndex(LocalId(usize::MAX)) {
                    cover_locals
                } else {
                    original_locals
                };
                // A carrier location of a generic body instance (`__jr_fmt<T>`
                // with T = Option<&Vec<&str>>) is written against the exact
                // substituted shape; the generic source local (`&T`) cannot
                // follow its variant steps. Project such a location through the
                // instance cover instead of misreporting exhaustion.
                let follows = |locals: &[Ty]| -> bool {
                    let Some(ty) = locals.get(local.0) else {
                        return true;
                    };
                    let ty = match location {
                        Some(location) => {
                            match ir::projected_ty_optional(ty, &value_projection(location), None) {
                                Ok(Ty::Ref { referent, .. }) => referent.as_ref().clone(),
                                _ => return false,
                            }
                        }
                        None => ty.clone(),
                    };
                    ir::projected_ty_optional(&ty, &candidate.place.projection, None).is_ok()
                };
                let locals = if !follows(locals) && follows(cover_locals) {
                    cover_locals
                } else {
                    locals
                };
                let Some(ty) = locals.get(local.0) else {
                    return Err(Exhausted);
                };
                let mut ty = ty.clone();
                if let Some(location) = location {
                    budget.spend(location.path.0.len() + 1)?;
                    budget.reserve_bytes(
                        location
                            .path
                            .0
                            .len()
                            .checked_mul(2 * size_of::<Projection>())
                            .ok_or(Exhausted)?,
                    )?;
                    let path = value_projection(location);
                    ty = match ir::projected_ty_optional(&ty, &path, budget.plan_budget())
                        .map_err(|_| Exhausted)?
                    {
                        Ty::Ref { referent, .. } => referent.as_ref().clone(),
                        _ => return Err(Exhausted),
                    };
                }
                ty = ir::projected_ty_optional(
                    &ty,
                    &candidate.place.projection,
                    budget.plan_budget(),
                )
                .map_err(|_| Exhausted)?;
                if matches!(&ty, Ty::Array { element, .. } | Ty::DependentArray { element, .. } if !matches!(element.as_ref(), Ty::Ref { kind: BorrowKind::Shared, .. }))
                    && *step != Projection::DynamicIndex(LocalId(usize::MAX))
                {
                    let mut selected = candidate;
                    selected.place.projection.push(*step);
                    checked_vec_growth(&mut expanded, budget)?;
                    expanded.push(selected);
                    continue;
                }
                let count = match ty {
                    Ty::Array { len, .. } => len,
                    Ty::Slice { .. } => 1,
                    _ if *step == Projection::DynamicIndex(LocalId(usize::MAX))
                        && ty.owned_sequence_element().is_some() =>
                    {
                        1
                    }
                    _ if *step == Projection::DynamicIndex(LocalId(usize::MAX)) => {
                        // A declared slice may borrow opaque backing storage
                        // (for example Vec<T>). Its owner is not an array or
                        // slice cell domain. Keep the cover unresolved rather
                        // than inventing element cells or reporting exhaustion.
                        return Ok((vec![], trace, Some(ResolutionFailure::IncompleteCover)));
                    }
                    _ => return Err(Exhausted),
                };
                if count > ir::MAX_ARRAY_LEN
                    || expanded.len().checked_add(count).ok_or(Exhausted)? > MAX_MEMBERSHIPS
                {
                    return Err(Exhausted);
                }
                for index in 0..count {
                    budget.spend(1)?;
                    budget.reserve_bytes(target_payload_bytes(
                        1,
                        candidate.place.projection.len() + 1,
                        candidate.authority.len(),
                        3,
                    )?)?;
                    let mut selected = candidate.clone();
                    selected.place.projection.push(
                        if matches!(ty, Ty::Slice { .. }) || ty.owned_sequence_element().is_some() {
                            Projection::DynamicIndex(LocalId(usize::MAX))
                        } else {
                            Projection::ConstantIndex(index)
                        },
                    );
                    checked_vec_growth(&mut expanded, budget)?;
                    expanded.push(selected);
                }
            }
            candidates = expanded;
            continue;
        }
        if *step != Projection::Deref {
            for target in &mut candidates {
                if target.precision == TargetPrecision::Exact {
                    target.place.projection.push(*step);
                }
            }
            continue;
        }
        // A widened heap subtree has no exact current field coordinate.
        // The validated source place still tells us whether this operation is
        // an owning Box dereference, which preserves that conservative subtree
        // rather than reading a nonexistent reference-valued cell from it.
        let subtree_box = if candidates
            .iter()
            .any(|target| target.precision == TargetPrecision::Subtree)
        {
            if let Some(root) = cover_locals.get(place.local.0) {
                probe_projected_shape(root, &place.projection[..step_index], budget)?
                    .is_some_and(|ty| ty.owned_box_pointee().is_some())
            } else {
                false
            }
        } else {
            false
        };
        let mut next = Vec::new();
        for mut candidate in candidates {
            if subtree_box && candidate.precision == TargetPrecision::Subtree {
                checked_vec_growth(&mut next, budget)?;
                next.push(candidate);
                continue;
            }
            // An owning Box dereference remains within the owner's canonical
            // storage tree. Only actual references consume issued loan cells.
            let (local, location) = match &candidate.place.root {
                CanonicalRoot::Owned(local) => (*local, None),
                CanonicalRoot::ParameterReferent(local, location) => (*local, Some(location)),
            };
            // Box detection is only a representation probe. An original
            // generic shape may not expose a concrete carrier projection, so
            // use its validated substitution cover and leave non-Box or
            // unavailable projections to ordinary issued-cell resolution.
            if let Some(location) = location {
                let depth = match location.channel {
                    CarrierChannel::Fixed(position) => position.depth,
                    CarrierChannel::ParamTail { prefix, .. }
                    | CarrierChannel::ProjectionTail { prefix, .. }
                    | CarrierChannel::ArrayTail { prefix, .. }
                    | CarrierChannel::OpaqueTail { prefix, .. } => prefix,
                };
                let length = add(mul(location.path.0.len(), 2)?, usize::from(depth))?;
                budget.reserve_bytes(mul(length, size_of::<Projection>())?)?;
                budget.spend(add(length, 1)?)?;
            }
            let owner = (|| -> Result<Option<Ty>, String> {
                let Some(mut owner) = cover_locals.get(local.0).cloned() else {
                    return Ok(None);
                };
                if let Some(location) = location {
                    let path = value_projection(location);
                    let Ty::Ref { referent, .. } =
                        ir::projected_ty_optional(&owner, &path, budget.plan_budget())?
                    else {
                        return Ok(None);
                    };
                    owner = referent.as_ref().clone();
                }
                ir::projected_ty_optional(&owner, &candidate.place.projection, budget.plan_budget())
                    .map(Some)
            })();
            let owner = match owner {
                Ok(owner) => owner,
                Err(error) => {
                    let resource = budget.plan_budget().map_or_else(
                        || error.contains("limit") || error.contains("exhaust"),
                        |plan| super::traits::FactsError::from_message(plan, &error).is_resource(),
                    );
                    if resource {
                        return Err(Exhausted);
                    }
                    None
                }
            };
            if candidate.precision == TargetPrecision::Exact
                && owner
                    .as_ref()
                    .is_some_and(|owner| owner.owned_box_pointee().is_some())
            {
                budget.spend(candidate.place.projection.len() + 1)?;
                candidate.place.projection.push(Projection::Deref);
                checked_vec_growth(&mut next, budget)?;
                next.push(candidate);
                continue;
            }
            let Some(cover) = read_cell_cover(
                cover_locals,
                cell_index,
                &candidate,
                &CellChannel::ReferenceValue,
                budget,
            )?
            else {
                return Ok((vec![], trace, Some(ResolutionFailure::IncompleteCover)));
            };
            for cell in cover {
                budget.spend(mul(origins(cell).0.len(), 2)?)?;
                if origins(cell).count() == 0 && cells[cell].ty.is_shared_str() {
                    // A shared `&str` with no issued loan origin designates
                    // immutable `'static` program data (string literals).
                    // Initialization is proven separately by move analysis.
                    static_referent = true;
                    continue;
                }
                if origins(cell).count() == 0 {
                    // The layout and cell exist. Missing current provenance is
                    // semantic uninitialization/move, not an unsupported layout.
                    failure = Some(ResolutionFailure::EmptyOrigins);
                }
                for origin in origins(cell).members() {
                    if origins_targets[origin].is_empty() {
                        // A `&str` origin that resolved to no storage (for
                        // example a call result derived from a literal) is
                        // `'static` program data, and a reborrow of storage
                        // behind a raw pointer has unbounded provenance that
                        // the unsafe contract delegates to the programmer.
                        static_referent = true;
                    }
                    budget.spend(add(trace.len(), 1)?)?;
                    if !trace.contains(&ReferenceOriginId(origin)) {
                        checked_vec_growth(&mut trace, budget)?;
                        trace.push(ReferenceOriginId(origin));
                    }
                    for base in &origins_targets[origin] {
                        let carry = if matches!(
                            cells[cell].ty,
                            Ty::Ref {
                                kind: BorrowKind::Mut,
                                ..
                            }
                        ) {
                            candidate.authority.len()
                        } else {
                            0
                        };
                        budget.spend(
                            base.place.projection.len() + base.authority.len() + carry + 1,
                        )?;
                        budget.reserve_bytes(target_payload_bytes(
                            1,
                            base.place.projection.len(),
                            base.authority.len() + carry + 1,
                            3,
                        )?)?;
                        let mut target = base.clone();
                        if carry != 0 {
                            target.authority.extend_from_slice(&candidate.authority);
                        }
                        target.authority.push(origin);
                        target.authority.sort_unstable();
                        target.authority.dedup();
                        budget.spend(mul(
                            next.len(),
                            add(
                                target.authority.len(),
                                add(target.place.projection.len(), 1)?,
                            )?,
                        )?)?;
                        if !next.contains(&target) {
                            if next.len() >= MAX_MEMBERSHIPS {
                                return Err(Exhausted);
                            }
                            checked_vec_growth(&mut next, budget)?;
                            next.push(target);
                        }
                        if next.len() > MAX_MEMBERSHIPS {
                            return Err(Exhausted);
                        }
                    }
                }
            }
        }
        candidates = next;
    }
    if candidates.is_empty() && failure.is_none() && !static_referent {
        failure = Some(ResolutionFailure::EmptyOrigins);
    }
    Ok((candidates, trace, failure))
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct FlowState {
    cells: Vec<CarrierOrigins>,
    overwritten: Set,
}
impl FlowState {
    fn union(&mut self, other: &Self, budget: &mut Budget) -> Result<bool, Exhausted> {
        let mut changed = false;
        for (a, b) in self.cells.iter_mut().zip(&other.cells) {
            changed |= a.union(b, budget)?;
        }
        let before = self.overwritten.clone();
        self.overwritten.intersect(&other.overwritten, budget)?;
        Ok(changed || before != self.overwritten)
    }
}

fn key_channel(ty: &Ty) -> Option<CellChannel> {
    match ty {
        Ty::Ref { .. } => Some(CellChannel::ReferenceValue),
        Ty::DependentArray { .. } => Some(CellChannel::Family(ir::CarrierFamily::Array(Arc::new(
            ty.clone(),
        )))),
        Ty::Param(parameter) => Some(CellChannel::Family(ir::CarrierFamily::Parameter(
            *parameter,
        ))),
        Ty::Opaque(opaque) => Some(CellChannel::Family(ir::CarrierFamily::Opaque(
            opaque.clone(),
        ))),
        Ty::AssociatedType(projection) => Some(CellChannel::Family(ir::CarrierFamily::Projection(
            projection.clone(),
        ))),
        _ => None,
    }
}

fn result_flow_problem(
    view: Option<&super::traits::CarrierContractView>,
    facts: Option<&super::traits::ValidatedFacts>,
    subject: ir::SignatureSubject<'_>,
    signature: &ir::Signature,
    location: &CarrierLocation,
    view_validated: &mut bool,
) -> Result<Option<Problem>, Exhausted> {
    let Some(facts) = facts else {
        // Concrete legacy fixtures have no declaration authority to replay.
        return Ok(None);
    };
    let Some(view) = view else {
        return Ok(Some(Problem::Invalid(
            "carrier result has no validated declaration view".into(),
        )));
    };
    match view.validate_result_flow_with(facts, subject, signature, location, view_validated) {
        Ok(()) => Ok(None),
        Err(error) if error.is_resource() => Err(Exhausted),
        Err(error) => Ok(Some(Problem::Invalid(
            error
                .into_diagnostic(facts.budget())
                .map_err(|_| Exhausted)?,
        ))),
    }
}

fn physical_result_flow_problem(
    physical: Option<&super::opaque_physical::PhysicalCarrierFlow>,
    view: Option<&super::traits::CarrierContractView>,
    facts: Option<&super::traits::ValidatedFacts>,
    subject: ir::SignatureSubject<'_>,
    signature: &ir::Signature,
    location: &CarrierLocation,
    view_validated: &mut bool,
) -> Result<Option<Problem>, Exhausted> {
    let Some(physical) = physical else {
        return result_flow_problem(view, facts, subject, signature, location, view_validated);
    };
    let (Some(view), Some(facts)) = (view, facts) else {
        return Ok(Some(Problem::Invalid(
            "physical result lost its view or facts".into(),
        )));
    };
    match physical.validate_result(view, facts, subject, signature, location) {
        Ok(()) => Ok(None),
        Err(message) => {
            let error = super::traits::FactsError::from_message(facts.budget(), &message);
            if error.is_resource() {
                Err(Exhausted)
            } else {
                Ok(Some(Problem::Invalid(message)))
            }
        }
    }
}

/// A root generic result instantiated to shared tuple components retains each
/// element's declared family lifetime, not the lifetime of the slice descriptor.
/// Selection replays both input and result component mappings before the existing
/// authenticated result-flow and origin checks, never synthesizing a source loan.
fn borrowed_slice_family_result(
    view: &super::traits::CarrierContractView,
    location: &CarrierLocation,
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    let fixed = CarrierChannel::Fixed(ReferencePosition { depth: 0 });
    if location.channel != fixed {
        return Ok(false);
    }
    for original in view.original_contract().results.iter() {
        budget.spend(1)?;
        let CarrierChannel::ParamTail {
            parameter,
            prefix: 0,
        } = original.location.channel
        else {
            continue;
        };
        let family = ir::CarrierFamily::Parameter(parameter);
        if !original.location.path.0.is_empty() || original.sources.is_empty() {
            continue;
        }
        let mut declared = true;
        for source in original.sources.iter() {
            budget.spend(view.original_contract().inputs.len() + 1)?;
            declared &= source.location.channel == original.location.channel
                && source.location.path.0.as_ref()
                    == [CarrierStep::Referent, CarrierStep::SliceElement]
                && view.original_contract().inputs.contains(source);
        }
        if !declared {
            continue;
        }
        for mapping in view.component_mappings() {
            budget.spend(1)?;
            if mapping.original_family != family
                || mapping.original != original.location
                || mapping.substituted != *location
            {
                continue;
            }
            for result in view.contract().results.iter() {
                budget.spend(1)?;
                if result.location != *location || result.sources.len() != original.sources.len() {
                    continue;
                }
                let mut exact = true;
                for (source, original_source) in result.sources.iter().zip(original.sources.iter())
                {
                    budget.spend(view.contract().inputs.len() + 1)?;
                    let original_path = original_source.location.path.0.as_ref();
                    let actual_path = source.location.path.0.as_ref();
                    exact &= source.param == original_source.param
                        && actual_path.strip_prefix(original_path)
                            == Some(location.path.0.as_ref())
                        && source.location.channel == fixed
                        && view.contract().inputs.contains(source);
                    let mut input_mapping = false;
                    for input in view.component_mappings() {
                        budget.spend(1)?;
                        input_mapping |= input.original_family == family
                            && input.original == original_source.location
                            && input.substituted == source.location;
                    }
                    exact &= input_mapping;
                }
                if exact {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// One owned parameter-family value returned unchanged from a field or variant.
/// This shape does not prove that its actual is a reference or grant targets.
fn owned_shared_family_result_rows<'a>(
    original: &'a ir::CarrierContract,
    expanded: &'a ir::CarrierContract,
    location: &CarrierLocation,
    budget: &mut Budget,
) -> Result<Option<(ir::ParamId, &'a InputCarrier, &'a InputCarrier)>, Exhausted> {
    budget.spend(1)?;
    if *location != fixed_location(ReferencePosition { depth: 0 })
        || original.results.len() != 1
        || !expanded.writes.is_empty()
    {
        return Ok(None);
    }
    let row = &original.results[0];
    let CarrierChannel::ParamTail {
        parameter,
        prefix: 0,
    } = row.location.channel
    else {
        return Ok(None);
    };
    if !row.location.path.0.is_empty() || row.sources.len() != 1 {
        return Ok(None);
    }
    let source = &row.sources[0];
    budget.spend(original.inputs.len() + source.location.path.0.len() + 1)?;
    if source.location.channel != row.location.channel
        || !original.inputs.contains(source)
        || !source.location.path.0.iter().all(|step| {
            matches!(
                step,
                CarrierStep::Field(_) | CarrierStep::VariantField { .. }
            )
        })
    {
        return Ok(None);
    }
    let mut result = None;
    for row in expanded.results.iter() {
        budget.spend(1)?;
        if row.location == *location {
            if result.is_some() || row.sources.len() != 1 {
                return Ok(None);
            }
            result = Some(&row.sources[0]);
        }
    }
    let Some(actual) = result else {
        return Ok(None);
    };
    budget.spend(expanded.inputs.len() + 1)?;
    if actual.param != source.param
        || actual.location.channel != CarrierChannel::Fixed(ReferencePosition { depth: 0 })
        || !expanded.inputs.contains(actual)
    {
        return Ok(None);
    }
    Ok(Some((parameter, source, actual)))
}

/// Whether returned fixed `location` lies inside a substituted type-parameter
/// family result: the original contract declares a family result row with
/// family sources, and the substituted contract has a row at `location` whose
/// every source is the same parameter path as a declared family source.
fn substituted_family_result(
    view: &super::traits::CarrierContractView,
    location: &CarrierLocation,
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    let CarrierChannel::Fixed(position) = location.channel else {
        return Ok(false);
    };
    if !location.path.0.is_empty() {
        return Ok(false);
    }
    for original in view.original_contract().results.iter() {
        budget.spend(1)?;
        let CarrierChannel::ParamTail { prefix, .. } = original.location.channel else {
            continue;
        };
        if !original.location.path.0.is_empty()
            || usize::from(prefix) > usize::from(position.depth)
            || original.sources.is_empty()
            || !original
                .sources
                .iter()
                .all(|source| matches!(source.location.channel, CarrierChannel::ParamTail { .. }))
        {
            continue;
        }
        for result in view.contract().results.iter() {
            budget.spend(result.sources.len() + 1)?;
            if result.location != *location || result.sources.is_empty() {
                continue;
            }
            let mut mapped = true;
            for source in result.sources.iter() {
                budget.spend(original.sources.len() + view.contract().inputs.len() + 1)?;
                mapped &= matches!(source.location.channel, CarrierChannel::Fixed(_))
                    && view.contract().inputs.contains(source)
                    && original.sources.iter().any(|declared| {
                        declared.param == source.param
                            && declared.location.path == source.location.path
                    });
            }
            if mapped {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Exact descriptor reborrow shape, not an admission of arbitrary non-Copy
/// referents. The descriptor loan and stored element family remain separate.
fn identity_slice_reborrow_rows(
    original: &ir::CarrierContract,
    expanded: &ir::CarrierContract,
    budget: &mut Budget,
) -> Result<Option<ir::ParamId>, Exhausted> {
    budget.spend(64)?;
    if original.inputs.len() != 2
        || original.results.len() != 2
        || original.writes.len() != 1
        || expanded.inputs.len() != 2
        || expanded.results.len() != 2
        || expanded.writes.len() != 1
    {
        return Ok(None);
    }
    let family = &original.writes[0].destination;
    let CarrierChannel::ParamTail {
        parameter,
        prefix: 0,
    } = family.location.channel
    else {
        return Ok(None);
    };
    if family.location.path.0.as_ref() != [CarrierStep::Referent, CarrierStep::SliceElement]
        || !exact_identity_write_rows(original, budget)?
        || !exact_identity_write_rows(expanded, budget)?
    {
        return Ok(None);
    }
    let root = InputCarrier {
        param: family.param,
        location: fixed_location(ReferencePosition { depth: 0 }),
    };
    if !original.inputs.contains(&root) {
        return Ok(None);
    }
    let Some(original_root) = original
        .results
        .iter()
        .find(|row| row.location == root.location)
    else {
        return Ok(None);
    };
    let Some(original_family) = original
        .results
        .iter()
        .find(|row| row.location == family.location)
    else {
        return Ok(None);
    };
    if original_root.sources.len() != 2
        || !original_root.sources.contains(&root)
        || !original_root.sources.contains(family)
        || original_family.sources.as_ref() != std::slice::from_ref(family)
    {
        return Ok(None);
    }
    let actual = &expanded.writes[0].destination;
    if actual.param != family.param
        || actual.location.path != family.location.path
        || actual.location.channel != CarrierChannel::Fixed(ReferencePosition { depth: 0 })
        || !expanded.inputs.contains(&root)
    {
        return Ok(None);
    }
    let Some(expanded_root) = expanded
        .results
        .iter()
        .find(|row| row.location == root.location)
    else {
        return Ok(None);
    };
    let Some(expanded_family) = expanded
        .results
        .iter()
        .find(|row| row.location == actual.location)
    else {
        return Ok(None);
    };
    if expanded_root.sources.as_ref() != std::slice::from_ref(&root)
        || expanded_family.sources.as_ref() != std::slice::from_ref(actual)
    {
        return Ok(None);
    }
    Ok(Some(parameter))
}

/// One returned mutable slot containing one mutable reference is a linked
/// reborrow, not two independent affine results. Shape alone grants nothing:
/// the consumer also authenticates both component anchors and the exact actual.
fn identity_nested_slot_reborrow_rows(
    original: &ir::CarrierContract,
    expanded: &ir::CarrierContract,
    budget: &mut Budget,
) -> Result<Option<ir::ParamId>, Exhausted> {
    budget.spend(96)?;
    if original.inputs.len() != 2
        || original.results.len() != 2
        || original.writes.len() != 1
        || expanded.inputs.len() != 2
        || expanded.results.len() != 2
        || expanded.writes.len() != 1
        || !exact_identity_write_rows(original, budget)?
        || !exact_identity_write_rows(expanded, budget)?
    {
        return Ok(None);
    }
    let family = &original.writes[0].destination;
    let CarrierChannel::ParamTail {
        parameter,
        prefix: 0,
    } = family.location.channel
    else {
        return Ok(None);
    };
    if family.location.path.0.as_ref() != [CarrierStep::Referent, CarrierStep::SliceElement] {
        return Ok(None);
    }
    let root = InputCarrier {
        param: family.param,
        location: fixed_location(ReferencePosition { depth: 0 }),
    };
    let nested_original = CarrierLocation {
        path: CarrierPath::default(),
        channel: CarrierChannel::ParamTail {
            parameter,
            prefix: 1,
        },
    };
    let nested_expanded = fixed_location(ReferencePosition { depth: 1 });
    let actual = &expanded.writes[0].destination;
    if original.inputs[0] != root
        || original.inputs[1] != *family
        || original.results[0].location != root.location
        || original.results[0].sources.as_ref() != [root.clone(), family.clone()]
        || original.results[1].location != nested_original
        || original.results[1].sources.as_ref() != std::slice::from_ref(family)
        || actual.param != family.param
        || actual.location.path != family.location.path
        || actual.location.channel != CarrierChannel::Fixed(ReferencePosition { depth: 0 })
        || expanded.inputs[0] != root
        || expanded.inputs[1] != *actual
        || expanded.results[0].location != root.location
        || expanded.results[0].sources.as_ref() != std::slice::from_ref(&root)
        || expanded.results[1].location != nested_expanded
        || expanded.results[1].sources.as_ref() != std::slice::from_ref(actual)
    {
        return Ok(None);
    }
    Ok(Some(parameter))
}

/// An identity envelope is a no-replacement proof only when its one source is
/// the exact original input, not a type-equal sibling or a duplicated row.
/// This predicate grants no loan or write permission by itself.
fn exact_identity_write_rows(
    contract: &ir::CarrierContract,
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    if contract.writes.is_empty() {
        return Ok(false);
    }
    for (index, row) in contract.writes.iter().enumerate() {
        budget.spend(add(contract.inputs.len(), add(index, row.sources.len())?)?)?;
        if row.sources.len() != 1
            || row.sources[0] != row.destination
            || !contract.inputs.contains(&row.destination)
            || contract.writes[..index]
                .iter()
                .any(|prior| prior.destination == row.destination)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Compare declaration-owned fixed coordinates after the authenticated nominal
/// substitution. This is coordinate replay only, never a type/loan permission.
fn fixed_reborrow_location_matches(
    original: &CarrierLocation,
    expanded: &CarrierLocation,
    nominals: &[ir::NominalCarrierMapping],
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    budget.spend(add(original.path.0.len(), 1)?)?;
    if original.channel != CarrierChannel::Fixed(ReferencePosition { depth: 0 })
        || expanded.channel != original.channel
        || original.path.0.len() != expanded.path.0.len()
    {
        return Ok(false);
    }
    for (before, after) in original.path.0.iter().zip(expanded.path.0.iter()) {
        match (before, after) {
            (CarrierStep::Field(a), CarrierStep::Field(b)) if a == b => {}
            (
                CarrierStep::VariantField {
                    variant: a,
                    field: af,
                },
                CarrierStep::VariantField {
                    variant: b,
                    field: bf,
                },
            ) => {
                budget.spend(add(nominals.len(), 1)?)?;
                let mut owner = a.owner;
                let mut mapped = false;
                for mapping in nominals {
                    if mapping.original == a.owner {
                        if mapped {
                            return Ok(false);
                        }
                        mapped = true;
                        owner = mapping.instantiated;
                    }
                }
                if owner != b.owner || a.index != b.index || af != bf {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// The one affine result must replay one exact original fixed source. Other
/// family sources retain their separate component/Copy checks at the consumer.
fn copy_family_fixed_result_rows(
    original: &ir::CarrierContract,
    expanded: &ir::CarrierReturn,
    nominals: &[ir::NominalCarrierMapping],
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    budget.spend(1)?;
    if expanded.sources.len() != 1 {
        return Ok(false);
    }
    let mut matched = false;
    for row in original.results.iter() {
        if !fixed_reborrow_location_matches(&row.location, &expanded.location, nominals, budget)? {
            continue;
        }
        if matched {
            return Ok(false);
        }
        matched = true;
        budget.spend(add(add(row.sources.len(), original.inputs.len())?, 1)?)?;
        let mut fixed = row
            .sources
            .iter()
            .filter(|source| matches!(source.location.channel, CarrierChannel::Fixed(_)));
        let Some(source) = fixed.next() else {
            return Ok(false);
        };
        if fixed.next().is_some()
            || !original.inputs.contains(source)
            || source.param != expanded.sources[0].param
            || !fixed_reborrow_location_matches(
                &source.location,
                &expanded.sources[0].location,
                nominals,
                budget,
            )?
        {
            return Ok(false);
        }
    }
    Ok(matched)
}

/// Shape only: every authored write exchanges carriers of one exact original
/// parameter family. Return only families with a nonidentity authored row:
/// identity rows retain their exact source without copying the whole value.
/// Actual carrier Copy permission and complete instantiation replay remain
/// independent consumer checks, never inferred from this shape.
fn copy_family_effect_rows(
    contract: &ir::CarrierContract,
    budget: &mut Budget,
) -> Result<Option<Vec<ir::ParamId>>, Exhausted> {
    if contract.writes.is_empty() {
        return Ok(None);
    }
    let mut families = Vec::new();
    for (index, row) in contract.writes.iter().enumerate() {
        budget.spend(contract.inputs.len() + index + row.sources.len() + 1)?;
        let CarrierChannel::ParamTail {
            parameter,
            prefix: 0,
        } = row.destination.location.channel
        else {
            return Ok(None);
        };
        if !contract.inputs.contains(&row.destination)
            || contract.writes[..index]
                .iter()
                .any(|old| old.destination == row.destination)
            || row.sources.is_empty()
        {
            return Ok(None);
        }
        for (source_index, source) in row.sources.iter().enumerate() {
            budget.spend(contract.inputs.len() + source_index + 1)?;
            if source.location.channel != row.destination.location.channel
                || !contract.inputs.contains(source)
                || row.sources[..source_index].contains(source)
            {
                return Ok(None);
            }
        }
        budget.spend(families.len() + 1)?;
        if row.sources.as_ref() != std::slice::from_ref(&row.destination)
            && !families.contains(&parameter)
        {
            checked_vec_growth(&mut families, budget)?;
            families.push(parameter);
        }
    }
    Ok(Some(families))
}

/// Exact original owned input paired with one stored occurrence of the same
/// rigid family. This shape is not a Copy permission for the owned value.
fn owned_shared_family_write_source<'a>(
    contract: &'a ir::CarrierContract,
    schema: &ir::SignatureOriginSchema,
    parameter: ir::ParamId,
    budget: &mut Budget,
) -> Result<Option<&'a InputCarrier>, Exhausted> {
    let mut owner = None;
    for tail in schema.input_tails.iter() {
        budget.spend(1)?;
        if tail.parameter == parameter && (tail.prefix != 0 || owner.replace(tail.param).is_some())
        {
            return Ok(None);
        }
    }
    let Some(owner) = owner else { return Ok(None) };
    // The returned borrowed family must reciprocate this exact original
    // by-value input. Contract rows alone cannot authenticate origin tails.
    let mut result_tail = false;
    for tail in schema.result_tails.iter() {
        budget.spend(tail.inputs.len() + 1)?;
        if tail.parameter == parameter {
            if result_tail
                || tail.prefix != 1
                || tail.inputs.as_ref()
                    != [ir::InputCarrierTail {
                        param: owner,
                        prefix: 0,
                        parameter,
                    }]
            {
                return Ok(None);
            }
            result_tail = true;
        }
    }
    if !result_tail {
        return Ok(None);
    }
    let channel = CarrierChannel::ParamTail {
        parameter,
        prefix: 0,
    };
    let mut owned = None;
    let mut stored = None;
    for input in contract.inputs.iter() {
        budget.spend(input.location.path.0.len() + 1)?;
        if matches!(input.location.channel, CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
        {
            if input.location.channel != channel {
                return Ok(None);
            }
            if input.param == owner && input.location.path.0.is_empty() {
                if owned.replace(input).is_some() {
                    return Ok(None);
                }
            } else {
                let path = input.location.path.0.as_ref();
                if input.param == owner
                    || path.last() != Some(&CarrierStep::SliceElement)
                    || path
                        .iter()
                        .filter(|step| **step == CarrierStep::Referent)
                        .count()
                        != 1
                    || !path[..path.len() - 1]
                        .iter()
                        .all(|step| matches!(step, CarrierStep::Field(_) | CarrierStep::Referent))
                    || stored.replace(input).is_some()
                {
                    return Ok(None);
                }
            }
        }
    }
    let (Some(owned), Some(stored)) = (owned, stored) else {
        return Ok(None);
    };
    let mut owned_row = false;
    let mut stored_row = false;
    for row in contract.writes.iter() {
        budget.spend(row.sources.len() + 1)?;
        if matches!(row.destination.location.channel, CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
        {
            if row.sources.len() != 2
                || !row.sources.contains(owned)
                || !row.sources.contains(stored)
            {
                return Ok(None);
            }
            if row.destination == *owned {
                if owned_row {
                    return Ok(None);
                }
                owned_row = true;
            } else if row.destination == *stored {
                if stored_row {
                    return Ok(None);
                }
                stored_row = true;
            } else {
                return Ok(None);
            }
        }
    }
    if !owned_row || !stored_row {
        return Ok(None);
    }
    let mut borrowed_result = false;
    for row in contract.results.iter() {
        budget.spend(row.sources.len() + 1)?;
        if matches!(row.location.channel, CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
        {
            if borrowed_result
                || row.location.channel
                    != (CarrierChannel::ParamTail {
                        parameter,
                        prefix: 1,
                    })
                || !row.location.path.0.is_empty()
                || row.sources.len() != 2
                || !row.sources.contains(owned)
                || !row.sources.contains(stored)
            {
                return Ok(None);
            }
            borrowed_result = true;
        }
    }
    Ok(borrowed_result.then_some(owned))
}

fn body_signature(f: &cfg::Function, budget: &mut Budget) -> Result<ir::Signature, Exhausted> {
    budget.spend(f.params.len() + 1)?;
    budget.reserve_bytes(mul(f.params.len(), size_of::<Ty>())?)?;
    let mut params = Vec::new();
    params
        .try_reserve_exact(f.params.len())
        .map_err(|_| Exhausted)?;
    budget.reserve_bytes(mul(
        params.capacity().saturating_sub(f.params.len()),
        size_of::<Ty>(),
    )?)?;
    for local in &f.params {
        params.push(f.locals.get(local.0).ok_or(Exhausted)?.ty.clone());
    }
    Ok(ir::Signature {
        params,
        result: f.result.clone(),
        ref_contract: f.ref_contract.clone(),
    })
}

/// Shape evidence only. The caller must additionally prove every retained
/// family has a closed, carrier-free actual before admitting an affine pair.
#[allow(clippy::type_complexity)]
fn independent_slice_pair_rows(
    contract: &ir::CarrierContract,
    budget: &mut Budget,
) -> Result<Option<([usize; 2], [Option<ir::ParamId>; 2])>, Exhausted> {
    budget.spend(1)?;
    if !(2..=4).contains(&contract.results.len()) || !contract.writes.is_empty() {
        return Ok(None);
    }
    let fixed = CarrierChannel::Fixed(ReferencePosition { depth: 0 });
    let mut roots = [None; 2];
    let mut next_root = 0;
    for row in contract.results.iter() {
        budget.spend(1)?;
        if row.location.channel != fixed {
            continue;
        }
        let [CarrierStep::Field(index)] = row.location.path.0.as_ref() else {
            return Ok(None);
        };
        let mut fixed_source = None;
        for source in row.sources.iter() {
            budget.spend(contract.inputs.len() + 1)?;
            if !contract.inputs.contains(source) {
                return Ok(None);
            }
            if source.location.channel == fixed {
                if fixed_source.replace(source).is_some() || !source.location.path.0.is_empty() {
                    return Ok(None);
                }
            } else if !matches!(
                source.location.channel,
                CarrierChannel::ParamTail { prefix: 0, .. }
            ) || source.location.path.0.as_ref()
                != [CarrierStep::Referent, CarrierStep::SliceElement]
            {
                return Ok(None);
            }
        }
        let Some(source) = fixed_source else {
            return Ok(None);
        };
        if *index != next_root || *index >= 2 {
            return Ok(None);
        }
        roots[*index] = Some(source.param);
        next_root += 1;
    }
    let [Some(left), Some(right)] = roots else {
        return Ok(None);
    };
    if left == right {
        return Ok(None);
    }
    let roots = [left, right];
    let mut families = [None; 2];
    let mut outputs = [None; 2];
    for row in contract.results.iter() {
        budget.spend(1)?;
        if row.location.channel != fixed {
            let CarrierChannel::ParamTail {
                parameter,
                prefix: 0,
            } = row.location.channel
            else {
                return Ok(None);
            };
            let [
                CarrierStep::Field(index),
                CarrierStep::Referent,
                CarrierStep::SliceElement,
            ] = row.location.path.0.as_ref()
            else {
                return Ok(None);
            };
            if *index >= 2 || outputs[*index].replace(parameter).is_some() || row.sources.is_empty()
            {
                return Ok(None);
            }
        }
        for source in row.sources.iter() {
            budget.spend(contract.inputs.len() + 1)?;
            if source.location.channel == fixed {
                if row.location.channel != fixed {
                    return Ok(None);
                }
                continue;
            }
            let CarrierChannel::ParamTail {
                parameter,
                prefix: 0,
            } = source.location.channel
            else {
                return Ok(None);
            };
            let Some(index) = roots.iter().position(|root| *root == source.param) else {
                return Ok(None);
            };
            if source.location.path.0.as_ref() != [CarrierStep::Referent, CarrierStep::SliceElement]
                || !contract.inputs.contains(source)
                || families[index].is_some_and(|previous| previous != parameter)
            {
                return Ok(None);
            }
            families[index] = Some(parameter);
        }
    }
    for index in 0..2 {
        budget.spend(1)?;
        if outputs[index].is_some() && outputs[index] != families[index] {
            return Ok(None);
        }
    }
    Ok(Some((roots, families)))
}

/// An unexported witness minted only at successful complete body-view replay.
/// It cannot outlive the original builder inputs and is consumed at sealing.
struct BodyViewValidated<'a> {
    function: &'a cfg::Function,
    facts: &'a super::traits::ValidatedFacts,
    view: Arc<super::traits::CarrierContractView>,
}

/// The sole cell-state builder feeds the existing Plan queries, conflict and
/// activation logic. Families are value relations, never fabricated loans.
struct CarrierBuilder<'a, 'b> {
    physical: Option<&'a super::opaque_physical::PhysicalValidation<'a>>,
    f: &'a cfg::Function,
    facts: FactsCtx<'a>,
    ledger: ir::PlanBudget,
    budget: &'b mut Budget,
    max_bytes: usize,
    bytes: usize,
    allocation_start: usize,
    memberships: usize,
    diagnosing: bool,
    cells: Vec<CellSlot>,
    cell_index: HashMap<CellKey, usize>,
    original_locals: Vec<Ty>,
    family_cover_locals: Option<Vec<Ty>>,
    initial: FlowState,
    reference_inputs: Vec<(InputCarrier, ReferenceOriginId)>,
    /// Issued only for literal data, never inferred from a reference type.
    static_origin: Option<ReferenceOriginId>,
    family_inputs: Vec<FamilyOrigin>,
    targets: Vec<Vec<Target>>,
    kinds: Vec<BorrowKind>,
    parents: Vec<Set>,
    dependencies: Vec<Set>,
    /// Liveness-only `T: 'r` edges from a borrow to loans held inside its
    /// referent. Not region provenance: the borrow does not derive from them.
    outlives: Vec<Set>,
    calls: Vec<Option<Arc<super::traits::CarrierContractView>>>,
    physical_calls: Vec<Option<super::opaque_physical::PhysicalCarrierFlow>>,
    // Each block has one immutable callee/view and this builder has one fixed
    // facts/table pair. Cache only successful lookups, including no summary,
    // so dataflow revisits do not replay identical authority proofs.
    call_effects: Vec<Option<Option<Arc<CarrierEffects>>>>,
    body: Option<Arc<super::traits::CarrierContractView>>,
    body_validated: Option<BodyViewValidated<'a>>,
    physical_body: Option<super::opaque_physical::PhysicalCarrierFlow>,
    body_signature: Option<ir::Signature>,
    call_rows: Vec<Vec<(CarrierLocation, ReferenceOriginId)>>,
    raw_slice_rows: Vec<(BlockId, usize, ReferenceOriginId)>,
    function: Option<Arc<cfg::Function>>,
    authority: Option<Arc<super::traits::ValidatedFacts>>,
    /// Exact `(function, facts)` pair whose provenance already validated in
    /// this plan. Validation is a pure function of these immutable Arcs, so a
    /// repeated per-call check would only redo O(blocks * evidence) work.
    validated_provenance: Option<(Arc<cfg::Function>, Arc<super::traits::ValidatedFacts>)>,
    // Fixed-point revisits query the same immutable carrier types. The trait
    // scratch retains exact payload/caller proofs and resets on facts changes.
    copy_scratch: super::traits::TraitScratch,
    /// Construction and solver effect replay share exact caller-owned proof
    /// arenas. Each lookup still validates its full immutable view and table.
    carrier_scratch: super::traits::TraitScratch,
    bindings: Vec<CallCarrierBinding>,
    regions: RegionRelations,
    effect_table: Option<Arc<super::ownership::CarrierEffectTable>>,
    input_regions: Vec<(InputCarrier, AnalysisRegionId)>,
    /// Explicit `'static` sequence-element inputs: they outlive every region.
    static_inputs: Vec<InputCarrier>,
    family_regions: Vec<(ir::CarrierFamily, CarrierLocation, AnalysisRegionId)>,
    singleton_heap_slot: Option<SingletonHeapSlot>,
    native_storage: Vec<NativeStorageTransfer>,
    native_exchanges: Vec<NativeStorageExchange>,
    problems: Vec<(BlockId, Option<usize>, Problem)>,
}
impl CarrierBuilder<'_, '_> {
    /// Validate the exact caller CFG provenance at most once per plan.
    /// Failures are not memoized, so every failing call still reports.
    fn ensure_provenance(
        &mut self,
        function: &Arc<cfg::Function>,
        facts: &Arc<super::traits::ValidatedFacts>,
    ) -> Result<(), String> {
        if let Some((done_function, done_facts)) = &self.validated_provenance
            && Arc::ptr_eq(done_function, function)
            && Arc::ptr_eq(done_facts, facts)
        {
            return Ok(());
        }
        if let Some(physical) = self.physical {
            physical.validate_cfg(function, facts)?;
            if let Some(proof) = &function.body_substitution {
                proof.validate_cfg_with_scratch(
                    function,
                    facts,
                    &self.ledger,
                    &mut self.carrier_scratch,
                )?;
            }
        } else {
            // Canonical provenance includes the exact body-substitution proof.
            function.validate_provenance_with_scratch(
                facts,
                &self.ledger,
                &mut self.carrier_scratch,
            )?;
        }
        self.validated_provenance = Some((function.clone(), facts.clone()));
        Ok(())
    }
    fn bind_call(
        &mut self,
        state: &FlowState,
        block: BlockId,
        view: &Arc<super::traits::CarrierContractView>,
        args: &[Operand],
        signature: &ir::Signature,
        target: &ir::Callee,
    ) -> Result<Option<CallCarrierBinding>, Exhausted> {
        let (Some(function), Some(facts)) = (self.function.clone(), self.authority.clone()) else {
            return Ok(None);
        };
        let Some(call) = self.f.blocks[block.0].terminator.origin else {
            self.problem(
                block,
                None,
                Problem::Invalid("call has no original expression identity".into()),
            )?;
            return Ok(None);
        };
        self.budget.spend(function.call_evidence.len())?;
        let evidence = if let Some(physical) = self.physical {
            match physical.original_call_evidence(&function, &facts, block) {
                Ok(evidence) => Some(evidence),
                Err(message) => {
                    if super::traits::FactsError::from_message(&self.ledger, &message).is_resource()
                    {
                        return Err(Exhausted);
                    }
                    self.problem(block, None, Problem::Invalid(message))?;
                    return Ok(None);
                }
            }
        } else {
            function
                .call_evidence
                .iter()
                .find(|evidence| evidence.call() == call)
                .cloned()
        };
        let Some(evidence) = evidence else {
            self.problem(
                block,
                None,
                Problem::Invalid("call lacks exact validated source-stage evidence".into()),
            )?;
            return Ok(None);
        };
        if let Err(message) = self.ensure_provenance(&function, &facts) {
            if message.contains("limit") || message.contains("exhaust") {
                return Err(Exhausted);
            }
            self.problem(block, None, Problem::Invalid(message))?;
            return Ok(None);
        }
        let capsule_facts = evidence.facts();
        let same_context = Arc::ptr_eq(capsule_facts, &facts)
            || (function.body_substitution.is_none()
                && capsule_facts.phase() == super::traits::FactsPhase::Closed
                && facts.phase() == super::traits::FactsPhase::Closed
                && Arc::ptr_eq(capsule_facts.world(), facts.world())
                && Arc::ptr_eq(capsule_facts.predicates(), facts.predicates())
                && Arc::ptr_eq(capsule_facts.env(), facts.env())
                && capsule_facts.registry_snapshot() == facts.registry_snapshot()
                && capsule_facts.budget().same_authority(facts.budget())
                && capsule_facts.declaration() == facts.declaration()
                && capsule_facts.target().is_some_and(|target| {
                    target.declaration == facts.declaration() && target.type_args.is_empty()
                })
                && facts
                    .target()
                    .is_some_and(|target| target.type_args.is_empty()));
        let bridge = function.body_substitution.as_ref().is_some_and(|proof| {
            Arc::ptr_eq(proof.template().source(), evidence.source())
                && Arc::ptr_eq(proof.template().facts(), capsule_facts)
                && facts.phase() == super::traits::FactsPhase::Closed
        });
        // In this branch `evidence` was obtained only by the strict borrowed
        // original_call_evidence replay above. The physical view itself is
        // independently rederived below before any carrier rows are consumed.
        let physical_bridge = self.physical.is_some();
        if !same_context && !bridge && !physical_bridge {
            self.problem(
                block,
                None,
                Problem::Invalid(
                    "call evidence has no exact original-to-closed phase bridge".into(),
                ),
            )?;
            return Ok(None);
        }
        // `calls` is populated only by the checked call-view constructors in
        // `new`, after result-flow validation. Reuse that exact immutable Arc
        // instead of replaying its entire nominal authority for each binding.
        if !self.calls[block.0]
            .as_ref()
            .is_some_and(|validated| Arc::ptr_eq(validated, view))
        {
            self.problem(
                block,
                None,
                Problem::Invalid("call binding changed its validated carrier view".into()),
            )?;
            return Ok(None);
        }
        let Some(expression) = evidence.source().exprs.get(call.0) else {
            return Err(Exhausted);
        };
        let ir::ExprKind::Call {
            args: original_args,
            ..
        } = &expression.kind
        else {
            self.problem(
                block,
                None,
                Problem::Invalid("source-stage call capsule does not name a call".into()),
            )?;
            return Ok(None);
        };
        if evidence.trace().arguments.len() != args.len()
            || original_args.len() != args.len()
            || evidence
                .trace()
                .arguments
                .iter()
                .enumerate()
                .any(|(index, row)| row.argument != index || row.value != original_args[index])
        {
            self.problem(
                block,
                None,
                Problem::Invalid("source-stage call argument trace changed".into()),
            )?;
            return Ok(None);
        }
        // Type arguments can be present without any value carrying that
        // parameter (`probe::<T>()`). Such calls still owe the declared
        // outlives bounds, even in an otherwise unused generic function.
        if self.diagnosing && function.body_substitution.is_none() {
            let offset = match target {
                ir::Callee::Direct(_) => 0,
                ir::Callee::TraitItem(call)
                | ir::Callee::Virtual(ir::VirtualCall { call })
                | ir::Callee::Untupled(call) => 1 + call.trait_ref.args.len(),
            };
            for (index, parameter) in view.original_env().ordered_params().enumerate() {
                if !view.original_env().outlives_static(parameter) {
                    continue;
                }
                let actual = view.arguments().get(index).ok_or(Exhausted)?;
                if let Err(message) =
                    ir::check_static_bound_metadata(actual, facts.env(), &self.ledger)
                {
                    if self.ledger.has_resource_failure() {
                        return Err(Exhausted);
                    }
                    self.problem(block, None, Problem::Invalid(message))?;
                }
                let locations =
                    ir::carrier_locations(actual, &self.ledger).map_err(|_| Exhausted)?;
                for location in locations.iter() {
                    self.budget.spend(1)?;
                    if let Some(family) = location.channel.family() {
                        let permitted = match family {
                            ir::CarrierFamily::Parameter(own) => facts
                                .parameter_outlives_static(own)
                                .map_err(|_| Exhausted)?,
                            _ => false,
                        };
                        if !permitted {
                            self.problem(
                                block,
                                None,
                                Problem::Invalid(
                                    "type argument does not satisfy its declared `'static` bound"
                                        .into(),
                                ),
                            )?;
                        }
                    }
                }
                if let Some(source_index) = index.checked_sub(offset) {
                    for row in evidence
                        .trace()
                        .type_actual_regions
                        .iter()
                        .filter(|row| row.argument == source_index)
                    {
                        self.budget.spend(1)?;
                        if matches!(row.source, ir::ActualRegion::Named(_)) {
                            self.problem(block, None, Problem::Invalid(
                                "lifetime may not live long enough: type argument must outlive `'static`".into(),
                            ))?;
                        }
                    }
                }
            }
        }
        let mut entry = Vec::new();
        for input in view.contract().inputs.iter() {
            let origins = self.input_value(state, args, input)?;
            self.retain(
                4 * size_of::<CarrierEntrySnapshot>()
                    + (self.targets.len().div_ceil(64) + self.family_inputs.len().div_ceil(64)) * 8,
                1,
            )?;
            checked_vec_growth(&mut entry, self.budget)?;
            entry.push(CarrierEntrySnapshot {
                input: input.clone(),
                origins,
            });
        }
        let mut destinations = Vec::new();
        for row in view.contract().writes.iter() {
            let Some(Operand::Copy(actual) | Operand::Move(actual)) =
                args.get(row.destination.param)
            else {
                continue;
            };
            let mut place = actual.clone();
            place
                .projection
                .extend(value_projection(&row.destination.location));
            let (targets, _, _) = self.lens(state, &place)?;
            let mut old = Vec::new();
            for target in &targets {
                if target.precision != TargetPrecision::Exact {
                    continue;
                }
                for (index, cell) in self.cells.iter().enumerate() {
                    self.budget.spend(1 + cell.key.storage.projection.len())?;
                    if cell.key.storage == target.place {
                        self.budget.reserve_bytes(
                            4 * size_of::<(CellKey, CarrierOrigins)>()
                                + mul(cell.key.storage.projection.len(), size_of::<Projection>())?
                                + carrier_bitmap_bytes(
                                    1,
                                    self.targets.len(),
                                    self.family_inputs.len(),
                                )?,
                        )?;
                        checked_vec_growth(&mut old, self.budget)?;
                        old.push((cell.key.clone(), state.cells[index].clone()));
                    }
                }
            }
            self.retain(4 * size_of::<CarrierDestinationBinding>(), 1)?;
            checked_vec_growth(&mut destinations, self.budget)?;
            destinations.push(CarrierDestinationBinding {
                destination: row.destination.clone(),
                targets,
                old,
            });
        }
        let rigid = self.regions.copied(self.budget)?;
        let mut relation = rigid.copied(self.budget)?;
        let mut regions = Vec::new();
        for index in 0..view.ref_contract().region_count {
            let term = ir::CarrierRegionTerm::Fixed(ir::RegionParamId(index));
            let region = relation.node(
                RegionKey::Occurrence {
                    block,
                    term: term.clone(),
                    component: None,
                },
                self.budget,
            )?;
            checked_vec_growth(&mut regions, self.budget)?;
            regions.push(CallRegionBinding {
                schema: term,
                component: None,
                region,
            });
        }
        for index in 0..view.source_region_env().region_count {
            let source = view.source_region_env();
            let term = ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                declaration: source.declaration,
                index,
            });
            let region = if let Some(fixed) = source.normalized[usize::from(index)] {
                regions
                    .iter()
                    .find(|row| row.schema == ir::CarrierRegionTerm::Fixed(fixed))
                    .ok_or(Exhausted)?
                    .region
            } else {
                relation.node(
                    RegionKey::Occurrence {
                        block,
                        term: term.clone(),
                        component: None,
                    },
                    self.budget,
                )?
            };
            self.retain(4 * size_of::<CallRegionBinding>(), 1)?;
            checked_vec_growth(&mut regions, self.budget)?;
            regions.push(CallRegionBinding {
                schema: term,
                component: None,
                region,
            });
        }
        for input in view.ref_contract().input_regions.iter() {
            let actual = self.input_value(
                state,
                args,
                &InputCarrier {
                    param: input.input.param,
                    location: fixed_location(input.input.position),
                },
            )?;
            let sources = self.origin_regions(&actual)?;
            // A `'static` parameter admits only data that itself outlives
            // every region: static program data, loan-free values, or the
            // caller's own `'static` inputs. Local loans and non-static
            // caller parameters are rejected.
            if view.ref_contract().static_regions & (1u64 << input.region.0) != 0 && self.diagnosing
            {
                let own_static = self
                    .body
                    .as_ref()
                    .map_or(0, |body| body.ref_contract().static_regions);
                let escapes = sources.iter().any(|region| {
                    !matches!(self.regions.keys[region.0], RegionKey::Fixed(id)
                        if own_static & (1u64 << id.0) != 0)
                });
                if escapes {
                    self.problem(
                        block,
                        None,
                        Problem::Invalid(
                            "borrowed value does not live long enough: argument requires that it is borrowed for `'static`".into(),
                        ),
                    )?;
                }
            }
            let source_keys: Vec<_> = sources
                .iter()
                .map(|region| self.regions.keys[region.0].clone())
                .collect();
            let variable = regions
                .iter()
                .find(|row| row.schema == ir::CarrierRegionTerm::Fixed(input.region))
                .ok_or(Exhausted)?
                .region;
            let mut invariant = false;
            let location = fixed_location(input.input.position);
            let prefix = value_projection(&location);
            for (depth, step) in prefix.iter().enumerate() {
                if *step == Projection::Deref
                    && matches!(
                        ir::projected_ty(
                            &signature.params[input.input.param],
                            &prefix[..depth],
                            &self.ledger
                        )
                        .map_err(|_| Exhausted)?,
                        Ty::Ref {
                            kind: BorrowKind::Mut,
                            ..
                        }
                    )
                {
                    invariant = true;
                }
            }
            let external = if invariant {
                if let Some(Operand::Copy(actual) | Operand::Move(actual)) =
                    args.get(input.input.param)
                {
                    let mut component = actual.clone();
                    component.projection.extend(prefix);
                    self.lens(state, &component)?.0.iter().any(|target| {
                        matches!(target.place.root, CanonicalRoot::ParameterReferent(..))
                    })
                } else {
                    false
                }
            } else {
                false
            };
            for key in source_keys {
                let source = relation.node(key, self.budget)?;
                relation.edge(source, variable, self.budget)?;
                if external {
                    relation.edge(variable, source, self.budget)?;
                }
            }
        }
        // Concrete sequence carriers are not represented by the positional
        // RefContract rows. Bind their original source locations independently,
        // using exactly the same input snapshot and occurrence as the call.
        for row in view.source_region_env().carriers.iter().filter(|row| {
            row.parameter.is_some()
                && row.location.path.0.contains(&CarrierStep::SliceElement)
                && matches!(
                    row.region,
                    ir::CarrierRegionTerm::Named(_) | ir::CarrierRegionTerm::Fixed(_)
                )
        }) {
            let Some(input) = source_region_input(view, row, self.budget)? else {
                self.problem(
                    block,
                    None,
                    Problem::Invalid(
                        "source carrier region has no exact original issued input".into(),
                    ),
                )?;
                continue;
            };
            let param = input.param;
            let Some(snapshot) = entry.iter().find(|snapshot| snapshot.input == input) else {
                return Err(Exhausted);
            };
            let actual = snapshot.origins.clone();
            let sources = self.origin_regions(&actual)?;
            self.retain(mul(sources.len(), size_of::<RegionKey>())?, sources.len())?;
            let source_keys: Vec<_> = sources
                .iter()
                .map(|region| self.regions.keys[region.0].clone())
                .collect();
            let variable = regions
                .iter()
                .find(|binding| binding.schema == row.region && binding.component.is_none())
                .ok_or(Exhausted)?
                .region;
            let prefix = value_projection(&row.location);
            let mut invariant = false;
            for (depth, step) in prefix.iter().enumerate() {
                self.budget.spend(1)?;
                if *step == Projection::Deref
                    && matches!(
                        ir::projected_ty(&signature.params[param], &prefix[..depth], &self.ledger)
                            .map_err(|_| Exhausted)?,
                        Ty::Ref {
                            kind: BorrowKind::Mut,
                            ..
                        }
                    )
                {
                    invariant = true;
                }
            }
            let external = if invariant {
                if let Some(Operand::Copy(actual) | Operand::Move(actual)) = args.get(param) {
                    let mut component = actual.clone();
                    component.projection.extend(prefix);
                    self.lens(state, &component)?.0.iter().any(|target| {
                        matches!(target.place.root, CanonicalRoot::ParameterReferent(..))
                    })
                } else {
                    false
                }
            } else {
                false
            };
            for key in source_keys {
                let source = relation.node(key, self.budget)?;
                relation.edge(source, variable, self.budget)?;
                if external {
                    relation.edge(variable, source, self.budget)?;
                }
            }
        }
        // Explicit 'static slice element carriers admit only data that
        // outlives every region, exactly like a fixed `'static` parameter.
        if self.diagnosing {
            let own_static = self
                .body
                .as_ref()
                .map_or(0, |body| body.ref_contract().static_regions);
            let rows: Vec<_> = view
                .source_region_env()
                .carriers
                .iter()
                .filter(|row| {
                    matches!(row.region, ir::CarrierRegionTerm::Static)
                        && row.location.path.0.contains(&CarrierStep::SliceElement)
                })
                .filter_map(|row| Some((row.parameter?, row.location.clone())))
                .collect();
            for (param, location) in rows {
                self.budget.spend(location.path.0.len() + 1)?;
                let actual = self.input_value(state, args, &InputCarrier { param, location })?;
                let sources = self.origin_regions(&actual)?;
                let escapes = sources.iter().any(|region| {
                    !matches!(self.regions.keys[region.0], RegionKey::Fixed(id)
                        if own_static & (1u64 << id.0) != 0)
                });
                if escapes {
                    self.problem(
                        block,
                        None,
                        Problem::Invalid(
                            "borrowed value does not live long enough: argument requires that it is borrowed for `'static`".into(),
                        ),
                    )?;
                }
            }
        }
        for original in view.original_contract().inputs.iter() {
            let Some(family) = original.location.channel.family() else {
                continue;
            };
            self.retain(
                mul(
                    view.component_mappings().len(),
                    size_of::<ir::CarrierComponentMapping>(),
                )?,
                view.component_mappings().len(),
            )?;
            let mappings: Vec<_> = view
                .component_mappings()
                .iter()
                .filter(|mapping| mapping.original_family == family)
                .cloned()
                .collect();
            for mapping in mappings {
                let location = self.compose_component(
                    &original.location,
                    &mapping.substituted,
                    view.nominal_mappings(),
                )?;
                let actual = self.input_value(
                    state,
                    args,
                    &InputCarrier {
                        param: original.param,
                        location: location.clone(),
                    },
                )?;
                let sources = self.origin_regions(&actual)?;
                // `T: 'static` on the callee admits only actuals free of
                // borrowed storage: static program data, the caller's own
                // `'static` inputs, or the caller's own `'static` parameters.
                // An instantiated template body was already checked in its
                // universal form, so it is not diagnosed a second time.
                if self.diagnosing
                    && function.body_substitution.is_none()
                    && let ir::CarrierFamily::Parameter(parameter) = &family
                    && view.original_env().outlives_static(*parameter)
                {
                    let own_static = self
                        .body
                        .as_ref()
                        .map_or(0, |body| body.ref_contract().static_regions);
                    self.budget.spend(sources.len() + 1)?;
                    let mut escapes = false;
                    for region in &sources {
                        escapes |= match &self.regions.keys[region.0] {
                            RegionKey::Fixed(id) => own_static & (1u64 << id.0) == 0,
                            RegionKey::Family {
                                family: ir::CarrierFamily::Parameter(own),
                                ..
                            } => !facts
                                .parameter_outlives_static(*own)
                                .map_err(|_| Exhausted)?,
                            _ => true,
                        };
                    }
                    if escapes {
                        self.problem(
                            block,
                            None,
                            Problem::Invalid(
                                "borrowed value does not live long enough: type argument must outlive `'static`".into(),
                            ),
                        )?;
                    }
                }
                // Bind the caller rigid nodes into this occurrence by key, not
                // by accidentally reusing a callee-local numeric region ID.
                self.retain(mul(sources.len(), size_of::<RegionKey>())?, sources.len())?;
                let source_keys: Vec<_> = sources
                    .iter()
                    .map(|region| self.regions.keys[region.0].clone())
                    .collect();
                let component = matches!(mapping.substituted.channel, CarrierChannel::Fixed(_))
                    .then(|| mapping.substituted.clone());
                let term = family_region_term(&family);
                let variable = if let Some(row) = regions
                    .iter()
                    .find(|row| row.schema == term && row.component == component)
                {
                    row.region
                } else {
                    let region = relation.node(
                        RegionKey::Occurrence {
                            block,
                            term: term.clone(),
                            component: component.clone(),
                        },
                        self.budget,
                    )?;
                    checked_vec_growth(&mut regions, self.budget)?;
                    regions.push(CallRegionBinding {
                        schema: term,
                        component: component.clone(),
                        region,
                    });
                    region
                };
                let mut invariant = false;
                // `location` is the substituted image of the original row, so
                // its nominal identities match the call's substituted signature.
                let prefix = value_projection(&location);
                for (index, step) in prefix.iter().enumerate() {
                    if *step == Projection::Deref
                        && matches!(
                            ir::projected_ty(
                                &signature.params[original.param],
                                &prefix[..index],
                                &self.ledger
                            )
                            .map_err(|_| Exhausted)?,
                            Ty::Ref {
                                kind: BorrowKind::Mut,
                                ..
                            }
                        )
                    {
                        invariant = true;
                    }
                }
                for key in source_keys {
                    let source = relation.node(key, self.budget)?;
                    relation.edge(source, variable, self.budget)?;
                    if invariant {
                        let Some(Operand::Copy(actual_place) | Operand::Move(actual_place)) =
                            args.get(original.param)
                        else {
                            continue;
                        };
                        let mut actual_place = actual_place.clone();
                        actual_place.projection.extend(value_projection(&location));
                        let (targets, _, _) = self.lens(state, &actual_place)?;
                        if targets.iter().any(|target| {
                            matches!(target.place.root, CanonicalRoot::ParameterReferent(..))
                        }) {
                            relation.edge(variable, source, self.budget)?;
                        }
                    }
                }
                if let ir::CarrierFamily::Parameter(parameter) = &family
                    && let Some((index, _)) =
                        view.parameter_actual(*parameter).map_err(|_| Exhausted)?
                {
                    let offset = match target {
                        ir::Callee::Direct(_) => 0,
                        ir::Callee::TraitItem(call)
                        | ir::Callee::Virtual(ir::VirtualCall { call })
                        | ir::Callee::Untupled(call) => 1 + call.trait_ref.args.len(),
                    };
                    if let Some(source_index) = index.checked_sub(offset) {
                        for row in evidence.trace().type_actual_regions.iter().filter(|row| {
                            row.argument == source_index && row.location == mapping.substituted
                        }) {
                            if let ir::ActualRegion::Named(named) = row.source {
                                // A caller lifetime parameter is never
                                // `'static`, so it cannot instantiate a
                                // `T: 'static` parameter.
                                if self.diagnosing
                                    && function.body_substitution.is_none()
                                    && view.original_env().outlives_static(*parameter)
                                {
                                    self.problem(
                                        block,
                                        None,
                                        Problem::Invalid(
                                            "lifetime may not live long enough: type argument must outlive `'static`".into(),
                                        ),
                                    )?;
                                }
                                let source_env = if let Some(body) =
                                    self.body.as_ref().filter(|body| {
                                        body.source_region_env().declaration == named.declaration
                                    }) {
                                    body.source_region_env().clone()
                                } else {
                                    facts
                                        .source_region_env(named.declaration)
                                        .map_err(|_| Exhausted)?
                                };
                                if named.index >= source_env.region_count {
                                    return Err(Exhausted);
                                }
                                let key = source_env.normalized[usize::from(named.index)]
                                    .map(RegionKey::Fixed)
                                    .unwrap_or(RegionKey::Named(named));
                                let source = relation.node(key, self.budget)?;
                                relation.edge(variable, source, self.budget)?;
                                relation.edge(source, variable, self.budget)?;
                            }
                        }
                    }
                }
            }
        }
        for constraint in view.region_schema().constraints.iter() {
            let longer: Vec<_> = regions
                .iter()
                .filter(|row| row.schema == constraint.longer)
                .cloned()
                .collect();
            let shorter: Vec<_> = regions
                .iter()
                .filter(|row| row.schema == constraint.shorter)
                .cloned()
                .collect();
            for a in &longer {
                for b in &shorter {
                    if a.component.is_some() && b.component.is_some() && a.component != b.component
                    {
                        continue;
                    }
                    relation.edge(a.region, b.region, self.budget)?;
                }
            }
        }
        relation.close(self.budget)?;
        for (local, key) in relation.keys.iter().enumerate() {
            if !matches!(key, RegionKey::Local(_)) {
                continue;
            }
            for (external, key) in relation.keys.iter().enumerate() {
                if !matches!(
                    key,
                    RegionKey::Fixed(_) | RegionKey::Named(_) | RegionKey::Family { .. }
                ) {
                    continue;
                }
                self.budget.spend(1)?;
                if relation.entails(AnalysisRegionId(local), AnalysisRegionId(external)) {
                    self.problem(block, None, Problem::Invalid("frame-local carrier cannot satisfy a rigid external named or family lifetime".into()))?;
                }
            }
        }
        for (a, key_a) in rigid.keys.iter().enumerate() {
            if matches!(key_a, RegionKey::Local(_)) {
                continue;
            }
            for (b, key_b) in rigid.keys.iter().enumerate() {
                if matches!(key_b, RegionKey::Local(_)) {
                    continue;
                }
                let a_new = relation
                    .keys
                    .iter()
                    .position(|key| key == key_a)
                    .ok_or(Exhausted)?;
                let b_new = relation
                    .keys
                    .iter()
                    .position(|key| key == key_b)
                    .ok_or(Exhausted)?;
                if relation.entails(AnalysisRegionId(a_new), AnalysisRegionId(b_new))
                    && !rigid.entails(AnalysisRegionId(a), AnalysisRegionId(b))
                {
                    self.problem(block, None, Problem::Invalid("call carrier occurrence requires an undeclared rigid lifetime or invariant relation".into()))?;
                }
            }
        }
        self.retain(size_of::<CallCarrierBinding>() + 128, 1)?;
        Ok(Some(CallCarrierBinding {
            source: evidence.source().clone(),
            function,
            call,
            block,
            facts,
            contract: view.clone(),
            evidence,
            entry,
            destinations,
            regions,
            relation,
            actual_writes: Vec::new(),
        }))
    }
    fn compose_component(
        &mut self,
        original: &CarrierLocation,
        relative: &CarrierLocation,
        nominals: &[ir::NominalCarrierMapping],
    ) -> Result<CarrierLocation, Exhausted> {
        if original.channel.family().is_none() {
            return Err(Exhausted);
        }
        let prefix = original.channel.prefix();
        self.retain(
            mul(
                original.path.0.len() + relative.path.0.len() + usize::from(prefix),
                size_of::<CarrierStep>(),
            )?,
            original.path.0.len() + relative.path.0.len() + nominals.len() + 1,
        )?;
        let mut path = original.path.0.to_vec();
        for step in &mut path {
            if let CarrierStep::VariantField { variant, .. } = step
                && let Some(mapping) = nominals
                    .iter()
                    .find(|mapping| mapping.original == variant.owner)
            {
                variant.owner = mapping.instantiated;
            }
        }
        let channel = if relative.path.0.is_empty() {
            match relative.channel.clone() {
                CarrierChannel::Fixed(position) => CarrierChannel::Fixed(ReferencePosition {
                    depth: prefix.checked_add(position.depth).ok_or(Exhausted)?,
                }),
                CarrierChannel::ParamTail {
                    parameter,
                    prefix: inner,
                } => CarrierChannel::ParamTail {
                    parameter,
                    prefix: prefix.checked_add(inner).ok_or(Exhausted)?,
                },
                CarrierChannel::ArrayTail {
                    array,
                    prefix: inner,
                } => CarrierChannel::ArrayTail {
                    array,
                    prefix: prefix.checked_add(inner).ok_or(Exhausted)?,
                },
                CarrierChannel::OpaqueTail {
                    opaque,
                    prefix: inner,
                } => CarrierChannel::OpaqueTail {
                    opaque,
                    prefix: prefix.checked_add(inner).ok_or(Exhausted)?,
                },
                CarrierChannel::ProjectionTail {
                    projection,
                    prefix: inner,
                } => CarrierChannel::ProjectionTail {
                    projection,
                    prefix: prefix.checked_add(inner).ok_or(Exhausted)?,
                },
            }
        } else {
            path.extend(std::iter::repeat_n(
                CarrierStep::Referent,
                usize::from(prefix),
            ));
            path.extend_from_slice(&relative.path.0);
            relative.channel.clone()
        };
        Ok(CarrierLocation {
            path: CarrierPath(path.into()),
            channel,
        })
    }
    fn family_is_open(
        &mut self,
        view: &super::traits::CarrierContractView,
        family: &ir::CarrierFamily,
    ) -> Result<bool, Exhausted> {
        self.family_is_open_with_replay(view, family, None)
    }
    fn family_is_open_with_replay(
        &mut self,
        view: &super::traits::CarrierContractView,
        family: &ir::CarrierFamily,
        replay: Option<&super::traits::CarrierViewReplay<'_>>,
    ) -> Result<bool, Exhausted> {
        match family {
            ir::CarrierFamily::Parameter(parameter) => Ok(view
                .parameter_actual(*parameter)
                .map_err(|_| Exhausted)?
                .is_none()),
            ir::CarrierFamily::Projection(_)
            | ir::CarrierFamily::Opaque(_)
            | ir::CarrierFamily::Array(_) => {
                let facts = self.authority.as_ref().ok_or(Exhausted)?;
                let actual = match replay {
                    Some(replay) => view.family_actual_with_replay(
                        family,
                        facts,
                        replay,
                        &mut self.carrier_scratch,
                    ),
                    None => {
                        view.family_actual_with_scratch(family, facts, &mut self.carrier_scratch)
                    }
                }
                .map_err(|_| Exhausted)?;
                match actual {
                    Some(Ty::AssociatedType(_))
                    | Some(Ty::Opaque(_))
                    | Some(Ty::DependentArray { .. }) => Ok(true),
                    Some(_) => Ok(false),
                    None => Err(Exhausted),
                }
            }
        }
    }
    /// Transport carrier coordinates only, without granting mutation authority.
    /// Native lifetime invariants use this same owned-component projection but
    /// are not write envelopes: their only application is a monotone origin union.
    fn transport_input_components_with_replay(
        &mut self,
        input: &InputCarrier,
        view: &super::traits::CarrierContractView,
        owned_destination: bool,
        replay: Option<&super::traits::CarrierViewReplay<'_>>,
    ) -> Result<Vec<(InputCarrier, Option<CarrierLocation>)>, Exhausted> {
        let mut result = Vec::new();
        match &input.location.channel {
            CarrierChannel::ParamTail { .. }
            | CarrierChannel::ProjectionTail { .. }
            | CarrierChannel::ArrayTail { .. }
            | CarrierChannel::OpaqueTail { .. } => {
                let family = input.location.channel.family().ok_or(Exhausted)?;
                for mapping in view
                    .component_mappings()
                    .iter()
                    .filter(|mapping| mapping.original_family == family)
                {
                    // A whole-value replacement transports owned channels only.
                    // Deeper referent channels describe existing pointed-to cells,
                    // not additional values overwritten by this assignment.
                    if owned_destination
                        && value_projection(&mapping.substituted).contains(&Projection::Deref)
                    {
                        continue;
                    }
                    let location = self.compose_component(
                        &input.location,
                        &mapping.substituted,
                        view.nominal_mappings(),
                    )?;
                    self.retain(size_of::<(InputCarrier, Option<CarrierLocation>)>(), 1)?;
                    result.push((
                        InputCarrier {
                            param: input.param,
                            location,
                        },
                        Some(mapping.original.clone()),
                    ));
                }
                if result.is_empty() && self.family_is_open_with_replay(view, &family, replay)? {
                    self.retain(size_of::<(InputCarrier, Option<CarrierLocation>)>(), 1)?;
                    result.push((input.clone(), None));
                }
            }
            CarrierChannel::Fixed(_) => {
                self.retain(
                    size_of::<(InputCarrier, Option<CarrierLocation>)>()
                        + mul(input.location.path.0.len(), size_of::<CarrierStep>())?,
                    input.location.path.0.len() + 1,
                )?;
                let mut path = input.location.path.0.to_vec();
                for step in &mut path {
                    if let CarrierStep::VariantField { variant, .. } = step
                        && let Some(mapping) = view
                            .nominal_mappings()
                            .iter()
                            .find(|mapping| mapping.original == variant.owner)
                    {
                        variant.owner = mapping.instantiated;
                    }
                }
                result.push((
                    InputCarrier {
                        param: input.param,
                        location: CarrierLocation {
                            path: CarrierPath(path.into()),
                            channel: input.location.channel.clone(),
                        },
                    },
                    None,
                ));
            }
        }
        Ok(result)
    }
    fn transport_effect_input(
        &mut self,
        input: &InputCarrier,
        view: &super::traits::CarrierContractView,
        owned_destination: bool,
    ) -> Result<Vec<(InputCarrier, Option<CarrierLocation>)>, Exhausted> {
        self.transport_effect_input_with_replay(input, view, owned_destination, None)
    }
    fn transport_effect_input_with_replay(
        &mut self,
        input: &InputCarrier,
        view: &super::traits::CarrierContractView,
        owned_destination: bool,
        replay: Option<&super::traits::CarrierViewReplay<'_>>,
    ) -> Result<Vec<(InputCarrier, Option<CarrierLocation>)>, Exhausted> {
        let mut result =
            self.transport_input_components_with_replay(input, view, owned_destination, replay)?;
        if owned_destination {
            // A symbolic family may carry a deferred write envelope, but its
            // concrete expansion can include owned values that are not writable
            // referent cells. The declaration's instantiated contract already
            // performed that exact mutability check. Do not resurrect filtered
            // destinations in either admission or effect application.
            let mut retained = 0;
            for index in 0..result.len() {
                let mut authorized = false;
                for envelope in view.contract().writes.iter() {
                    self.budget.spend(
                        envelope
                            .destination
                            .location
                            .path
                            .0
                            .len()
                            .saturating_add(result[index].0.location.path.0.len())
                            .saturating_add(1),
                    )?;
                    if envelope.destination == result[index].0 {
                        authorized = true;
                        break;
                    }
                }
                if authorized {
                    result.swap(retained, index);
                    retained += 1;
                }
            }
            result.truncate(retained);
        }
        Ok(result)
    }
    fn initialize_regions(&mut self) -> Result<(), Exhausted> {
        let Some(body) = self.body.clone() else {
            return Ok(());
        };
        for index in 0..body.ref_contract().region_count {
            self.regions
                .node(RegionKey::Fixed(ir::RegionParamId(index)), self.budget)?;
        }
        for row in body.ref_contract().input_regions.iter() {
            let region = self
                .regions
                .node(RegionKey::Fixed(row.region), self.budget)?;
            self.retain(4 * size_of::<(InputCarrier, AnalysisRegionId)>(), 1)?;
            self.input_regions.push((
                InputCarrier {
                    param: row.input.param,
                    location: fixed_location(row.input.position),
                },
                region,
            ));
        }
        for &(longer, shorter) in body.ref_contract().outlives.iter() {
            let longer = self.regions.node(RegionKey::Fixed(longer), self.budget)?;
            let shorter = self.regions.node(RegionKey::Fixed(shorter), self.budget)?;
            self.regions.edge(longer, shorter, self.budget)?;
        }
        let source = body.source_region_env();
        for index in 0..source.region_count {
            let key = source.normalized[usize::from(index)]
                .map(RegionKey::Fixed)
                .unwrap_or(RegionKey::Named(ir::SourceRegionId {
                    declaration: source.declaration,
                    index,
                }));
            self.regions.node(key, self.budget)?;
        }
        for &(longer, shorter) in source.outlives.iter() {
            let a = source.normalized[usize::from(longer.index)]
                .map(RegionKey::Fixed)
                .unwrap_or(RegionKey::Named(longer));
            let b = source.normalized[usize::from(shorter.index)]
                .map(RegionKey::Fixed)
                .unwrap_or(RegionKey::Named(shorter));
            let a = self.regions.node(a, self.budget)?;
            let b = self.regions.node(b, self.budget)?;
            self.regions.edge(a, b, self.budget)?;
        }
        for input in body.original_contract().inputs.iter() {
            let Some(family) = input.location.channel.family() else {
                continue;
            };
            let mappings: Vec<_> = body
                .component_mappings()
                .iter()
                .filter(|mapping| mapping.original_family == family)
                .collect();
            if mappings.is_empty() && self.family_is_open(&body, &family)? {
                let component = CarrierLocation {
                    path: CarrierPath(Arc::from([])),
                    channel: family_channel(&family, 0),
                };
                let region = self.regions.node(
                    RegionKey::Family {
                        family: family.clone(),
                        component: component.clone(),
                    },
                    self.budget,
                )?;
                self.retain(
                    4 * (size_of::<(InputCarrier, AnalysisRegionId)>()
                        + size_of::<(ir::CarrierFamily, CarrierLocation, AnalysisRegionId)>()),
                    1,
                )?;
                self.input_regions.push((input.clone(), region));
                self.family_regions
                    .push((family.clone(), component, region));
            }
            for mapping in mappings {
                let location = self.compose_component(
                    &input.location,
                    &mapping.substituted,
                    body.nominal_mappings(),
                )?;
                let key = if let Some(caller) = mapping.substituted.channel.family() {
                    RegionKey::Family {
                        family: caller,
                        component: mapping.substituted.clone(),
                    }
                } else {
                    RegionKey::Family {
                        family: family.clone(),
                        component: mapping.original.clone(),
                    }
                };
                let region = self.regions.node(key, self.budget)?;
                self.retain(
                    4 * (size_of::<(InputCarrier, AnalysisRegionId)>()
                        + size_of::<(ir::CarrierFamily, CarrierLocation, AnalysisRegionId)>()),
                    1,
                )?;
                self.input_regions.push((
                    InputCarrier {
                        param: input.param,
                        location,
                    },
                    region,
                ));
                self.family_regions
                    .push((family.clone(), mapping.original.clone(), region));
            }
        }
        // Well-formedness of an input `&'r T`: every lifetime or family
        // reached strictly through that reference's referent outlives 'r.
        let channel_depth = |channel: &CarrierChannel| match channel {
            CarrierChannel::Fixed(position) => usize::from(position.depth),
            CarrierChannel::ParamTail { prefix, .. }
            | CarrierChannel::ProjectionTail { prefix, .. }
            | CarrierChannel::ArrayTail { prefix, .. }
            | CarrierChannel::OpaqueTail { prefix, .. } => usize::from(*prefix),
        };
        let inputs = self.input_regions.clone();
        for (outer, outer_region) in &inputs {
            if !matches!(outer.location.channel, CarrierChannel::Fixed(_)) {
                continue;
            }
            let outer_depth = channel_depth(&outer.location.channel);
            for (inner, inner_region) in &inputs {
                self.budget.spend(outer.location.path.0.len() + 1)?;
                // One input carrier has exactly one lifetime: a fixed row and
                // its substituted family row name the same region.
                if inner == outer && inner_region != outer_region {
                    self.regions
                        .edge(*inner_region, *outer_region, self.budget)?;
                    self.regions
                        .edge(*outer_region, *inner_region, self.budget)?;
                    continue;
                }
                if inner.param == outer.param
                    && inner_region != outer_region
                    && inner.location.path.0.starts_with(&outer.location.path.0)
                    && (inner.location.path.0.len() > outer.location.path.0.len()
                        || channel_depth(&inner.location.channel) > outer_depth)
                {
                    self.regions
                        .edge(*inner_region, *outer_region, self.budget)?;
                }
            }
        }
        let issued_input_regions = self.input_regions.len();
        for row in source.carriers.iter().filter(|row| {
            row.parameter.is_some() && row.location.path.0.contains(&CarrierStep::SliceElement)
        }) {
            let input = InputCarrier {
                param: row.parameter.ok_or(Exhausted)?,
                location: row.location.clone(),
            };
            let regions = self.body_term_regions(&row.region)?;
            if matches!(row.region, ir::CarrierRegionTerm::Static) {
                self.retain(4 * size_of::<InputCarrier>(), 1)?;
                self.static_inputs.push(input);
                continue;
            }
            if matches!(
                row.region,
                ir::CarrierRegionTerm::Family { .. }
                    | ir::CarrierRegionTerm::ProjectionFamily { .. }
                    | ir::CarrierRegionTerm::ArrayFamily { .. }
                    | ir::CarrierRegionTerm::OpaqueFamily { .. }
            ) {
                self.budget
                    .spend(mul(issued_input_regions + 1, regions.len() + 1)?)?;
                if !source_family_region_is_issued(
                    &self.input_regions[..issued_input_regions],
                    &input,
                    &regions,
                ) {
                    self.problem(
                        BlockId(0),
                        None,
                        Problem::Invalid(
                            "source family region has no exact original component mapping".into(),
                        ),
                    )?;
                }
                // Exact family pairings were already emitted above. Repeating
                // all regions here would collapse distinct component lifetimes.
                continue;
            }
            for (_, region) in regions {
                self.retain(4 * size_of::<(InputCarrier, AnalysisRegionId)>(), 1)?;
                self.input_regions.push((input.clone(), region));
            }
        }
        for constraint in body.region_schema().constraints.iter() {
            let longer = self.body_term_regions(&constraint.longer)?;
            let shorter = self.body_term_regions(&constraint.shorter)?;
            for (a_component, a) in &longer {
                for (b_component, b) in &shorter {
                    self.budget.spend(1)?;
                    if a_component.is_some() && b_component.is_some() && a_component != b_component
                    {
                        continue;
                    }
                    self.regions.edge(*a, *b, self.budget)?;
                }
            }
        }
        self.regions.close(self.budget)
    }
    fn body_term_regions(
        &mut self,
        term: &ir::CarrierRegionTerm,
    ) -> Result<Vec<(Option<CarrierLocation>, AnalysisRegionId)>, Exhausted> {
        let Some(body) = self.body.clone() else {
            return Ok(vec![]);
        };
        match term {
            // Static storage has no finite input-region variable. Its absence
            // from a result's input envelope rejects borrowed caller storage.
            ir::CarrierRegionTerm::Static => Ok(vec![]),
            ir::CarrierRegionTerm::Fixed(region) => Ok(vec![(
                None,
                self.regions.node(RegionKey::Fixed(*region), self.budget)?,
            )]),
            ir::CarrierRegionTerm::Named(region) => {
                let source = body.source_region_env();
                if region.declaration != source.declaration || region.index >= source.region_count {
                    return Err(Exhausted);
                }
                let key = source.normalized[usize::from(region.index)]
                    .map(RegionKey::Fixed)
                    .unwrap_or(RegionKey::Named(*region));
                Ok(vec![(None, self.regions.node(key, self.budget)?)])
            }
            ir::CarrierRegionTerm::Family { .. }
            | ir::CarrierRegionTerm::ProjectionFamily { .. }
            | ir::CarrierRegionTerm::ArrayFamily { .. }
            | ir::CarrierRegionTerm::OpaqueFamily { .. } => {
                let family = match term {
                    ir::CarrierRegionTerm::Family { parameter } => {
                        ir::CarrierFamily::Parameter(*parameter)
                    }
                    ir::CarrierRegionTerm::ArrayFamily { array } => {
                        ir::CarrierFamily::Array(array.clone())
                    }
                    ir::CarrierRegionTerm::OpaqueFamily { opaque } => {
                        ir::CarrierFamily::Opaque(opaque.clone())
                    }
                    ir::CarrierRegionTerm::ProjectionFamily { projection } => {
                        ir::CarrierFamily::Projection(projection.clone())
                    }
                    _ => unreachable!(),
                };
                self.budget.spend(self.family_regions.len())?;
                Ok(self
                    .family_regions
                    .iter()
                    .filter(|(candidate, _, _)| candidate == &family)
                    .map(|(_, component, region)| (Some(component.clone()), *region))
                    .collect())
            }
        }
    }
    fn origin_regions(
        &mut self,
        origins: &CarrierOrigins,
    ) -> Result<Vec<AnalysisRegionId>, Exhausted> {
        self.budget
            .spend(self.dependencies.iter().map(|row| row.0.len()).sum())?;
        let edges = self
            .dependencies
            .iter()
            .try_fold(0usize, |total, row| add(total, row.count()))?;
        if edges > MAX_MEMBERSHIPS {
            return Err(Exhausted);
        }
        let pending_bound = add(edges, origins.references.0.count())?;
        let output_bound = add(self.input_regions.len(), self.targets.len())?;
        self.retain(
            add(
                mul(add(pending_bound, output_bound)?, 4 * size_of::<usize>())?,
                mul(self.targets.len().div_ceil(64), size_of::<u64>())?,
            )?,
            pending_bound,
        )?;
        let mut output = Vec::new();
        for family in origins.families.0.members() {
            let input = &self.family_inputs[family].input;
            self.budget.spend(self.input_regions.len())?;
            for (candidate, region) in &self.input_regions {
                if candidate == input && !output.contains(region) {
                    output.push(*region);
                }
            }
        }
        let mut pending: Vec<_> = origins.references.0.members().collect();
        let mut seen = Set::new(self.targets.len());
        while let Some(origin) = pending.pop() {
            // Immutable program data outlives every input and output region.
            if self.static_origin == Some(ReferenceOriginId(origin)) {
                continue;
            }
            if seen.has(origin) {
                continue;
            }
            seen.insert(origin);
            self.budget
                .spend(self.dependencies[origin].0.len() + self.input_regions.len() + 1)?;
            let mut mapped = false;
            let mut static_input = false;
            for target in &self.targets[origin] {
                if let CanonicalRoot::ParameterReferent(local, location) = &target.place.root {
                    self.budget.spend(self.static_inputs.len())?;
                    static_input |= self.static_inputs.iter().any(|input| {
                        self.f.params.get(input.param) == Some(local) && input.location == *location
                    });
                    for (input, region) in &self.input_regions {
                        if self.f.params.get(input.param) == Some(local)
                            && input.location == *location
                        {
                            mapped = true;
                            if !output.contains(region) {
                                output.push(*region);
                            }
                        }
                    }
                }
            }
            // Every target is explicit `'static` caller data.
            if static_input && !mapped && self.targets[origin].len() == 1 {
                pending.extend(self.dependencies[origin].members());
                continue;
            }
            if !mapped {
                // An origin with no storage targets and no dependencies holds
                // no loan (for example a reference carrier inside a value
                // freshly returned by an input-free call such as
                // `Default::default()`). Nothing can invalidate it, so like
                // static data it outlives every input and output region.
                if self.targets[origin].is_empty()
                    && self.dependencies[origin].members().next().is_none()
                {
                    continue;
                }
                let region = self
                    .regions
                    .node(RegionKey::Local(ReferenceOriginId(origin)), self.budget)?;
                if !output.contains(&region) {
                    output.push(region);
                }
            }
            pending.extend(self.dependencies[origin].members());
        }
        Ok(output)
    }
    fn retain(&mut self, bytes: usize, work: usize) -> Result<(), Exhausted> {
        self.bytes = add(self.bytes, bytes)?;
        if self.bytes > self.max_bytes {
            return Err(Exhausted);
        }
        self.budget.reserve_bytes(bytes)?;
        self.budget.spend(work)
    }
    fn membership(&mut self, amount: usize) -> Result<(), Exhausted> {
        self.memberships = add(self.memberships, amount)?;
        if self.memberships > MAX_MEMBERSHIPS {
            return Err(Exhausted);
        }
        Ok(())
    }
    fn cell(&mut self, key: &CellKey) -> Result<Option<usize>, Exhausted> {
        self.budget.spend(key.storage.projection.len() + 1)?;
        Ok(self.cell_index.get(key).copied())
    }
    fn add_cell(&mut self, key: CellKey, ty: &Ty) -> Result<usize, Exhausted> {
        if let Some(index) = self.cell(&key)? {
            return Ok(index);
        }
        self.retain(
            4 * (size_of::<CellSlot>() + size_of::<CarrierOrigins>())
                + 8 * size_of::<(CellKey, usize)>()
                + 2 * key.storage.projection.len() * size_of::<Projection>()
                + carrier_bitmap_bytes(1, self.targets.len(), self.family_inputs.len())?,
            1,
        )?;
        self.cell_index.try_reserve(1).map_err(|_| Exhausted)?;
        let index = self.cells.len();
        self.cell_index.insert(key.clone(), index);
        self.cells.push(CellSlot {
            key,
            ty: ty.clone(),
            input: None,
        });
        self.initial.cells.push(CarrierOrigins::new(
            self.targets.len(),
            self.family_inputs.len(),
        ));
        Ok(index)
    }
    fn problem(
        &mut self,
        block: BlockId,
        statement: Option<usize>,
        problem: Problem,
    ) -> Result<(), Exhausted> {
        self.budget.spend(self.problems.len() + 1)?;
        if self
            .problems
            .iter()
            .any(|row| row.0 == block && row.1 == statement && row.2 == problem)
        {
            return Ok(());
        }
        if self.problems.len() >= 256 {
            return Err(Exhausted);
        }
        self.retain(4 * size_of::<Problem>() + 256, 1)?;
        self.problems.push((block, statement, problem));
        Ok(())
    }
    fn push_target(
        &mut self,
        origin: ReferenceOriginId,
        mut target: Target,
    ) -> Result<bool, Exhausted> {
        self.refine_singleton_target(origin, &mut target)?;
        // Repeated traversal of an owning recursive allocation must converge.
        // Canonical Deref steps here are owning Box boundaries, never reference
        // cells (those have already been resolved to their issued origins).
        // Widen only the same origin and authority chain, retaining the original
        // exact target as well as a conservative subtree covering descendants.
        for previous in &self.targets[origin.0] {
            self.budget
                .spend(previous.place.projection.len() + previous.authority.len() + 1)?;
            if previous.place.root != target.place.root || previous.authority != target.authority {
                continue;
            }
            let Some(suffix) = target
                .place
                .projection
                .strip_prefix(previous.place.projection.as_slice())
            else {
                continue;
            };
            if previous.precision == TargetPrecision::Subtree {
                return Ok(false);
            }
            if target.precision == TargetPrecision::Exact && suffix.contains(&Projection::Deref) {
                target
                    .place
                    .projection
                    .truncate(previous.place.projection.len());
                target.precision = TargetPrecision::Subtree;
                break;
            }
        }
        self.budget.spend(
            self.targets[origin.0].len()
                + target.authority.len()
                + target.place.projection.len()
                + 1,
        )?;
        if self.targets[origin.0].contains(&target) {
            return Ok(false);
        }
        self.retain(
            target_payload_bytes(1, target.place.projection.len(), target.authority.len(), 3)?,
            1,
        )?;
        self.membership(target.authority.len() + 1)?;
        self.targets[origin.0].push(target);
        Ok(true)
    }
    fn lens(&mut self, state: &FlowState, place: &Place) -> Result<CellResolution, Exhausted> {
        resolve_cells(
            &self.original_locals,
            self.family_cover_locals
                .as_deref()
                .unwrap_or(&self.original_locals),
            &self.cells,
            &self.cell_index,
            &self.targets,
            |index| &state.cells[index].references.0,
            place,
            self.budget,
        )
    }
    fn owned_shared_family_result_source(
        &mut self,
        signature: &ir::Signature,
        view: &super::traits::CarrierContractView,
        location: &CarrierLocation,
    ) -> Result<bool, Exhausted> {
        let Some((parameter, declared, source)) = owned_shared_family_result_rows(
            view.original_contract(),
            view.contract(),
            location,
            self.budget,
        )?
        else {
            return Ok(false);
        };
        if !matches!(
            signature.result,
            Ty::Ref {
                kind: BorrowKind::Shared,
                ..
            }
        ) {
            return Ok(false);
        }
        let Some((_, actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)? else {
            return Ok(false);
        };
        if *actual != signature.result {
            return Ok(false);
        }
        let Some(input) = signature.params.get(source.param) else {
            return Ok(false);
        };
        let Some(semantic) =
            self.metadata_value(ir::carrier_at(input, &source.location, &self.ledger))?
        else {
            return Ok(false);
        };
        if semantic != signature.result {
            return Ok(false);
        }
        for mapping in view.component_mappings() {
            self.budget.spend(1)?;
            if mapping.original_family == ir::CarrierFamily::Parameter(parameter)
                && mapping.original
                    == (CarrierLocation {
                        path: CarrierPath(Arc::from([])),
                        channel: CarrierChannel::ParamTail {
                            parameter,
                            prefix: 0,
                        },
                    })
                && mapping.substituted == *location
            {
                return Ok(self.compose_component(
                    &declared.location,
                    &mapping.substituted,
                    view.nominal_mappings(),
                )? == source.location);
            }
        }
        Ok(false)
    }
    fn structural_result_target(
        &mut self,
        mut target: Target,
        result: &Ty,
        location: &CarrierLocation,
    ) -> Result<Target, Exhausted> {
        // A lifetime source proves containment, not identical storage. Preserve
        // an exact referent only when its original typed coordinate agrees
        // with the result. Otherwise keep every issued descendant in the cover.
        if target.precision == TargetPrecision::Exact
            && let Ty::Ref { referent, .. } =
                ir::carrier_at(result, location, &self.ledger).map_err(|_| Exhausted)?
            && let CanonicalRoot::Owned(local) = target.place.root
            && let Some(root) = self.original_locals.get(local.0)
            && probe_projected_shape(root, &target.place.projection, self.budget)?
                .is_none_or(|ty| ty != *referent)
        {
            target.precision = TargetPrecision::Subtree;
        }
        // A parameter lifetime source proves containment in its issued referent,
        // not identity with the returned storage. This includes mutable native
        // sequence elements as well as shared slice elements. Widening keeps
        // the original object, every loan authority and its complete issued
        // cell domain. It cannot mint a cell or a disjoint element target.
        if target.precision == TargetPrecision::Exact
            && let CanonicalRoot::ParameterReferent(local, source) = &target.place.root
            && let Ty::Ref {
                kind: result_kind,
                referent: result_referent,
            } = ir::carrier_at(result, location, &self.ledger).map_err(|_| Exhausted)?
        {
            // This is an applicability probe, not new storage authority. The
            // canonical parameter carrier must already have been issued by
            // this exact validated CFG before an opaque original shape may
            // decline the slice-specific precision adjustment.
            self.budget.spend(self.reference_inputs.len())?;
            if !self.reference_inputs.iter().any(|(input, _)| {
                self.f.params.get(input.param) == Some(local) && input.location == *source
            }) {
                return Err(Exhausted);
            }
            let locals = self
                .family_cover_locals
                .as_deref()
                .unwrap_or(&self.original_locals);
            let root = locals.get(local.0).ok_or(Exhausted)?;
            let CarrierChannel::Fixed(position) = source.channel else {
                return Err(Exhausted);
            };
            let length = add(mul(source.path.0.len(), 2)?, usize::from(position.depth))?;
            self.budget
                .reserve_bytes(mul(length, size_of::<Projection>())?)?;
            let path = value_projection(source);
            if probe_projected_shape(root, &path, self.budget)?
                .is_none_or(|shape| matches!(shape, Ty::Param(_)))
            {
                // A generic original may deliberately retain less cell shape
                // than its canonical closed family. Keep its already-issued
                // target unchanged, with every dependency and authority intact.
                return Ok(target);
            }
            let original = ir::carrier_at(root, source, &self.ledger).map_err(|_| Exhausted)?;
            if let Ty::Ref {
                kind: original_kind,
                referent,
            } = original
                && (original_kind == BorrowKind::Mut || result_kind == BorrowKind::Shared)
                && let Some(projected) =
                    probe_projected_shape(&referent, &target.place.projection, self.budget)?
                && projected != *result_referent
            {
                target.precision = TargetPrecision::Subtree;
            }
        }
        Ok(target)
    }
    fn read_value(
        &mut self,
        state: &FlowState,
        place: &Place,
        ty: &Ty,
    ) -> Result<Vec<(CarrierLocation, CarrierOrigins)>, Exhausted> {
        let locations = ir::carrier_locations(ty, &self.ledger).map_err(|_| Exhausted)?;
        let mut output = Vec::new();
        if ir::raw_deref_index(
            &self.f.locals[place.local.0].ty,
            &place.projection,
            &self.ledger,
        )
        .map_err(|_| Exhausted)?
        .is_some()
        {
            // A raw pointer read has no tracked provenance (an unbounded
            // lifetime in Rust). Reference components gain no origins here.
            return Ok(output);
        }
        for location in locations.iter() {
            // Reading a value transports only its OWNED components. Pointee
            // values remain in their current canonical cells, not a snapshot.
            let path = value_projection(location);
            if path.contains(&Projection::Deref) {
                continue;
            }
            let mut component = place.clone();
            component.projection.extend_from_slice(&path);
            let (targets, _, mut failure) = self.lens(state, &component)?;
            let semantic = ir::carrier_at(ty, location, &self.ledger).map_err(|_| Exhausted)?;
            let Some(channel) = key_channel(&semantic) else {
                continue;
            };
            self.retain(
                4 * size_of::<(CarrierLocation, CarrierOrigins)>()
                    + carrier_bitmap_bytes(1, self.targets.len(), self.family_inputs.len())?,
                1,
            )?;
            let mut origins = CarrierOrigins::new(self.targets.len(), self.family_inputs.len());
            let mut matched_cell = false;
            for target in targets {
                if let Some(cover) = read_cell_cover_with_ledger(
                    self.family_cover_locals
                        .as_deref()
                        .unwrap_or(&self.original_locals),
                    &self.cell_index,
                    &target,
                    &channel,
                    Some(&self.ledger),
                    self.budget,
                )? {
                    for cell in cover {
                        matched_cell = true;
                        origins.union(&state.cells[cell], self.budget)?;
                    }
                } else {
                    failure = Some(ResolutionFailure::IncompleteCover);
                }
            }
            if !matched_cell && failure.is_none() {
                failure = Some(ResolutionFailure::EmptyOrigins);
            }
            if matches!(
                channel,
                CellChannel::Family(ir::CarrierFamily::Parameter(_))
            ) {
                failure = None;
            }
            if let Some(failure) = failure.filter(|_| self.diagnosing) {
                self.problem(
                    self.f.entry,
                    None,
                    failure.problem(
                        "carrier read lacks a complete original typed cell correspondence",
                    ),
                )?;
            }
            output.push((location.clone(), origins));
        }
        Ok(output)
    }
    fn place_ty(&self, f: &cfg::Function, place: &Place) -> Result<Ty, Exhausted> {
        ir::projected_ty(&f.locals[place.local.0].ty, &place.projection, &self.ledger)
            .map_err(|_| Exhausted)
    }
    fn consume(&mut self, state: &mut FlowState, operand: &Operand) -> Result<(), Exhausted> {
        if let Operand::Move(place) = operand {
            let ty = self.place_ty(self.f, place)?.clone();
            let value = self.read_value(state, place, &ty)?;
            for (location, _) in value {
                let mut component = place.clone();
                component.projection.extend(value_projection(&location));
                let (targets, _, failure) = self.lens(state, &component)?;
                if let Some(failure) = failure {
                    if self.diagnosing {
                        self.problem(
                            self.f.entry,
                            None,
                            failure.problem(
                                "affine carrier move lacks a complete original typed cell cover",
                            ),
                        )?;
                    }
                    continue;
                }
                if targets.len() != 1 || targets[0].precision != TargetPrecision::Exact {
                    if self.diagnosing {
                        self.problem(
                            self.f.entry,
                            None,
                            Problem::Unsupported(
                                "affine carrier move requires one exact canonical source".into(),
                            ),
                        )?;
                    }
                    continue;
                }
                for target in targets {
                    let semantic =
                        ir::carrier_at(&ty, &location, &self.ledger).map_err(|_| Exhausted)?;
                    if let Some(channel) = key_channel(&semantic)
                        && let Some(cell) = self.cell(&CellKey {
                            storage: target.place,
                            channel,
                        })?
                    {
                        state.cells[cell].clear();
                    }
                }
            }
        }
        Ok(())
    }
}

impl<'a, 'b> CarrierBuilder<'a, 'b> {
    fn new(
        f: &'a cfg::Function,
        facts: FactsCtx<'a>,
        ledger: ir::PlanBudget,
        budget: &'b mut Budget,
        max_bytes: usize,
        physical: Option<(
            &'a super::opaque_physical::PhysicalValidation<'a>,
            &'a Arc<super::traits::ValidatedFacts>,
        )>,
    ) -> Result<Self, Exhausted> {
        let (physical, physical_facts) = match physical {
            Some((validation, facts)) => (Some(validation), Some(facts)),
            None => (None, None),
        };
        let mut carrier_scratch = super::traits::TraitScratch::default();
        let allocation_start = budget.reserved_bytes();
        // Closed layouts do not grant new read-cover authority. Retain only
        // original types from the exact validated template, without owning a
        // CFG/proof cycle or reconstructing a template from substituted types.
        let original = f
            .body_substitution
            .as_ref()
            .map_or(f, |proof| proof.template().function().as_ref());
        // Canonical lowering reserves source locals, one slot per expression,
        // then the return slot before any type-dependent extra temporaries.
        // BodySubstitutionProof certifies those source/expression ordinals.
        // Do not assume the later CFG-generated temporary ordinals correspond.
        let original_count = if let Some(proof) = &f.body_substitution {
            add(
                add(
                    proof.template().source().locals.len(),
                    proof.template().source().exprs.len(),
                )?,
                1,
            )?
        } else {
            original.locals.len()
        };
        if original_count > original.locals.len() || original_count > f.locals.len() {
            return Err(Exhausted);
        }
        let original_bytes = mul(original_count, size_of::<Ty>())?;
        if original_bytes > max_bytes {
            return Err(Exhausted);
        }
        budget.reserve_bytes(original_bytes)?;
        budget.spend(original_count)?;
        budget.nodes(1)?;
        let mut original_locals = Vec::new();
        original_locals
            .try_reserve_exact(original_count)
            .map_err(|_| Exhausted)?;
        original_locals.extend(
            original
                .locals
                .iter()
                .take(original_count)
                .map(|local| local.ty.clone()),
        );
        // Additional cover structure is issued only by the exact original
        // template substitution. Keep original_locals unchanged for pointer
        // provenance, runtime dynamic indexing and other source-shape checks.
        // Only issued summary projections may use an authenticated cover.
        let family_cover_locals = if let Some(proof) = &f.body_substitution {
            let authority = facts.validated().ok_or(Exhausted)?;
            proof
                .validate_cfg_with_scratch(f, authority, &ledger, &mut carrier_scratch)
                .map_err(|_| Exhausted)?;
            budget.reserve_bytes(original_bytes)?;
            budget.spend(original_count)?;
            let mut covers = Vec::new();
            covers
                .try_reserve_exact(original_count)
                .map_err(|_| Exhausted)?;
            let mut changed = false;
            for (index, original) in original_locals.iter().enumerate() {
                if let Some(cover) = proof
                    .original_family_cover_local(index, authority, &ledger)
                    .map_err(|_| Exhausted)?
                {
                    changed = true;
                    covers.push(cover);
                } else {
                    covers.push(original.clone());
                }
            }
            changed.then_some(covers)
        } else {
            None
        };
        budget.spend(f.locals.len() + f.params.len() + f.blocks.len())?;
        budget.reserve_bytes(
            mul(f.locals.len(), size_of::<Arc<[CarrierLocation]>>())?
                .checked_add(mul(f.loan_sites.len(), size_of::<BorrowKind>())?)
                .ok_or(Exhausted)?,
        )?;
        budget.reserve_bytes(plan_metadata_bytes(f.locals.len(), f.blocks.len())?)?;
        let mut layouts = Vec::new();
        for local in &f.locals {
            layouts.push(ir::carrier_locations(&local.ty, &ledger).map_err(|_| Exhausted)?);
        }
        let mut reference_inputs = Vec::new();
        let mut family_inputs = Vec::new();
        let mut kinds: Vec<_> = f.loan_sites.iter().map(|site| site.kind).collect();
        for (param, local) in f.params.iter().enumerate() {
            for location in layouts[local.0].iter() {
                ledger.charge_work(1).map_err(|_| Exhausted)?;
                let input = InputCarrier {
                    param,
                    location: location.clone(),
                };
                match location.channel {
                    CarrierChannel::Fixed(_) => {
                        let Ty::Ref { kind, .. } =
                            ir::carrier_at(&f.locals[local.0].ty, location, &ledger)
                                .map_err(|_| Exhausted)?
                        else {
                            return Err(Exhausted);
                        };
                        if kinds.len() >= MAX_LOANS {
                            return Err(Exhausted);
                        }
                        ledger
                            .retain_bytes(4 * (size_of::<InputCarrier>() + size_of::<BorrowKind>()))
                            .map_err(|_| Exhausted)?;
                        reference_inputs.push((input, ReferenceOriginId(kinds.len())));
                        kinds.push(kind);
                    }
                    CarrierChannel::ParamTail { .. }
                    | CarrierChannel::ProjectionTail { .. }
                    | CarrierChannel::ArrayTail { .. }
                    | CarrierChannel::OpaqueTail { .. } => {
                        ledger.charge_nodes(1).map_err(|_| Exhausted)?;
                        ledger
                            .retain_bytes(4 * size_of::<FamilyOrigin>())
                            .map_err(|_| Exhausted)?;
                        family_inputs.push(FamilyOrigin { input });
                    }
                }
            }
        }
        let mut raw_slice_rows = Vec::new();
        let mut raw_provenance_validated = false;
        for (block_index, block) in f.blocks.iter().enumerate() {
            ledger
                .charge_work(block.statements.len())
                .map_err(|_| Exhausted)?;
            for (statement, located) in block.statements.iter().enumerate() {
                let cfg::Statement::Assign {
                    value: Rvalue::RawSlice { kind, args },
                    ..
                } = &located.value
                else {
                    continue;
                };
                if f.lowering_proof.is_none()
                    || located.origin.is_none()
                    || args.len() != 2
                    || kinds.len() >= MAX_LOANS
                {
                    return Err(Exhausted);
                }
                if !raw_provenance_validated {
                    let FactsCtx::Validated(authority) = facts else {
                        return Err(Exhausted);
                    };
                    f.validate_provenance_with_scratch(authority, &ledger, &mut carrier_scratch)
                        .map_err(|_| Exhausted)?;
                    raw_provenance_validated = true;
                }
                ledger
                    .retain_bytes(
                        4 * (size_of::<(BlockId, usize, ReferenceOriginId)>()
                            + size_of::<BorrowKind>()),
                    )
                    .map_err(|_| Exhausted)?;
                ledger.charge_work(1).map_err(|_| Exhausted)?;
                raw_slice_rows.push((
                    BlockId(block_index),
                    statement,
                    ReferenceOriginId(kinds.len()),
                ));
                kinds.push(*kind);
            }
        }
        let mut call_rows = Vec::new();
        for block in &f.blocks {
            let mut rows = Vec::new();
            if let Terminator::Call { signature, .. } = &block.terminator.value {
                for location in ir::carrier_locations(&signature.result, &ledger)
                    .map_err(|_| Exhausted)?
                    .iter()
                {
                    ledger
                        .charge_work(location.path.0.len() + 1)
                        .map_err(|_| Exhausted)?;
                    if !matches!(location.channel, CarrierChannel::Fixed(_))
                        || value_projection(location).contains(&Projection::Deref)
                    {
                        continue;
                    }
                    let Ty::Ref { kind, .. } = ir::carrier_at(&signature.result, location, &ledger)
                        .map_err(|_| Exhausted)?
                    else {
                        return Err(Exhausted);
                    };
                    if kinds.len() >= MAX_LOANS {
                        return Err(Exhausted);
                    }
                    ledger
                        .retain_bytes(
                            4 * (size_of::<CarrierLocation>()
                                + size_of::<ReferenceOriginId>()
                                + size_of::<BorrowKind>()),
                        )
                        .map_err(|_| Exhausted)?;
                    rows.push((location.clone(), ReferenceOriginId(kinds.len())));
                    kinds.push(kind);
                }
            }
            call_rows.push(rows);
        }
        let static_origin = if f.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.value,
                    Statement::Assign {
                        value: Rvalue::ByteStr(_)
                            | Rvalue::ByteRepeat { .. }
                            | Rvalue::PromotedArray(_)
                            | Rvalue::PromotedScalar(_),
                        ..
                    }
                )
            })
        }) {
            if kinds.len() >= MAX_LOANS {
                return Err(Exhausted);
            }
            ledger
                .retain_bytes(size_of::<BorrowKind>())
                .map_err(|_| Exhausted)?;
            let origin = ReferenceOriginId(kinds.len());
            kinds.push(BorrowKind::Shared);
            Some(origin)
        } else {
            None
        };
        let n = kinds.len();
        if physical.is_some() {
            budget.reserve_bytes(mul(
                f.blocks.len(),
                size_of::<Option<super::opaque_physical::PhysicalCarrierFlow>>(),
            )?)?;
        }
        budget.reserve_bytes(mul(
            n,
            3 * size_of::<Set>() + 2 * n.div_ceil(64) * 8 + size_of::<BorrowKind>(),
        )?)?;
        let mut builder = Self {
            f,
            facts,
            physical,
            ledger,
            budget,
            max_bytes,
            bytes: plan_metadata_bytes(f.locals.len(), f.blocks.len())?,
            allocation_start,
            memberships: 0,
            diagnosing: false,
            cells: vec![],
            cell_index: HashMap::new(),
            original_locals,
            family_cover_locals,
            initial: FlowState {
                cells: vec![],
                overwritten: Set::default(),
            },
            reference_inputs,
            static_origin,
            family_inputs,
            targets: vec![vec![]; n],
            kinds,
            parents: vec![Set::new(n); n],
            dependencies: vec![Set::new(n); n],
            outlives: vec![Set::new(n); n],
            calls: vec![None; f.blocks.len()],
            physical_calls: if physical.is_some() {
                (0..f.blocks.len()).map(|_| None).collect()
            } else {
                Vec::new()
            },
            call_effects: vec![None; f.blocks.len()],
            body: None,
            body_validated: None,
            physical_body: None,
            body_signature: None,
            call_rows,
            raw_slice_rows,
            function: None,
            authority: None,
            validated_provenance: None,
            copy_scratch: super::traits::TraitScratch::default(),
            carrier_scratch,
            bindings: vec![],
            regions: RegionRelations::default(),
            effect_table: None,
            input_regions: vec![],
            static_inputs: vec![],
            singleton_heap_slot: None,
            native_storage: Vec::new(),
            native_exchanges: Vec::new(),
            family_regions: vec![],
            problems: vec![],
        };
        builder.budget.spend(n)?;
        for (local, locations) in layouts.iter().enumerate() {
            for location in locations.iter() {
                let path = value_projection(location);
                if path.contains(&Projection::Deref) {
                    continue;
                }
                let ty = ir::carrier_at(&f.locals[local].ty, location, &builder.ledger)
                    .map_err(|_| Exhausted)?;
                let channel = key_channel(&ty).ok_or(Exhausted)?;
                builder.add_cell(
                    CellKey {
                        storage: CanonicalPlace {
                            root: CanonicalRoot::Owned(LocalId(local)),
                            projection: path,
                        },
                        channel,
                    },
                    &ty,
                )?;
            }
        }
        let mut next_reference = 0usize;
        let mut next_family = 0usize;
        for (param, local) in f.params.iter().enumerate() {
            for location in layouts[local.0].iter() {
                let component = Place {
                    local: *local,
                    projection: value_projection(location),
                };
                let semantic = ir::carrier_at(&f.locals[local.0].ty, location, &builder.ledger)
                    .map_err(|_| Exhausted)?;
                let state = std::mem::replace(
                    &mut builder.initial,
                    FlowState {
                        cells: vec![],
                        overwritten: Set::default(),
                    },
                );
                let resolved = builder.lens(&state, &component);
                builder.initial = state;
                let (targets, _, _) = resolved?;
                if targets.len() != 1 {
                    return Err(Exhausted);
                }
                let cell = builder.add_cell(
                    CellKey {
                        storage: targets[0].place.clone(),
                        channel: key_channel(&semantic).ok_or(Exhausted)?,
                    },
                    &semantic,
                )?;
                let input = InputCarrier {
                    param,
                    location: location.clone(),
                };
                builder.cells[cell].input = Some(input.clone());
                match location.channel {
                    CarrierChannel::Fixed(_) => {
                        builder.budget.spend(1)?;
                        let (candidate, origin) = builder
                            .reference_inputs
                            .get(next_reference)
                            .ok_or(Exhausted)?;
                        if *candidate != input {
                            return Err(Exhausted);
                        }
                        let origin = *origin;
                        next_reference += 1;
                        builder.initial.cells[cell].references.0.insert(origin.0);
                        builder.push_target(
                            origin,
                            Target {
                                place: CanonicalPlace {
                                    root: CanonicalRoot::ParameterReferent(
                                        *local,
                                        location.clone(),
                                    ),
                                    projection: vec![],
                                },
                                precision: TargetPrecision::Exact,
                                authority: vec![],
                            },
                        )?;
                    }
                    CarrierChannel::ParamTail { .. }
                    | CarrierChannel::ProjectionTail { .. }
                    | CarrierChannel::ArrayTail { .. }
                    | CarrierChannel::OpaqueTail { .. } => {
                        builder.budget.spend(1)?;
                        if builder
                            .family_inputs
                            .get(next_family)
                            .ok_or(Exhausted)?
                            .input
                            != input
                        {
                            return Err(Exhausted);
                        }
                        let index = FamilyOriginId(next_family);
                        next_family += 1;
                        builder.initial.cells[cell].families.0.insert(index.0);
                    }
                }
            }
        }
        builder
            .budget
            .reserve_bytes(mul(builder.cells.len().div_ceil(64), size_of::<u64>())?)?;
        builder.initial.overwritten = Set::new(builder.cells.len());
        if let FactsCtx::Validated(authority) = facts {
            let mut nominal_calls = super::traits::NominalCarrierSource::with_scratch(
                f,
                authority,
                std::mem::take(&mut builder.carrier_scratch),
            );
            let signature = body_signature(f, builder.budget)?;
            let body_view = if let Some(physical) = physical {
                physical
                    .body_carriers(f, authority)
                    .and_then(|boundary| {
                        boundary.realize_flow(physical_facts.ok_or("physical facts missing")?)
                    })
                    .map(|flow| {
                        let view = flow.view().clone();
                        builder.physical_body = Some(flow);
                        view
                    })
                    .map_err(|message| {
                        super::traits::FactsError::from_message(authority.budget(), &message)
                    })
            } else {
                nominal_calls.body()
            };
            match body_view {
                Ok(view) => {
                    let mut valid = true;
                    let mut view_validated = false;
                    for result in view.contract().results.iter() {
                        if let Some(problem) = physical_result_flow_problem(
                            builder.physical_body.as_ref(),
                            Some(&view),
                            Some(authority),
                            ir::SignatureSubject::Body,
                            &signature,
                            &result.location,
                            &mut view_validated,
                        )? {
                            builder.problem(f.entry, None, problem)?;
                            valid = false;
                        }
                    }
                    if valid {
                        if view_validated && physical.is_none() && builder.physical_body.is_none() {
                            builder
                                .budget
                                .reserve_bytes(size_of::<Option<BodyViewValidated<'_>>>())?;
                            builder.body_validated = Some(BodyViewValidated {
                                function: f,
                                facts: authority,
                                view: view.clone(),
                            });
                        }
                        builder.body = Some(view);
                    }
                }
                Err(error) if error.is_resource() => return Err(Exhausted),
                Err(error) => builder.problem(
                    f.entry,
                    None,
                    Problem::Invalid(
                        error
                            .into_diagnostic(authority.budget())
                            .map_err(|_| Exhausted)?,
                    ),
                )?,
            }
            for (index, block) in f.blocks.iter().enumerate() {
                if let Terminator::Call {
                    target, signature, ..
                } = &block.terminator.value
                {
                    let call_view = if let Some(physical) = physical {
                        physical
                            .call_carriers(f, authority, BlockId(index))
                            .and_then(|boundary| {
                                boundary
                                    .realize_flow(physical_facts.ok_or("physical facts missing")?)
                            })
                            .map(|flow| {
                                let view = flow.view().clone();
                                builder.physical_calls[index] = Some(flow);
                                view
                            })
                            .map_err(|message| {
                                super::traits::FactsError::from_message(
                                    authority.budget(),
                                    &message,
                                )
                            })
                    } else {
                        nominal_calls.call(BlockId(index))
                    };
                    match call_view {
                        Ok(view) => {
                            let mut valid = true;
                            let mut view_validated = false;
                            for result in view.contract().results.iter() {
                                if let Some(problem) = physical_result_flow_problem(
                                    builder.physical_calls.get(index).and_then(Option::as_ref),
                                    Some(&view),
                                    Some(authority),
                                    ir::SignatureSubject::Call(target),
                                    signature,
                                    &result.location,
                                    &mut view_validated,
                                )? {
                                    builder.problem(BlockId(index), None, problem)?;
                                    valid = false;
                                }
                            }
                            if valid {
                                builder.calls[index] = Some(view);
                            }
                        }
                        Err(error) if error.is_resource() => return Err(Exhausted),
                        Err(error) => builder.problem(
                            BlockId(index),
                            None,
                            Problem::Invalid(
                                error
                                    .into_diagnostic(authority.budget())
                                    .map_err(|_| Exhausted)?,
                            ),
                        )?,
                    }
                }
            }
            builder.carrier_scratch = nominal_calls.into_scratch();
            builder.body_signature = Some(signature);
        }
        builder.prepare_native_storage()?;
        builder.prepare_native_exchange()?;
        Ok(builder)
    }
}

impl CarrierBuilder<'_, '_> {
    fn empty_origins(&mut self) -> Result<CarrierOrigins, Exhausted> {
        self.retain(
            carrier_bitmap_bytes(1, self.targets.len(), self.family_inputs.len())?,
            1,
        )?;
        Ok(CarrierOrigins::new(
            self.targets.len(),
            self.family_inputs.len(),
        ))
    }
    fn operand_value(
        &mut self,
        state: &FlowState,
        operand: &Operand,
    ) -> Result<Vec<(CarrierLocation, CarrierOrigins)>, Exhausted> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let ty = self.place_ty(self.f, place)?.clone();
                self.read_value(state, place, &ty)
            }
            Operand::Constant(_) => Ok(vec![]),
        }
    }
    fn input_value(
        &mut self,
        state: &FlowState,
        args: &[Operand],
        input: &InputCarrier,
    ) -> Result<CarrierOrigins, Exhausted> {
        let Some(Operand::Copy(place) | Operand::Move(place)) = args.get(input.param) else {
            return self.empty_origins();
        };
        let mut component = place.clone();
        component
            .projection
            .extend(value_projection(&input.location));
        let semantic = self.place_ty(self.f, &component)?.clone();
        let Some(channel) = key_channel(&semantic) else {
            return self.empty_origins();
        };
        let (targets, _, mut failure) = self.lens(state, &component)?;
        let mut value = self.empty_origins()?;
        let mut matched_cell = false;
        for target in targets {
            if let Some(cover) = read_cell_cover_with_ledger(
                self.family_cover_locals
                    .as_deref()
                    .unwrap_or(&self.original_locals),
                &self.cell_index,
                &target,
                &channel,
                Some(&self.ledger),
                self.budget,
            )? {
                for cell in cover {
                    matched_cell = true;
                    value.union(&state.cells[cell], self.budget)?;
                }
            } else {
                failure = Some(ResolutionFailure::IncompleteCover);
            }
        }
        if !matched_cell && failure.is_none() {
            failure = Some(ResolutionFailure::EmptyOrigins);
        }
        // Generic actuals are never references (ir validate_argument), so a
        // type-parameter family carries no loan origins; a missing family
        // cell behind a borrowed or returned reference is simply empty.
        if matches!(
            channel,
            CellChannel::Family(ir::CarrierFamily::Parameter(_))
        ) {
            failure = None;
        }
        if let Some(failure) = failure.filter(|_| self.diagnosing) {
            self.problem(
                self.f.entry,
                None,
                failure.problem("carrier input requires a complete original typed cell cover"),
            )?;
        }
        Ok(value)
    }
    fn origin_allowed(
        &mut self,
        value: &CarrierOrigins,
        allowed: &[InputCarrier],
    ) -> Result<bool, Exhausted> {
        self.budget
            .spend(allowed.len() + self.family_inputs.len() + self.reference_inputs.len())?;
        for family in value.families.0.members() {
            if !allowed.contains(&self.family_inputs[family].input) {
                return Ok(false);
            }
        }
        for origin in value.references.0.members() {
            if self.static_origin == Some(ReferenceOriginId(origin)) {
                continue;
            }
            if self.targets[origin].is_empty() {
                return Ok(false);
            }
            for target in &self.targets[origin] {
                self.budget
                    .spend(allowed.len() + target.place.projection.len() + 1)?;
                let CanonicalRoot::ParameterReferent(local, location) = &target.place.root else {
                    return Ok(false);
                };
                if !allowed.iter().any(|input| {
                    self.f.params.get(input.param) == Some(local) && input.location == *location
                }) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    fn check_result_slot_write(
        &mut self,
        targets: &[Target],
        origins: &CarrierOrigins,
        block: BlockId,
        statement: Option<usize>,
    ) -> Result<(), Exhausted> {
        // Authority follows the reference origin through moves, reborrows and
        // forwarding. Do not infer authored lifetimes from the erased value Ty.
        let mut obligations = Vec::new();
        for (call, rows) in self.call_rows.iter().enumerate() {
            self.budget.spend(rows.len())?;
            for (location, origin) in rows {
                self.budget.spend(
                    targets
                        .iter()
                        .map(|target| target.authority.len() + 1)
                        .sum(),
                )?;
                if targets
                    .iter()
                    .any(|target| target.authority.contains(&origin.0))
                {
                    checked_vec_growth(&mut obligations, self.budget)?;
                    self.budget
                        .reserve_bytes(location.path.0.len() * size_of::<CarrierStep>())?;
                    obligations.push((BlockId(call), location.clone()));
                }
            }
        }
        if obligations.is_empty() {
            return Ok(());
        }
        let sources = self.origin_regions(origins)?;
        let own_static = self
            .body
            .as_ref()
            .map_or(0, |body| body.ref_contract().static_regions);
        for (call, location) in obligations {
            let Some(index) = call_binding_index(&self.bindings, call, self.budget)? else {
                self.problem(
                    block,
                    statement,
                    Problem::Invalid(
                        "carrier write lacks its exact sealed call-result region binding".into(),
                    ),
                )?;
                continue;
            };
            let term = match call_result_slot_region(
                &self.bindings[index],
                self.f,
                self.function.as_ref(),
                self.authority.as_ref(),
                &self.calls,
                &location,
                self.budget,
            )? {
                Ok(Some(term)) => term,
                Ok(None) => continue,
                Err(problem) => {
                    self.problem(block, statement, problem)?;
                    continue;
                }
            };
            let binding = &self.bindings[index];
            self.budget.spend(add(
                binding.regions.len(),
                mul(sources.len(), add(binding.relation.keys.len(), 1)?)?,
            )?)?;
            let invalid = if term == ir::CarrierRegionTerm::Static {
                sources.iter().any(|region| {
                    !matches!(self.regions.keys[region.0], RegionKey::Fixed(id)
                        if own_static & (1u64 << id.0) != 0)
                })
            } else {
                let expected = binding
                    .regions
                    .iter()
                    .find(|row| row.schema == term && row.component.is_none());
                sources.iter().any(|source| {
                    let key = &self.regions.keys[source.0];
                    let actual = binding
                        .relation
                        .keys
                        .iter()
                        .position(|candidate| candidate == key);
                    match (actual, expected) {
                        (Some(actual), Some(expected)) => !binding
                            .relation
                            .entails(AnalysisRegionId(actual), expected.region),
                        _ => true,
                    }
                })
            };
            if invalid {
                self.problem(
                    block,
                    statement,
                    Problem::Invalid(
                        "carrier store does not outlive the authored call-result referent lifetime"
                            .into(),
                    ),
                )?;
            }
        }
        Ok(())
    }
    fn write_components(
        &mut self,
        state: &mut FlowState,
        destination: &Place,
        value: &[(CarrierLocation, CarrierOrigins)],
        definite: bool,
        context: (BlockId, Option<usize>, bool),
    ) -> Result<(), Exhausted> {
        let (block, statement, diagnose) = context;
        // Storage behind a raw pointer has no canonical cells; the unsafe
        // contract delegates its validity to the programmer.
        if ir::raw_deref_index(
            &self.f.locals[destination.local.0].ty,
            &destination.projection,
            &self.ledger,
        )
        .map_err(|_| Exhausted)?
        .is_some()
        {
            self.write_native_storage(state, value, block, statement, diagnose)?;
            self.write_native_exchange(state, value, block, statement, diagnose)?;
            return Ok(());
        }
        let ty = self.place_ty(self.f, destination)?.clone();
        for (location, origins) in value {
            let semantic = ir::carrier_at(&ty, location, &self.ledger)
                .map_err(|_| Exhausted)?
                .clone();
            let Some(channel) = key_channel(&semantic) else {
                continue;
            };
            let mut component = destination.clone();
            component.projection.extend(value_projection(location));
            let (targets, _, failure) = self.lens(state, &component)?;
            if let Some(failure) = failure {
                if diagnose {
                    self.problem(
                        block,
                        statement,
                        failure.problem("carrier write lacks a complete original typed cell cover"),
                    )?;
                }
                continue;
            }
            let mut indices = Vec::new();
            for target in &targets {
                if target.precision != TargetPrecision::Exact {
                    continue;
                }
                if let Some(index) = self.cell(&CellKey {
                    storage: target.place.clone(),
                    channel: channel.clone(),
                })? && !indices.contains(&index)
                {
                    indices.push(index);
                }
            }
            let copy = self.copy_permission(&semantic)?;
            let mut weak_cover = false;
            if copy && indices.len() != targets.len() {
                // Even a definite store through a summarized result selects
                // only one element of its issued cover. Join every covered
                // cell without claiming that any one was overwritten.
                // Same weak subtree join as call effects: a store through a
                // summarized element family updates every covered cell.
                let mut covered = Vec::new();
                let mut complete = true;
                for target in &targets {
                    match read_cell_cover_with_ledger(
                        self.family_cover_locals
                            .as_deref()
                            .unwrap_or(&self.original_locals),
                        &self.cell_index,
                        target,
                        &channel,
                        Some(&self.ledger),
                        self.budget,
                    )? {
                        Some(cover) if !cover.is_empty() => {
                            for cell in cover {
                                if !covered.contains(&cell) {
                                    checked_vec_growth(&mut covered, self.budget)?;
                                    covered.push(cell);
                                }
                            }
                        }
                        _ => complete = false,
                    }
                }
                if complete {
                    indices = covered;
                    weak_cover = true;
                }
            }
            // Copy elements whose weak cover failed take the same summary
            // route; the join below never overwrites the summary.
            if !weak_cover
                && indices.len() != targets.len()
                && let Some(summary) =
                    self.indexed_summary_write(state, destination, location, &channel, &targets)?
            {
                // A source-issued element summary is one abstract cell, not
                // one selected physical element. Preserve OLD, never record
                // this write as overwriting the entire owner or as disjoint.
                indices.clear();
                checked_vec_growth(&mut indices, self.budget)?;
                indices.push(summary.cell());
                weak_cover = true;
            }
            if diagnose && !weak_cover && indices.len() != targets.len() {
                self.problem(
                    block,
                    statement,
                    Problem::Unsupported(
                        "carrier write has no complete original typed cell cover".into(),
                    ),
                )?;
            }
            if diagnose && !copy && indices.len() != 1 {
                self.problem(
                    block,
                    statement,
                    Problem::Unsupported(
                        "affine carrier write requires one exact canonical destination".into(),
                    ),
                )?;
            }
            if diagnose && !indices.is_empty() {
                self.check_result_slot_write(&targets, origins, block, statement)?;
            }
            for index in indices {
                // A by-value parameter's own reference slot keeps its declared
                // lifetime: a store must outlive it (`c = d` needs 'd: 'c).
                if diagnose
                    && let CanonicalRoot::Owned(local) = self.cells[index].key.storage.root
                    && self.f.params.contains(&local)
                    && let Some(input) = self.cells[index].input.clone()
                    && matches!(input.location.channel, CarrierChannel::Fixed(_))
                    && let Some(body) = self.body.clone()
                {
                    self.budget.spend(self.input_regions.len() + 1)?;
                    let mut slots: Vec<AnalysisRegionId> = self
                        .input_regions
                        .iter()
                        .filter(|(row, _)| *row == input)
                        .map(|(_, region)| *region)
                        .collect();
                    if slots.is_empty() {
                        self.budget
                            .spend(body.source_region_env().carriers.len() + 1)?;
                        if let Some(row) = body.source_region_env().carriers.iter().find(|row| {
                            row.parameter == Some(input.param) && row.location == input.location
                        }) {
                            let term = row.region.clone();
                            for (_, region) in self.body_term_regions(&term)? {
                                slots.push(region);
                            }
                        }
                    }
                    let sources = self.origin_regions(origins)?;
                    if slots.is_empty() {
                        // An elided slot is an anonymous lifetime that only
                        // `'static` (or loan-free) data is known to outlive.
                        if !sources.is_empty() {
                            self.problem(block, statement, Problem::Invalid("assignment to a reference parameter requires its source to outlive the parameter's declared lifetime".into()))?;
                        }
                    } else {
                        if sources.iter().any(|region| {
                            slots
                                .iter()
                                .any(|slot| !self.regions.entails(*region, *slot))
                        }) {
                            self.problem(block, statement, Problem::Invalid("assignment to a reference parameter requires its source to outlive the parameter's declared lifetime".into()))?;
                        }
                    }
                }
                if diagnose
                    && matches!(
                        self.cells[index].key.storage.root,
                        CanonicalRoot::ParameterReferent(..)
                    )
                    && let (Some(body), Some(input)) = (&self.body, &self.cells[index].input)
                {
                    let allowed = body
                        .contract()
                        .writes
                        .iter()
                        .find(|row| row.destination == *input)
                        .map(|row| row.sources.clone());
                    if let Some(allowed) = allowed {
                        if !self.origin_allowed(origins, &allowed)? {
                            self.problem(block, statement, Problem::Invalid("carrier store would escape its declared input lifetime/family envelope".into()))?;
                        }
                    } else {
                        self.problem(
                            block,
                            statement,
                            Problem::Invalid(
                                "carrier store is absent from the canonical writable envelope"
                                    .into(),
                            ),
                        )?;
                    }
                }
                if definite && !weak_cover && targets.len() == 1 {
                    state.cells[index] = origins.clone();
                    state.overwritten.insert(index);
                } else {
                    state.cells[index].union(origins, self.budget)?;
                }
                self.join_singleton_summary(state, index, origins)?;
                self.membership(origins.count())?;
            }
        }
        Ok(())
    }
    fn assignment_value(
        &mut self,
        state: &FlowState,
        value: &Rvalue,
        block: BlockId,
        statement: usize,
    ) -> Result<Vec<(CarrierLocation, CarrierOrigins)>, Exhausted> {
        match value {
            Rvalue::ByteStr(_)
            | Rvalue::ByteRepeat { .. }
            | Rvalue::PromotedArray(_)
            | Rvalue::PromotedScalar(_) => {
                let origin = self.static_origin.ok_or(Exhausted)?;
                let mut origins = self.empty_origins()?;
                origins.references.0.insert(origin.0);
                Ok(vec![(
                    fixed_location(ReferencePosition { depth: 0 }),
                    origins,
                )])
            }
            Rvalue::RawSlice { .. } => {
                self.budget.spend(self.raw_slice_rows.len() + 1)?;
                let row = self
                    .raw_slice_rows
                    .iter()
                    .find(|(owner, index, _)| *owner == block && *index == statement)
                    .map(|(_, _, origin)| *origin)
                    .ok_or(Exhausted)?;
                let mut origins = self.empty_origins()?;
                origins.references.0.insert(row.0);
                Ok(vec![(
                    fixed_location(ReferencePosition { depth: 0 }),
                    origins,
                )])
            }
            Rvalue::OpaquePack { value, proof } => {
                // The CFG gate replays the exact publication and operand type.
                // A normal value read copies owned cells only. Packing also
                // retains lifetimes reached through a captured reference, so
                // validate the complete typed inventory before reading each
                // exact source through the existing provenance lens.
                let sources = proof.captured_sources();
                let locations =
                    ir::carrier_locations(proof.hidden(), &self.ledger).map_err(|_| Exhausted)?;
                self.budget.spend(locations.len() + sources.len() + 1)?;
                if locations.as_ref() != sources {
                    self.problem(
                        block,
                        Some(statement),
                        Problem::Invalid(
                            "opaque pack changed its complete ordered capture rows".into(),
                        ),
                    )?;
                    return Ok(Vec::new());
                }
                if sources.is_empty() {
                    return Ok(Vec::new());
                }
                let mut merged = self.empty_origins()?;
                for source in sources {
                    let input = InputCarrier {
                        param: 0,
                        location: source.clone(),
                    };
                    let origins = self.input_value(state, std::slice::from_ref(value), &input)?;
                    merged.union(&origins, self.budget)?;
                }
                Ok(vec![(proof.captured_destination(), merged)])
            }
            Rvalue::CellNew { .. } | Rvalue::CellGet { .. } | Rvalue::CellSet { .. } => {
                Ok(Vec::new())
            }
            Rvalue::ManuallyDropIntoInner { value, .. } => {
                let mut carriers = self.operand_value(state, value)?;
                for (location, _) in &mut carriers {
                    if location.path.0.first() != Some(&CarrierStep::ManuallyDropPayload) {
                        self.problem(
                            block,
                            Some(statement),
                            Problem::Invalid(
                                "ManuallyDrop into_inner carrier lacks exact stored payload prefix"
                                    .into(),
                            ),
                        )?;
                        return Ok(Vec::new());
                    }
                    let tail = &location.path.0[1..];
                    let bytes = mul(tail.len(), size_of::<CarrierStep>())?;
                    self.retain(add(mul(bytes, 2)?, 2 * size_of::<usize>())?, tail.len())?;
                    let mut path = Vec::new();
                    path.try_reserve_exact(tail.len()).map_err(|_| Exhausted)?;
                    if path.capacity() > tail.len() {
                        self.retain(
                            mul(path.capacity() - tail.len(), size_of::<CarrierStep>())?,
                            0,
                        )?;
                    }
                    path.extend_from_slice(tail);
                    location.path = CarrierPath(path.into());
                }
                Ok(carriers)
            }
            Rvalue::ManuallyDropTake { payload, .. } => {
                let ty = self.place_ty(self.f, payload)?.clone();
                self.read_value(state, payload, &ty)
            }
            Rvalue::Use(operand) => {
                if let Some(value) = self.read_native_exchange(state, block, statement)? {
                    Ok(value)
                } else {
                    self.operand_value(state, operand)
                }
            }
            // The slice handle carries exactly the input reference's origins
            // and loan set at the same fixed carrier position. Length metadata
            // contributes no provenance.
            Rvalue::ArrayToSlice { reference } => self.operand_value(state, reference),
            // A subslice carries exactly the source handle's loan and origins.
            Rvalue::Subslice { slice, .. } => self.operand_value(state, slice),
            // A trait object carrier holds exactly the input reference's loan
            // and origins; the vtable word is static metadata.
            Rvalue::RefToDyn { reference, .. } => self.operand_value(state, reference),
            Rvalue::Borrow(id) => {
                let site = &self.f.loan_sites[id.0];
                let (targets, trace, failure) = self.lens(state, &site.place)?;
                if let Some(failure) = failure.filter(|_| self.diagnosing) {
                    self.problem(
                        block,
                        Some(statement),
                        failure.problem("borrow target lacks a complete original typed cell cover"),
                    )?;
                }
                for target in targets {
                    // The typed lens retains unique-reference intermediates,
                    // but a copied shared reference starts a fresh authority
                    // chain. Its referent does not borrow the containing handle.
                    for parent in &target.authority {
                        self.dependencies[id.0].insert(*parent);
                    }
                    // `&'r T` implies `T: 'r`: every loan carried by a
                    // reference stored inside the borrowed storage outlives
                    // this loan, so it stays live while this loan does.
                    for (index, slot) in self.cells.iter().enumerate() {
                        self.budget.spend(slot.key.storage.projection.len() + 1)?;
                        if slot.key.storage.root == target.place.root
                            && slot
                                .key
                                .storage
                                .projection
                                .starts_with(&target.place.projection)
                        {
                            let held = &state.cells[index].references.0;
                            self.budget.spend(held.0.len() + 1)?;
                            self.outlives[id.0].union(held, self.budget)?;
                        }
                    }
                    self.push_target(ReferenceOriginId(id.0), target)?;
                }
                for parent in trace {
                    self.parents[id.0].insert(parent.0);
                }
                let mut origins = self.empty_origins()?;
                origins.references.0.insert(id.0);
                Ok(vec![(
                    fixed_location(ReferencePosition { depth: 0 }),
                    origins,
                )])
            }
            Rvalue::Aggregate { fields, .. } | Rvalue::EnumVariant { fields, .. } => {
                let mut output = Vec::new();
                // Only non-generic nominals: generic carriers transport their
                // actual reference arguments through certified substitution.
                let nominal = match value {
                    Rvalue::EnumVariant { ty, .. } => ty.def().type_args.is_empty(),
                    Rvalue::Aggregate {
                        ty: Ty::Struct(ty), ..
                    } => ty.def().type_args.is_empty(),
                    _ => false,
                };
                for (index, operand) in fields.iter().enumerate() {
                    // Only a `&str` field without a declared lifetime is
                    // `&'static str`. Other reference fields belong to a
                    // lifetime-parameterized nominal and carry their loans.
                    let static_field = match value {
                        Rvalue::EnumVariant { ty, variant, .. } => {
                            match ty.def().variants.get(*variant) {
                                Some(variant) => variant
                                    .fields
                                    .read(&self.ledger)
                                    .map_err(|_| Exhausted)?
                                    .get(index)
                                    .is_some_and(|f| f.ty.is_shared_str() && !f.lifetime_bound),
                                None => false,
                            }
                        }
                        Rvalue::Aggregate {
                            ty: Ty::Struct(ty), ..
                        } => ty
                            .def()
                            .fields
                            .read(&self.ledger)
                            .map_err(|_| Exhausted)?
                            .get(index)
                            .is_some_and(|f| f.ty.is_shared_str() && !f.lifetime_bound),
                        _ => false,
                    };
                    for (mut location, origins) in self.operand_value(state, operand)? {
                        // A `&'static str` field admits no borrowed storage.
                        if nominal
                            && static_field
                            && self.diagnosing
                            && origins
                                .references
                                .0
                                .members()
                                .any(|origin| !self.targets[origin].is_empty())
                        {
                            self.problem(
                                block,
                                Some(statement),
                                Problem::Invalid(
                                    "borrowed data does not live long enough for a `&'static str` field".into(),
                                ),
                            )?;
                        }
                        let step = match value {
                            Rvalue::Aggregate {
                                ty: Ty::Array { .. },
                                ..
                            } => CarrierStep::ConstantIndex(index),
                            Rvalue::EnumVariant { ty, variant, .. } => CarrierStep::VariantField {
                                variant: ir::VariantId {
                                    owner: ty.def().id,
                                    index: *variant,
                                },
                                field: index,
                            },
                            Rvalue::Aggregate {
                                ty: Ty::ManuallyDrop(_),
                                ..
                            } => CarrierStep::ManuallyDropPayload,
                            _ => CarrierStep::Field(index),
                        };
                        self.retain(
                            (location.path.0.len() + 1) * 4 * size_of::<CarrierStep>(),
                            1,
                        )?;
                        let mut path = vec![step];
                        path.extend_from_slice(&location.path.0);
                        location.path = CarrierPath(path.into());
                        output.push((location, origins));
                    }
                }
                Ok(output)
            }
            Rvalue::Repeat {
                ty: Ty::Array { len, .. },
                value,
            } => {
                let original = self.operand_value(state, value)?;
                let mut output = Vec::new();
                for index in 0..*len {
                    for (location, origins) in &original {
                        self.retain(
                            4 * (size_of::<CarrierLocation>() + size_of::<CarrierOrigins>())
                                + (location.path.0.len() + 1) * size_of::<CarrierStep>(),
                            1,
                        )?;
                        let mut location = location.clone();
                        let mut path = vec![CarrierStep::ConstantIndex(index)];
                        path.extend_from_slice(&location.path.0);
                        location.path = CarrierPath(path.into());
                        output.push((location, origins.clone()));
                    }
                }
                Ok(output)
            }
            Rvalue::SliceIndex { slice, index: _ } => {
                let (Operand::Copy(place) | Operand::Move(place)) = slice else {
                    return Err(Exhausted);
                };
                let ty = self.place_ty(self.f, place)?.clone();
                let Some(element) = ty.shared_slice_element() else {
                    return Err(Exhausted);
                };
                self.retain(
                    place
                        .projection
                        .len()
                        .checked_add(2)
                        .and_then(|len| len.checked_mul(3 * size_of::<Projection>()))
                        .ok_or(Exhausted)?,
                    1,
                )?;
                let mut selected = place.clone();
                selected.projection.push(Projection::Deref);
                selected
                    .projection
                    .push(Projection::DynamicIndex(LocalId(usize::MAX)));
                self.read_value(state, &selected, element)
            }
            Rvalue::IndexCopy { array, index } => {
                let Ty::Array { len, .. } = self.place_ty(self.f, array)? else {
                    return Err(Exhausted);
                };
                let mut output: Vec<(CarrierLocation, CarrierOrigins)> = Vec::new();
                let known = match index {
                    Operand::Constant(cfg::Constant::Int(value)) => {
                        usize::try_from(value.bits).ok()
                    }
                    _ => None,
                };
                for position in 0..len {
                    if known.is_some_and(|known| known != position) {
                        continue;
                    }
                    let mut place = array.clone();
                    place.projection.push(Projection::ConstantIndex(position));
                    let ty = self.place_ty(self.f, &place)?.clone();
                    for (location, origins) in self.read_value(state, &place, &ty)? {
                        if let Some((_, existing)) = output
                            .iter_mut()
                            .find(|(candidate, _)| *candidate == location)
                        {
                            existing.union(&origins, self.budget)?;
                        } else {
                            output.push((location, origins));
                        }
                    }
                }
                Ok(output)
            }
            _ => {
                self.budget.spend(block.0 + statement.min(1) + 1)?;
                Ok(vec![])
            }
        }
    }
    fn step(
        &mut self,
        state: &mut FlowState,
        block: BlockId,
        index: usize,
        diagnose: bool,
    ) -> Result<(), Exhausted> {
        let statement = self.f.blocks[block.0].statements[index].value.clone();
        match &statement {
            Statement::StorageLive(local) | Statement::StorageDead(local) => {
                for (index, slot) in self.cells.iter().enumerate() {
                    if slot.key.storage.root == CanonicalRoot::Owned(*local) {
                        state.cells[index].clear();
                        state.overwritten.remove(index);
                    }
                }
            }
            Statement::Assign { destination, value } => {
                let saved = self.assignment_value(state, value, block, index)?;
                let mut moved = Vec::new();
                visit_rvalue(self.f, value, |place, movement| {
                    if movement {
                        moved.push(Operand::Move(place.clone()));
                    }
                });
                for operand in moved {
                    self.consume(state, &operand)?;
                }
                self.write_components(
                    state,
                    destination,
                    &saved,
                    true,
                    (block, Some(index), diagnose),
                )?;
                self.initialize_singleton_slot(state, block, Some(index))?;
            }
            Statement::IndexAssign {
                array,
                index: subscript,
                value,
            } => {
                // `(*s)[i] = v` through a slice carrier (MUTABLE-SLICES.md):
                // slice elements are reference-free (slice element boundary),
                // so the write cannot create or replace a carrier component.
                if let Ty::Slice { element, .. } = ir::indexed_array_ty(
                    &self.f.locals[array.local.0].ty,
                    &array.projection,
                    false,
                    &self.ledger,
                )
                .map_err(|_| Exhausted)?
                {
                    fn generic(ty: &Ty) -> bool {
                        match ty {
                            Ty::Param(_) | Ty::AssociatedType(_) | Ty::Opaque(_) => true,
                            Ty::Ref { referent, .. } => generic(referent),
                            Ty::Array { element, .. } | Ty::Slice { element } => generic(element),
                            Ty::Tuple(fields) => fields.iter().any(generic),
                            Ty::Struct(t) => t.def().type_args.iter().any(generic),
                            Ty::Enum(t) => t.def().type_args.iter().any(generic),
                            _ => false,
                        }
                    }
                    // Generic elements keep the reference-free contract: the
                    // caller instantiates them, and the callee's write envelope
                    // is checked against the instantiated signature.
                    let carries = !generic(&element)
                        && !ir::carrier_locations(&element, &self.ledger)
                            .map_err(|_| Exhausted)?
                            .is_empty();
                    if !carries {
                        self.consume(state, value)?;
                        return Ok(());
                    }
                    // A reference-carrying element store is a weak update of
                    // every element the slice may denote: the stored origins
                    // join the referent's element cells, never replace them.
                    let saved = self.operand_value(state, value)?;
                    self.consume(state, value)?;
                    let mut destination = array.clone();
                    destination
                        .projection
                        .push(Projection::DynamicIndex(LocalId(usize::MAX)));
                    self.write_components(
                        state,
                        &destination,
                        &saved,
                        false,
                        (block, Some(index), diagnose),
                    )?;
                    return Ok(());
                }
                // A runtime row prefix (`m[i][j] = v`, NESTED-INDEX.md) has no
                // constant element enumeration. Its admitted elements are
                // scalar/unit (ir::projected_ty), so the write cannot create,
                // replace or duplicate a reference carrier component.
                if array
                    .projection
                    .iter()
                    .any(|part| matches!(part, Projection::DynamicIndex(_)))
                {
                    let Ty::Array { element, .. } = ir::indexed_array_ty(
                        &self.f.locals[array.local.0].ty,
                        &array.projection,
                        false,
                        &self.ledger,
                    )
                    .map_err(|_| Exhausted)?
                    else {
                        return Err(Exhausted);
                    };
                    if !matches!(
                        element.as_ref(),
                        Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool | Ty::Unit
                    ) {
                        return Err(Exhausted);
                    }
                    self.consume(state, value)?;
                    return Ok(());
                }
                let saved = self.operand_value(state, value)?;
                let Ty::Array { len: count, .. } = self.place_ty(self.f, array)? else {
                    return Err(Exhausted);
                };
                let selected = match subscript {
                    Operand::Constant(cfg::Constant::Int(value)) => usize::try_from(value.bits)
                        .ok()
                        .filter(|index| *index < count),
                    _ => None,
                };
                self.consume(state, value)?;
                self.budget.spend(count)?;
                for element in 0..count {
                    if selected.is_some_and(|selected| selected != element) {
                        continue;
                    }
                    let mut destination = array.clone();
                    destination
                        .projection
                        .push(Projection::ConstantIndex(element));
                    self.write_components(
                        state,
                        &destination,
                        &saved,
                        selected.is_some(),
                        (block, Some(index), diagnose),
                    )?;
                }
            }
            // Indexed compound assignment has integer operands only, hence
            // cannot manufacture, replace or duplicate a carrier component.
            Statement::IndexAssignOp { .. }
            | Statement::IndexInspect { .. }
            | Statement::Inspect(_)
            | Statement::FakeRead(_) => {}
        }
        Ok(())
    }

    fn call(
        &mut self,
        state: &mut FlowState,
        block: BlockId,
        diagnose: bool,
    ) -> Result<(), Exhausted> {
        let Terminator::Call {
            args,
            signature,
            destination,
            target,
            ..
        } = self.f.blocks[block.0].terminator.value.clone()
        else {
            return Ok(());
        };
        self.retain(
            flow_state_bytes(
                state.cells.len(),
                self.targets.len(),
                self.family_inputs.len(),
            )?,
            args.len() + 1,
        )?;
        let entry = state.clone();
        if diagnose
            && let Some(view) = self.calls[block.0].clone()
            && let Some(binding) =
                self.bind_call(&entry, block, &view, &args, &signature, &target)?
        {
            checked_vec_growth(&mut self.bindings, self.budget)?;
            self.bindings.push(binding);
        }

        let mut result_values = Vec::new();
        let rows = self.call_rows[block.0].clone();
        let view = self.calls[block.0].clone();
        let authority = self.authority.clone();
        let facts = authority.as_deref().or(self.facts.validated());
        let family_replay = match (&view, facts, self.physical) {
            (Some(view), Some(facts), Some(physical)) => {
                let validation = physical
                    .validate_cfg(self.f, facts)
                    .and_then(|()| {
                        physical.validate_call(
                            self.f.blocks[block.0].terminator.origin,
                            &target,
                            &signature,
                        )
                    })
                    .and_then(|()| {
                        self.physical_calls
                            .get(block.0)
                            .and_then(Option::as_ref)
                            .ok_or_else(|| "physical call lost its exact carrier flow".to_string())?
                            .validate_boundary(
                                view,
                                facts,
                                ir::SignatureSubject::Call(&target),
                                &signature,
                            )
                    });
                if let Err(message) = validation {
                    if super::traits::FactsError::from_message(&self.ledger, &message).is_resource()
                    {
                        return Err(Exhausted);
                    }
                    self.problem(block, None, Problem::Invalid(message))?;
                    return Ok(());
                }
                // Physical ingress has a separate, independently normalized
                // boundary. Its original declaration can retain opaque tails
                // already proven empty by that boundary. They cannot request
                // an ordinary declaration replay for this physical view.
                let contract = view.contract();
                self.budget
                    .spend(contract.inputs.len() + contract.results.len() + 1)?;
                if contract
                    .inputs
                    .iter()
                    .map(|row| &row.location)
                    .chain(contract.results.iter().map(|row| &row.location))
                    .any(|location| location.channel.family().is_some())
                {
                    self.problem(
                        block,
                        None,
                        Problem::Unsupported(
                            "unsupported: physical call retains a symbolic carrier family".into(),
                        ),
                    )?;
                    return Ok(());
                }
                None
            }
            (Some(view), Some(facts), None) => {
                let contract = view.original_contract();
                self.budget
                    .spend(contract.inputs.len() + contract.results.len() + 1)?;
                let needed = contract
                    .inputs
                    .iter()
                    .map(|row| &row.location)
                    .chain(contract.results.iter().map(|row| &row.location))
                    .any(|location| {
                        matches!(
                            location.channel,
                            CarrierChannel::ProjectionTail { .. }
                                | CarrierChannel::ArrayTail { .. }
                                | CarrierChannel::OpaqueTail { .. }
                        )
                    });
                if needed {
                    Some(
                        view.replay_for_families(facts, &mut self.carrier_scratch)
                            .map_err(|_| Exhausted)?,
                    )
                } else {
                    None
                }
            }
            _ => None,
        };
        if facts.is_some() && view.is_none() {
            self.problem(
                block,
                None,
                Problem::Invalid("call result has no validated declaration view".into()),
            )?;
            return Ok(());
        }
        if let Some(view) = &view {
            // `new` seals this exact block's immutable callee/signature/view
            // only after validating every result row. Dataflow revisits use
            // that proof rather than reconstructing the same source metadata.
            for result in view.contract().results.iter() {
                // Return rows are entry relations. Pointee rows are not new
                // owned values and remain current-cell views of their anchors.
                if value_projection(&result.location).contains(&Projection::Deref) {
                    continue;
                }
                let mut value = self.empty_origins()?;
                for input in result.sources.iter() {
                    let input_value = self.input_value(&entry, &args, input)?;
                    value.union(&input_value, self.budget)?;
                }
                result_values.push((result.location.clone(), value));
            }
        }
        for (location, row) in rows {
            if let Some(view) = &view {
                self.budget.spend(view.contract().results.len() + 1)?;
                if !view
                    .contract()
                    .results
                    .iter()
                    .any(|result| result.location == location)
                {
                    self.problem(
                        block,
                        None,
                        Problem::Invalid(
                            "call result row is outside its validated carrier view".into(),
                        ),
                    )?;
                    return Ok(());
                }
            }
            let mut value = self.empty_origins()?;
            let mut fixed_contract = false;
            let mut fixed_positioned = false;
            // A fixed result row whose region is reached only through a
            // parameter nested in an input aggregate (`Option<T>::unwrap`
            // with `T = &'a U`) names no positioned input. Its origin is the
            // structural carrier source, which must still flow below.
            let mut fixed_sourceless = false;
            if location.path.0.is_empty()
                && let CarrierChannel::Fixed(position) = location.channel
                && let Some(result) = signature
                    .ref_contract
                    .results
                    .iter()
                    .find(|result| result.position == position)
            {
                fixed_contract = true;
                // Positioned rows cannot name a reference nested in a
                // by-value input aggregate (`self: Option<T>`), so the
                // structural sources always join the result's liveness.
                fixed_sourceless = true;
                // Declaration envelopes remain canonical. Only an exact
                // original complete-source projection can refine this call's
                // storage identity, independently of callee body analysis.
                let source_path = match (&view, facts) {
                    (Some(view), Some(facts)) => match view
                        .source_return_path(facts, &target, &signature, &location)
                    {
                        Ok(path) => path,
                        Err(error) if error.is_resource() => return Err(Exhausted),
                        Err(error) => {
                            self.problem(
                                block,
                                None,
                                Problem::Invalid(
                                    error.into_diagnostic(&self.ledger).map_err(|_| Exhausted)?,
                                ),
                            )?;
                            return Ok(());
                        }
                    },
                    _ => None,
                };
                let exact = source_path
                    .as_ref()
                    .map(|path| (path.input, path.projection.as_ref(), TargetPrecision::Exact));
                for (input, suffix, precision) in exact
                    .into_iter()
                    .chain(return_inputs(&result.returns).filter(|_| source_path.is_none()))
                {
                    fixed_positioned = true;
                    let input_value = self.input_value(
                        &entry,
                        &args,
                        &InputCarrier {
                            param: input.param,
                            location: fixed_location(input.position),
                        },
                    )?;
                    for parent in input_value.references.0.members() {
                        self.parents[row.0].insert(parent);
                        self.dependencies[row.0].insert(parent);
                        for target in self.targets[parent].clone() {
                            let mut next = target;
                            if next.precision == TargetPrecision::Exact
                                && precision == TargetPrecision::Exact
                            {
                                next.place.projection.extend_from_slice(suffix);
                            } else {
                                next.precision = TargetPrecision::Subtree;
                            }
                            if !next.authority.contains(&parent) {
                                next.authority.push(parent);
                                next.authority.sort_unstable();
                            }
                            self.push_target(row, next)?;
                        }
                    }
                }
                let closure = signature.ref_contract.closure().map_err(|_| Exhausted)?;
                for input in signature.ref_contract.input_regions.iter() {
                    if closure[input.region.0 as usize] & (1u64 << result.region.0) != 0 {
                        let actual = self.input_value(
                            &entry,
                            &args,
                            &InputCarrier {
                                param: input.input.param,
                                location: fixed_location(input.input.position),
                            },
                        )?;
                        self.dependencies[row.0].union(&actual.references.0, self.budget)?;
                    }
                }
            }
            // Fixed contracts retain exact referent suffixes. Named nominal
            // inputs can contribute additional origins independently, but their
            // coarse root rows must not replace those precise fixed targets.
            if fixed_contract
                && let Some(view) = &view
                && view.source_region_env().carriers.iter().any(|source| {
                    source.parameter.is_none()
                        && source.location == location
                        && matches!(source.region, ir::CarrierRegionTerm::Named(_))
                })
                && let Some(result) = view
                    .contract()
                    .results
                    .iter()
                    .find(|result| result.location == location)
            {
                let result_carrier = ir::carrier_at(&signature.result, &location, &self.ledger)
                    .map_err(|_| Exhausted)?;
                let result_stores_carriers = if let Ty::Ref { referent, .. } = result_carrier {
                    !ir::carrier_locations(&referent, &self.ledger)
                        .map_err(|_| Exhausted)?
                        .is_empty()
                } else {
                    false
                };
                for source in result
                    .sources
                    .iter()
                    .filter(|source| !source.location.path.0.is_empty())
                {
                    let input = self.input_value(&entry, &args, source)?;
                    let source_carrier = ir::carrier_at(
                        signature.params.get(source.param).ok_or(Exhausted)?,
                        &source.location,
                        &self.ledger,
                    )
                    .map_err(|_| Exhausted)?;
                    // An outlives source is not necessarily the returned
                    // storage. A reference-free pointee (`str` here) cannot
                    // contain the reference slot selected by a positioned
                    // `&mut &str` result. Keep every loan dependency, but do
                    // not turn that pointee into a carrier-storage target.
                    let liveness_only = fixed_positioned
                        && result_stores_carriers
                        && matches!(source_carrier, Ty::Ref { ref referent, .. }
                            if ir::carrier_locations(referent, &self.ledger)
                                .map_err(|_| Exhausted)?.is_empty());
                    for parent in input.references.0.members() {
                        self.parents[row.0].insert(parent);
                        self.dependencies[row.0].insert(parent);
                        if liveness_only {
                            continue;
                        }
                        for target in self.targets[parent].clone() {
                            let mut target = self.structural_result_target(
                                target,
                                &signature.result,
                                &location,
                            )?;
                            if !target.authority.contains(&parent) {
                                target.authority.push(parent);
                                target.authority.sort_unstable();
                            }
                            self.push_target(row, target)?;
                        }
                    }
                }
            }
            if (!fixed_contract || fixed_sourceless)
                && let Some((_, input)) = result_values
                    .iter()
                    .find(|(candidate, _)| *candidate == location)
            {
                let captured_storage = if fixed_sourceless && !fixed_positioned {
                    match (&view, facts) {
                        (Some(view), Some(facts)) => match view
                            .captured_result_storage(facts, &target, &signature, &location)
                        {
                            Ok(storage) => storage,
                            Err(error) if error.is_resource() => return Err(Exhausted),
                            Err(error) => {
                                self.problem(
                                    block,
                                    None,
                                    Problem::Invalid(
                                        error
                                            .into_diagnostic(&self.ledger)
                                            .map_err(|_| Exhausted)?,
                                    ),
                                )?;
                                return Ok(());
                            }
                        },
                        _ => None,
                    }
                } else {
                    None
                };
                for parent in input.references.0.members() {
                    self.parents[row.0].insert(parent);
                    self.dependencies[row.0].insert(parent);
                    if fixed_sourceless {
                        // Lifetime-only rows still never identify storage.
                        // An exact original capture Borrow/Read is separate
                        // source authority, not a rigid parameter-family proof.
                        let inherited = !fixed_positioned
                            && (captured_storage.is_some()
                                || match &view {
                                    Some(view) => self.owned_shared_family_result_source(
                                        &signature, view, &location,
                                    )?,
                                    None => false,
                                });
                        if !inherited {
                            continue;
                        }
                    }
                    for mut target in self.targets[parent].clone() {
                        if let Some(storage) = &captured_storage
                            && target.precision == TargetPrecision::Exact
                        {
                            self.budget.spend(storage.projection().len())?;
                            target
                                .place
                                .projection
                                .extend_from_slice(storage.projection());
                        }
                        let mut target =
                            self.structural_result_target(target, &signature.result, &location)?;
                        if !target.authority.contains(&parent) {
                            target.authority.push(parent);
                            target.authority.sort_unstable();
                        }
                        self.push_target(row, target)?;
                    }
                }
            }
            // A fixed carrier inside an authenticated empty native sequence is
            // not an initialized reference. Absence is carried by the exact
            // selected universal source proof, never inferred from no targets.
            let empty_native = if args.is_empty()
                && signature.params.is_empty()
                && value.count() == 0
                && self.targets[row.0].is_empty()
                && self.parents[row.0].count() == 0
                && self.dependencies[row.0].count() == 0
                && matches!(location.path.0.first(), Some(CarrierStep::SliceElement))
                && let (Some(table), Some(facts), Some(view)) = (
                    self.effect_table.clone(),
                    self.authority.clone(),
                    view.as_ref(),
                ) {
                let caller_validated = match self.function.clone() {
                    Some(function) if std::ptr::eq(Arc::as_ptr(&function), self.f) => {
                        match self.ensure_provenance(&function, &facts) {
                            Ok(()) => true,
                            Err(message)
                                if message.contains("limit") || message.contains("exhaust") =>
                            {
                                return Err(Exhausted);
                            }
                            Err(_) => false,
                        }
                    }
                    _ => false,
                };
                match table.empty_result_with_scratch(
                    &target,
                    self.f,
                    &facts,
                    view,
                    &signature,
                    &location,
                    caller_validated,
                    &mut self.carrier_scratch,
                ) {
                    Ok(empty) => empty,
                    Err(error) if error.is_resource() => return Err(Exhausted),
                    Err(error) => {
                        if diagnose {
                            self.problem(
                                block,
                                None,
                                Problem::Invalid(
                                    error.into_diagnostic(&self.ledger).map_err(|_| Exhausted)?,
                                ),
                            )?;
                        }
                        false
                    }
                }
            } else {
                false
            };
            if !empty_native {
                value.references.0.insert(row.0);
            }
            if let Some((_, old)) = result_values
                .iter_mut()
                .find(|(candidate, _)| *candidate == location)
            {
                *old = value;
            } else {
                result_values.push((location, value));
            }
        }

        let mut writes: Vec<(usize, CarrierOrigins, bool)> = Vec::new();
        if let Some(view) = &view {
            let summary = if let Some(summary) = &self.call_effects[block.0] {
                summary.clone()
            } else if let (Some(table), Some(facts)) =
                (self.effect_table.clone(), self.authority.clone())
            {
                let caller_validated = match self.function.clone() {
                    Some(function) if std::ptr::eq(Arc::as_ptr(&function), self.f) => {
                        match self.ensure_provenance(&function, &facts) {
                            Ok(()) => true,
                            Err(message)
                                if message.contains("limit") || message.contains("exhaust") =>
                            {
                                return Err(Exhausted);
                            }
                            // Let lookup report the exact original diagnostic.
                            Err(_) => false,
                        }
                    }
                    _ => false,
                };
                let summary = match family_replay.as_ref() {
                    Some(replay) => table.lookup_replayed(
                        &target,
                        self.f,
                        replay,
                        caller_validated,
                        &mut self.carrier_scratch,
                    ),
                    None => table.lookup(
                        &target,
                        self.f,
                        &facts,
                        view,
                        caller_validated,
                        &mut self.carrier_scratch,
                    ),
                };
                match summary {
                    Ok(summary) => {
                        self.call_effects[block.0] = Some(summary.clone());
                        summary
                    }
                    Err(error) => {
                        if error.is_resource() {
                            return Err(Exhausted);
                        }
                        if diagnose {
                            self.problem(
                                block,
                                None,
                                Problem::Invalid(
                                    error.into_diagnostic(&self.ledger).map_err(|_| Exhausted)?,
                                ),
                            )?;
                        }
                        None
                    }
                }
            } else {
                None
            };
            let mut selected = Vec::new();
            let declared_writes = summary
                .as_ref()
                .map_or(view.original_contract().writes.as_ref(), |summary| {
                    summary.writes.as_ref()
                });
            for write in declared_writes.iter() {
                for (destination, component) in self.transport_effect_input_with_replay(
                    &write.destination,
                    view,
                    true,
                    family_replay.as_ref(),
                )? {
                    let mut sources = Vec::new();
                    for source in write.sources.iter() {
                        let same_family =
                            write
                                .destination
                                .location
                                .channel
                                .family()
                                .is_some_and(|family| {
                                    source.location.channel.family().as_ref() == Some(&family)
                                });
                        for (source, source_component) in self.transport_effect_input_with_replay(
                            source,
                            view,
                            false,
                            family_replay.as_ref(),
                        )? {
                            // Family substitution is elementwise, never a Cartesian
                            // union of unrelated actual carrier components.
                            if same_family
                                && component.is_some()
                                && source_component.is_some()
                                && component != source_component
                            {
                                continue;
                            }
                            self.retain(size_of::<InputCarrier>(), 1)?;
                            checked_vec_growth(&mut sources, self.budget)?;
                            sources.push(source);
                        }
                    }
                    let must = summary
                        .as_ref()
                        .is_some_and(|summary| summary.must_overwrite.contains(&write.destination));
                    self.retain(size_of::<(ir::CarrierWriteEnvelope, bool)>(), 1)?;
                    checked_vec_growth(&mut selected, self.budget)?;
                    checked_arc_transfer::<InputCarrier>(sources.len(), self.budget)?;
                    selected.push((
                        ir::CarrierWriteEnvelope {
                            destination,
                            sources: sources.into(),
                        },
                        must,
                    ));
                }
            }
            for (write, must) in selected {
                let Some(Operand::Copy(actual) | Operand::Move(actual)) =
                    args.get(write.destination.param)
                else {
                    continue;
                };
                let mut destination = actual.clone();
                destination
                    .projection
                    .extend(value_projection(&write.destination.location));
                let ty = self.place_ty(self.f, &destination)?.clone();
                let Some(channel) = key_channel(&ty) else {
                    continue;
                };
                let mut value = self.empty_origins()?;
                for source in write.sources.iter() {
                    let old = self.input_value(&entry, &args, source)?;
                    value.union(&old, self.budget)?;
                }
                let (targets, _, failure) = self.lens(&entry, &destination)?;
                if let Some(failure) = failure {
                    if diagnose {
                        self.problem(
                            block,
                            None,
                            failure.problem(
                                "call carrier effect lacks a complete original typed cell cover",
                            ),
                        )?;
                    }
                    continue;
                }
                if diagnose {
                    for target in &targets {
                        self.retain(
                            size_of::<Target>()
                                + mul(target.place.projection.len(), size_of::<Projection>())?
                                + mul(target.authority.len(), size_of::<usize>())?,
                            target.place.projection.len() + target.authority.len() + 1,
                        )?;
                        self.budget.spend(self.bindings.len())?;
                        if let Some(binding) = self
                            .bindings
                            .iter_mut()
                            .find(|binding| binding.block == block)
                        {
                            checked_vec_growth(&mut binding.actual_writes, self.budget)?;
                            binding.actual_writes.push(target.clone());
                        }
                    }
                }
                let mut indices = Vec::new();
                for target in &targets {
                    if target.precision != TargetPrecision::Exact {
                        continue;
                    }
                    if let Some(index) = self.cell(&CellKey {
                        storage: target.place.clone(),
                        channel: channel.clone(),
                    })? && !indices.contains(&index)
                    {
                        indices.push(index);
                    }
                }
                let copy = self.copy_permission(&ty)?;
                // A Copy carrier written through a summarized element family
                // (`&mut [&'a T]` over owned storage) weakly joins every cell
                // its subtree cover denotes. Affine carriers still need one
                // exact destination.
                let mut weak_cover = false;
                if copy && indices.len() != targets.len() {
                    let mut covered = Vec::new();
                    let mut complete = true;
                    for target in &targets {
                        match read_cell_cover_with_ledger(
                            self.family_cover_locals
                                .as_deref()
                                .unwrap_or(&self.original_locals),
                            &self.cell_index,
                            target,
                            &channel,
                            Some(&self.ledger),
                            self.budget,
                        )? {
                            Some(cover) if !cover.is_empty() => {
                                for cell in cover {
                                    if !covered.contains(&cell) {
                                        checked_vec_growth(&mut covered, self.budget)?;
                                        covered.push(cell);
                                    }
                                }
                            }
                            _ => complete = false,
                        }
                    }
                    if complete {
                        indices = covered;
                        weak_cover = true;
                    }
                }
                if diagnose
                    && !weak_cover
                    && (indices.len() != targets.len() || (!copy && indices.len() != 1))
                {
                    self.problem(block, None, Problem::Unsupported("call carrier effect lacks one affine target or a complete typed copy-cell cover".into()))?;
                }
                let mutable = match destination
                    .projection
                    .iter()
                    .rposition(|step| *step == Projection::Deref)
                {
                    Some(depth) => matches!(
                        ir::projected_ty(
                            &self.f.locals[destination.local.0].ty,
                            &destination.projection[..depth],
                            &self.ledger,
                        )
                        .map_err(|_| Exhausted)?,
                        Ty::Ref {
                            kind: BorrowKind::Mut,
                            ..
                        }
                    ),
                    None => false,
                };
                let strong = must
                    && mutable
                    && indices.len() == 1
                    && targets.len() == 1
                    && targets[0].precision == TargetPrecision::Exact;
                for index in indices {
                    // OLD is retained unless a sealed all-normal-exit proof and
                    // a unique exact mutable destination justify replacement.
                    let mut post = value.clone();
                    if !strong {
                        post.union(&entry.cells[index], self.budget)?;
                    }
                    if diagnose
                        && matches!(
                            self.cells[index].key.storage.root,
                            CanonicalRoot::ParameterReferent(..)
                        )
                        && let (Some(body), Some(destination)) =
                            (self.body.clone(), self.cells[index].input.clone())
                    {
                        if let Some(envelope) = body
                            .contract()
                            .writes
                            .iter()
                            .find(|row| row.destination == destination)
                        {
                            if !self.origin_allowed(&value, &envelope.sources)? {
                                self.problem(block, None, Problem::Invalid("callee writes origins outside the caller declaration envelope".into()))?;
                            }
                        } else {
                            self.problem(block, None, Problem::Invalid("callee writes a carrier cell outside the caller declaration envelope".into()))?;
                        }
                    }
                    if let Some((_, existing, existing_strong)) = writes
                        .iter_mut()
                        .find(|(candidate, _, _)| *candidate == index)
                    {
                        existing.union(&post, self.budget)?;
                        *existing_strong &= strong;
                        if !*existing_strong {
                            existing.union(&entry.cells[index], self.budget)?;
                        }
                    } else {
                        checked_vec_growth(&mut writes, self.budget)?;
                        writes.push((index, post, strong));
                    }
                }
            }
        }

        // Invariant generic arguments constrain stored value lifetimes even
        // without an executable effect summary. This is a monotone union only:
        // never a target/write authorization, replacement or must-overwrite.
        if let Some(view) = &view {
            for invariant in view.native_heap_invariants() {
                for (storage, component) in self.transport_input_components_with_replay(
                    &invariant.storage,
                    view,
                    true,
                    family_replay.as_ref(),
                )? {
                    let Some(Operand::Copy(actual) | Operand::Move(actual)) =
                        args.get(storage.param)
                    else {
                        continue;
                    };
                    let mut destination = actual.clone();
                    destination
                        .projection
                        .extend(value_projection(&storage.location));
                    let ty = self.place_ty(self.f, &destination)?.clone();
                    let Some(channel) = key_channel(&ty) else {
                        continue;
                    };
                    let mut origins = self.empty_origins()?;
                    for source in invariant.sources.iter() {
                        for (source, source_component) in self.transport_effect_input_with_replay(
                            source,
                            view,
                            false,
                            family_replay.as_ref(),
                        )? {
                            if component.is_some()
                                && source_component.is_some()
                                && component != source_component
                            {
                                continue;
                            }
                            let incoming = self.input_value(&entry, &args, &source)?;
                            origins.union(&incoming, self.budget)?;
                        }
                    }
                    let (targets, _, failure) = self.lens(&entry, &destination)?;
                    if let Some(failure) = failure {
                        if diagnose {
                            self.problem(
                                block,
                                None,
                                failure.problem(
                                    "invariant heap lifetime lacks an issued carrier cover",
                                ),
                            )?;
                        }
                        continue;
                    }
                    for target in targets {
                        let Some(cover) = read_cell_cover_with_ledger(
                            self.family_cover_locals
                                .as_deref()
                                .unwrap_or(&self.original_locals),
                            &self.cell_index,
                            &target,
                            &channel,
                            Some(&self.ledger),
                            self.budget,
                        )?
                        else {
                            if diagnose {
                                self.problem(
                                    block,
                                    None,
                                    Problem::Invalid(
                                        "invariant heap lifetime lost its issued origin cell"
                                            .into(),
                                    ),
                                )?;
                            }
                            continue;
                        };
                        // This relation only retains lifetimes. A conservative
                        // cover weakly joins every issued cell, never granting
                        // replacement, write authorization or disjointness.
                        for index in cover {
                            let mut post = entry.cells[index].clone();
                            post.union(&origins, self.budget)?;
                            if let Some((_, existing, strong)) = writes
                                .iter_mut()
                                .find(|(candidate, _, _)| *candidate == index)
                            {
                                existing.union(&post, self.budget)?;
                                *strong = false;
                            } else {
                                checked_vec_growth(&mut writes, self.budget)?;
                                writes.push((index, post, false));
                            }
                        }
                    }
                }
            }
        }

        for operand in &args {
            self.consume(state, operand)?;
        }
        // Merge destination aliases from the entry snapshot before installing
        // any poststate, never interpret a source as a sequential new read.
        for (index, post, strong) in writes {
            self.membership(post.count())?;
            state.cells[index] = post;
            if strong {
                state.overwritten.insert(index);
            }
        }
        self.write_components(
            state,
            &destination,
            &result_values,
            true,
            (block, None, diagnose),
        )?;
        self.initialize_singleton_slot(state, block, None)?;
        Ok(())
    }

    fn used_place(
        &mut self,
        state: &FlowState,
        place: &Place,
        live: &mut Set,
        defs: &mut Set,
        moved: bool,
    ) -> Result<(), Exhausted> {
        // Scalar-only bodies have no carrier liveness to compute. Ordinary
        // initialization and move checking remains in the ownership analysis.
        if self.cells.is_empty() {
            return Ok(());
        }
        let (targets, trace, _) = self.lens(state, place)?;
        for parent in trace {
            for (index, value) in state.cells.iter().enumerate() {
                self.budget.spend(value.references.0.0.len() + 1)?;
                if value.references.0.has(parent.0) {
                    live.insert(index);
                }
            }
        }
        for target in targets {
            for (index, slot) in self.cells.iter().enumerate() {
                self.budget.spend(slot.key.storage.projection.len() + 1)?;
                if slot.key.storage.root == target.place.root
                    && slot
                        .key
                        .storage
                        .projection
                        .starts_with(&target.place.projection)
                {
                    live.insert(index);
                    if moved && target.precision == TargetPrecision::Exact {
                        defs.insert(index);
                    }
                }
            }
        }
        Ok(())
    }
    fn uses(
        &mut self,
        state: &FlowState,
        block: BlockId,
        statement: usize,
    ) -> Result<(Set, Set), Exhausted> {
        let mut used = Set::new(self.cells.len());
        let mut defs = Set::new(self.cells.len());
        if let Some(item) = self.f.blocks[block.0].statements.get(statement) {
            let mut operands = Vec::new();
            visit_rvalue_for_statement(self.f, &item.value, |place, moved| {
                operands.push((place.clone(), moved))
            });
            for (place, moved) in operands {
                self.used_place(state, &place, &mut used, &mut defs, moved)?;
            }
            match &item.value {
                Statement::Assign { destination, .. } => {
                    let mut ignored = Set::new(self.cells.len());
                    self.used_place(state, destination, &mut ignored, &mut defs, true)?;
                    if destination.projection.contains(&Projection::Deref) {
                        used.union(&ignored, self.budget)?;
                    }
                }
                Statement::StorageLive(local) | Statement::StorageDead(local) => {
                    for (index, cell) in self.cells.iter().enumerate() {
                        if cell.key.storage.root == CanonicalRoot::Owned(*local) {
                            defs.insert(index);
                        }
                    }
                }
                _ => {}
            }
        } else {
            match &self.f.blocks[block.0].terminator.value {
                Terminator::Call {
                    args, destination, ..
                } => {
                    for operand in args {
                        if let Operand::Copy(place) | Operand::Move(place) = operand {
                            self.used_place(
                                state,
                                place,
                                &mut used,
                                &mut defs,
                                matches!(operand, Operand::Move(_)),
                            )?;
                        }
                    }
                    // OLD contents of every potential write destination are
                    // semantically read from ENTRY, even if argument moves kill
                    // their own handle locals.
                    if let Some(view) = self.calls[block.0].clone() {
                        for write in view.contract().writes.iter() {
                            if let Some(Operand::Copy(place) | Operand::Move(place)) =
                                args.get(write.destination.param)
                            {
                                let mut place = place.clone();
                                place
                                    .projection
                                    .extend(value_projection(&write.destination.location));
                                self.used_place(
                                    state,
                                    &place,
                                    &mut used,
                                    &mut Set::new(self.cells.len()),
                                    false,
                                )?;
                            }
                        }
                    }
                    self.used_place(
                        state,
                        destination,
                        &mut Set::new(self.cells.len()),
                        &mut defs,
                        true,
                    )?;
                }
                Terminator::Drop { cleanup, .. } => {
                    // The prelude `Vec<T>` drops `#[may_dangle] T`: with a
                    // Copy (glue-free) element, dropping the vector does not
                    // use reference carriers stored in its element family.
                    let mut dangle = false;
                    if cleanup.owner().projection.is_empty()
                        && let Ty::Struct(owner) = &self.f.locals[cleanup.owner().local.0].ty
                        && owner.def().name
                            == format!(
                                "crate::{}::vec::Vec",
                                crate::compiler::source::source_prelude::PRELUDE_MODULE
                            )
                    {
                        dangle = true;
                        for arg in owner.def().type_args.iter() {
                            if !self.copy_permission(arg)? {
                                dangle = false;
                            }
                        }
                    }
                    let mut drop_used = Set::new(self.cells.len());
                    self.used_place(state, cleanup.owner(), &mut drop_used, &mut defs, true)?;
                    if dangle {
                        let root = CanonicalRoot::Owned(cleanup.owner().local);
                        for (index, cell) in self.cells.iter().enumerate() {
                            self.budget.spend(cell.key.storage.projection.len() + 1)?;
                            if cell.key.storage.root == root
                                && matches!(
                                    cell.key.storage.projection.first(),
                                    Some(Projection::DynamicIndex(_))
                                )
                                && cell.key.storage.projection[1..].iter().all(|step| {
                                    matches!(
                                        step,
                                        Projection::Field(_) | Projection::ConstantIndex(_)
                                    )
                                })
                            {
                                drop_used.remove(index);
                            }
                        }
                    }
                    used.union(&drop_used, self.budget)?;
                }
                Terminator::Return => self.used_place(
                    state,
                    &Place {
                        local: self.f.return_local,
                        projection: vec![],
                    },
                    &mut used,
                    &mut defs,
                    false,
                )?,
                Terminator::Switch {
                    condition: Operand::Copy(place) | Operand::Move(place),
                    ..
                }
                | Terminator::SwitchVariant { place, .. } => {
                    self.used_place(state, place, &mut used, &mut defs, false)?
                }
                _ => {}
            }
        }
        Ok((used, defs))
    }
    fn copy_permission(&mut self, ty: &Ty) -> Result<bool, Exhausted> {
        let authority = self.authority.clone();
        self.copy_permission_in(ty, authority.as_deref())
    }
    fn copy_permission_in(
        &mut self,
        ty: &Ty,
        authority: Option<&super::traits::ValidatedFacts>,
    ) -> Result<bool, Exhausted> {
        if let Some(facts) = authority.or(match self.facts {
            FactsCtx::Validated(facts) => Some(facts),
            _ => None,
        }) {
            match facts.must_copy_scratch(ty, &mut self.copy_scratch) {
                Ok(copy) => Ok(copy),
                Err(error) if error.is_resource() => Err(Exhausted),
                Err(error) => {
                    let unsupported = error.kind() == super::traits::FactsErrorKind::Unsupported;
                    let message = error.into_diagnostic(&self.ledger).map_err(|_| Exhausted)?;
                    self.problem(
                        self.f.entry,
                        None,
                        if unsupported {
                            Problem::Unsupported(message)
                        } else {
                            Problem::Invalid(message)
                        },
                    )?;
                    Ok(false)
                }
            }
        } else {
            match self.facts.must_copy_scratch(ty, &mut self.copy_scratch) {
                Ok(copy) => Ok(copy),
                Err(message) if message.contains("limit") || message.contains("exhaust") => {
                    Err(Exhausted)
                }
                Err(message) => {
                    self.problem(self.f.entry, None, Problem::Invalid(message))?;
                    Ok(false)
                }
            }
        }
    }
    fn metadata_value<T>(&mut self, value: Result<T, String>) -> Result<Option<T>, Exhausted> {
        match value {
            Ok(value) => Ok(Some(value)),
            Err(message) => {
                let error = super::traits::FactsError::from_message(&self.ledger, &message);
                if error.is_resource() {
                    return Err(Exhausted);
                }
                let unsupported = error.kind() == super::traits::FactsErrorKind::Unsupported;
                let message = error.into_diagnostic(&self.ledger).map_err(|_| Exhausted)?;
                self.problem(
                    self.f.entry,
                    None,
                    if unsupported {
                        Problem::Unsupported(message)
                    } else {
                        Problem::Invalid(message)
                    },
                )?;
                Ok(None)
            }
        }
    }
    fn identity_slice_component_matches(
        &mut self,
        parameter: ir::ParamId,
        original: &CarrierLocation,
        expanded: &CarrierLocation,
        mapping: &ir::CarrierComponentMapping,
        nominals: &[ir::NominalCarrierMapping],
    ) -> Result<bool, Exhausted> {
        self.budget.spend(1)?;
        if mapping.original_family != ir::CarrierFamily::Parameter(parameter) {
            return Ok(false);
        }
        // Component mappings are relative to the substituted actual, not to
        // the parameter's occurrence within the function signature. Replay
        // both sides at that exact occurrence using the existing composer.
        Ok(
            self.compose_component(original, &mapping.original, nominals)? == *original
                && self.compose_component(original, &mapping.substituted, nominals)? == *expanded,
        )
    }
    fn identity_copy_slice_reborrow_admitted(
        &mut self,
        params: &[Ty],
        referent: &Ty,
        view: &super::traits::CarrierContractView,
        authority: Option<&super::traits::ValidatedFacts>,
    ) -> Result<bool, Exhausted> {
        let Ty::Slice { element } = referent else {
            return Ok(false);
        };
        let Some(parameter) =
            identity_slice_reborrow_rows(view.original_contract(), view.contract(), self.budget)?
        else {
            return Ok(false);
        };
        let original = &view.original_contract().writes[0].destination;
        let expanded = &view.contract().writes[0].destination;
        let Some(Ty::Ref {
            kind: BorrowKind::Mut,
            referent: input,
        }) = params.get(original.param)
        else {
            return Ok(false);
        };
        if input.owned_sequence_element().is_none() {
            return Ok(false);
        }
        let mut mapped = false;
        for mapping in view.component_mappings() {
            mapped |= self.identity_slice_component_matches(
                parameter,
                &original.location,
                &expanded.location,
                mapping,
                view.nominal_mappings(),
            )?;
        }
        if !mapped {
            return Ok(false);
        }
        let Some((_, actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)? else {
            return Ok(false);
        };
        // The entire slice is unsized and non-Copy. Permission is restricted
        // instead to this sealed family actual and the result's stored element.
        Ok(self.copy_permission_in(actual, authority)?
            && self.copy_permission_in(element, authority)?)
    }
    fn identity_nested_slot_reborrow_admitted(
        &mut self,
        params: &[Ty],
        result: &Ty,
        view: &super::traits::CarrierContractView,
        authority: Option<&super::traits::ValidatedFacts>,
    ) -> Result<bool, Exhausted> {
        let Ty::Ref {
            kind: BorrowKind::Mut,
            referent: actual,
        } = result
        else {
            return Ok(false);
        };
        let Ty::Ref {
            kind: BorrowKind::Mut,
            referent: leaf,
        } = actual.as_ref()
        else {
            return Ok(false);
        };
        let Some(parameter) = identity_nested_slot_reborrow_rows(
            view.original_contract(),
            view.contract(),
            self.budget,
        )?
        else {
            return Ok(false);
        };
        let original = view.original_contract();
        let expanded = view.contract();
        let owner = original.writes[0].destination.param;
        let Some(Ty::Ref {
            kind: BorrowKind::Mut,
            referent: sequence,
        }) = params.get(owner)
        else {
            return Ok(false);
        };
        if sequence.owned_sequence_element() != Some(actual.as_ref()) {
            return Ok(false);
        }
        let Some((_, sealed_actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)?
        else {
            return Ok(false);
        };
        if sealed_actual != actual.as_ref() || view.component_mappings().len() != 1 {
            return Ok(false);
        }
        let mapping = &view.component_mappings()[0];
        if !self.identity_slice_component_matches(
            parameter,
            &original.writes[0].destination.location,
            &expanded.writes[0].destination.location,
            mapping,
            view.nominal_mappings(),
        )? || !self.identity_slice_component_matches(
            parameter,
            &original.results[1].location,
            &expanded.results[1].location,
            mapping,
            view.nominal_mappings(),
        )? {
            return Ok(false);
        }
        let facts = authority
            .or(self.authority.as_deref())
            .or(match self.facts {
                FactsCtx::Validated(facts) => Some(facts),
                _ => None,
            });
        if !facts.is_some_and(|facts| facts.phase() == super::traits::FactsPhase::Closed) {
            return Ok(false);
        }
        // The mutable reference itself stays affine. Only the reference-free
        // pointee must be Copy, excluding nested or hidden carrier families.
        let Some(locations) = self.metadata_value(ir::carrier_locations(leaf, &self.ledger))?
        else {
            return Ok(false);
        };
        Ok(locations.is_empty() && self.copy_permission_in(leaf, authority)?)
    }
    /// The authored envelope of a generic declaration may add the carrier
    /// family of a type parameter to an otherwise identity row. A family whose
    /// sealed actual stores no carrier is empty, so the row is still identity.
    fn original_identity_write_rows(
        &mut self,
        view: &super::traits::CarrierContractView,
    ) -> Result<bool, Exhausted> {
        let contract = view.original_contract();
        if exact_identity_write_rows(contract, self.budget)? {
            return Ok(true);
        }
        if contract.writes.is_empty() {
            return Ok(false);
        }
        for (index, row) in contract.writes.iter().enumerate() {
            self.budget
                .spend(add(contract.inputs.len(), add(index, row.sources.len())?)?)?;
            if !contract.inputs.contains(&row.destination)
                || !row.sources.contains(&row.destination)
                || contract.writes[..index]
                    .iter()
                    .any(|prior| prior.destination == row.destination)
            {
                return Ok(false);
            }
            for source in row.sources.iter() {
                if *source == row.destination {
                    continue;
                }
                let CarrierChannel::ParamTail { parameter, .. } = source.location.channel else {
                    return Ok(false);
                };
                let Some((_, actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)?
                else {
                    return Ok(false);
                };
                let Some(locations) =
                    self.metadata_value(ir::carrier_locations(actual, &self.ledger))?
                else {
                    return Ok(false);
                };
                if !locations.is_empty() {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    /// A single mutable reborrow may coexist with unchanged Copy referent
    /// cells. This is not the general identity-effect fallback: both authored
    /// and instantiated envelopes must be exact identities, every expanded
    /// destination must be accounted for once, and no affine slot is writable.
    fn identity_copy_reborrow_admitted(
        &mut self,
        params: &[Ty],
        result: &Ty,
        view: &super::traits::CarrierContractView,
        authority: Option<&super::traits::ValidatedFacts>,
    ) -> Result<bool, Exhausted> {
        // The one affine result may be the root reference or one mutable
        // reference stored by value in the result (`Option<&mut T>`): the
        // caller has already counted exactly one owned result channel.
        let Some(locations) = self.metadata_value(ir::carrier_locations(result, &self.ledger))?
        else {
            return Ok(false);
        };
        let mut owned = None;
        for location in locations.iter() {
            self.budget.spend(location.path.0.len() + 1)?;
            if value_projection(location).contains(&Projection::Deref) {
                continue;
            }
            if owned.is_some()
                || !location.path.0.iter().all(|step| {
                    matches!(
                        step,
                        CarrierStep::Field(_) | CarrierStep::VariantField { .. }
                    )
                })
            {
                return Ok(false);
            }
            owned = Some(location.clone());
        }
        let Some(owned) = owned else {
            return Ok(false);
        };
        let Some(Ty::Ref {
            kind: BorrowKind::Mut,
            referent,
        }) = self.metadata_value(ir::carrier_at(result, &owned, &self.ledger))?
        else {
            return Ok(false);
        };
        let referent = &*referent;
        let referent_copy = self.copy_permission_in(referent, authority)?;
        let slice_copy = !referent_copy
            && self.identity_copy_slice_reborrow_admitted(params, referent, view, authority)?;
        let reference_free = if referent_copy || slice_copy {
            false
        } else {
            let facts = authority
                .or(self.authority.as_deref())
                .or(match self.facts {
                    FactsCtx::Validated(facts) => Some(facts),
                    _ => None,
                });
            if facts.is_some_and(|facts| facts.phase() == super::traits::FactsPhase::Closed) {
                self.metadata_value(ir::carrier_locations(referent, &self.ledger))?
                    .is_some_and(|locations| locations.is_empty())
            } else {
                false
            }
        };
        if !(referent_copy || slice_copy || reference_free)
            || !self.original_identity_write_rows(view)?
            || !exact_identity_write_rows(view.contract(), self.budget)?
        {
            return Ok(false);
        }
        let mut destinations = Vec::new();
        for write in view.original_contract().writes.iter() {
            for (destination, _) in self.transport_effect_input(&write.destination, view, true)? {
                self.budget
                    .spend(destinations.len() + view.contract().writes.len() + 1)?;
                if destinations.contains(&destination)
                    || !view
                        .contract()
                        .writes
                        .iter()
                        .any(|row| row.destination == destination)
                {
                    return Ok(false);
                }
                let Some(parameter) = params.get(destination.param) else {
                    return Ok(false);
                };
                let Some(semantic) = self.metadata_value(ir::carrier_at(
                    parameter,
                    &destination.location,
                    &self.ledger,
                ))?
                else {
                    return Ok(false);
                };
                if !self.copy_permission_in(&semantic, authority)? {
                    return Ok(false);
                }
                self.retain(
                    size_of::<InputCarrier>()
                        + mul(destination.location.path.0.len(), size_of::<CarrierStep>())?,
                    1,
                )?;
                checked_vec_growth(&mut destinations, self.budget)?;
                destinations.push(destination);
            }
        }
        Ok(destinations.len() == view.contract().writes.len())
    }
    /// Copy effects may retain an owned nested component of a non-Copy value.
    /// Authenticate its relative coordinate against the selected actual, then
    /// replay that coordinate at this exact original parameter occurrence.
    /// The root-only identity-slice proof deliberately remains unchanged.
    fn copy_family_component_matches(
        &mut self,
        parameter: ir::ParamId,
        original: &CarrierLocation,
        expanded: &CarrierLocation,
        mapping: &ir::CarrierComponentMapping,
        view: &super::traits::CarrierContractView,
    ) -> Result<bool, Exhausted> {
        self.budget.spend(1)?;
        if mapping.original_family != ir::CarrierFamily::Parameter(parameter)
            || original.channel
                != (CarrierChannel::ParamTail {
                    parameter,
                    prefix: 0,
                })
        {
            return Ok(false);
        }
        let Some((_, actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)? else {
            return Ok(false);
        };
        let Some(locations) = self.metadata_value(ir::carrier_locations(actual, &self.ledger))?
        else {
            return Ok(false);
        };
        for relative in locations.iter() {
            self.budget
                .spend(relative.path.0.len() + mapping.original.path.0.len() + 1)?;
            if *relative == mapping.substituted
                && mapping.original.path == relative.path
                && mapping.original.channel
                    == (CarrierChannel::ParamTail {
                        parameter,
                        prefix: relative.channel.prefix(),
                    })
            {
                return Ok(
                    self.compose_component(original, relative, view.nominal_mappings())?
                        == *expanded,
                );
            }
        }
        Ok(false)
    }
    /// Admit only transport of a nonempty, complete set of Copy shared-reference
    /// carriers from an exact original by-value input. The owned actual itself
    /// is never declared Copy, and ordinary move/loan/effect checks still run.
    fn owned_shared_family_write_admitted(
        &mut self,
        params: &[Ty],
        parameter: ir::ParamId,
        view: &super::traits::CarrierContractView,
        authority: Option<&super::traits::ValidatedFacts>,
    ) -> Result<bool, Exhausted> {
        if !authority
            .or(self.authority.as_deref())
            .or(match self.facts {
                FactsCtx::Validated(facts) => Some(facts),
                _ => None,
            })
            .is_some_and(|facts| facts.phase() == super::traits::FactsPhase::Closed)
        {
            return Ok(false);
        }
        let Some(owned) = owned_shared_family_write_source(
            view.original_contract(),
            view.origin_schema(),
            parameter,
            self.budget,
        )?
        else {
            return Ok(false);
        };
        let Some((_, actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)? else {
            return Ok(false);
        };
        if params.get(owned.param) != Some(actual) {
            return Ok(false);
        }
        let Some(locations) = self.metadata_value(ir::carrier_locations(actual, &self.ledger))?
        else {
            return Ok(false);
        };
        if locations.is_empty() {
            return Ok(false);
        }
        let mut mapping_count = 0usize;
        for mapping in view.component_mappings() {
            self.budget.spend(1)?;
            if mapping.original_family == ir::CarrierFamily::Parameter(parameter) {
                mapping_count = add(mapping_count, 1)?;
            }
        }
        if mapping_count != locations.len() {
            return Ok(false);
        }
        for location in locations.iter() {
            self.budget.spend(view.component_mappings().len() + 1)?;
            let Some(semantic) =
                self.metadata_value(ir::carrier_at(actual, location, &self.ledger))?
            else {
                return Ok(false);
            };
            if !matches!(
                semantic,
                Ty::Ref {
                    kind: BorrowKind::Shared,
                    ..
                }
            ) || !self.copy_permission_in(&semantic, authority)?
            {
                return Ok(false);
            }
            let mut matched = false;
            for mapping in view.component_mappings() {
                if mapping.original_family == ir::CarrierFamily::Parameter(parameter)
                    && mapping.substituted == *location
                    && self.copy_family_component_matches(
                        parameter,
                        &owned.location,
                        location,
                        mapping,
                        view,
                    )?
                {
                    if matched {
                        return Ok(false);
                    }
                    matched = true;
                }
            }
            if !matched {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// Copy-only family writes do not transport affine authority. This proof
    /// replays every original write/source and the one returned mutable loan;
    /// it does not alter target, lifetime, alias or callee-envelope checks.
    fn copy_family_reborrow_admitted(
        &mut self,
        params: &[Ty],
        result: &Ty,
        view: &super::traits::CarrierContractView,
        authority: Option<&super::traits::ValidatedFacts>,
    ) -> Result<bool, Exhausted> {
        let facts = authority
            .or(self.authority.as_deref())
            .or(match self.facts {
                FactsCtx::Validated(facts) => Some(facts),
                _ => None,
            });
        if !facts.is_some_and(|facts| facts.phase() == super::traits::FactsPhase::Closed) {
            return Ok(false);
        }
        let Some(families) = copy_family_effect_rows(view.original_contract(), self.budget)? else {
            return Ok(false);
        };
        for parameter in families {
            let Some((_, actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)? else {
                return Ok(false);
            };
            if !self.copy_permission_in(actual, authority)?
                && !self.owned_shared_family_write_admitted(params, parameter, view, authority)?
            {
                return Ok(false);
            }
        }
        let mut affine_result = false;
        for row in view.contract().results.iter() {
            self.budget.spend(
                row.sources.len()
                    + view.contract().inputs.len()
                    + view.original_contract().results.len()
                    + 1,
            )?;
            let Some(semantic) =
                self.metadata_value(ir::carrier_at(result, &row.location, &self.ledger))?
            else {
                return Ok(false);
            };
            if self.copy_permission_in(&semantic, authority)? {
                continue;
            }
            if affine_result
                || !matches!(
                    semantic,
                    Ty::Ref {
                        kind: BorrowKind::Mut,
                        ..
                    }
                )
                || row.location.channel != CarrierChannel::Fixed(ReferencePosition { depth: 0 })
                || !row.location.path.0.iter().all(|step| {
                    matches!(
                        step,
                        CarrierStep::Field(_) | CarrierStep::VariantField { .. }
                    )
                })
                || row.sources.len() != 1
            {
                return Ok(false);
            }
            let input = &row.sources[0];
            if input.location.channel != CarrierChannel::Fixed(ReferencePosition { depth: 0 })
                || !input
                    .location
                    .path
                    .0
                    .iter()
                    .all(|step| matches!(step, CarrierStep::Field(_)))
                || !view.contract().inputs.contains(input)
            {
                return Ok(false);
            }
            let Some(parameter) = params.get(input.param) else {
                return Ok(false);
            };
            let Some(source) =
                self.metadata_value(ir::carrier_at(parameter, &input.location, &self.ledger))?
            else {
                return Ok(false);
            };
            if !matches!(
                source,
                Ty::Ref {
                    kind: BorrowKind::Mut,
                    ..
                }
            ) {
                return Ok(false);
            }
            if !copy_family_fixed_result_rows(
                view.original_contract(),
                row,
                view.nominal_mappings(),
                self.budget,
            )? {
                return Ok(false);
            }
            affine_result = true;
        }
        if !affine_result || view.contract().writes.len() != 1 {
            return Ok(false);
        }
        let mut destinations = Vec::new();
        for write in view.original_contract().writes.iter() {
            let CarrierChannel::ParamTail {
                parameter,
                prefix: 0,
            } = write.destination.location.channel
            else {
                return Ok(false);
            };
            for (destination, component) in
                self.transport_effect_input(&write.destination, view, true)?
            {
                self.budget
                    .spend(destinations.len() + view.contract().writes.len() + 1)?;
                if destinations.contains(&destination) {
                    return Ok(false);
                }
                let Some(expanded) = view
                    .contract()
                    .writes
                    .iter()
                    .find(|row| row.destination == destination)
                else {
                    return Ok(false);
                };
                let mut mapped = false;
                for mapping in view.component_mappings() {
                    mapped |= component.as_ref() == Some(&mapping.original)
                        && self.copy_family_component_matches(
                            parameter,
                            &write.destination.location,
                            &destination.location,
                            mapping,
                            view,
                        )?;
                }
                if !mapped {
                    return Ok(false);
                }
                let Some(input) = params.get(destination.param) else {
                    return Ok(false);
                };
                let Some(semantic) = self.metadata_value(ir::carrier_at(
                    input,
                    &destination.location,
                    &self.ledger,
                ))?
                else {
                    return Ok(false);
                };
                if !self.copy_permission_in(&semantic, authority)? {
                    return Ok(false);
                }
                let mut sources = Vec::new();
                for source in write.sources.iter() {
                    for (source, source_component) in
                        self.transport_effect_input(source, view, false)?
                    {
                        if component.is_some()
                            && source_component.is_some()
                            && component != source_component
                        {
                            continue;
                        }
                        self.budget
                            .spend(sources.len() + expanded.sources.len() + 1)?;
                        if sources.contains(&source) || !expanded.sources.contains(&source) {
                            return Ok(false);
                        }
                        let Some(input) = params.get(source.param) else {
                            return Ok(false);
                        };
                        let Some(semantic) = self.metadata_value(ir::carrier_at(
                            input,
                            &source.location,
                            &self.ledger,
                        ))?
                        else {
                            return Ok(false);
                        };
                        if !self.copy_permission_in(&semantic, authority)? {
                            return Ok(false);
                        }
                        self.retain(
                            size_of::<InputCarrier>()
                                + mul(source.location.path.0.len(), size_of::<CarrierStep>())?,
                            1,
                        )?;
                        checked_vec_growth(&mut sources, self.budget)?;
                        sources.push(source);
                    }
                }
                if sources.len() != expanded.sources.len() {
                    return Ok(false);
                }
                self.retain(
                    size_of::<InputCarrier>()
                        + mul(destination.location.path.0.len(), size_of::<CarrierStep>())?,
                    1,
                )?;
                checked_vec_growth(&mut destinations, self.budget)?;
                destinations.push(destination);
            }
        }
        Ok(destinations.len() == view.contract().writes.len())
    }
    fn affine_contract_admitted(
        &mut self,
        params: &[Ty],
        result: &Ty,
        view: &super::traits::CarrierContractView,
        authority: Option<&super::traits::ValidatedFacts>,
    ) -> Result<bool, Exhausted> {
        let mut affine_results = 0usize;
        let mut result_channels = 0usize;
        let Some(locations) = self.metadata_value(ir::carrier_locations(result, &self.ledger))?
        else {
            return Ok(false);
        };
        for location in locations.iter() {
            if value_projection(location).contains(&Projection::Deref) {
                continue;
            }
            result_channels = add(result_channels, 1)?;
            let Some(semantic) =
                self.metadata_value(ir::carrier_at(result, location, &self.ledger))?
            else {
                return Ok(false);
            };
            if !self.copy_permission_in(&semantic, authority)? {
                affine_results = add(affine_results, 1)?;
            }
        }
        let mut affine_writes = 0usize;
        let mut write_channels = 0usize;
        for write in view.original_contract().writes.iter() {
            // An identity row (the carrier's only source is itself) leaves the
            // carrier unchanged: it moves and duplicates no affine authority.
            let identity = write.sources.len() == 1 && write.sources[0] == write.destination;
            for (destination, _) in self.transport_effect_input(&write.destination, view, true)? {
                write_channels = add(write_channels, 1)?;
                if identity {
                    continue;
                }
                let Some(parameter) = params.get(destination.param) else {
                    self.problem(
                        self.f.entry,
                        None,
                        Problem::Invalid(
                            "carrier write parameter is outside its canonical signature".into(),
                        ),
                    )?;
                    return Ok(false);
                };
                // The transported path names nominal owners through the view's
                // own instantiated signature (its nominal mappings target that
                // signature's applications). A graph-realized caller signature
                // describes the same call with different application ids, so
                // fall back to the view's signature for the same parameter.
                let semantic = match ir::carrier_at(parameter, &destination.location, &self.ledger)
                {
                    Ok(ty) => Ok(ty),
                    Err(error) => match view.signature_for_replay().params.get(destination.param) {
                        Some(own) => ir::carrier_at(own, &destination.location, &self.ledger),
                        None => Err(error),
                    },
                };
                let Some(semantic) = self.metadata_value(semantic)? else {
                    return Ok(false);
                };
                if !self.copy_permission_in(&semantic, authority)? {
                    affine_writes = add(affine_writes, 1)?;
                }
            }
        }
        // A flat pair is not a partition of one loan. Each component must
        // retain its own, distinct declaration input. Ordinary call/body loan
        // checks still reject overlapping actuals and invalid returned origins.
        let mut independent_pair = affine_results == 2
            && result_channels == 2
            && write_channels == 0
            && matches!(result, Ty::Tuple(fields) if fields.len() == 2
                && fields.iter().all(|field| field.mut_slice_element().is_some()));
        let mut original_roots = None;
        for contract in [view.original_contract(), view.contract()] {
            if !independent_pair {
                break;
            }
            let Some((roots, families)) = independent_slice_pair_rows(contract, self.budget)?
            else {
                independent_pair = false;
                break;
            };
            if original_roots.is_some_and(|original| original != roots) {
                independent_pair = false;
                break;
            }
            original_roots = Some(roots);
            for input in roots {
                self.budget.spend(1)?;
                if params.get(input).and_then(Ty::mut_slice_element).is_none() {
                    independent_pair = false;
                    break;
                }
            }
            for parameter in families.into_iter().flatten() {
                self.budget.spend(1)?;
                let Some((_, actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)?
                else {
                    independent_pair = false;
                    break;
                };
                let Some(facts) = self.authority.as_deref().or(authority) else {
                    independent_pair = false;
                    break;
                };
                if facts.phase() != super::traits::FactsPhase::Closed {
                    independent_pair = false;
                    break;
                }
                let validation =
                    FactsCtx::from_validated(facts).validate_ty_with_budget(actual, &self.ledger);
                if self.metadata_value(validation)?.is_none() {
                    independent_pair = false;
                    break;
                }
                let Some(locations) =
                    self.metadata_value(ir::carrier_locations(actual, &self.ledger))?
                else {
                    independent_pair = false;
                    break;
                };
                if !locations.is_empty() {
                    independent_pair = false;
                    break;
                }
            }
        }
        let bounded = (affine_results == 0 && affine_writes == 0)
            || (affine_results <= 1 && write_channels == 0)
            || (affine_writes <= 1 && result_channels == 0)
            || independent_pair;
        if bounded
            || self.linear_owned_family_result_admitted(result, view, authority)?
            || (result_channels == 0 && self.indexed_family_write_admitted(params, view)?)
        {
            return Ok(true);
        }
        if affine_results == 1
            && affine_writes == 0
            && write_channels == 1
            && result_channels <= 2
            && self.copy_family_reborrow_admitted(params, result, view, authority)?
        {
            return Ok(true);
        }
        Ok(affine_results == 1
            && result_channels == 1
            && affine_writes == 0
            && (self.identity_copy_reborrow_admitted(params, result, view, authority)?
                || self.identity_nested_slot_reborrow_admitted(params, result, view, authority)?))
    }
    fn audit_call_admission(&mut self) -> Result<(), Exhausted> {
        for index in 0..self.f.blocks.len() {
            let Terminator::Call {
                target, signature, ..
            } = &self.f.blocks[index].terminator.value
            else {
                continue;
            };
            let actuals = match target {
                ir::Callee::Direct(target) => target.type_args.as_ref(),
                ir::Callee::TraitItem(target)
                | ir::Callee::Virtual(ir::VirtualCall { call: target })
                | ir::Callee::Untupled(target) => target.method_args.as_ref(),
            };
            self.budget.spend(actuals.len())?;
            if !actuals.iter().any(|ty| matches!(ty, Ty::Ref { .. })) {
                continue;
            }
            self.retain(
                mul(signature.params.len(), size_of::<Ty>())?,
                signature.params.len(),
            )?;
            let signature = signature.clone();
            let Some(view) = self.calls[index].clone() else {
                if self.authority.is_some() {
                    self.problem(
                        BlockId(index),
                        None,
                        Problem::Invalid(
                            "validated reference-actual call has no canonical carrier contract"
                                .into(),
                        ),
                    )?;
                }
                continue;
            };
            if !self.affine_contract_admitted(&signature.params, &signature.result, &view, None)? {
                self.problem(
                    BlockId(index),
                    None,
                    Problem::Unsupported(
                        "call expands to out-of-domain affine carrier return/write flow".into(),
                    ),
                )?;
            }
        }
        // Capsules retain the original arena, including expressions that never
        // receive a CFG point. This audit grants no ENTRY or loan permission.
        if let Some(function) = self.function.clone() {
            // Identity set of audited sources replaces an O(n^2) prefix scan.
            let mut audited_sources = std::collections::HashSet::new();
            audited_sources
                .try_reserve(function.call_evidence.len())
                .map_err(|_| Exhausted)?;
            for capsule_index in 0..function.call_evidence.len() {
                let capsule = &function.call_evidence[capsule_index];
                self.budget.spend(1)?;
                if !audited_sources.insert(Arc::as_ptr(capsule.source())) {
                    continue;
                }
                let source = capsule.source();
                let facts = capsule.facts();
                for expression in &source.exprs {
                    self.budget.spend(1)?;
                    let ir::ExprKind::Call {
                        target, signature, ..
                    } = &expression.kind
                    else {
                        continue;
                    };
                    let actuals = match target {
                        ir::Callee::Direct(target) => target.type_args.as_ref(),
                        ir::Callee::TraitItem(target)
                        | ir::Callee::Virtual(ir::VirtualCall { call: target })
                        | ir::Callee::Untupled(target) => target.method_args.as_ref(),
                    };
                    self.budget.spend(actuals.len())?;
                    if !actuals.iter().any(|ty| matches!(ty, Ty::Ref { .. })) {
                        continue;
                    }
                    let view = match facts.carrier_contract(ir::SignatureSubject::Call(target)) {
                        Ok(view) => view,
                        Err(error) if error.is_resource() => return Err(Exhausted),
                        Err(error) => {
                            let unsupported =
                                error.kind() == super::traits::FactsErrorKind::Unsupported;
                            let message =
                                error.into_diagnostic(&self.ledger).map_err(|_| Exhausted)?;
                            self.problem(
                                self.f.entry,
                                None,
                                if unsupported {
                                    Problem::Unsupported(message)
                                } else {
                                    Problem::Invalid(message)
                                },
                            )?;
                            continue;
                        }
                    };
                    if !self.affine_contract_admitted(
                        &signature.params,
                        &signature.result,
                        &view,
                        Some(facts),
                    )? {
                        self.problem(self.f.entry, None, Problem::Unsupported("original call expands to out-of-domain affine carrier return/write flow".into()))?;
                    }
                }
            }
        }
        Ok(())
    }
    fn affine_admission(&mut self) -> Result<bool, Exhausted> {
        // Universal relations transport whole opaque families linearly. They
        // do not assert how many affine components an actual will expose.
        // Every closed realization reruns this exact expanded-value gate.
        if self
            .authority
            .as_ref()
            .is_some_and(|facts| facts.phase() == super::traits::FactsPhase::Universal)
        {
            return Ok(true);
        }
        let Some(body) = self.body.clone() else {
            return Ok(false);
        };
        self.retain(
            mul(self.f.params.len(), size_of::<Ty>())?,
            self.f.params.len(),
        )?;
        let params: Vec<_> = self
            .f
            .params
            .iter()
            .map(|local| self.f.locals[local.0].ty.clone())
            .collect();
        let result = self.f.result.clone();
        let admitted = self.affine_contract_admitted(&params, &result, &body, None)?;
        if !admitted
            && self.authority.as_ref().is_some_and(|facts| {
                facts.phase() == super::traits::FactsPhase::Closed
                    && facts.target().is_some_and(|target| {
                        target
                            .type_args
                            .iter()
                            .any(|ty| matches!(ty, Ty::Ref { .. }))
                    })
            })
        {
            self.problem(
                BlockId(0),
                None,
                Problem::Unsupported(
                    "expanded affine carrier flow exceeds the bounded transport/write domain"
                        .into(),
                ),
            )?;
        }
        Ok(admitted)
    }
    /// The shape-only affine gate cannot certify a partition of one input.
    /// A closed pair can instead be certified from its actual output flow:
    /// no nested carriers, no write envelope, and distinct live mutable origins
    /// at every return. Ordinary ownership checks still validate each issuance
    /// and reject duplication, overlapping reborrows and escaping local data.
    fn closed_pair_result_admitted(
        &mut self,
        body: &super::traits::CarrierContractView,
        outgoing: &[Option<Point>],
    ) -> Result<bool, Exhausted> {
        if !self
            .authority
            .as_ref()
            .is_some_and(|facts| facts.phase() == super::traits::FactsPhase::Closed)
            || !body.contract().writes.is_empty()
        {
            return Ok(false);
        }
        let Ty::Tuple(fields) = self.f.result.clone() else {
            return Ok(false);
        };
        if fields.len() != 2 {
            return Ok(false);
        }
        for field in fields.iter() {
            let Some(element) = field.mut_slice_element() else {
                return Ok(false);
            };
            let Some(carriers) =
                self.metadata_value(ir::carrier_locations(element, &self.ledger))?
            else {
                return Ok(false);
            };
            if !carriers.is_empty() {
                return Ok(false);
            }
        }
        self.budget.reserve_bytes(2 * size_of::<Projection>())?;
        let mut cells = [0; 2];
        for (index, cell) in cells.iter_mut().enumerate() {
            self.budget.spend(1)?;
            let key = CellKey {
                storage: CanonicalPlace {
                    root: CanonicalRoot::Owned(self.f.return_local),
                    projection: vec![Projection::Field(index)],
                },
                channel: CellChannel::ReferenceValue,
            };
            let Some(found) = self.cell_index.get(&key) else {
                return Ok(false);
            };
            *cell = *found;
        }
        let mut returned = false;
        for (block, point) in outgoing.iter().enumerate() {
            self.budget.spend(1)?;
            if !matches!(self.f.blocks[block].terminator.value, Terminator::Return) {
                continue;
            }
            let Some(point) = point else { continue };
            returned = true;
            let [left, right] = cells;
            self.budget.spend(
                point.origins[left].0.len()
                    + point.origins[right].0.len()
                    + point.families[left].0.0.len()
                    + point.families[right].0.0.len(),
            )?;
            if point.origins[left].count() == 0
                || point.origins[right].count() == 0
                || point.families[left].0.count() != 0
                || point.families[right].0.count() != 0
            {
                return Ok(false);
            }
            for origin in point.origins[left].members() {
                self.budget.spend(1)?;
                if self.kinds[origin] != BorrowKind::Mut || point.origins[right].has(origin) {
                    return Ok(false);
                }
            }
            for origin in point.origins[right].members() {
                self.budget.spend(1)?;
                if self.kinds[origin] != BorrowKind::Mut {
                    return Ok(false);
                }
            }
        }
        Ok(returned)
    }

    fn solve(mut self) -> Result<Plan, Exhausted> {
        self.prepare_singleton_heap_slot()?;
        let family_admitted = self.affine_admission()?;
        self.audit_call_admission()?;
        let f = self.f;
        let blocks = f.blocks.len();
        let slots = self.cells.len();
        let references = self.targets.len();
        let families = self.family_inputs.len();
        let mut offsets = vec![0usize];
        for block in &f.blocks {
            offsets.push(add(
                *offsets.last().ok_or(Exhausted)?,
                block.statements.len() + 1,
            )?);
        }
        let points_count = *offsets.last().ok_or(Exhausted)?;
        // Include the symbolic universe, cells, entry snapshots, dependency
        // relation and old/new scratch, even with zero concrete loan origins.
        let storage = plan_storage_bytes(
            f.locals.len(),
            blocks,
            points_count,
            slots,
            add(references, families)?,
        )?;
        self.retain(
            storage,
            plan_initial_work(blocks, points_count, slots, add(references, families)?)?,
        )?;
        let mut reachable = vec![false; blocks];
        let mut entries: Vec<Option<FlowState>> = vec![None; blocks];
        entries[f.entry.0] = Some(self.initial.clone());
        let mut before: Vec<Option<FlowState>> = vec![None; points_count];
        let mut after: Vec<Option<FlowState>> = vec![None; blocks];
        loop {
            let old_targets: usize = self.targets.iter().map(Vec::len).sum();
            let mut queue: VecDeque<_> = entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| entry.as_ref().map(|_| BlockId(index)))
                .collect();
            let mut queued: Vec<_> = entries.iter().map(Option::is_some).collect();
            while let Some(block) = queue.pop_front() {
                queued[block.0] = false;
                reachable[block.0] = true;
                self.budget
                    .spend(slots * (references.div_ceil(64) + families.div_ceil(64)) + 1)?;
                let mut state = entries[block.0].as_ref().ok_or(Exhausted)?.clone();
                for index in 0..f.blocks[block.0].statements.len() {
                    before[offsets[block.0] + index] = Some(state.clone());
                    self.step(&mut state, block, index, false)?;
                }
                before[offsets[block.0 + 1] - 1] = Some(state.clone());
                self.call(&mut state, block, false)?;
                after[block.0] = Some(state.clone());
                for next in successors(&f.blocks[block.0].terminator.value)
                    .into_iter()
                    .flatten()
                {
                    let changed = if let Some(entry) = &mut entries[next.0] {
                        entry.union(&state, self.budget)?
                    } else {
                        entries[next.0] = Some(state.clone());
                        true
                    };
                    if changed && !queued[next.0] {
                        queue.push_back(next);
                        queued[next.0] = true;
                    }
                }
            }
            if old_targets == self.targets.iter().map(Vec::len).sum::<usize>() {
                break;
            }
        }
        let mut uses = vec![Set::new(slots); points_count];
        let mut defs = vec![Set::new(slots); points_count];
        for block in 0..blocks {
            if !reachable[block] {
                continue;
            }
            for index in 0..=f.blocks[block].statements.len() {
                let state = before[offsets[block] + index].as_ref().ok_or(Exhausted)?;
                let (used, defined) = self.uses(state, BlockId(block), index)?;
                uses[offsets[block] + index] = used;
                defs[offsets[block] + index] = defined;
            }
        }
        let mut ins = vec![Set::new(slots); blocks];
        let mut exits = vec![Set::new(slots); blocks];
        loop {
            let mut changed = false;
            for block in (0..blocks).filter(|block| reachable[*block]).rev() {
                let mut out = Set::new(slots);
                for next in successors(&f.blocks[block].terminator.value)
                    .into_iter()
                    .flatten()
                {
                    out.union(&ins[next.0], self.budget)?;
                }
                exits[block] = out.clone();
                for point in (offsets[block]..offsets[block + 1]).rev() {
                    self.budget.spend(out.0.len())?;
                    for ((word, defined), used) in
                        out.0.iter_mut().zip(&defs[point].0).zip(&uses[point].0)
                    {
                        *word = (*word & !defined) | used;
                    }
                }
                if out != ins[block] {
                    ins[block] = out;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut points: Vec<Option<Point>> = (0..points_count).map(|_| None).collect();
        let mut outgoing: Vec<Option<Point>> = (0..blocks).map(|_| None).collect();
        let mut final_overwrites: Option<Set> = None;
        for block in 0..blocks {
            if !reachable[block] {
                continue;
            }
            let mut live = exits[block].clone();
            for point in (offsets[block]..offsets[block + 1]).rev() {
                for ((word, defined), used) in
                    live.0.iter_mut().zip(&defs[point].0).zip(&uses[point].0)
                {
                    *word = (*word & !defined) | used;
                }
                let state = before[point].as_ref().ok_or(Exhausted)?;
                self.membership(state.cells.iter().map(CarrierOrigins::count).sum())?;
                points[point] = Some(Point {
                    origins: state
                        .cells
                        .iter()
                        .map(|value| value.references.0.clone())
                        .collect(),
                    families: state
                        .cells
                        .iter()
                        .map(|value| value.families.clone())
                        .collect(),
                    live: live.clone(),
                    activated: Set::new(references),
                });
            }
            self.diagnosing = true;
            let mut state = entries[block].as_ref().ok_or(Exhausted)?.clone();
            for index in 0..f.blocks[block].statements.len() {
                self.step(&mut state, BlockId(block), index, true)?;
            }
            self.call(&mut state, BlockId(block), true)?;
            if matches!(f.blocks[block].terminator.value, Terminator::Return) {
                if let Some(must) = &mut final_overwrites {
                    must.intersect(&state.overwritten, self.budget)?;
                } else {
                    final_overwrites = Some(state.overwritten.clone());
                }
            }
            outgoing[block] = Some(Point {
                origins: state
                    .cells
                    .iter()
                    .map(|value| value.references.0.clone())
                    .collect(),
                families: state
                    .cells
                    .iter()
                    .map(|value| value.families.clone())
                    .collect(),
                live: exits[block].clone(),
                activated: Set::new(references),
            });
        }
        let mut two_phase = Set::new(references);
        let mut activations = vec![vec![]; blocks];
        for (index, site) in f.loan_sites.iter().enumerate() {
            if let cfg::LoanTiming::TwoPhase { activation, .. } = site.timing {
                two_phase.insert(index);
                activations[activation.0].push(LoanId(index));
            }
        }
        let effects = if let Some(body) = self.body.clone() {
            let mut writes = Vec::new();
            let mut must_overwrite = Vec::new();
            let mut exact_identity_cells = true;
            self.retain(
                mul(
                    body.contract().writes.len(),
                    size_of::<ir::CarrierWriteEnvelope>() + size_of::<InputCarrier>(),
                )?,
                body.contract().writes.len(),
            )?;
            for envelope in body.contract().writes.iter() {
                let mut sources = Vec::new();
                let mut matching = Vec::new();
                self.budget.spend(self.cells.len())?;
                for (index, cell) in self.cells.iter().enumerate() {
                    if cell.input.as_ref() == Some(&envelope.destination) {
                        checked_vec_growth(&mut matching, self.budget)?;
                        matching.push(index);
                    }
                }
                exact_identity_cells &= matching.len() == 1;
                let mut definite = !matching.is_empty() && final_overwrites.is_some();
                for index in matching {
                    definite &= final_overwrites
                        .as_ref()
                        .is_some_and(|must| must.has(index));
                    for (block, exit) in outgoing.iter().enumerate() {
                        if !matches!(f.blocks[block].terminator.value, Terminator::Return) {
                            continue;
                        }
                        let Some(exit) = exit else {
                            continue;
                        };
                        self.budget
                            .spend(exit.families[index].0.count() + exit.origins[index].count())?;
                        for family in exit.families[index].0.members() {
                            let input = self.family_inputs[family].input.clone();
                            self.budget.spend(sources.len() + 1)?;
                            if !sources.contains(&input) {
                                self.retain(size_of::<InputCarrier>(), 1)?;
                                checked_vec_growth(&mut sources, self.budget)?;
                                sources.push(input);
                            }
                        }
                        for origin in exit.origins[index].members() {
                            self.budget.spend(self.targets[origin].len())?;
                            for target in &self.targets[origin] {
                                let CanonicalRoot::ParameterReferent(local, location) =
                                    &target.place.root
                                else {
                                    continue;
                                };
                                let Some(param) =
                                    f.params.iter().position(|candidate| candidate == local)
                                else {
                                    continue;
                                };
                                let input = InputCarrier {
                                    param,
                                    location: location.clone(),
                                };
                                self.budget.spend(sources.len() + 1)?;
                                if !sources.contains(&input) {
                                    self.budget.reserve_bytes(size_of::<InputCarrier>())?;
                                    checked_vec_growth(&mut sources, self.budget)?;
                                    sources.push(input);
                                }
                            }
                        }
                    }
                }
                self.membership(sources.len())?;
                checked_vec_growth(&mut writes, self.budget)?;
                checked_arc_transfer::<InputCarrier>(sources.len(), self.budget)?;
                writes.push(ir::CarrierWriteEnvelope {
                    destination: envelope.destination.clone(),
                    sources: sources.into(),
                });
                if definite {
                    checked_vec_growth(&mut must_overwrite, self.budget)?;
                    must_overwrite.push(envelope.destination.clone());
                }
            }
            self.retain(
                mul(self.bindings.len(), size_of::<CallCarrierBinding>())?,
                self.bindings.len(),
            )?;
            let mut invalid_binding = !ordered_call_bindings(&self.bindings, self.budget)?;
            for binding in &self.bindings {
                self.budget.spend(
                    binding.entry.len() + binding.destinations.len() + binding.regions.len() + 1,
                )?;
                invalid_binding |= !call_binding_subject_matches(
                    binding,
                    self.f,
                    self.function.as_ref(),
                    self.authority.as_ref(),
                );
                // `new` authenticated these immutable views, and the subject
                // check above retains the exact function and facts pair.
                // A binding must carry that same per-block Arc, not merely an
                // equivalent contract that requires authority replay.
                invalid_binding |= !call_binding_view_matches(binding, &self.calls);
                for entry in &binding.entry {
                    self.budget
                        .spend(binding.contract.contract().inputs.len() + entry.origins.count())?;
                    invalid_binding |= !binding.contract.contract().inputs.contains(&entry.input)
                        || entry
                            .origins
                            .references
                            .0
                            .members()
                            .any(|origin| origin >= references)
                        || entry
                            .origins
                            .families
                            .0
                            .members()
                            .any(|origin| origin >= families);
                }
                for destination in &binding.destinations {
                    self.budget.spend(
                        binding.contract.contract().writes.len()
                            + destination.targets.len()
                            + destination.old.len(),
                    )?;
                    invalid_binding |= !binding
                        .contract
                        .contract()
                        .writes
                        .iter()
                        .any(|write| write.destination == destination.destination);
                    for (cell, old) in &destination.old {
                        invalid_binding |= !self.cell_index.contains_key(cell)
                            || old
                                .references
                                .0
                                .members()
                                .any(|origin| origin >= references)
                            || old.families.0.members().any(|origin| origin >= families);
                    }
                }
                for region in &binding.regions {
                    invalid_binding |= region.region.0 >= binding.relation.keys.len()
                        || (region.component.is_some()
                            && !matches!(
                                region.schema,
                                ir::CarrierRegionTerm::Family { .. }
                                    | ir::CarrierRegionTerm::ProjectionFamily { .. }
                                    | ir::CarrierRegionTerm::ArrayFamily { .. }
                                    | ir::CarrierRegionTerm::OpaqueFamily { .. }
                            ));
                }
            }
            if invalid_binding {
                self.problem(f.entry, None, Problem::Invalid("retained carrier occurrence changed source, snapshot, region or authority identity".into()))?;
            }
            let identity_admitted = if !family_admitted
                && !invalid_binding
                && exact_identity_cells
                && self.problems.is_empty()
                && self
                    .authority
                    .as_ref()
                    .is_some_and(|facts| facts.phase() == super::traits::FactsPhase::Closed)
            {
                let result = self.f.result.clone();
                let result_empty = self
                    .metadata_value(ir::carrier_locations(&result, &self.ledger))?
                    .is_some_and(|locations| locations.is_empty());
                result_empty
                    && identity_effect_rows(body.contract(), &writes, &must_overwrite, self.budget)?
            } else {
                false
            };
            let pair_admitted = !family_admitted
                && !invalid_binding
                && self.problems.is_empty()
                && self.closed_pair_result_admitted(&body, &outgoing)?;
            // Raw-pointer stores currently have no physical SAME cell. An
            // empty computed row is not evidence that such a store preserves
            // the entry value. Keep the existing declaration-envelope fallback
            // until the exact raw storage object is modeled. No must-overwrite
            // or replacement summary may be published from this incomplete view.
            let mut unmodeled_raw_write = false;
            for (block_index, block) in f.blocks.iter().enumerate() {
                self.budget.spend(block.statements.len() + 1)?;
                for (statement_index, statement) in block.statements.iter().enumerate() {
                    if let Statement::Assign { destination, .. } = &statement.value {
                        self.budget.spend(self.native_storage.len() + 1)?;
                        let modeled = self.native_storage.iter().any(|transfer| {
                            transfer.block == BlockId(block_index)
                                && transfer.statement == statement_index
                        });
                        self.budget.spend(
                            self.native_exchanges
                                .len()
                                .saturating_mul(2)
                                .saturating_add(1),
                        )?;
                        let modeled = modeled
                            || self.native_exchanges.iter().any(|exchange| {
                                exchange.writes(BlockId(block_index), statement_index)
                            });
                        unmodeled_raw_write |= !modeled
                            && ir::raw_deref_index_optional(
                                &f.locals[destination.local.0].ty,
                                &destination.projection,
                                Some(&self.ledger),
                            )
                            .map_err(|_| Exhausted)?
                            .is_some();
                    }
                }
            }
            let aggregate_admitted = !family_admitted
                && !identity_admitted
                && !pair_admitted
                && self.problems.is_empty()
                && !unmodeled_raw_write
                && self.closed_shared_aggregate_reborrow_admitted(
                    &body,
                    &outgoing,
                    &points,
                    &offsets,
                    &writes,
                    &must_overwrite,
                )?;
            let finite_result_admitted = !family_admitted
                && !identity_admitted
                && !pair_admitted
                && !aggregate_admitted
                && !invalid_binding
                && writes.is_empty()
                && must_overwrite.is_empty()
                && !unmodeled_raw_write
                && self.problems.is_empty()
                && self.closed_aggregate_result_admitted(&body, &outgoing)?;
            if (family_admitted
                || identity_admitted
                || pair_admitted
                || aggregate_admitted
                || finite_result_admitted)
                && self.problems.is_empty()
                && !unmodeled_raw_write
            {
                checked_arc_transfer::<ir::CarrierWriteEnvelope>(writes.len(), self.budget)?;
                checked_arc_transfer::<InputCarrier>(must_overwrite.len(), self.budget)?;
                checked_arc_transfer::<CallCarrierBinding>(self.bindings.len(), self.budget)?;
                self.retain(size_of::<CarrierEffects>() + size_of::<[usize; 8]>(), 1)?;
                // Keep the escaping rows and nested owned allocations in the
                // SAME ledger after temporary analysis reservations are dropped.
                retain_carrier_effects_payload(
                    &writes,
                    &must_overwrite,
                    &self.bindings,
                    self.budget,
                    &self.ledger,
                )?;
                checked_arc_transfer::<NativeStorageTransfer>(
                    self.native_storage.len(),
                    self.budget,
                )?;
                let mut native_bytes = self
                    .native_storage
                    .len()
                    .checked_mul(size_of::<NativeStorageTransfer>())
                    .and_then(|bytes| bytes.checked_add(2 * size_of::<usize>()))
                    .ok_or(Exhausted)?;
                for transfer in &self.native_storage {
                    native_bytes = add(native_bytes, transfer.retained_paths(self.budget)?)?;
                }
                self.ledger
                    .retain_bytes(native_bytes)
                    .map_err(|_| Exhausted)?;
                checked_arc_transfer::<NativeStorageExchange>(
                    self.native_exchanges.len(),
                    self.budget,
                )?;
                self.ledger
                    .retain_bytes(add(
                        mul(
                            self.native_exchanges.len(),
                            size_of::<NativeStorageExchange>(),
                        )?,
                        2 * size_of::<usize>(),
                    )?)
                    .map_err(|_| Exhausted)?;
                Some(Arc::new(CarrierEffects {
                    _native_exchanges: std::mem::take(&mut self.native_exchanges).into(),
                    _native_storage: std::mem::take(&mut self.native_storage).into(),
                    writes: writes.into(),
                    must_overwrite: must_overwrite.into(),
                    bindings: std::mem::take(&mut self.bindings).into(),
                }))
            } else {
                None
            }
        } else {
            None
        };
        let mut births = vec![None; references];
        // Call-result origins are issued by the call terminator of their block.
        let mut call_issuers = vec![None; references];
        for (block, body) in f.blocks.iter().enumerate() {
            for statement in &body.statements {
                if let Statement::Assign {
                    value: Rvalue::Borrow(id),
                    ..
                } = statement.value
                {
                    births[id.0] = Some(block);
                }
            }
            for (_, origin) in &self.call_rows[block] {
                births[origin.0] = Some(block);
                call_issuers[origin.0] = Some(block);
            }
        }
        let authority = if let Some(authority) = self.authority {
            Some(authority)
        } else if let Some(facts) = self.facts.validated() {
            // The compatibility entry point borrows facts instead of an Arc.
            // Preserve that same sealed authority for return consumers too.
            self.budget.reserve_bytes(add(
                size_of::<super::traits::ValidatedFacts>(),
                2 * size_of::<usize>(),
            )?)?;
            self.budget.nodes(1)?;
            Some(Arc::new(facts.clone()))
        } else {
            None
        };
        let mut plan = Plan {
            offsets,
            points,
            outgoing,
            parents: self.parents,
            lifetime_deps: self.dependencies,
            outlives: self.outlives,
            targets: self.targets,
            kinds: self.kinds,
            call_issuers,
            two_phase,
            activations,
            cells: self.cells,
            cell_index: self.cell_index,
            original_locals: self.original_locals,
            family_cover_locals: self.family_cover_locals,
            family_inputs: self.family_inputs,
            body: self.body,
            body_validation: None,
            physical_body: self.physical_body,
            body_signature: self.body_signature,
            authority,
            effects,
            problems: self.problems,
            bytes: self
                .budget
                .reserved_bytes()
                .checked_sub(self.allocation_start)
                .ok_or(Exhausted)?,
        };
        plan.compute_activation(f, &mut self.memberships, self.budget)?;
        plan.check_backedges(f, &reachable, &ins, &births, self.budget)?;
        plan.bytes = self
            .budget
            .reserved_bytes()
            .checked_sub(self.allocation_start)
            .ok_or(Exhausted)?;
        if plan.bytes > self.max_bytes {
            return Err(Exhausted);
        }
        Ok(plan)
    }
}

fn visit_rvalue_for_statement(
    f: &cfg::Function,
    statement: &Statement,
    mut visit: impl FnMut(&Place, bool),
) {
    match statement {
        Statement::Assign { value, .. } => visit_rvalue(f, value, &mut visit),
        Statement::Inspect(place) | Statement::FakeRead(place) => visit(place, false),
        Statement::IndexInspect { array, index } => {
            visit(array, false);
            visit_operand(index, &mut visit);
        }
        Statement::IndexAssign {
            array,
            index,
            value,
        }
        | Statement::IndexAssignOp {
            array,
            index,
            value,
            ..
        } => {
            visit(array, false);
            visit_operand(index, &mut visit);
            visit_operand(value, &mut visit);
        }
        Statement::StorageLive(_) | Statement::StorageDead(_) => {}
    }
}

pub(super) const MAX_LOANS: usize = 4096;
pub(super) const MAX_MEMBERSHIPS: usize = 1_000_000;

#[derive(Clone, Copy, Debug)]
pub(super) enum Phase {
    Read,
    Write,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct AccessPoint {
    pub block: BlockId,
    pub statement: Option<usize>,
    pub phase: Phase,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Access {
    CellRead,
    CellWrite,
    Read,
    Write,
    Move,
    StorageEnd,
    SharedBorrow,
    MutBorrow,
    ReserveMut,
    ActivateMut(LoanId),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    Conflict(String),
    Invalid(String),
    Unsupported(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum CanonicalRoot {
    Owned(LocalId),
    ParameterReferent(LocalId, CarrierLocation),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct CanonicalPlace {
    pub root: CanonicalRoot,
    pub projection: Vec<Projection>,
}
impl CanonicalPlace {
    pub fn owned(place: &Place) -> Self {
        Self {
            root: CanonicalRoot::Owned(place.local),
            projection: place.projection.clone(),
        }
    }
    /// Diagnostic coordinates only. External roots are never CFG stack places.
    pub fn diagnostic(&self) -> Place {
        let local = match self.root {
            CanonicalRoot::Owned(local) | CanonicalRoot::ParameterReferent(local, _) => local,
        };
        Place {
            local,
            projection: self.projection.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TargetPrecision {
    Exact,
    Subtree,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub place: CanonicalPlace,
    /// A subtree is an analysis footprint, never coordinates in the result type.
    pub precision: TargetPrecision,
    /// Only this candidate's ancestors, never a union of alternative exemptions.
    pub authority: Vec<usize>,
}

/// A returned element path with runtime index steps (`&s[i]`) is admitted
/// only when, for every runtime step, the declared contract contains the
/// path for every constant index `0..len` of that array: whatever index is
/// taken at run time, the result stays inside declared input storage.
fn dynamic_target_covered(
    f: &cfg::Function,
    local: LocalId,
    position: &ir::CarrierLocation,
    target: &[Projection],
    returns: &ReturnContract,
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    let ir::CarrierChannel::Fixed(reference) = &position.channel else {
        return Ok(false);
    };
    if !position.path.0.is_empty() {
        return Ok(false);
    }
    let Some(param) = f.params.iter().position(|p| *p == local) else {
        return Ok(false);
    };
    let mut ty = &f.locals[local.0].ty;
    for _ in 0..=reference.depth {
        let Ty::Ref { referent, .. } = ty else {
            return Ok(false);
        };
        ty = referent;
    }
    let mut ty = ty.clone();
    // Lengths of each runtime step, and the total concretization count.
    let mut lens = Vec::new();
    let mut total: usize = 1;
    for step in target {
        budget.spend(1)?;
        ty = match step {
            Projection::ManuallyDropPayload => {
                let Ty::ManuallyDrop(wrapper) = &ty else {
                    return Ok(false);
                };
                wrapper.argument().clone()
            }
            Projection::Field(index) => match ty
                .field_ty_optional(*index, budget.plan_budget())
                .map_err(|_| Exhausted)?
            {
                Some(ty) => ty,
                None => return Ok(false),
            },
            Projection::ConstantIndex(_) | Projection::DynamicIndex(_) => {
                let Ty::Array { element, len } = ty else {
                    return Ok(false);
                };
                if matches!(step, Projection::DynamicIndex(_)) {
                    lens.push(len);
                    total = match total.checked_mul(len) {
                        Some(total) if total <= 4096 => total,
                        _ => return Ok(false),
                    };
                }
                element.as_ref().clone()
            }
            _ => return Ok(false),
        };
    }
    let mut seen = std::collections::BTreeSet::new();
    for (input, projection, precision) in return_inputs(returns) {
        budget.spend(projection.len() + 1)?;
        if input.param != param || input.position != *reference {
            continue;
        }
        if precision == TargetPrecision::Subtree {
            return Ok(true);
        }
        if projection.len() != target.len() {
            continue;
        }
        let mut key = Vec::new();
        let matches = projection
            .iter()
            .zip(target)
            .all(|(declared, step)| match step {
                Projection::DynamicIndex(_) => match declared {
                    Projection::ConstantIndex(k) => {
                        key.push(*k);
                        true
                    }
                    _ => false,
                },
                other => declared == other,
            });
        if matches && key.iter().zip(&lens).all(|(k, len)| k < len) {
            seen.insert(key);
        }
    }
    Ok(seen.len() == total)
}
fn return_inputs(
    contract: &ReturnContract,
) -> impl Iterator<Item = (InputReference, &[Projection], TargetPrecision)> {
    let (paths, envelopes): (&[super::ir::InputPath], &[super::ir::InputEnvelope]) = match contract
    {
        ReturnContract::SharedInputPaths(paths) | ReturnContract::MutInputPaths(paths) => {
            (paths, &[])
        }
        ReturnContract::SharedInputEnvelopes(inputs)
        | ReturnContract::MutInputEnvelopes(inputs) => (&[], inputs),
        ReturnContract::NoReference => (&[], &[]),
    };
    paths
        .iter()
        .map(|p| (p.input, p.projection.as_ref(), TargetPrecision::Exact))
        .chain(
            envelopes
                .iter()
                .map(|p| (p.input, &[][..], TargetPrecision::Subtree)),
        )
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Set(Vec<u64>);
impl Set {
    fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }
    fn has(&self, n: usize) -> bool {
        self.0[n / 64] & (1 << (n % 64)) != 0
    }
    fn insert(&mut self, n: usize) {
        self.0[n / 64] |= 1 << (n % 64);
    }
    fn remove(&mut self, n: usize) {
        self.0[n / 64] &= !(1 << (n % 64));
    }
    fn intersect(&mut self, other: &Self, budget: &mut Budget) -> Result<(), Exhausted> {
        budget.spend(self.0.len())?;
        for (ours, theirs) in self.0.iter_mut().zip(&other.0) {
            *ours &= *theirs;
        }
        Ok(())
    }
    fn clear(&mut self) {
        self.0.fill(0);
    }
    fn count(&self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }
    fn members(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(word, bits)| {
            let mut bits = *bits;
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(word * 64 + bit)
            })
        })
    }
    fn union(&mut self, other: &Self, budget: &mut Budget) -> Result<bool, Exhausted> {
        budget.spend(self.0.len())?;
        let mut changed = false;
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            let next = *a | *b;
            changed |= next != *a;
            *a = next;
        }
        Ok(changed)
    }
}
type Origins = Vec<Set>;

struct Point {
    origins: Origins,
    families: Vec<FamilyOriginSet>,
    live: Set,
    activated: Set,
}

/// Private, immutable evidence from one successful validated Plan constructor.
/// Not a graph Fact and never returned independently of its borrow plan.
struct BodyValidationReceipt {
    function: Arc<cfg::Function>,
    facts: Arc<super::traits::ValidatedFacts>,
    view: Arc<super::traits::CarrierContractView>,
    signature: ir::Signature,
}
impl BodyValidationReceipt {
    fn payload_bytes(params: usize) -> Result<usize, Exhausted> {
        add(
            size_of::<Option<Self>>(),
            add(mul(params, size_of::<Ty>())?, 2 * size_of::<usize>())?,
        )
    }
    fn issue(
        plan: &Plan,
        validated: Option<BodyViewValidated<'_>>,
        function: &Arc<cfg::Function>,
        facts: &Arc<super::traits::ValidatedFacts>,
        budget: &mut Budget,
        max_bytes: usize,
    ) -> Result<Option<Self>, Exhausted> {
        let Some(validated) = validated else {
            return Ok(None);
        };
        let (Some(view), Some(signature), Some(original)) =
            (&plan.body, &plan.body_signature, &plan.authority)
        else {
            return Ok(None);
        };
        if !std::ptr::eq(validated.function, function.as_ref())
            || !std::ptr::eq(validated.facts, facts.as_ref())
            || !Arc::ptr_eq(&validated.view, view)
            || plan.physical_body.is_some()
            || view.contract().results.is_empty()
            || !Arc::ptr_eq(original, facts)
            || !plan.problems.is_empty()
        {
            return Ok(None);
        }
        if let Err(error) = view.check_body_replay_context(facts) {
            if error.is_resource() {
                return Err(Exhausted);
            }
            return Ok(None);
        }
        let bytes = Self::payload_bytes(signature.params.len())?;
        if add(plan.bytes, bytes)? > max_bytes {
            return Err(Exhausted);
        }
        // The copied parameter slots are new payload. All referenced semantic
        // inputs remain charged to their original producers. No refund on drop.
        budget.reserve_bytes(bytes)?;
        budget.spend(add(signature.params.len(), 1)?)?;
        facts
            .budget()
            .charge_work(add(signature.params.len(), 1)?)
            .map_err(|_| Exhausted)?;
        facts.budget().retain_bytes(bytes).map_err(|_| Exhausted)?;
        let mut params = Vec::new();
        params
            .try_reserve_exact(signature.params.len())
            .map_err(|_| Exhausted)?;
        if params.capacity() > signature.params.len() {
            return Err(Exhausted);
        }
        params.extend(signature.params.iter().cloned());
        Ok(Some(Self {
            function: function.clone(),
            facts: facts.clone(),
            view: view.clone(),
            signature: ir::Signature {
                ref_contract: signature.ref_contract.clone(),
                params,
                result: signature.result.clone(),
            },
        }))
    }
    fn check_result(
        &self,
        function: &cfg::Function,
        view: &Arc<super::traits::CarrierContractView>,
        facts: &super::traits::ValidatedFacts,
        signature: &ir::Signature,
        location: &CarrierLocation,
    ) -> Result<Option<Problem>, Exhausted> {
        // Check the ORIGINAL payer/context even if the supplied context changed.
        // In particular an old poisoned payer cannot be replaced by a fresh one.
        let live = self.view.check_body_replay_context(&self.facts);
        if let Err(error) = live {
            if error.is_resource() {
                return Err(Exhausted);
            }
            return Ok(Some(Problem::Invalid(
                error
                    .into_diagnostic(self.facts.budget())
                    .map_err(|_| Exhausted)?,
            )));
        }
        if !std::ptr::eq(function, self.function.as_ref()) {
            let error = super::traits::FactsError::from_message(
                self.facts.budget(),
                "body replay receipt changed its original CFG allocation",
            );
            return Ok(Some(Problem::Invalid(
                error
                    .into_diagnostic(self.facts.budget())
                    .map_err(|_| Exhausted)?,
            )));
        }
        let exact_identity = Arc::ptr_eq(view, &self.view)
            && std::ptr::eq(facts, self.facts.as_ref())
            && Arc::ptr_eq(&signature.ref_contract, &self.signature.ref_contract);
        let exact = if exact_identity {
            match self
                .facts
                .world()
                .signatures_equal(signature, &self.signature)
            {
                Ok(same) => same,
                Err(error) if error.is_resource() => return Err(Exhausted),
                Err(error) => {
                    return Ok(Some(Problem::Invalid(
                        error
                            .into_diagnostic(self.facts.budget())
                            .map_err(|_| Exhausted)?,
                    )));
                }
            }
        } else {
            false
        };
        if !exact {
            // No permission transfers to a changed function/view/facts/signature.
            return result_flow_problem(
                Some(view),
                Some(facts),
                ir::SignatureSubject::Body,
                signature,
                location,
                &mut false,
            );
        }
        // The constructor validated every result row against this exact
        // immutable function, view, facts and signature before issuing its
        // private witness. Reuse that proof only for an original canonical
        // location. Reconstructing the declaration signature here on every
        // dataflow visit repeats the already-proved module/token traversal.
        for row in self.view.contract().results.iter() {
            self.facts
                .budget()
                .charge_work(add(
                    row.location.path.0.len(),
                    add(location.path.0.len(), 1)?,
                )?)
                .map_err(|_| Exhausted)?;
            if row.location == *location {
                return Ok(None);
            }
        }
        Ok(Some(Problem::Invalid(
            "body replay result location was not validated by its constructor".into(),
        )))
    }
}

pub(super) struct Plan {
    offsets: Vec<usize>,
    points: Vec<Option<Point>>,
    outgoing: Vec<Option<Point>>,
    parents: Vec<Set>,
    lifetime_deps: Vec<Set>,
    outlives: Vec<Set>,
    targets: Vec<Vec<Target>>,
    kinds: Vec<BorrowKind>,
    // Distinct owned mutable result carriers of one call are exclusive, while
    // their shared lifetime dependencies still keep the input borrow alive.
    call_issuers: Vec<Option<usize>>,
    two_phase: Set,
    activations: Vec<Vec<LoanId>>,
    cells: Vec<CellSlot>,
    cell_index: HashMap<CellKey, usize>,
    original_locals: Vec<Ty>,
    family_cover_locals: Option<Vec<Ty>>,
    family_inputs: Vec<FamilyOrigin>,
    body: Option<Arc<super::traits::CarrierContractView>>,
    body_validation: Option<BodyValidationReceipt>,
    physical_body: Option<super::opaque_physical::PhysicalCarrierFlow>,
    body_signature: Option<ir::Signature>,
    authority: Option<Arc<super::traits::ValidatedFacts>>,
    effects: Option<Arc<CarrierEffects>>,
    pub problems: Vec<(BlockId, Option<usize>, Problem)>,
    pub bytes: usize,
}

/// A separate, evidence-based case for closed bodies outside the fast affine
/// envelope: every exact declared cell survives with only its own ENTRY origin.
/// This cannot admit a swap, a missing cell, an abstract/summary destination, or
/// a strong overwrite. The caller separately proves zero result carriers,
/// unique cells, valid occurrence bindings, and an otherwise successful plan.
fn identity_effect_rows(
    declared: &ir::CarrierContract,
    computed: &[ir::CarrierWriteEnvelope],
    must_overwrite: &[InputCarrier],
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    budget.spend(
        computed
            .len()
            .saturating_add(declared.writes.len())
            .saturating_add(1),
    )?;
    if !must_overwrite.is_empty()
        || !declared.results.is_empty()
        || computed.len() != declared.writes.len()
    {
        return Ok(false);
    }
    for (index, (row, expected)) in computed.iter().zip(declared.writes.iter()).enumerate() {
        budget.spend(
            row.destination
                .location
                .path
                .0
                .len()
                .saturating_add(expected.destination.location.path.0.len())
                .saturating_add(1),
        )?;
        if row.destination != expected.destination
            || row.sources.len() != 1
            || !matches!(row.destination.location.channel, CarrierChannel::Fixed(_))
            || row.destination.location.path.0.iter().any(|step| {
                !matches!(
                    step,
                    CarrierStep::Referent
                        | CarrierStep::Field(_)
                        | CarrierStep::ManuallyDropPayload
                        | CarrierStep::ConstantIndex(_)
                )
            })
        {
            return Ok(false);
        }
        budget.spend(
            row.destination
                .location
                .path
                .0
                .len()
                .saturating_add(row.sources[0].location.path.0.len())
                .saturating_add(1),
        )?;
        if row.sources[0] != row.destination {
            return Ok(false);
        }
        let mut declared_self = false;
        for source in expected.sources.iter() {
            budget.spend(
                source
                    .location
                    .path
                    .0
                    .len()
                    .saturating_add(row.destination.location.path.0.len())
                    .saturating_add(1),
            )?;
            declared_self |= source == &row.destination;
        }
        if !declared_self {
            return Ok(false);
        }
        for previous in &computed[..index] {
            budget.spend(
                previous
                    .destination
                    .location
                    .path
                    .0
                    .len()
                    .saturating_add(row.destination.location.path.0.len())
                    .saturating_add(1),
            )?;
            if previous.destination == row.destination {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn add(a: usize, b: usize) -> Result<usize, Exhausted> {
    a.checked_add(b).ok_or(Exhausted)
}
fn mul(a: usize, b: usize) -> Result<usize, Exhausted> {
    a.checked_mul(b).ok_or(Exhausted)
}
fn successors(term: &Terminator) -> [Option<BlockId>; 2] {
    match term {
        Terminator::Goto(next) | Terminator::Call { next, .. } | Terminator::Drop { next, .. } => {
            [Some(*next), None]
        }
        Terminator::Switch {
            then_block,
            else_block,
            ..
        }
        | Terminator::SwitchVariant {
            then_block,
            else_block,
            ..
        } => [Some(*then_block), Some(*else_block)],
        Terminator::Return | Terminator::Panic(_) | Terminator::Unreachable => [None, None],
    }
}
fn visit_operand(op: &Operand, mut visit: impl FnMut(&Place, bool)) {
    match op {
        Operand::Copy(p) => visit(p, false),
        Operand::Move(p) => visit(p, true),
        _ => {}
    }
}
fn visit_rvalue(f: &cfg::Function, value: &Rvalue, mut visit: impl FnMut(&Place, bool)) {
    match value {
        Rvalue::CellNew { value, .. } => visit_operand(value, &mut visit),
        Rvalue::CellGet { reference, .. } => visit(reference, false),
        Rvalue::CellSet {
            reference, value, ..
        } => {
            visit(reference, false);
            visit_operand(value, &mut visit);
        }
        Rvalue::ManuallyDropTake { payload, .. } => visit(payload, true),
        Rvalue::Borrow(id) => visit(&f.loan_sites[id.0].place, false),
        Rvalue::ManuallyDropIntoInner { value: op, .. }
        | Rvalue::OpaquePack { value: op, .. }
        | Rvalue::Use(op)
        | Rvalue::LeadingZeros { value: op }
        | Rvalue::Unary { value: op, .. }
        | Rvalue::Repeat { value: op, .. }
        | Rvalue::ArrayToSlice { reference: op }
        | Rvalue::RefToDyn { reference: op, .. }
        | Rvalue::SliceDataPointer { slice: op }
        | Rvalue::SliceLen { slice: op }
        | Rvalue::CharToInt { value: op, .. }
        | Rvalue::U8ToChar { value: op }
        | Rvalue::PtrCast { value: op, .. }
        | Rvalue::IntCast { value: op, .. }
        | Rvalue::FloatCast { value: op, .. } => visit_operand(op, &mut visit),
        Rvalue::SliceIndex { slice, index } => {
            visit_operand(slice, &mut visit);
            visit_operand(index, &mut visit);
        }
        Rvalue::Subslice {
            slice, start, end, ..
        } => {
            visit_operand(slice, &mut visit);
            visit_operand(start, &mut visit);
            if let Some(end) = end {
                visit_operand(end, &mut visit);
            }
        }
        Rvalue::Binary { left, right, .. } => {
            visit_operand(left, &mut visit);
            visit_operand(right, &mut visit);
        }
        Rvalue::NativeSyscall(fields)
        | Rvalue::RawSlice { args: fields, .. }
        | Rvalue::Aggregate { fields, .. }
        | Rvalue::EnumVariant { fields, .. }
        | Rvalue::Intrinsic { args: fields, .. }
        | Rvalue::Format {
            arguments: fields, ..
        } => {
            for op in fields {
                visit_operand(op, &mut visit);
            }
        }
        Rvalue::Discriminant(p) => visit(p, false),
        Rvalue::IndexCopy { array, index } => {
            visit(array, false);
            visit_operand(index, &mut visit);
        }
        Rvalue::RuntimeQuery(_)
        | Rvalue::TypeAlignment(_)
        | Rvalue::Println(_)
        | Rvalue::Output(_)
        | Rvalue::Str(_)
        | Rvalue::ByteStr(_)
        | Rvalue::PromotedArray(_)
        | Rvalue::PromotedScalar(_)
        | Rvalue::ByteRepeat { .. } => {}
    }
}

fn overlap(a: &CanonicalPlace, b: &CanonicalPlace, budget: &mut Budget) -> Result<bool, Exhausted> {
    budget.spend(a.projection.len().min(b.projection.len()) + 1)?;
    if a.root != b.root {
        return Ok(false);
    }
    for (a, b) in a.projection.iter().zip(&b.projection) {
        // Safe Rust's ordinary indexed borrows overlap the entire array in B1.
        if matches!(
            a,
            Projection::ConstantIndex(_) | Projection::DynamicIndex(_)
        ) || matches!(
            b,
            Projection::ConstantIndex(_) | Projection::DynamicIndex(_)
        ) {
            return Ok(true);
        }
        if a != b {
            return Ok(!matches!(
                (a, b),
                (Projection::Field(_), Projection::Field(_))
            ));
        }
    }
    Ok(true)
}

/// The same checked accounting is used before recipe registration and before
/// the real solver allocates. These are sizes, never evidence or permissions.
pub(super) fn plan_metadata_bytes(locals: usize, blocks: usize) -> Result<usize, Exhausted> {
    add(
        add(mul(locals, 96)?, mul(add(blocks, 1)?, 96)?)?,
        mul(blocks, size_of::<Option<Option<Arc<CarrierEffects>>>>())?,
    )
}

fn carrier_bitmap_bytes(
    slots: usize,
    references: usize,
    families: usize,
) -> Result<usize, Exhausted> {
    mul(
        slots,
        add(
            size_of::<CarrierOrigins>(),
            mul(
                add(references.div_ceil(64), families.div_ceil(64))?,
                size_of::<u64>(),
            )?,
        )?,
    )
}
fn flow_state_bytes(slots: usize, references: usize, families: usize) -> Result<usize, Exhausted> {
    add(
        carrier_bitmap_bytes(slots, references, families)?,
        add(size_of::<Set>(), mul(slots.div_ceil(64), size_of::<u64>())?)?,
    )
}
pub(super) fn region_storage_bytes(nodes: usize) -> Result<usize, Exhausted> {
    // Includes Vec growth and old/new rows coexisting during expansion.
    mul(
        8,
        mul(
            nodes,
            add(
                size_of::<RegionKey>() + size_of::<Set>(),
                mul(nodes.div_ceil(64), size_of::<u64>())?,
            )?,
        )?,
    )
}
pub(super) fn plan_storage_bytes(
    locals: usize,
    blocks: usize,
    points: usize,
    slots: usize,
    origins: usize,
) -> Result<usize, Exhausted> {
    let metadata = plan_metadata_bytes(locals, blocks)?;
    // Known loans and opaque families use disjoint typed bitmaps. Their
    // combined rounding needs at most one extra word, even if one is empty.
    let origin_size = mul(
        slots,
        add(2 * size_of::<Set>(), mul(add(origins.div_ceil(64), 1)?, 8)?)?,
    )?;
    let live_size = add(size_of::<Set>(), mul(slots.div_ceil(64), 8)?)?;
    let mut bytes = add(
        metadata,
        add(
            mul(add(add(points, mul(blocks, 2)?)?, 2)?, origin_size)?,
            mul(add(mul(points, 3)?, add(mul(blocks, 3)?, 4)?)?, live_size)?,
        )?,
    )?;
    bytes = add(
        bytes,
        add(
            mul(blocks, 256)?,
            add(
                mul(points, 128)?,
                mul(locals, size_of::<[Option<usize>; 2]>())?,
            )?,
        )?,
    )?;
    bytes = add(
        bytes,
        mul(origins, add(256, mul(origins.div_ceil(64), 16)?)?)?,
    )?;
    let activation_size = add(size_of::<Set>(), mul(origins.div_ceil(64), 8)?)?;
    bytes = add(
        bytes,
        add(
            mul(add(points, add(mul(blocks, 4)?, 8)?)?, activation_size)?,
            add(
                mul(blocks, 4 * (size_of::<Vec<LoanId>>() + size_of::<usize>()))?,
                mul(origins, 4 * (size_of::<LoanId>() + size_of::<usize>()))?,
            )?,
        )?,
    )?;
    bytes = add(bytes, mul(origins, 64)?)?;
    bytes = add(
        bytes,
        mul(
            slots,
            8 * (size_of::<CellKey>() + size_of::<CellSlot>() + size_of::<usize>()),
        )?,
    )?;
    add(
        bytes,
        2 * (size_of::<Target>() + 128 * size_of::<Projection>()),
    )
}

pub(super) fn plan_initial_work(
    blocks: usize,
    points: usize,
    slots: usize,
    origins: usize,
) -> Result<usize, Exhausted> {
    add(
        mul(add(points, blocks)?, mul(slots, origins.div_ceil(64))?)?,
        mul(add(mul(points, 3)?, mul(blocks, 2)?)?, slots.div_ceil(64))?,
    )
}

/// Counts allocation attempts, including deduplicated candidates. The factor of
/// four retains the existing Vec-growth and simultaneous resolved/issued peak.
pub(super) fn target_payload_bytes(
    allocations: usize,
    projection_steps: usize,
    authority_entries: usize,
    overhead_words: usize,
) -> Result<usize, Exhausted> {
    target_payload_bytes_with_growth(
        allocations,
        projection_steps,
        authority_entries,
        overhead_words,
        128,
    )
}

fn target_payload_bytes_with_growth(
    allocations: usize,
    projection_steps: usize,
    authority_entries: usize,
    overhead_words: usize,
    projection_growth: usize,
) -> Result<usize, Exhausted> {
    mul(
        4,
        add(
            mul(allocations, size_of::<Target>())?,
            add(
                mul(
                    add(projection_steps, mul(allocations, projection_growth)?)?,
                    size_of::<Projection>(),
                )?,
                mul(add(authority_entries, overhead_words)?, size_of::<usize>())?,
            )?,
        )?,
    )
}

impl Plan {
    pub(super) fn build_physical<'a>(
        f: &'a Arc<cfg::Function>,
        facts: &'a Arc<super::traits::ValidatedFacts>,
        physical: &'a super::opaque_physical::PhysicalValidation<'a>,
        table: Option<&Arc<super::ownership::CarrierEffectTable>>,
        budget: &mut Budget,
        max_bytes: usize,
    ) -> Result<Self, Exhausted> {
        if physical.validate_cfg(f, facts).is_err() {
            return Err(Exhausted);
        }
        let mut builder = CarrierBuilder::new(
            f,
            FactsCtx::Validated(facts),
            facts.budget().clone(),
            budget,
            max_bytes,
            Some((physical, facts)),
        )?;
        builder.function = Some(f.clone());
        builder.authority = Some(facts.clone());
        builder.effect_table = table.cloned();
        builder.initialize_regions()?;
        builder.solve()
    }
    pub fn build_validated(
        f: &Arc<cfg::Function>,
        facts: &Arc<super::traits::ValidatedFacts>,
        table: Option<&Arc<super::ownership::CarrierEffectTable>>,
        budget: &mut Budget,
        max_bytes: usize,
    ) -> Result<Self, Exhausted> {
        let mut builder = CarrierBuilder::new(
            f,
            FactsCtx::Validated(facts),
            facts.budget().clone(),
            budget,
            max_bytes,
            None,
        )?;
        builder.function = Some(f.clone());
        builder.authority = Some(facts.clone());
        builder.effect_table = table.cloned();
        builder.initialize_regions()?;
        let body_validated = builder.body_validated.take();
        let mut plan = builder.solve()?;
        // Only this constructor has the exact original function/facts Arcs.
        // The solver's private body slot is populated only after every result
        // row authenticated in CarrierBuilder::new. Legacy and physical plans
        // never receive this receipt.
        plan.body_validation =
            BodyValidationReceipt::issue(&plan, body_validated, f, facts, budget, max_bytes)?;
        if let Some(receipt) = &plan.body_validation {
            plan.bytes = add(
                plan.bytes,
                BodyValidationReceipt::payload_bytes(receipt.signature.params.len())?,
            )?;
        }
        Ok(plan)
    }
    pub(super) fn effects(&self) -> Option<&Arc<CarrierEffects>> {
        self.effects.as_ref()
    }
    #[cfg(test)]
    pub fn build(
        f: &cfg::Function,
        budget: &mut Budget,
        max_bytes: usize,
    ) -> Result<Self, Exhausted> {
        Self::build_with_facts(f, FactsCtx::Concrete, budget, max_bytes)
    }
    pub fn build_with_facts(
        f: &cfg::Function,
        facts: FactsCtx<'_>,
        budget: &mut Budget,
        max_bytes: usize,
    ) -> Result<Self, Exhausted> {
        let ledger = match facts {
            FactsCtx::Validated(facts) => facts.budget().clone(),
            _ => budget
                .plan_budget()
                .cloned()
                .unwrap_or_else(|| ir::PlanBudget::new(ir::PlanLimits::default())),
        };
        CarrierBuilder::new(f, facts, ledger, budget, max_bytes, None)?.solve()
    }
    fn check_backedges(
        &mut self,
        f: &cfg::Function,
        reachable: &[bool],
        ins: &[Set],
        births: &[Option<usize>],
        budget: &mut Budget,
    ) -> Result<(), Exhausted> {
        let b = f.blocks.len();
        // DFS backedges and SCC membership are computed without recursion.
        let mut colors = vec![0u8; b];
        let mut order = vec![];
        let mut backedges = vec![];
        let mut reverse = vec![vec![]; b];
        for (block, body) in f.blocks.iter().enumerate() {
            for next in successors(&body.terminator.value).into_iter().flatten() {
                reverse[next.0].push(block);
            }
        }
        for start in 0..b {
            if !reachable[start] || colors[start] != 0 {
                continue;
            }
            colors[start] = 1;
            let mut stack = vec![(start, 0usize)];
            while let Some((block, edge)) = stack.last_mut() {
                budget.spend(1)?;
                if *edge == 2 {
                    colors[*block] = 2;
                    order.push(*block);
                    stack.pop();
                    continue;
                }
                let next = successors(&f.blocks[*block].terminator.value)[*edge];
                *edge += 1;
                if let Some(next) = next {
                    if colors[next.0] == 1 {
                        backedges.push((*block, next.0));
                    } else if colors[next.0] == 0 {
                        colors[next.0] = 1;
                        stack.push((next.0, 0));
                    }
                }
            }
        }
        let mut component = vec![usize::MAX; b];
        for start in order.into_iter().rev() {
            if component[start] != usize::MAX {
                continue;
            }
            component[start] = start;
            let mut stack = vec![start];
            while let Some(block) = stack.pop() {
                budget.spend(1)?;
                for previous in &reverse[block] {
                    if reachable[*previous] && component[*previous] == usize::MAX {
                        component[*previous] = start;
                        stack.push(*previous);
                    }
                }
            }
        }
        for (block, next) in backedges {
            let point = self.outgoing[block].as_ref().unwrap();
            let active = self.active(&point.origins, &ins[next], budget)?;
            for loan in active.members() {
                // A shared origin reissued each iteration keeps one static
                // identity whose target memberships the fixed point unions.
                // Every overlapping write and storage end in the loop still
                // observes all of those targets live, whether issuance came
                // from a direct borrow or a call. Exclusive origins remain
                // rejected: distinct dynamic `&mut` issuances must not alias
                // one static identity.
                if self.kinds[loan] == BorrowKind::Shared {
                    continue;
                }
                if let Some(birth) = births[loan]
                    && component[birth] == component[block]
                    && !self.killed_before_rebirth(
                        f,
                        BlockId(next),
                        loan,
                        self.call_issuers[loan].map(BlockId),
                        budget,
                    )?
                {
                    self.problems.push((
                        BlockId(block),
                        None,
                        Problem::Unsupported(
                            "SCC-born loan live across a backedge is unsupported in B1".into(),
                        ),
                    ));
                    break;
                }
            }
        }
        if self.problems.len() > 256 {
            return Err(Exhausted);
        }
        Ok(())
    }
    fn compute_activation(
        &mut self,
        f: &cfg::Function,
        memberships: &mut usize,
        budget: &mut Budget,
    ) -> Result<(), Exhausted> {
        budget.spend(self.two_phase.0.len())?;
        if self.two_phase.count() == 0 {
            return Ok(());
        }
        let loans = self.kinds.len();
        let mut entries = vec![Set::new(loans); f.blocks.len()];
        let mut seen = vec![false; f.blocks.len()];
        let mut queued = vec![false; f.blocks.len()];
        let mut reentrant = Set::new(loans);
        let mut queue = VecDeque::from([f.entry.0]);
        seen[f.entry.0] = true;
        queued[f.entry.0] = true;
        while let Some(block) = queue.pop_front() {
            queued[block] = false;
            budget.spend(entries[block].0.len() + 1)?;
            let mut state = entries[block].clone();
            for statement in 0..=f.blocks[block].statements.len() {
                let index = self.offsets[block] + statement;
                let point = self.points[index].as_ref().expect("reachable point");
                let live = self.active(&point.origins, &point.live, budget)?;
                state.intersect(&live, budget)?;
                Self::store_activation(
                    self.points[index].as_mut().expect("reachable point"),
                    &state,
                    memberships,
                    budget,
                )?;
                if let Some(item) = f.blocks[block].statements.get(statement)
                    && let Statement::Assign {
                        value: Rvalue::Borrow(id),
                        ..
                    } = item.value
                    && self.two_phase.has(id.0)
                {
                    if live.has(id.0) {
                        // A fresh execution cannot recycle a still-live origin.
                        // Ordinary per-iteration loans die before their backedge.
                        reentrant.insert(id.0);
                    }
                    state.remove(id.0);
                }
            }
            budget.spend(self.activations[block].len())?;
            for loan in &self.activations[block] {
                state.insert(loan.0);
            }
            let point = self.outgoing[block]
                .as_ref()
                .expect("reachable outgoing point");
            let live = self.active(&point.origins, &point.live, budget)?;
            state.intersect(&live, budget)?;
            Self::store_activation(
                self.outgoing[block]
                    .as_mut()
                    .expect("reachable outgoing point"),
                &state,
                memberships,
                budget,
            )?;
            for next in successors(&f.blocks[block].terminator.value)
                .into_iter()
                .flatten()
            {
                budget.spend(state.0.len() * 2 + 1)?;
                let before = entries[next.0].count();
                let changed = entries[next.0].union(&state, budget)?;
                *memberships = add(*memberships, entries[next.0].count() - before)?;
                if *memberships > MAX_MEMBERSHIPS {
                    return Err(Exhausted);
                }
                if (changed || !seen[next.0]) && !queued[next.0] {
                    queue.push_back(next.0);
                    queued[next.0] = true;
                }
                seen[next.0] = true;
            }
        }
        budget.spend(reentrant.0.len())?;
        for loan in reentrant.members() {
            if self.problems.len() >= 256 {
                return Err(Exhausted);
            }
            let cfg::LoanTiming::TwoPhase { activation, .. } = f.loan_sites[loan].timing else {
                unreachable!()
            };
            self.problems.push((
                activation,
                None,
                Problem::Unsupported(
                    "reentrant two-phase origin is still live at fresh issuance".into(),
                ),
            ));
        }
        Ok(())
    }

    fn store_activation(
        point: &mut Point,
        state: &Set,
        memberships: &mut usize,
        budget: &mut Budget,
    ) -> Result<(), Exhausted> {
        budget.spend(state.0.len() * 3)?;
        *memberships = add(
            *memberships,
            state.count().saturating_sub(point.activated.count()),
        )?;
        if *memberships > MAX_MEMBERSHIPS {
            return Err(Exhausted);
        }
        point.activated.0.copy_from_slice(&state.0);
        Ok(())
    }

    pub fn activations(&self, block: BlockId) -> &[LoanId] {
        &self.activations[block.0]
    }

    fn point_data(&self, block: BlockId, statement: Option<usize>, phase: Phase) -> &Point {
        let end = self.offsets[block.0 + 1] - 1;
        let current = self.offsets[block.0] + statement.unwrap_or(end - self.offsets[block.0]);
        let point = if matches!(phase, Phase::Write) && current < end {
            current + 1
        } else {
            current
        };
        let data = self.points[point]
            .as_ref()
            .expect("reachable ownership point");
        if matches!(phase, Phase::Write) && current == end {
            self.outgoing[block.0]
                .as_ref()
                .expect("reachable outgoing point")
        } else {
            data
        }
    }
    fn point(&self, block: BlockId, statement: Option<usize>, phase: Phase) -> (&Origins, &Set) {
        let point = self.point_data(block, statement, phase);
        (&point.origins, &point.live)
    }
    fn active(&self, origins: &Origins, live: &Set, budget: &mut Budget) -> Result<Set, Exhausted> {
        budget.spend(live.0.len() + self.kinds.len().div_ceil(64))?;
        let mut active = Set::new(self.kinds.len());
        for local in live.members() {
            active.union(&origins[local], budget)?;
        }
        let mut queue: Vec<usize> = active.members().collect();
        while let Some(loan) = queue.pop() {
            budget.spend(self.lifetime_deps[loan].0.len() + self.outlives[loan].0.len() + 1)?;
            for parent in self.lifetime_deps[loan]
                .members()
                .chain(self.outlives[loan].members())
            {
                if !active.has(parent) {
                    active.insert(parent);
                    queue.push(parent);
                }
            }
        }
        Ok(active)
    }
    pub fn resolve(
        &self,
        place: &Place,
        block: BlockId,
        statement: Option<usize>,
        phase: Phase,
        budget: &mut Budget,
    ) -> Result<Result<Vec<Target>, Problem>, Exhausted> {
        let (origins, _) = self.point(block, statement, phase);
        let (targets, _, failure) = resolve_cells(
            &self.original_locals,
            self.family_cover_locals
                .as_deref()
                .unwrap_or(&self.original_locals),
            &self.cells,
            &self.cell_index,
            &self.targets,
            |index| &origins[index],
            place,
            budget,
        )?;
        if let Some(failure) = failure {
            Ok(Err(failure.problem(
                "reference lacks a complete original typed cell cover",
            )))
        } else {
            Ok(Ok(targets))
        }
    }
    pub fn call_write_targets(
        &self,
        block: BlockId,
        budget: &mut Budget,
    ) -> Result<Vec<Target>, Exhausted> {
        let Some(effects) = &self.effects else {
            return Ok(Vec::new());
        };
        let Some(index) = call_binding_index(&effects.bindings, block, budget)? else {
            return Ok(Vec::new());
        };
        let binding = &effects.bindings[index];
        let mut bytes = mul(binding.actual_writes.len(), size_of::<Target>())?;
        for target in &binding.actual_writes {
            bytes = add(
                bytes,
                add(
                    mul(target.place.projection.len(), size_of::<Projection>())?,
                    mul(target.authority.len(), size_of::<usize>())?,
                )?,
            )?;
            budget.spend(target.place.projection.len() + target.authority.len() + 1)?;
        }
        budget.reserve_bytes(bytes)?;
        Ok(binding.actual_writes.clone())
    }
    pub fn issued(&self, id: LoanId, budget: &mut Budget) -> Result<Vec<Target>, Exhausted> {
        for target in &self.targets[id.0] {
            budget.spend(target.place.projection.len() + target.authority.len() + 1)?;
        }
        Ok(self.targets[id.0].clone())
    }
    pub fn check_return(
        &self,
        f: &cfg::Function,
        block: BlockId,
        facts: FactsCtx<'_>,
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let authority = self.authority.as_deref().or(facts.validated());
        if authority.is_some() && self.body.is_none() {
            return Ok(Some(Problem::Invalid(
                "body result has no validated declaration view".into(),
            )));
        }
        let mut slice_carriers = false;
        if let Some(body) = &self.body {
            for row in body.source_region_env().carriers.iter() {
                budget.spend(row.location.path.0.len() + 1)?;
                slice_carriers |= row.parameter.is_some()
                    && row.location.path.0.contains(&CarrierStep::SliceElement);
            }
        }
        if let Some(body) = &self.body {
            let Some(signature) = &self.body_signature else {
                return Ok(Some(Problem::Invalid(
                    "body result lost its original signature".into(),
                )));
            };
            if let Some(authority) = authority {
                // Reuse the charged snapshot, but never authorize a different
                // CFG signature with it. Types use the bounded world comparator.
                let same = (|| -> Result<bool, super::traits::FactsError> {
                    if signature.params.len() != f.params.len()
                        || !Arc::ptr_eq(&signature.ref_contract, &f.ref_contract)
                        || !authority.world().types_equal(
                            std::slice::from_ref(&signature.result),
                            std::slice::from_ref(&f.result),
                        )?
                    {
                        return Ok(false);
                    }
                    for (expected, local) in signature.params.iter().zip(&f.params) {
                        let Some(actual) = f.locals.get(local.0) else {
                            return Ok(false);
                        };
                        if !authority.world().types_equal(
                            std::slice::from_ref(expected),
                            std::slice::from_ref(&actual.ty),
                        )? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                })();
                match same {
                    Ok(true) => {}
                    Ok(false) => {
                        return Ok(Some(Problem::Invalid(
                            "body result changed its original CFG signature".into(),
                        )));
                    }
                    Err(error) if error.is_resource() => return Err(Exhausted),
                    Err(error) => {
                        return Ok(Some(Problem::Invalid(
                            error
                                .into_diagnostic(authority.budget())
                                .map_err(|_| Exhausted)?,
                        )));
                    }
                }
            }
            let mut view_validated = false;
            for row in body.contract().results.iter() {
                let problem = if let (Some(receipt), Some(actual)) =
                    (&self.body_validation, facts.validated())
                    && self.physical_body.is_none()
                {
                    receipt.check_result(f, body, actual, signature, &row.location)?
                } else {
                    physical_result_flow_problem(
                        self.physical_body.as_ref(),
                        Some(body),
                        authority,
                        ir::SignatureSubject::Body,
                        signature,
                        &row.location,
                        &mut view_validated,
                    )?
                };
                if let Some(problem) = problem {
                    return Ok(Some(problem));
                }
                let projection = value_projection(&row.location);
                // Returning a handle transports its owned carrier components,
                // not the current contents of its pointee. In particular an
                // opaque T behind &T is not read as a reference-valued cell.
                // Positioned reference origins are checked independently below.
                if projection.contains(&Projection::Deref) {
                    if let Some(problem) = self.check_invariant_nested_result(
                        f,
                        block,
                        &row.location,
                        &row.sources,
                        budget,
                    )? {
                        return Ok(Some(problem));
                    }
                    continue;
                }
                let place = Place {
                    local: f.return_local,
                    projection,
                };
                let origins =
                    match self.carrier_origins_at(&place, block, None, Phase::Read, budget)? {
                        Ok(origins) => origins,
                        Err(failure) => {
                            return Ok(Some(failure.problem(
                                "returned carrier lacks a complete original typed cell cover",
                            )));
                        }
                    };
                for family in origins.families.0.members() {
                    budget.spend(row.sources.len() + 1)?;
                    if !row.sources.contains(&self.family_inputs[family].input) {
                        return Ok(Some(Problem::Invalid("returned carrier family violates its declaration-owned origin contract".into())));
                    }
                }
                for origin in origins.references.0.members() {
                    for target in &self.targets[origin] {
                        budget.spend(row.sources.len() + f.params.len() + 1)?;
                        let CanonicalRoot::ParameterReferent(local, location) = &target.place.root
                        else {
                            return Ok(Some(Problem::Conflict(
                                "E0515: cannot return a value referencing owned frame storage"
                                    .into(),
                            )));
                        };
                        if !row.sources.iter().any(|input| {
                            f.params.get(input.param) == Some(local) && input.location == *location
                        }) {
                            return Ok(Some(Problem::Invalid(format!(
                                "returned carrier violates its declaration-owned input occurrence: expected {:?}, got {:?} at {:?}",
                                row.sources, local, location
                            ))));
                        }
                    }
                }
                let mapped_slice_family = authority.is_some()
                    && borrowed_slice_family_result(body, &row.location, budget)?;
                if mapped_slice_family
                    || body.source_region_env().carriers.iter().any(|source| {
                        source.parameter.is_none()
                            && source.location == row.location
                            && (matches!(source.region, ir::CarrierRegionTerm::Named(_))
                                || slice_carriers)
                    })
                {
                    // Source carrier rows authenticate the complete dependency
                    // closure, not merely the immediate returned address.
                    let mut seen = origins.references.0.clone();
                    let mut pending: Vec<_> = seen.members().collect();
                    while let Some(origin) = pending.pop() {
                        budget.spend(self.lifetime_deps[origin].0.len() + 1)?;
                        for target in &self.targets[origin] {
                            budget.spend(row.sources.len() + f.params.len() + 1)?;
                            let CanonicalRoot::ParameterReferent(local, location) =
                                &target.place.root
                            else {
                                return Ok(Some(Problem::Conflict(
                                    "E0515: returned carrier lifetime depends on owned frame storage"
                                        .into(),
                                )));
                            };
                            if !row.sources.iter().any(|input| {
                                f.params.get(input.param) == Some(local)
                                    && input.location == *location
                            }) {
                                return Ok(Some(Problem::Invalid("returned carrier dependency violates its declaration-owned lifetime".into())));
                            }
                        }
                        for parent in self.lifetime_deps[origin].members() {
                            if !seen.has(parent) {
                                seen.insert(parent);
                                pending.push(parent);
                            }
                        }
                    }
                }
            }
        }
        for result in f.ref_contract.results.iter() {
            if let Some(body) = &self.body
                && authority.is_some()
                && borrowed_slice_family_result(body, &fixed_location(result.position), budget)?
            {
                // The exact mapped source family and all lifetime dependencies
                // were checked above. Do not substitute the descriptor region.
                continue;
            }
            if self.body.as_ref().is_some_and(|body| {
                body.source_region_env().carriers.iter().any(|source| {
                    source.parameter.is_none()
                        && source.location == fixed_location(result.position)
                        && (matches!(source.region, ir::CarrierRegionTerm::Named(_))
                            || (slice_carriers && result.position.depth == 0))
                })
            }) {
                // The generalized carrier check above already proved the source
                // lifetime against exact declaration-owned source occurrences.
                continue;
            }
            if let Some(body) = &self.body
                && authority.is_some()
                && substituted_family_result(body, &fixed_location(result.position), budget)?
            {
                // A reference layer inside a substituted type-parameter
                // family: the universal body already proved it against the
                // declared family result row, and callers check the
                // substituted row. The instance adds no new lifetime facts.
                continue;
            }
            if let Some(problem) = self.check_return_position(f, block, result, facts, budget)? {
                return Ok(Some(problem));
            }
        }
        Ok(None)
    }
    /// A reference stored behind a returned `&mut` is invariant: the caller
    /// can read it back at the declared region, so its current origins must
    /// be exactly the declaration-owned sources of that nested carrier.
    fn check_invariant_nested_result(
        &self,
        f: &cfg::Function,
        block: BlockId,
        location: &CarrierLocation,
        sources: &[InputCarrier],
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let CarrierChannel::Fixed(position) = location.channel else {
            return Ok(None);
        };
        // Find whether some enclosing reference on the path is `&mut`.
        let mut ty = &f.result;
        let mut behind_mut = false;
        for step in location.path.0.iter() {
            budget.spend(1)?;
            match (step, ty) {
                (CarrierStep::Referent, Ty::Ref { kind, referent }) => {
                    behind_mut |= *kind == BorrowKind::Mut;
                    ty = referent;
                }
                _ => return Ok(None),
            }
        }
        for _ in 0..position.depth {
            budget.spend(1)?;
            let Ty::Ref { kind, referent } = ty else {
                return Ok(None);
            };
            behind_mut |= *kind == BorrowKind::Mut;
            ty = referent;
        }
        if !behind_mut {
            return Ok(None);
        }
        let place = Place {
            local: f.return_local,
            projection: value_projection(location),
        };
        let Ok(origins) = self.carrier_origins_at(&place, block, None, Phase::Read, budget)? else {
            return Ok(None);
        };
        for origin in origins.references.0.members() {
            for target in &self.targets[origin] {
                budget.spend(sources.len() + 1)?;
                let CanonicalRoot::ParameterReferent(local, at) = &target.place.root else {
                    continue;
                };
                if !target.place.projection.is_empty() {
                    continue;
                }
                if !sources
                    .iter()
                    .any(|input| f.params.get(input.param) == Some(local) && input.location == *at)
                {
                    return Ok(Some(Problem::Invalid(
                        "lifetime may not live long enough: reference behind a returned `&mut` is invariant"
                            .into(),
                    )));
                }
            }
        }
        Ok(None)
    }
    fn carrier_origins_at(
        &self,
        place: &Place,
        block: BlockId,
        statement: Option<usize>,
        phase: Phase,
        budget: &mut Budget,
    ) -> Result<Result<CarrierOrigins, ResolutionFailure>, Exhausted> {
        let point = self.point_data(block, statement, phase);
        let (targets, _, failure) = resolve_cells(
            &self.original_locals,
            self.family_cover_locals
                .as_deref()
                .unwrap_or(&self.original_locals),
            &self.cells,
            &self.cell_index,
            &self.targets,
            |index| &point.origins[index],
            place,
            budget,
        )?;
        if let Some(failure) = failure {
            return Ok(Err(failure));
        }
        let mut value = CarrierOrigins::new(self.targets.len(), self.family_inputs.len());
        for target in targets {
            if target.precision == TargetPrecision::Exact {
                let mut found = false;
                for (index, cell) in self.cells.iter().enumerate() {
                    budget.spend(add(cell.key.storage.projection.len(), 1)?)?;
                    if cell.key.storage == target.place {
                        found = true;
                        value.references.0.union(&point.origins[index], budget)?;
                        value.families.0.union(&point.families[index].0, budget)?;
                    }
                }
                if !found {
                    return Ok(Err(ResolutionFailure::IncompleteCover));
                }
            } else {
                let Some(cover) = read_cell_cover(
                    self.family_cover_locals
                        .as_deref()
                        .unwrap_or(&self.original_locals),
                    &self.cell_index,
                    &target,
                    &CellChannel::ReferenceValue,
                    budget,
                )?
                else {
                    return Ok(Err(ResolutionFailure::IncompleteCover));
                };
                for index in cover {
                    value.references.0.union(&point.origins[index], budget)?;
                }
            }
        }
        Ok(Ok(value))
    }
    fn check_return_position(
        &self,
        f: &cfg::Function,
        block: BlockId,
        result: &super::ir::ResultReference,
        facts: FactsCtx<'_>,
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let regions = usize::from(f.ref_contract.region_count);
        budget.spend(
            regions * regions
                + f.ref_contract.input_regions.len()
                + f.ref_contract.outlives.len() * 2
                + 1,
        )?;
        let closure = f
            .ref_contract
            .closure()
            .expect("validated universal contract");
        let value = match self.carrier_origins_at(
            &Place {
                local: f.return_local,
                projection: vec![Projection::Deref; usize::from(result.position.depth)],
            },
            block,
            None,
            Phase::Read,
            budget,
        )? {
            Ok(value) => value,
            Err(failure) => {
                return Ok(Some(failure.problem(
                    "returned reference lacks a complete original typed cell cover",
                )));
            }
        };
        let origins = &value.references.0;
        budget.spend(origins.0.len())?;
        if origins.count() == 0 {
            // A shared `&str` without any loan origin is a string literal:
            // `'static` program data that outlives every region.
            let mut ty = &f.result;
            for _ in 0..result.position.depth {
                let Ty::Ref { referent, .. } = ty else {
                    break;
                };
                ty = referent;
            }
            if ty.is_shared_str() {
                return Ok(None);
            }
            return Ok(Some(Problem::Invalid(
                "returned reference has no issued provenance".into(),
            )));
        }
        let mut dependencies = origins.clone();
        let mut queue: Vec<_> = dependencies.members().collect();
        for origin in origins.members() {
            if self.targets[origin].is_empty() {
                // A reborrow through a raw pointer carries no tracked
                // provenance: its lifetime is unbounded, as in Rust.
                continue;
            }
            for target in &self.targets[origin] {
                budget.spend(f.params.len() + target.place.projection.len() + 1)?;
                let CanonicalRoot::ParameterReferent(local, position) = &target.place.root else {
                    return Ok(Some(Problem::Invalid(
                        "cannot return a reference to owned frame storage".into(),
                    )));
                };
                // A scalar anchor has no proper descendants. This proves only
                // one existing expected path, never reclassifies the canonical
                // signature or turns the target into Exact for other accesses.
                let singleton = target.precision == TargetPrecision::Subtree
                    && Self::scalar_return_footprint(f, target, result.position, facts, budget)?;
                let mut contained = false;
                let mut declared_input = false;
                for (input, projection, precision) in return_inputs(&result.returns) {
                    budget.spend(projection.len() + 1)?;
                    if f.params[input.param] == *local
                        && fixed_location(input.position) == *position
                    {
                        declared_input = true;
                        contained |= precision == TargetPrecision::Subtree
                            || ((target.precision == TargetPrecision::Exact || singleton)
                                && projection == target.place.projection.as_slice());
                    }
                }
                if !contained
                    && declared_input
                    && (target.precision == TargetPrecision::Exact || singleton)
                    && target
                        .place
                        .projection
                        .iter()
                        .any(|step| matches!(step, Projection::DynamicIndex(_)))
                {
                    contained = dynamic_target_covered(
                        f,
                        *local,
                        position,
                        &target.place.projection,
                        &result.returns,
                        budget,
                    )?;
                }
                if !contained {
                    // A sourceless fixed row (`Option<T>::unwrap` instance
                    // with `T = &U`): the result is carried by a by-value
                    // input aggregate. Callers bind it through the structural
                    // carrier source, so any parameter carrier is admissible.
                    // Also a carrier nested in a by-value aggregate input,
                    // which no positioned row can name (`unwrap_or`).
                    let by_value_nested = !position.path.0.is_empty()
                        && !matches!(f.locals[local.0].ty, Ty::Ref { .. });
                    if (return_inputs(&result.returns).next().is_none() || by_value_nested)
                        && f.params.iter().any(|p| p == local)
                    {
                        contained = true;
                    }
                }
                if !contained && !position.path.0.is_empty() {
                    // A reference stored inside an aggregate behind a borrowed
                    // parameter (`s.x` with `s: &'r S<'a>`). Nominal lifetimes
                    // are erased, so admit it only when the result region is
                    // the region of that parameter's outer borrow: the caller
                    // then keeps the aggregate, and its loans, borrowed for
                    // the whole result lifetime ('a: 'r by well-formedness).
                    if let Some(param) = f.params.iter().position(|p| p == local) {
                        contained = f.ref_contract.input_regions.iter().any(|row| {
                            row.input.param == param
                                && row.input.position.depth == 0
                                && (row.region == result.region
                                    || f.ref_contract
                                        .outlives
                                        .contains(&(row.region, result.region)))
                        });
                    }
                }
                if !contained {
                    if target.precision == TargetPrecision::Subtree && declared_input {
                        return Ok(Some(Problem::Unsupported(
                            "abstract aggregate return footprint cannot establish an exact path"
                                .into(),
                        )));
                    }
                    return Ok(Some(Problem::Invalid(
                        "returned input does not satisfy declared output lifetime".into(),
                    )));
                }
            }
        }
        while let Some(origin) = queue.pop() {
            budget.spend(self.lifetime_deps[origin].0.len() + 1)?;
            for target in &self.targets[origin] {
                budget.spend(f.params.len() + 1)?;
                let CanonicalRoot::ParameterReferent(local, position) = &target.place.root else {
                    return Ok(Some(Problem::Invalid(
                        "returned reference lifetime depends on owned frame storage".into(),
                    )));
                };
                let param = f
                    .params
                    .iter()
                    .position(|param| param == local)
                    .expect("formal canonical root");
                budget.spend(f.ref_contract.input_regions.len())?;
                let region = f
                    .ref_contract
                    .input_regions
                    .iter()
                    .find(|row| {
                        row.input.param == param && fixed_location(row.input.position) == *position
                    })
                    .or_else(|| {
                        // A carrier nested in an aggregate behind the outer
                        // borrow shares that borrow's (erased) region.
                        (!position.path.0.is_empty())
                            .then(|| {
                                f.ref_contract.input_regions.iter().find(|row| {
                                    row.input.param == param && row.input.position.depth == 0
                                })
                            })
                            .flatten()
                    });
                let Some(region) = region.map(|row| row.region) else {
                    if return_inputs(&result.returns).next().is_none()
                        || (!position.path.0.is_empty()
                            && !matches!(f.locals[local.0].ty, Ty::Ref { .. }))
                    {
                        // Parameter carrier admitted above.
                        continue;
                    }
                    return Ok(Some(Problem::Invalid(
                        "returned reference depends on an unbound input carrier region".into(),
                    )));
                };
                if closure[region.0 as usize] & (1u64 << result.region.0) == 0 {
                    // A sourceless result tail is the rigid parameter's own
                    // carrier region (`fn get(&self) -> &T`, `T = &U`). A
                    // carrier nested inside the borrowed aggregate holds a
                    // value of that parameter, whose region outlives the outer
                    // borrow by well-formedness, so the result row covers it.
                    if !position.path.0.is_empty()
                        && return_inputs(&result.returns).next().is_none()
                    {
                        continue;
                    }
                    return Ok(Some(Problem::Invalid(
                        "reference return requires an undeclared universal outlives bound".into(),
                    )));
                }
            }
            for parent in self.lifetime_deps[origin].members() {
                if !dependencies.has(parent) {
                    dependencies.insert(parent);
                    queue.push(parent);
                }
            }
        }
        Ok(None)
    }

    fn scalar_return_footprint(
        f: &cfg::Function,
        target: &Target,
        result_position: ReferencePosition,
        facts: FactsCtx<'_>,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        let FactsCtx::Validated(authority) = facts else {
            return Ok(false);
        };
        let CanonicalRoot::ParameterReferent(local, position) = &target.place.root else {
            return Ok(false);
        };
        let Ok(Ty::Ref {
            referent: input, ..
        }) = ir::carrier_at(&f.locals[local.0].ty, position, authority.budget())
        else {
            return Ok(false);
        };
        let Ok((_, output)) = super::ir::reference_at(&f.result, result_position) else {
            return Ok(false);
        };
        budget.spend(target.place.projection.len() + 1)?;
        if let Err(error) = authority.validate_ty(output) {
            return if error.is_resource() {
                Err(Exhausted)
            } else {
                Ok(false)
            };
        }
        let anchor = match authority.projected_ty(&input, &target.place.projection) {
            Ok(anchor) => anchor,
            Err(error) if error.is_resource() => return Err(Exhausted),
            Err(_) => return Ok(false),
        };
        Ok(match (anchor, output) {
            (Ty::Int(a), Ty::Int(b)) => a == *b,
            (Ty::Float(a), Ty::Float(b)) => a == *b,
            (Ty::Char, Ty::Char) | (Ty::Bool, Ty::Bool) => true,
            _ => false,
        })
    }
    pub fn check(
        &self,
        f: &cfg::Function,
        target: &Target,
        access: Access,
        point: AccessPoint,
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let AccessPoint {
            block,
            statement,
            phase,
        } = point;
        if matches!(access, Access::CellRead | Access::CellWrite)
            && let Some(problem) = self.cell_storage_problem(f, target, access, point, budget)?
        {
            return Ok(Some(problem));
        }
        let point = self.point_data(block, statement, phase);
        let active = self.active(&point.origins, &point.live, budget)?;
        'loans: for loan in active.members() {
            budget.spend(target.authority.len() + 1)?;
            if target.authority.contains(&loan)
                || matches!(access, Access::ActivateMut(id) if id.0 == loan)
            {
                continue;
            }
            budget.spend(self.activations[block.0].len())?;
            let activating =
                statement.is_none() && self.activations[block.0].contains(&LoanId(loan));
            let reserved = self.two_phase.has(loan) && !point.activated.has(loan) && !activating;
            if matches!(access, Access::CellWrite) && self.kinds[loan] == BorrowKind::Shared {
                continue;
            }
            if matches!(
                access,
                Access::Read | Access::SharedBorrow | Access::CellRead
            ) && (self.kinds[loan] == BorrowKind::Shared || reserved)
            {
                continue;
            }
            if matches!(access, Access::ReserveMut) && self.kinds[loan] == BorrowKind::Shared {
                continue;
            }
            for borrowed in &self.targets[loan] {
                if overlap(&target.place, &borrowed.place, budget)? {
                    for authority in &target.authority {
                        if active.has(*authority)
                            && self.exclusive_origins(
                                f,
                                *authority,
                                loan,
                                AccessPoint {
                                    block,
                                    statement,
                                    phase,
                                },
                                budget,
                            )?
                        {
                            continue 'loans;
                        }
                    }
                    if self.disjoint_call_results(target, borrowed, loan, budget)? {
                        continue;
                    }
                    // Stock rustc classes for the same conflicts.
                    let code = match (access, self.kinds[loan]) {
                        (Access::StorageEnd, _) => {
                            "E0597: borrowed value does not live long enough"
                        }
                        (Access::Move, _) => "E0505: cannot move out of value while borrowed",
                        (Access::Write | Access::CellWrite, _) => {
                            "E0506: cannot assign to value while borrowed"
                        }
                        (_, BorrowKind::Shared) => {
                            "E0502: cannot borrow as mutable because it is also borrowed as immutable"
                        }
                        (Access::Read | Access::SharedBorrow | Access::CellRead, _) => {
                            "E0502: cannot borrow as immutable because it is also borrowed as mutable"
                        }
                        _ => "E0499: cannot borrow as mutable more than once at a time",
                    };
                    return Ok(Some(Problem::Conflict(format!(
                        "{code}: access conflicts with live {:?} loan {}",
                        self.kinds[loan], loan
                    ))));
                }
            }
        }
        Ok(None)
    }
    pub fn check_handle_move(
        &self,
        local: LocalId,
        block: BlockId,
        statement: Option<usize>,
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let (origins, live) = self.point(block, statement, Phase::Read);
        let mut moved = Set::new(self.targets.len());
        for (index, cell) in self.cells.iter().enumerate() {
            if matches!(cell.key.storage.root, CanonicalRoot::Owned(owner) if owner == local) {
                moved.union(&origins[index], budget)?;
            }
        }
        let active = self.active(origins, live, budget)?;
        for child in active.members() {
            let mut seen = self.parents[child].clone();
            let mut queue: Vec<usize> = seen.members().collect();
            while let Some(parent) = queue.pop() {
                budget.spend(1 + self.parents[parent].0.len())?;
                if moved.has(parent) {
                    return Ok(Some(Problem::Conflict(
                        "cannot move mutable reference handle while a descendant reborrow is live"
                            .into(),
                    )));
                }
                for ancestor in self.parents[parent].members() {
                    if !seen.has(ancestor) {
                        seen.insert(ancestor);
                        queue.push(ancestor);
                    }
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod parameter_equality_tests {
    use super::*;
    use crate::compiler::{frontend, ownership};

    #[test]
    fn source_family_regions_keep_exact_issued_component_pairings() {
        let location = |field| CarrierLocation {
            path: CarrierPath(Arc::from([
                CarrierStep::Referent,
                CarrierStep::SliceElement,
                CarrierStep::Field(field),
            ])),
            channel: CarrierChannel::Fixed(ReferencePosition { depth: 0 }),
        };
        let first = InputCarrier {
            param: 0,
            location: location(0),
        };
        let second = InputCarrier {
            param: 0,
            location: location(1),
        };
        let issued = [
            (first.clone(), AnalysisRegionId(0)),
            (second.clone(), AnalysisRegionId(1)),
        ];
        let regions = [
            (Some(location(0)), AnalysisRegionId(0)),
            (Some(location(1)), AnalysisRegionId(1)),
        ];
        assert!(source_family_region_is_issued(&issued, &first, &regions));
        assert!(source_family_region_is_issued(&issued, &second, &regions));
        assert!(!source_family_region_is_issued(&[], &first, &regions));
        assert!(!source_family_region_is_issued(
            &issued,
            &first,
            &regions[1..]
        ));
        assert!(!source_family_region_is_issued(
            &issued,
            &second,
            &regions[..1]
        ));
        let foreign_parameter = InputCarrier {
            param: 1,
            ..first.clone()
        };
        assert!(!source_family_region_is_issued(
            &issued,
            &foreign_parameter,
            &regions
        ));
        let foreign_field = InputCarrier {
            location: location(2),
            ..first.clone()
        };
        assert!(!source_family_region_is_issued(
            &issued,
            &foreign_field,
            &regions
        ));
        let foreign_channel = InputCarrier {
            location: CarrierLocation {
                channel: CarrierChannel::Fixed(ReferencePosition { depth: 1 }),
                ..first.location.clone()
            },
            ..first.clone()
        };
        assert!(!source_family_region_is_issued(
            &issued,
            &foreign_channel,
            &regions
        ));
        assert!(!source_family_region_is_issued(
            &issued,
            &first,
            &[(None, AnalysisRegionId(0))]
        ));
    }

    fn body_receipt_fixture() -> (cfg::TemplateCfg, Plan) {
        let template = source_template("fn probe<T>(x:T)->T{x}fn main(){}", 0);
        let mut budget = Budget::new(32_000_000);
        let plan = Plan::build_validated(
            template.function(),
            template.facts(),
            None,
            &mut budget,
            128 * 1024 * 1024,
        )
        .unwrap();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert!(plan.body_validation.is_some());
        (template, plan)
    }

    #[test]
    fn body_receipt_requires_exact_successful_constructor_witness() {
        let (template, plan) = body_receipt_fixture();
        let (foreign, other_plan) = body_receipt_fixture();
        let mut budget = Budget::new(32_000_000);
        assert!(
            BodyValidationReceipt::issue(
                &plan,
                None,
                template.function(),
                template.facts(),
                &mut budget,
                128 * 1024 * 1024
            )
            .unwrap()
            .is_none()
        );
        for control in 0..3 {
            let mut witness = BodyViewValidated {
                function: template.function(),
                facts: template.facts(),
                view: plan.body.as_ref().unwrap().clone(),
            };
            match control {
                0 => witness.function = foreign.function(),
                1 => witness.facts = foreign.facts(),
                2 => witness.view = other_plan.body.as_ref().unwrap().clone(),
                _ => unreachable!(),
            }
            assert!(
                BodyValidationReceipt::issue(
                    &plan,
                    Some(witness),
                    template.function(),
                    template.facts(),
                    &mut budget,
                    128 * 1024 * 1024
                )
                .unwrap()
                .is_none()
            );
        }
    }

    #[test]
    fn body_receipt_changed_constructor_signature_never_issues_reuse() {
        let (template, _) = body_receipt_fixture();
        let mut changed = template.function().as_ref().clone();
        changed.result = Ty::Bool;
        let result = Plan::build_validated(
            &Arc::new(changed),
            template.facts(),
            None,
            &mut Budget::new(32_000_000),
            128 * 1024 * 1024,
        );
        if let Ok(plan) = result {
            assert!(!plan.problems.is_empty());
            assert!(plan.body_validation.is_none());
        }
    }

    #[test]
    fn body_receipt_exact_foreign_signature_location_and_missing_controls() {
        let (template, mut plan) = body_receipt_fixture();
        let receipt = plan.body_validation.as_ref().unwrap();
        let body = plan.body.as_ref().unwrap();
        let signature = plan.body_signature.as_ref().unwrap();
        let location = &body.contract().results[0].location;
        assert!(
            receipt
                .check_result(
                    template.function(),
                    body,
                    template.facts(),
                    signature,
                    location
                )
                .unwrap()
                .is_none()
        );
        // Another allocation with identical content must take the full path.
        let copied_view = Arc::new(body.as_ref().clone());
        assert!(
            receipt
                .check_result(
                    template.function(),
                    &copied_view,
                    template.facts(),
                    signature,
                    location
                )
                .unwrap()
                .is_none()
        );
        let copied_function = template.function().as_ref().clone();
        assert!(matches!(
            receipt.check_result(
                &copied_function,
                body,
                template.facts(),
                signature,
                location
            ),
            Ok(Some(Problem::Invalid(_)))
        ));
        let (foreign, _) = body_receipt_fixture();
        assert!(matches!(
            receipt.check_result(
                template.function(),
                body,
                foreign.facts(),
                signature,
                location
            ),
            Ok(Some(Problem::Invalid(_)))
        ));
        let mut changed_signature = signature.clone();
        changed_signature.result = Ty::Bool;
        assert!(matches!(
            receipt.check_result(
                template.function(),
                body,
                template.facts(),
                &changed_signature,
                location
            ),
            Ok(Some(Problem::Invalid(_)))
        ));
        let mut changed_location = location.clone();
        changed_location.path = CarrierPath(Arc::from([CarrierStep::Field(usize::MAX)]));
        assert!(matches!(
            receipt.check_result(
                template.function(),
                body,
                template.facts(),
                signature,
                &changed_location
            ),
            Ok(Some(Problem::Invalid(_)))
        ));
        let block = template
            .function()
            .blocks
            .iter()
            .position(|block| matches!(block.terminator.value, Terminator::Return))
            .unwrap();
        let mut changed_function = template.function().as_ref().clone();
        changed_function.result = Ty::Bool;
        assert!(matches!(
            plan.check_return(
                &changed_function,
                BlockId(block),
                FactsCtx::Validated(template.facts()),
                &mut Budget::new(32_000_000)
            ),
            Ok(Some(Problem::Invalid(_)))
        ));
        // Receipt absence is not permission and must keep the original full guard.
        plan.body_validation = None;
        assert!(
            plan.check_return(
                template.function(),
                BlockId(block),
                FactsCtx::Validated(template.facts()),
                &mut Budget::new(32_000_000)
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn body_receipt_forged_binding_and_live_short_poison_preserve_payload() {
        for control in 0..4 {
            let (template, mut plan) = body_receipt_fixture();
            let facts = template.facts();
            let ledger = facts.budget();
            let before = ledger.usage_snapshot();
            assert!(before.first_failure.is_none());
            assert!(!before.poisoned);
            let (foreign, _) = body_receipt_fixture();
            if control == 1 {
                // Change only one private binding, never grant another view
                // authority merely by pointing a receipt at fresh facts.
                plan.body_validation.as_mut().unwrap().facts = foreign.facts().clone();
            }
            if control == 2 {
                // Exact immutable replay allocates no scratch. Exhaust its
                // actual metered work rather than assuming reconstruction
                // must allocate declaration metadata again.
                ledger
                    .charge_work(ledger.limits().work - before.work)
                    .unwrap();
                let full = ledger.usage_snapshot();
                assert!(full.first_failure.is_none());
                assert!(!full.poisoned);
                assert_eq!(full.work, ledger.limits().work);
            } else if control == 3 {
                ledger.poison_ledger_for_tests();
                assert!(ledger.usage_snapshot().poisoned);
            }
            let receipt = plan.body_validation.as_ref().unwrap();
            let body = plan.body.as_ref().unwrap();
            let signature = plan.body_signature.as_ref().unwrap();
            let result = receipt.check_result(
                template.function(),
                body,
                facts,
                signature,
                &body.contract().results[0].location,
            );
            match control {
                0 => assert!(result.unwrap().is_none()),
                1 => assert!(matches!(result, Ok(Some(Problem::Invalid(_))))),
                2 => {
                    assert!(result.is_err());
                    let failure = ledger.first_resource_failure().unwrap();
                    assert_eq!(failure.kind, ir::ResourceBudgetKind::Work);
                    assert_eq!(failure.used, failure.limit);
                }
                3 => {
                    assert!(result.is_err());
                    // Even a supplied fresh payer cannot replace the original.
                    assert!(
                        receipt
                            .check_result(
                                template.function(),
                                body,
                                foreign.facts(),
                                signature,
                                &body.contract().results[0].location
                            )
                            .is_err()
                    );
                }
                _ => unreachable!(),
            }
            drop(plan);
            // Neither receipt drop nor the new reuse refunds retained evidence.
            assert!(ledger.usage_snapshot().bytes >= before.bytes);
            assert!(
                ledger.usage_snapshot().reservations.retentions >= before.reservations.retentions
            );
        }
    }

    #[test]
    fn effects_payload_counts_nested_rows_and_shared_path_occurrences() {
        let count = |writes: &[ir::CarrierWriteEnvelope], must: &[InputCarrier]| {
            carrier_effects_payload_bytes(writes, must, &[], &mut Budget::new(1_000)).unwrap()
        };
        assert_eq!(
            count(&[], &[]),
            size_of::<CarrierEffects>() + usize::BITS as usize
        );
        let input = InputCarrier {
            param: 0,
            location: CarrierLocation {
                path: CarrierPath(Arc::from([CarrierStep::Field(0), CarrierStep::Referent])),
                channel: CarrierChannel::Fixed(ReferencePosition { depth: 0 }),
            },
        };
        let writes = [ir::CarrierWriteEnvelope {
            destination: input.clone(),
            sources: Arc::from([input.clone()]),
        }];
        // Root, three outer slices and one source slice have five Arc headers.
        // The same path appears in destination, source and must-overwrite, and
        // is conservatively covered three times without identity-based reuse.
        let expected = size_of::<CarrierEffects>()
            + size_of::<ir::CarrierWriteEnvelope>()
            + 2 * size_of::<InputCarrier>()
            + 10 * size_of::<usize>()
            + 3 * (2 * size_of::<usize>() + 2 * size_of::<CarrierStep>());
        assert_eq!(count(&writes, &[input]), expected);
    }

    #[test]
    fn opaque_backing_summary_stays_incomplete_without_invented_cells() {
        let locals = [Ty::Int(ir::IntTy::I32)];
        let resolve = |index| {
            resolve_cells(
                &locals,
                &locals,
                &[],
                &HashMap::new(),
                &[],
                |_| panic!("no reference cell exists"),
                &Place {
                    local: LocalId(0),
                    projection: vec![Projection::DynamicIndex(index)],
                },
                &mut Budget::new(1_000_000),
            )
        };
        let (targets, origins, failure) = resolve(LocalId(usize::MAX)).unwrap();
        assert!(targets.is_empty());
        assert!(origins.is_empty());
        assert_eq!(failure, Some(ResolutionFailure::IncompleteCover));
        assert!(resolve(LocalId(0)).is_err());
    }

    #[test]
    fn original_parameter_slice_probe_requires_canonical_issued_carrier() {
        for (source, main_index, callee_suffix, input_path) in [
            (
                include_str!("../../tests/fixtures/full_programs/13_trait_objects.rs"),
                1,
                "max_by",
                vec![CarrierStep::Field(0)],
            ),
            (
                "use std::io::BufReader;fn main(){let bytes:&[u8]=&[1u8,2,3];let _reader=BufReader::new(bytes);}",
                0,
                "new",
                vec![],
            ),
        ] {
            let snapshot = Arc::new(crate::compiler::Snapshot {
                input: crate::compiler::Input::Text(source.into()),
                source_options: crate::compiler::source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let ledger = snapshot.budget.clone();
            struct ActualMaxByBody(crate::compiler::CompilerProvider, usize, &'static str);
            impl crate::engine::Provider for ActualMaxByBody {
                fn outputs(&self, key: &crate::engine::Key) -> usize {
                    if key.kind == u32::MAX {
                        1
                    } else {
                        crate::engine::Provider::outputs(&self.0, key)
                    }
                }
                fn start(
                    &self,
                    cx: crate::engine::Context,
                    key: crate::engine::Key,
                ) -> crate::engine::TaskFuture {
                    if key.kind != u32::MAX {
                        return crate::engine::Provider::start(&self.0, cx, key);
                    }
                    let ledger = self.0.0.budget.clone();
                    let main_index = self.1;
                    let callee_suffix = self.2;
                    Box::pin(async move {
                        let signatures = cx
                            .request(crate::compiler::keys::SignaturesDebtRoot.key())
                            .get(0);
                        let main = cx
                            .request(
                                crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(
                                    main_index,
                                )
                                .key(),
                            )
                            .get(0);
                        let plan: Arc<crate::compiler::frontend::PlanCtx> =
                            cx.read_fact(signatures).await?;
                        let main: Arc<crate::compiler::TemplateBodyFact> =
                            cx.read_fact(main).await?;
                        let facts = plan
                            .instance_facts(&ir::CallTarget {
                                declaration: plan.function_decl(main_index).unwrap(),
                                type_args: Arc::from([]),
                            })
                            .unwrap();
                        let callee = main
                            .checked
                            .exprs
                            .iter()
                            .find_map(|expression| {
                                let ir::ExprKind::Call { target, name, .. } = &expression.kind
                                else {
                                    return None;
                                };
                                name.ends_with(callee_suffix).then_some(target.clone())
                            })
                            .expect("unchanged original max_by call");
                        let target = match callee {
                            ir::Callee::Direct(target) => target,
                            ir::Callee::TraitItem(call) => {
                                let selected = ir::TraitCx::resolve_trait_call(
                                    facts.world().as_ref(),
                                    &call,
                                    &facts,
                                )
                                .unwrap();
                                let ir::Callee::Direct(target) = selected.callee() else {
                                    panic!("max_by selected non-direct body")
                                };
                                target.clone()
                            }
                            _ => panic!("unexpected original max_by callee"),
                        };
                        let body_key = crate::compiler::keys::words(
                            crate::compiler::CANONICAL_BODY,
                            crate::compiler::instances::graph_key::encode(&target, &ledger)
                                .unwrap(),
                        )
                        .unwrap();
                        let body = cx.request(body_key).get(0);
                        let body: Arc<crate::compiler::InstanceBodyFact> =
                            cx.read_fact(body).await?;
                        cx.publish_fact(
                            0,
                            crate::compiler::InstanceBodyFact {
                                facts: body.facts.clone(),
                                checked: body.checked.clone(),
                            },
                        )
                    })
                }
            }
            let engine = crate::compiler::Engine::new(ActualMaxByBody(
                crate::compiler::CompilerProvider(snapshot),
                main_index,
                callee_suffix,
            ));
            let body_handle = engine.request(crate::compiler::keys::test_key(u32::MAX, vec![0]));
            engine.run(1).unwrap();
            body_handle.completion().unwrap();
            let body = body_handle
                .fact::<crate::compiler::InstanceBodyFact>(0)
                .unwrap();
            let function =
                Arc::new(cfg::lower_with_facts(&body.checked, &body.facts, &ledger).unwrap());
            assert!(function.body_substitution.is_some());
            let mut budget = Budget::new(32_000_000);
            let mut builder = CarrierBuilder::new(
                &function,
                FactsCtx::Validated(&body.facts),
                ledger,
                &mut budget,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            let (input, origin) = builder
                .reference_inputs
                .iter()
                .find(|(input, _)| {
                    input.param == 0 && input.location.path.0.as_ref() == input_path.as_slice()
                })
                .cloned()
                .expect("original parameter family issued descriptor");
            let target = builder.targets[origin.0][0].clone();
            let original_root = &builder.original_locals[function.params[0].0];
            assert!(matches!(original_root, Ty::Param(_)));
            let path = value_projection(&input.location);
            let shape =
                probe_projected_shape(original_root, &path, &mut Budget::new(1_000_000)).unwrap();
            if input_path.is_empty() {
                assert!(matches!(shape, Some(Ty::Param(_))));
            } else {
                assert!(shape.is_none());
            }
            let result = function.result.clone();
            let result_location = ir::carrier_locations(&result, &builder.ledger)
                .unwrap()
                .iter()
                .find(|location| {
                    matches!(
                        ir::carrier_at(&result, location, &builder.ledger).unwrap(),
                        Ty::Ref {
                            kind: BorrowKind::Shared,
                            ..
                        }
                    )
                })
                .cloned()
                .expect("original max_by shared result carrier");
            let kept = builder
                .structural_result_target(target.clone(), &result, &result_location)
                .unwrap();
            assert_eq!(kept, target);
            let mut foreign = target.clone();
            foreign.place.root =
                CanonicalRoot::ParameterReferent(LocalId(usize::MAX), input.location.clone());
            assert!(
                builder
                    .structural_result_target(foreign, &result, &result_location)
                    .is_err()
            );
            let mut forged = target.clone();
            let mut wrong_source = input.location.clone();
            wrong_source.path = CarrierPath(Arc::from([CarrierStep::Field(1)]));
            forged.place.root = CanonicalRoot::ParameterReferent(function.params[0], wrong_source);
            assert!(
                builder
                    .structural_result_target(forged, &result, &result_location)
                    .is_err()
            );
            let issued = builder.reference_inputs.clone();
            builder.reference_inputs.clear();
            assert!(
                builder
                    .structural_result_target(target.clone(), &result, &result_location)
                    .is_err()
            );
            builder.reference_inputs = issued;
            // Exhaust the local ownership budget used by the applicability probe.
            // This path does not request new schema facts from the separate ledger.
            *builder.budget = Budget::new(0);
            assert!(
                builder
                    .structural_result_target(target, &result, &result_location)
                    .is_err()
            );
        }
    }

    #[test]
    fn parameter_result_widening_retains_original_issued_cover() {
        for (source, source_kind, field) in [
            (
                "fn inspect(values:&[&i32]){let _=values;}fn main(){}",
                BorrowKind::Shared,
                None,
            ),
            (
                "fn inspect(values:&mut [&i32]){let _=values;}fn main(){}",
                BorrowKind::Mut,
                None,
            ),
            (
                "fn inspect(values:&Vec<&i32>){let _=values;}fn main(){}",
                BorrowKind::Shared,
                None,
            ),
            (
                "fn inspect(values:&mut Vec<&i32>){let _=values;}fn main(){}",
                BorrowKind::Mut,
                None,
            ),
            (
                "fn inspect(values:&mut [&i32;2]){let _=values;}fn main(){}",
                BorrowKind::Mut,
                None,
            ),
            (
                "fn inspect(values:&mut (&i32,&i32)){let _=values;}fn main(){}",
                BorrowKind::Mut,
                None,
            ),
            (
                "fn inspect(values:&mut (Vec<&i32>,Vec<&i32>)){let _=values;}fn main(){}",
                BorrowKind::Mut,
                Some(1),
            ),
            (
                "fn inspect(values:&(Vec<&i32>,Vec<&i32>)){let _=values;}fn main(){}",
                BorrowKind::Shared,
                Some(0),
            ),
        ] {
            let snapshot = Arc::new(crate::compiler::Snapshot {
                input: crate::compiler::Input::Text(source.into()),
                source_options: crate::compiler::source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let ledger = snapshot.budget.clone();
            let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
            let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(0).key();
            let requested = engine.request(key.clone());
            engine.run(1).unwrap();
            requested.completion().unwrap();
            let body = engine
                .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
                .unwrap();
            let publication = body
                .publication
                .as_ref()
                .expect("genuine original slice parameter");
            let function = Arc::new(cfg::lower_source_closed(publication, &ledger).unwrap());
            let facts = publication.facts().clone();
            let mut budget = Budget::new(32_000_000);
            let mut builder = CarrierBuilder::new(
                &function,
                FactsCtx::Validated(&facts),
                ledger,
                &mut budget,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            let location = fixed_location(ReferencePosition { depth: 0 });
            let (_, origin) = builder
                .reference_inputs
                .iter()
                .find(|(input, _)| input.param == 0 && input.location == location)
                .unwrap();
            let mut target = builder.targets[origin.0][0].clone();
            if let Some(field) = field {
                target.place.projection.push(Projection::Field(field));
            }
            let element = Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(Ty::Int(ir::IntTy::I32)),
            };
            let result = Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(element.clone()),
            };
            let widened = builder
                .structural_result_target(target.clone(), &result, &location)
                .unwrap();
            assert_eq!(widened.precision, TargetPrecision::Subtree);
            assert_eq!(widened.place, target.place);
            assert_eq!(widened.authority, target.authority);
            let covers = builder
                .family_cover_locals
                .as_deref()
                .unwrap_or(&builder.original_locals);
            let complete = read_cell_cover(
                covers,
                &builder.cell_index,
                &widened,
                &CellChannel::ReferenceValue,
                &mut Budget::new(1_000_000),
            )
            .unwrap();
            assert!(complete.is_some_and(|cells| !cells.is_empty()));
            assert!(
                read_cell_cover(
                    covers,
                    &HashMap::new(),
                    &widened,
                    &CellChannel::ReferenceValue,
                    &mut Budget::new(1_000_000)
                )
                .unwrap()
                .is_none()
            );
            let mut foreign = target.clone();
            foreign.place.root =
                CanonicalRoot::ParameterReferent(LocalId(usize::MAX), location.clone());
            assert!(
                builder
                    .structural_result_target(foreign, &result, &location)
                    .is_err()
            );
            let mut forged = target.clone();
            let mut wrong_location = location.clone();
            wrong_location.path = CarrierPath(Arc::from([CarrierStep::Field(0)]));
            forged.place.root =
                CanonicalRoot::ParameterReferent(function.params[0], wrong_location);
            assert!(
                builder
                    .structural_result_target(forged, &result, &location)
                    .is_err()
            );
            let original = ir::carrier_at(
                &function.locals[function.params[0].0].ty,
                &location,
                &builder.ledger,
            )
            .unwrap();
            let Ty::Ref { kind, referent } = original else {
                panic!("issued parameter reference");
            };
            let same = Ty::Ref {
                kind,
                referent: Arc::new(
                    probe_projected_shape(&referent, &target.place.projection, builder.budget)
                        .unwrap()
                        .unwrap(),
                ),
            };
            assert_eq!(
                builder
                    .structural_result_target(target.clone(), &same, &location)
                    .unwrap()
                    .precision,
                TargetPrecision::Exact
            );
            let mutable = Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(element),
            };
            assert_eq!(
                builder
                    .structural_result_target(target, &mutable, &location)
                    .unwrap()
                    .precision,
                if source_kind == BorrowKind::Mut {
                    TargetPrecision::Subtree
                } else {
                    TargetPrecision::Exact
                }
            );
        }
    }

    #[test]
    fn slice_summary_cover_requires_the_issued_cell_and_original_domain() {
        let element = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Int(ir::IntTy::I32)),
        };
        let locals = vec![Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Slice {
                element: Arc::new(element.clone()),
            }),
        }];
        let root = CanonicalRoot::ParameterReferent(
            LocalId(0),
            fixed_location(ReferencePosition { depth: 0 }),
        );
        let target = Target {
            place: CanonicalPlace {
                root: root.clone(),
                projection: vec![],
            },
            precision: TargetPrecision::Exact,
            authority: vec![],
        };
        let key = CellKey {
            storage: CanonicalPlace {
                root,
                projection: vec![Projection::DynamicIndex(LocalId(usize::MAX))],
            },
            channel: CellChannel::ReferenceValue,
        };
        let mut cells = HashMap::from([(key.clone(), 17)]);
        let read = |locals: &[Ty], cells: &HashMap<CellKey, usize>, target: &Target| {
            read_cell_cover(
                locals,
                cells,
                target,
                &CellChannel::ReferenceValue,
                &mut Budget::new(1_000_000),
            )
            .unwrap()
        };
        assert_eq!(read(&locals, &cells, &target), Some(vec![17]));
        cells.clear();
        assert_eq!(read(&locals, &cells, &target), None);
        let mut forged = key.clone();
        forged.storage.projection = vec![Projection::ConstantIndex(0)];
        cells.insert(forged, 17);
        assert_eq!(read(&locals, &cells, &target), None);
        cells.insert(key, 17);
        let arrays = vec![Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Array {
                element: Arc::new(element),
                len: 1,
            }),
        }];
        assert_eq!(read(&arrays, &cells, &target), None);
        let scalars = vec![Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Slice {
                element: Arc::new(Ty::Int(ir::IntTy::I32)),
            }),
        }];
        assert_eq!(read(&scalars, &cells, &target), None);
        let mut foreign = target.clone();
        foreign.place.root = CanonicalRoot::ParameterReferent(
            LocalId(1),
            fixed_location(ReferencePosition { depth: 0 }),
        );
        assert_eq!(read(&locals, &cells, &foreign), None);
    }

    #[test]
    fn heap_subtree_cover_requires_original_issued_reference_cells() {
        let snapshot = Arc::new(crate::compiler::Snapshot {
            input: crate::compiler::Input::Text(
                "fn probe(values:&mut Vec<&'static str>,shared:&Vec<&'static str>,nested:&mut Vec<&&'static str>){}fn main(){}".into(),
            ),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let budget = snapshot.budget.clone();
        let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
        let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(0).key();
        let requested = engine.request(key.clone());
        engine.run(1).unwrap();
        requested.completion().unwrap();
        let body = engine
            .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
            .unwrap();
        let publication = body
            .publication
            .as_ref()
            .expect("genuine source publication");
        let function = match publication.facts().phase() {
            crate::compiler::traits::FactsPhase::Universal => {
                cfg::lower_source_template(publication, &budget)
                    .unwrap()
                    .function()
                    .clone()
            }
            crate::compiler::traits::FactsPhase::Closed => {
                Arc::new(cfg::lower_source_closed(publication, &budget).unwrap())
            }
            crate::compiler::traits::FactsPhase::Schema => {
                panic!("expected executable source facts")
            }
        };
        let locals: Vec<_> = function
            .locals
            .iter()
            .map(|local| local.ty.clone())
            .collect();
        let slot = CarrierLocation {
            path: CarrierPath(Arc::from([
                CarrierStep::Referent,
                CarrierStep::SliceElement,
            ])),
            channel: CarrierChannel::Fixed(ReferencePosition { depth: 0 }),
        };
        let view = publication
            .facts()
            .carrier_contract(ir::SignatureSubject::Body)
            .unwrap();
        let original = view
            .source_region_env()
            .carriers
            .iter()
            .find(|row| row.parameter == Some(0) && row.location == slot)
            .expect("original typed sequence input region");
        let issued = InputCarrier {
            param: 0,
            location: slot.clone(),
        };
        assert_eq!(
            source_region_input(&view, original, &mut Budget::new(1_000_000)).unwrap(),
            Some(issued)
        );
        for control in 0..3 {
            let mut forged = original.clone();
            match control {
                0 => forged.parameter = Some(usize::MAX),
                1 => forged.parameter = None,
                2 => forged.location.path = CarrierPath(Arc::from([CarrierStep::Field(0)])),
                _ => unreachable!(),
            }
            assert!(
                source_region_input(&view, &forged, &mut Budget::new(1_000_000))
                    .unwrap()
                    .is_none()
            );
        }
        let world = publication.facts().world();
        assert!(
            world
                .carrier_writable_for_tests(&locals[function.params[0].0], &slot)
                .unwrap()
        );
        assert!(
            !world
                .carrier_writable_for_tests(&locals[function.params[1].0], &slot)
                .unwrap()
        );
        let mut nested_slot = slot.clone();
        nested_slot.channel = CarrierChannel::Fixed(ReferencePosition { depth: 1 });
        assert!(
            !world
                .carrier_writable_for_tests(&locals[function.params[2].0], &nested_slot)
                .unwrap()
        );
        assert!(
            !world
                .carrier_writable_for_tests(&locals[function.params[0].0], &nested_slot)
                .unwrap()
        );
        let local = function.params[0];
        let root =
            CanonicalRoot::ParameterReferent(local, fixed_location(ReferencePosition { depth: 0 }));
        let target = Target {
            place: CanonicalPlace {
                root: root.clone(),
                projection: vec![],
            },
            precision: TargetPrecision::Subtree,
            authority: vec![],
        };
        let key = CellKey {
            storage: CanonicalPlace {
                root,
                projection: vec![Projection::DynamicIndex(LocalId(usize::MAX))],
            },
            channel: CellChannel::ReferenceValue,
        };
        let read = |locals: &[Ty], cells: &HashMap<CellKey, usize>, target: &Target| {
            read_cell_cover(
                locals,
                cells,
                target,
                &CellChannel::ReferenceValue,
                &mut Budget::new(1_000_000),
            )
            .unwrap()
        };
        let cells = HashMap::from([(key.clone(), 17)]);
        assert_eq!(read(&locals, &cells, &target), Some(vec![17]));
        assert_eq!(read(&locals, &HashMap::new(), &target), None);
        for projection in [
            vec![Projection::ConstantIndex(0)],
            vec![Projection::DynamicIndex(LocalId(0))],
            vec![Projection::Field(0)],
        ] {
            let mut forged = key.clone();
            forged.storage.projection = projection;
            assert_eq!(read(&locals, &HashMap::from([(forged, 17)]), &target), None);
        }
        let mut foreign = target.clone();
        foreign.place.root = CanonicalRoot::Owned(local);
        assert_eq!(read(&locals, &cells, &foreign), None);
        let mut wrong_original = locals.clone();
        wrong_original[local.0] = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(Ty::Slice {
                element: Arc::new(Ty::Int(ir::IntTy::I32)),
            }),
        };
        assert_eq!(read(&wrong_original, &cells, &target), Some(vec![]));
    }

    pub(super) fn source_template(source: &str, index: usize) -> cfg::TemplateCfg {
        let program = frontend::parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(index).unwrap();
        let frontend::PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected genuine source publication")
        };
        cfg::lower_source_template(&publication, &budget).unwrap()
    }

    #[test]
    fn independent_slice_pair_rows_reject_forged_families() {
        let program = frontend::parse("fn pair<T,U>(){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let parameters: Vec<_> = facts.env().ordered_params().collect();
        let fixed = CarrierChannel::Fixed(ReferencePosition { depth: 0 });
        let input = |param| InputCarrier {
            param,
            location: CarrierLocation {
                path: CarrierPath(Arc::from([])),
                channel: fixed.clone(),
            },
        };
        let family = |param| InputCarrier {
            param,
            location: CarrierLocation {
                path: CarrierPath(Arc::from([
                    CarrierStep::Referent,
                    CarrierStep::SliceElement,
                ])),
                channel: CarrierChannel::ParamTail {
                    parameter: parameters[0],
                    prefix: 0,
                },
            },
        };
        let rows: Vec<_> = (0..2)
            .flat_map(|index| {
                [
                    ir::CarrierReturn {
                        location: CarrierLocation {
                            path: CarrierPath(Arc::from([CarrierStep::Field(index)])),
                            channel: fixed.clone(),
                        },
                        sources: Arc::from([input(index)]),
                    },
                    ir::CarrierReturn {
                        location: CarrierLocation {
                            path: CarrierPath(Arc::from([
                                CarrierStep::Field(index),
                                CarrierStep::Referent,
                                CarrierStep::SliceElement,
                            ])),
                            channel: family(index).location.channel,
                        },
                        sources: Arc::from([family(index)]),
                    },
                ]
            })
            .collect();
        let original = ir::CarrierContract {
            inputs: Arc::from([input(0), family(0), input(1), family(1)]),
            results: rows.into(),
            writes: Arc::from([]),
        };
        let check = |contract: &ir::CarrierContract| {
            independent_slice_pair_rows(contract, &mut Budget::new(1_000_000)).unwrap()
        };
        assert_eq!(
            check(&original),
            Some(([0, 1], [Some(parameters[0]), Some(parameters[0])]))
        );
        let mut union = original.clone();
        let mut union_rows = union.results.to_vec();
        for row in &mut union_rows {
            let mut sources = row.sources.to_vec();
            for candidate in [family(0), family(1)] {
                if !sources.contains(&candidate) {
                    sources.push(candidate);
                }
            }
            row.sources = sources.into();
        }
        union.results = union_rows.into();
        assert_eq!(check(&union), check(&original));
        for mode in 0..11 {
            let mut forged = original.clone();
            let mut rows = forged.results.to_vec();
            match mode {
                0 => rows[1].sources = Arc::from([family(1)]),
                1 => rows[1].sources = Arc::from([]),
                2 => {
                    rows[1].location.channel = CarrierChannel::ParamTail {
                        parameter: parameters[1],
                        prefix: 0,
                    }
                }
                3 => {
                    rows[1].location.channel = CarrierChannel::ParamTail {
                        parameter: parameters[1],
                        prefix: 0,
                    };
                    let mut source = family(0);
                    source.location.channel = rows[1].location.channel.clone();
                    rows[1].sources = Arc::from([source]);
                }
                4 => {
                    rows[1].location.path = CarrierPath(Arc::from([
                        CarrierStep::Field(1),
                        CarrierStep::Referent,
                        CarrierStep::SliceElement,
                    ]))
                }
                5 => {
                    rows[1].location.channel = CarrierChannel::ParamTail {
                        parameter: parameters[0],
                        prefix: 1,
                    }
                }
                6 => rows[2].sources = rows[0].sources.clone(),
                7 => rows.swap(0, 2),
                8 => rows[0].sources = Arc::from([input(0), input(1), family(0)]),
                9 => rows[0].sources = Arc::from([input(0), family(2)]),
                _ => {
                    let mut unknown = family(0);
                    unknown.location.channel = CarrierChannel::ParamTail {
                        parameter: parameters[1],
                        prefix: 0,
                    };
                    rows[0].sources = Arc::from([input(0), unknown]);
                }
            }
            forged.results = rows.into();
            assert!(check(&forged).is_none(), "forged mode {mode}");
        }
        assert!(independent_slice_pair_rows(&original, &mut Budget::new(0)).is_err());
    }

    #[test]
    fn closed_identity_effect_rows_reject_forged_or_incomplete_transport() {
        let input = |field| InputCarrier {
            param: 0,
            location: CarrierLocation {
                path: ir::CarrierPath(Arc::from([
                    CarrierStep::Referent,
                    CarrierStep::Field(field),
                ])),
                channel: CarrierChannel::Fixed(ReferencePosition { depth: 0 }),
            },
        };
        let rows = [input(0), input(1)].map(|destination| ir::CarrierWriteEnvelope {
            sources: Arc::from([destination.clone()]),
            destination,
        });
        let declared = ir::CarrierContract {
            inputs: Arc::from([input(0), input(1)]),
            results: Arc::from([]),
            writes: Arc::from(rows.clone()),
        };
        let admitted = |contract: &ir::CarrierContract,
                        rows: &[ir::CarrierWriteEnvelope],
                        must: &[InputCarrier]| {
            identity_effect_rows(contract, rows, must, &mut Budget::new(1_000_000)).unwrap()
        };
        assert!(admitted(&declared, &rows, &[]));
        assert!(!admitted(&declared, &rows[..1], &[]));
        assert!(!admitted(&declared, &rows, &[input(0)]));
        let mut changed = rows.clone();
        changed[1] = changed[0].clone();
        assert!(!admitted(&declared, &changed, &[]));
        let mut duplicate = declared.clone();
        duplicate.writes = Arc::from(changed.clone());
        assert!(!admitted(&duplicate, &changed, &[]));
        changed = rows.clone();
        changed[0].sources = Arc::from([]);
        assert!(!admitted(&declared, &changed, &[]));
        changed = rows.clone();
        changed[0].sources = Arc::from([input(1)]);
        assert!(!admitted(&declared, &changed, &[]));
        changed = rows.clone();
        changed[0].sources = Arc::from([input(0), input(1)]);
        assert!(!admitted(&declared, &changed, &[]));
        changed = rows.clone();
        changed.swap(0, 1);
        assert!(!admitted(&declared, &changed, &[]));
        let mut unauthorized = declared.clone();
        let mut envelopes = unauthorized.writes.to_vec();
        envelopes[0].sources = Arc::from([]);
        unauthorized.writes = envelopes.into();
        assert!(!admitted(&unauthorized, &rows, &[]));
        let mut with_result = declared.clone();
        with_result.results = Arc::from([ir::CarrierReturn {
            location: input(0).location,
            sources: Arc::from([input(0)]),
        }]);
        assert!(!admitted(&with_result, &rows, &[]));
        let mut summary = rows.clone();
        summary[0].destination.location.path =
            ir::CarrierPath(Arc::from([CarrierStep::SliceElement]));
        summary[0].sources = Arc::from([summary[0].destination.clone()]);
        let mut summary_contract = declared.clone();
        summary_contract.writes = Arc::from(summary.clone());
        assert!(!admitted(&summary_contract, &summary, &[]));
        assert!(identity_effect_rows(&declared, &rows, &[], &mut Budget::new(0)).is_err());
    }

    #[test]
    fn closed_shared_aggregate_identity_rows_reject_incomplete_or_replaced_cells() {
        let input = |field| InputCarrier {
            param: 0,
            location: CarrierLocation {
                path: CarrierPath(Arc::from([
                    CarrierStep::Referent,
                    CarrierStep::Field(field),
                ])),
                channel: CarrierChannel::Fixed(ReferencePosition { depth: 0 }),
            },
        };
        let rows = [input(0), input(1)].map(|destination| ir::CarrierWriteEnvelope {
            sources: Arc::from([destination.clone()]),
            destination,
        });
        let declared = ir::CarrierContract {
            inputs: Arc::from([input(0), input(1)]),
            results: Arc::from([ir::CarrierReturn {
                location: fixed_location(ReferencePosition { depth: 0 }),
                sources: Arc::from([input(0), input(1)]),
            }]),
            writes: Arc::from(rows.clone()),
        };
        let admitted = |contract: &ir::CarrierContract,
                        rows: &[ir::CarrierWriteEnvelope],
                        must: &[InputCarrier]| {
            aggregate_reborrow::computed_identity_writes(
                contract,
                rows,
                must,
                &mut Budget::new(1_000_000),
            )
            .unwrap()
        };
        assert!(admitted(&declared, &rows, &[]));
        assert!(!admitted(&declared, &rows[..1], &[]));
        assert!(!admitted(&declared, &rows, &[input(0)]));
        for mutation in 0..6 {
            let mut changed = rows.clone();
            match mutation {
                0 => changed[0].sources = Arc::from([]),
                1 => changed[0].sources = Arc::from([input(1)]),
                2 => changed[0].sources = Arc::from([input(0), input(1)]),
                3 => changed[1] = changed[0].clone(),
                4 => changed.swap(0, 1),
                5 => changed[0].destination.param = 1,
                _ => unreachable!(),
            }
            assert!(!admitted(&declared, &changed, &[]), "mutation {mutation}");
        }
        let mut duplicate = declared.clone();
        duplicate.writes = Arc::from([rows[0].clone(), rows[0].clone()]);
        assert!(!admitted(&duplicate, &rows, &[]));
        for path in [
            vec![],
            vec![CarrierStep::Field(0)],
            vec![CarrierStep::Referent],
            vec![CarrierStep::Referent, CarrierStep::SliceElement],
            vec![
                CarrierStep::Referent,
                CarrierStep::Referent,
                CarrierStep::Field(0),
            ],
        ] {
            let location = CarrierLocation {
                path: CarrierPath(path.into()),
                channel: CarrierChannel::Fixed(ReferencePosition { depth: 0 }),
            };
            assert!(!aggregate_reborrow::fixed_shared_aggregate_location(
                &location
            ));
        }
        assert!(aggregate_reborrow::fixed_shared_aggregate_location(
            &input(0).location
        ));
        let mut nested_depth = input(0).location;
        nested_depth.channel = CarrierChannel::Fixed(ReferencePosition { depth: 1 });
        assert!(!aggregate_reborrow::fixed_shared_aggregate_location(
            &nested_depth
        ));
        assert!(
            aggregate_reborrow::computed_identity_writes(
                &declared,
                &rows,
                &[],
                &mut Budget::new(0)
            )
            .is_err()
        );
    }

    fn shared_aggregate_body_fixture(
        source: &str,
    ) -> (
        Arc<cfg::Function>,
        Arc<super::super::traits::ValidatedFacts>,
    ) {
        let snapshot = Arc::new(crate::compiler::Snapshot {
            input: crate::compiler::Input::Text(source.into()),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let ledger = snapshot.budget.clone();
        let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
        let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(0).key();
        let requested = engine.request(key.clone());
        engine.run(1).unwrap();
        requested.completion().unwrap();
        let body = engine
            .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
            .unwrap();
        let publication = body.publication.as_ref().expect("genuine source body");
        (
            Arc::new(cfg::lower_source_closed(publication, &ledger).unwrap()),
            publication.facts().clone(),
        )
    }

    #[test]
    fn cell_subtree_access_requires_original_operation_and_complete_issued_target() {
        let source = "use std::cell::Cell;use std::rc::Rc;struct S{cell:Cell<u32>}fn update(s:&Rc<S>){s.cell.set(s.cell.get()+1u32);}fn main(){}";
        let (function, facts) = shared_aggregate_body_fixture(source);
        let plan = Plan::build_validated(
            &function,
            &facts,
            None,
            &mut Budget::new(32_000_000),
            128 * 1024 * 1024,
        )
        .unwrap();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        let mut checked = 0;
        for (block, body) in function.blocks.iter().enumerate() {
            for (statement, value) in body.statements.iter().enumerate() {
                let Statement::Assign {
                    value: Rvalue::CellGet { reference, .. },
                    ..
                } = &value.value
                else {
                    continue;
                };
                let point = AccessPoint {
                    block: BlockId(block),
                    statement: Some(statement),
                    phase: Phase::Read,
                };
                let mut storage = reference.clone();
                storage.projection.push(Projection::Deref);
                let mut budget = Budget::new(32_000_000);
                let targets = plan
                    .resolve(
                        &storage,
                        point.block,
                        point.statement,
                        point.phase,
                        &mut budget,
                    )
                    .unwrap()
                    .unwrap();
                assert!(!targets.is_empty());
                for target in &targets {
                    assert_eq!(target.precision, TargetPrecision::Subtree);
                    assert!(!target.authority.is_empty());
                    assert_eq!(
                        plan.cell_storage_problem(
                            &function,
                            target,
                            Access::CellRead,
                            point,
                            &mut budget
                        )
                        .unwrap(),
                        None
                    );
                    assert!(
                        plan.cell_storage_problem(
                            &function,
                            target,
                            Access::CellWrite,
                            point,
                            &mut budget
                        )
                        .unwrap()
                        .is_some()
                    );
                    let mut foreign = target.clone();
                    foreign.authority.clear();
                    assert!(
                        plan.cell_storage_problem(
                            &function,
                            &foreign,
                            Access::CellRead,
                            point,
                            &mut budget
                        )
                        .unwrap()
                        .is_some()
                    );
                    foreign = target.clone();
                    foreign.place.root = CanonicalRoot::Owned(LocalId(0));
                    assert!(
                        plan.cell_storage_problem(
                            &function,
                            &foreign,
                            Access::CellRead,
                            point,
                            &mut budget
                        )
                        .unwrap()
                        .is_some()
                    );
                    let mut forged = (*function).clone();
                    let Statement::Assign {
                        value: Rvalue::CellGet { reference, .. },
                        ..
                    } = &mut forged.blocks[block].statements[statement].value
                    else {
                        unreachable!()
                    };
                    *reference = Place {
                        local: LocalId(0),
                        projection: Vec::new(),
                    };
                    assert!(
                        plan.cell_storage_problem(
                            &forged,
                            target,
                            Access::CellRead,
                            point,
                            &mut budget
                        )
                        .unwrap()
                        .is_some()
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn closed_aggregate_result_replays_exact_source_and_affine_cells() {
        let source = "fn pack<'a,'b,'c>(a:&'a mut[i32],b:&'b mut i32,c:&'c[i32])->Option<(&'a mut[i32],(&'b mut i32,&'c[i32]))>{Some((a,(b,c)))}fn main(){}";
        let (function, facts) = shared_aggregate_body_fixture(source);
        let make_plan =
            |function: &Arc<cfg::Function>, facts: &Arc<super::super::traits::ValidatedFacts>| {
                Plan::build_validated(
                    function,
                    facts,
                    None,
                    &mut Budget::new(32_000_000),
                    128 * 1024 * 1024,
                )
            };
        let genuine = make_plan(&function, &facts).unwrap();
        assert!(genuine.problems.is_empty(), "{:?}", genuine.problems);
        assert!(
            genuine.effects().is_some(),
            "finite owned carrier aggregate requires complete effects"
        );
        for mutation in 0..4 {
            let mut forged = (*function).clone();
            match mutation {
                0 => forged.result = Ty::Unit,
                1 => {
                    forged.ref_contract = Arc::new(ir::RefContract {
                        region_count: 0,
                        ..(*forged.ref_contract).clone()
                    })
                }
                2 => {
                    forged.locals[forged.params[0].0].ty = Ty::Ref {
                        kind: BorrowKind::Shared,
                        referent: Arc::new(Ty::Slice {
                            element: Arc::new(Ty::Int(ir::IntTy::I32)),
                        }),
                    }
                }
                3 => forged.params.swap(0, 1),
                _ => unreachable!(),
            }
            if let Ok(plan) = make_plan(&Arc::new(forged), &facts) {
                assert!(
                    plan.effects().is_none() || !plan.problems.is_empty(),
                    "forgery {mutation}"
                );
            }
        }
        let (_, foreign) = shared_aggregate_body_fixture(source);
        if let Ok(plan) = make_plan(&function, &foreign) {
            assert!(
                plan.effects().is_none() || !plan.problems.is_empty(),
                "foreign graph/facts"
            );
        }
        assert!(
            Plan::build_validated(
                &function,
                &facts,
                None,
                &mut Budget::new(0),
                128 * 1024 * 1024
            )
            .is_err()
        );
    }

    #[test]
    fn closed_shared_aggregate_reborrow_keeps_exact_source_and_rejects_forged_body() {
        let source = "struct Target<'a>{values:&'a[i32;2]}struct Owner<'a>{target:Target<'a>}fn project<'a,'b>(owner:&'a mut Owner<'b>)->&'a mut Target<'b>{&mut owner.target}fn main(){}";
        let (function, facts) = shared_aggregate_body_fixture(source);
        let make_plan =
            |function: &Arc<cfg::Function>, facts: &Arc<super::super::traits::ValidatedFacts>| {
                Plan::build_validated(
                    function,
                    facts,
                    None,
                    &mut Budget::new(32_000_000),
                    128 * 1024 * 1024,
                )
            };
        let genuine = make_plan(&function, &facts).unwrap();
        assert!(genuine.problems.is_empty(), "{:?}", genuine.problems);
        assert!(
            genuine.effects().is_some(),
            "exact source aggregate reborrow needs complete effects"
        );
        for mutation in 0..4 {
            let mut forged = (*function).clone();
            match mutation {
                0 => forged.loan_sites[0].kind = BorrowKind::Shared,
                1 => forged.loan_sites[0]
                    .place
                    .projection
                    .push(Projection::Field(1)),
                2 => {
                    forged.ref_contract = Arc::new(ir::RefContract {
                        region_count: 0,
                        ..(*forged.ref_contract).clone()
                    })
                }
                3 => forged.result = Ty::Unit,
                _ => unreachable!(),
            }
            if let Ok(plan) = make_plan(&Arc::new(forged), &facts) {
                assert!(
                    plan.effects().is_none() || !plan.problems.is_empty(),
                    "forgery {mutation}"
                );
            }
        }
        let (_, foreign) = shared_aggregate_body_fixture(source);
        if let Ok(plan) = make_plan(&function, &foreign) {
            assert!(
                plan.effects().is_none() || !plan.problems.is_empty(),
                "foreign facts"
            );
        }
        let changed = "struct Target<'a>{values:&'a[i32;2]}struct Owner<'a>{target:Target<'a>}fn project<'a,'b>(owner:&'a mut Owner<'b>,replacement:&'b[i32;2])->&'a mut Target<'b>{owner.target.values=replacement;&mut owner.target}fn main(){}";
        let (function, facts) = shared_aggregate_body_fixture(changed);
        if let Ok(plan) = make_plan(&function, &facts) {
            assert!(
                plan.effects().is_none(),
                "a replacement must not acquire an identity proof"
            );
        }
    }

    #[test]
    fn owned_shared_family_result_retains_real_get_unwrap_targets_and_rejects_forgery() {
        let text = r#"use std::collections::BTreeMap;fn main(){let owner=String::from("value");let mut map:BTreeMap<usize,Vec<&str>>=BTreeMap::new();map.entry(0).or_default().push(owner.as_str());println!("{}",map.get(&0).unwrap().len());}"#;
        let snapshot = Arc::new(crate::compiler::Snapshot {
            input: crate::compiler::Input::Text(text.into()),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let ledger = snapshot.budget.clone();
        let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
        let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(0).key();
        let requested = engine.request(key.clone());
        engine.run(1).unwrap();
        requested.completion().unwrap();
        let body = engine
            .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
            .unwrap();
        let publication = body
            .publication
            .as_ref()
            .expect("genuine original main publication");
        let function = Arc::new(cfg::lower_source_closed(publication, &ledger).unwrap());
        let facts = publication.facts().clone();
        let mut budget = Budget::new(32_000_000);
        let mut builder = CarrierBuilder::new(
            &function,
            FactsCtx::Validated(&facts),
            ledger,
            &mut budget,
            128 * 1024 * 1024,
            None,
        )
        .unwrap();
        builder.function = Some(function.clone());
        builder.authority = Some(facts.clone());
        let (index, signature) = function
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| {
                let Terminator::Call {
                    name, signature, ..
                } = &block.terminator.value
                else {
                    return None;
                };
                (name == "unwrap").then_some((index, signature))
            })
            .expect("unchanged original get/unwrap source call");
        let view = builder.calls[index].clone().unwrap();
        let location = fixed_location(ReferencePosition { depth: 0 });
        assert!(
            builder
                .owned_shared_family_result_source(signature, &view, &location)
                .unwrap()
        );
        let original = view.original_contract();
        let expanded = view.contract();
        for control in 0..12 {
            let mut before = original.clone();
            let mut after = expanded.clone();
            let mut old_rows = before.results.to_vec();
            let mut new_rows = after.results.to_vec();
            let root = new_rows
                .iter()
                .position(|row| row.location == location)
                .unwrap();
            let mut candidate = location.clone();
            match control {
                0 => old_rows[0].sources = Arc::from([]),
                1 => {
                    old_rows[0].sources = Arc::from([
                        old_rows[0].sources[0].clone(),
                        old_rows[0].sources[0].clone(),
                    ])
                }
                2 => {
                    let mut input = old_rows[0].sources[0].clone();
                    let CarrierChannel::ParamTail { parameter, .. } = input.location.channel else {
                        unreachable!()
                    };
                    input.location.channel = CarrierChannel::ParamTail {
                        parameter,
                        prefix: 1,
                    };
                    old_rows[0].sources = Arc::from([input]);
                }
                3 => {
                    let CarrierChannel::ParamTail { parameter, .. } = old_rows[0].location.channel
                    else {
                        unreachable!()
                    };
                    old_rows[0].location.channel = CarrierChannel::ParamTail {
                        parameter: ir::ParamId {
                            owner: parameter.owner,
                            index: u16::MAX,
                        },
                        prefix: 0,
                    };
                }
                4 | 5 => {
                    let mut input = old_rows[0].sources[0].clone();
                    input.location.path = CarrierPath(Arc::from([if control == 4 {
                        CarrierStep::Referent
                    } else {
                        CarrierStep::SliceElement
                    }]));
                    old_rows[0].sources = Arc::from([input]);
                }
                6 => before.inputs = Arc::from([]),
                7 => new_rows[root].sources = Arc::from([]),
                8 => {
                    let mut input = new_rows[root].sources[0].clone();
                    input.param = usize::MAX;
                    new_rows[root].sources = Arc::from([input]);
                }
                9 => new_rows.push(new_rows[root].clone()),
                10 => {
                    after.writes = Arc::from([ir::CarrierWriteEnvelope {
                        destination: new_rows[root].sources[0].clone(),
                        sources: new_rows[root].sources.clone(),
                    }])
                }
                _ => candidate = fixed_location(ReferencePosition { depth: 1 }),
            }
            before.results = old_rows.into();
            after.results = new_rows.into();
            assert!(
                owned_shared_family_result_rows(
                    &before,
                    &after,
                    &candidate,
                    &mut Budget::new(1_000_000)
                )
                .unwrap()
                .is_none(),
                "forged owned family row {control}"
            );
        }
        assert!(
            owned_shared_family_result_rows(original, expanded, &location, &mut Budget::new(0))
                .is_err()
        );
        let mut mutable = signature.clone();
        let Ty::Ref { referent, .. } = &mutable.result else {
            unreachable!()
        };
        mutable.result = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: referent.clone(),
        };
        assert!(
            !builder
                .owned_shared_family_result_source(&mutable, &view, &location)
                .unwrap()
        );
        let root_origin = builder.call_rows[index]
            .iter()
            .find(|(row, _)| *row == location)
            .unwrap()
            .1;
        let plan = builder.solve().unwrap();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert!(
            !plan.targets[root_origin.0].is_empty(),
            "unwrap retains only its issued source storage targets"
        );
        assert!(plan.parents[root_origin.0].count() > 0);
    }

    #[test]
    fn owned_shared_family_write_replays_real_or_insert_and_rejects_forgery() {
        for (text, expected) in [
            (
                r#"use std::collections::BTreeMap;fn main(){let owner=String::from("value");let mut map:BTreeMap<usize,Vec<&str>>=BTreeMap::new();let mut values=Vec::new();values.push(owner.as_str());let _entry=map.entry(0).or_insert(values);}"#,
                true,
            ),
            (
                r#"use std::collections::BTreeMap;fn main(){let mut owner=1;let mut map:BTreeMap<usize,Vec<&mut i32>>=BTreeMap::new();let mut values=Vec::new();values.push(&mut owner);let _entry=map.entry(0).or_insert(values);}"#,
                false,
            ),
            (
                r#"use std::collections::BTreeMap;fn main(){let mut map:BTreeMap<usize,String>=BTreeMap::new();let _entry=map.entry(0).or_insert(String::new());}"#,
                false,
            ),
        ] {
            let snapshot = Arc::new(crate::compiler::Snapshot {
                input: crate::compiler::Input::Text(text.into()),
                source_options: crate::compiler::source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let ledger = snapshot.budget.clone();
            let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
            let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(0).key();
            let requested = engine.request(key.clone());
            engine.run(1).unwrap();
            requested.completion().unwrap();
            let body = engine
                .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
                .unwrap();
            let publication = body
                .publication
                .as_ref()
                .expect("genuine original main publication");
            let function = Arc::new(cfg::lower_source_closed(publication, &ledger).unwrap());
            let facts = publication.facts().clone();
            let mut budget = Budget::new(32_000_000);
            let mut builder = CarrierBuilder::new(
                &function,
                FactsCtx::Validated(&facts),
                ledger,
                &mut budget,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            builder.function = Some(function.clone());
            builder.authority = Some(facts.clone());
            let mut witnesses = 0;
            for (index, block) in function.blocks.iter().enumerate() {
                let Terminator::Call {
                    name, signature, ..
                } = &block.terminator.value
                else {
                    continue;
                };
                if name != "or_insert" {
                    continue;
                }
                witnesses += 1;
                let view = builder.calls[index]
                    .clone()
                    .expect("canonical or_insert view");
                let original = view.original_contract();
                let schema = view.origin_schema();
                let tail = schema
                    .input_tails
                    .iter()
                    .find(|tail| tail.prefix == 0)
                    .expect("original owned input tail");
                let parameter = tail.parameter;
                let owned = owned_shared_family_write_source(
                    original,
                    schema,
                    parameter,
                    &mut Budget::new(1_000_000),
                )
                .unwrap()
                .expect("exact original owned/stored family");
                assert_eq!(
                    builder
                        .owned_shared_family_write_admitted(
                            &signature.params,
                            parameter,
                            &view,
                            Some(&facts),
                        )
                        .unwrap(),
                    expected
                );
                if !expected {
                    continue;
                }
                let (_, actual) = view.parameter_actual(parameter).unwrap().unwrap();
                assert!(
                    !builder.copy_permission_in(actual, Some(&facts)).unwrap(),
                    "the owned Vec is not granted whole-value Copy"
                );
                assert!(
                    builder
                        .copy_family_reborrow_admitted(
                            &signature.params,
                            &signature.result,
                            &view,
                            Some(&facts),
                        )
                        .unwrap()
                );
                assert!(
                    builder
                        .affine_contract_admitted(
                            &signature.params,
                            &signature.result,
                            &view,
                            Some(&facts),
                        )
                        .unwrap()
                );
                let mut changed_params = signature.params.clone();
                changed_params[owned.param] = Ty::Unit;
                assert!(
                    !builder
                        .owned_shared_family_write_admitted(
                            &changed_params,
                            parameter,
                            &view,
                            Some(&facts),
                        )
                        .unwrap()
                );
                for control in 0..19 {
                    let mut forged = original.clone();
                    let mut origin = schema.as_ref().clone();
                    let mut tails = origin.input_tails.to_vec();
                    let tail_index = tails
                        .iter()
                        .position(|tail| tail.parameter == parameter)
                        .unwrap();
                    let mut inputs = forged.inputs.to_vec();
                    let owner_index = inputs.iter().position(|input| input == owned).unwrap();
                    let stored_index = inputs
                        .iter()
                        .position(|input| {
                            input.location.channel == owned.location.channel && input != owned
                        })
                        .unwrap();
                    let mut writes = forged.writes.to_vec();
                    let owner_write = writes
                        .iter()
                        .position(|row| row.destination == *owned)
                        .unwrap();
                    let mut results = forged.results.to_vec();
                    let result_index = results
                        .iter()
                        .position(|row| {
                            matches!(row.location.channel,
                        CarrierChannel::ParamTail { parameter: p, .. } if p == parameter)
                        })
                        .unwrap();
                    match control {
                        0 => tails.clear(),
                        1 => tails[tail_index].prefix = 1,
                        2 => tails[tail_index].parameter.index = u16::MAX,
                        3 => tails[tail_index].param = usize::MAX,
                        4 => tails.push(tails[tail_index]),
                        5 => {
                            inputs.remove(owner_index);
                        }
                        6 => {
                            inputs[stored_index].location.path =
                                CarrierPath(Arc::from([CarrierStep::SliceElement]))
                        }
                        7 => {
                            writes.remove(owner_write);
                        }
                        8 => writes[owner_write].sources = Arc::from([owned.clone()]),
                        9 => {
                            results[result_index].location.channel = owned.location.channel.clone()
                        }
                        10 => {
                            inputs[owner_index].location.path =
                                CarrierPath(Arc::from([CarrierStep::Field(0)]))
                        }
                        11 => writes.push(writes[owner_write].clone()),
                        12 => origin.result_tails = Arc::from([]),
                        13..=18 => {
                            let mut result_tails = origin.result_tails.to_vec();
                            let index = result_tails
                                .iter()
                                .position(|tail| tail.parameter == parameter)
                                .unwrap();
                            match control {
                                13 => result_tails[index].prefix = 0,
                                14 => result_tails[index].parameter.index = u16::MAX,
                                15 => result_tails[index].inputs = Arc::from([]),
                                16 => {
                                    let mut input = result_tails[index].inputs[0];
                                    input.param = usize::MAX;
                                    result_tails[index].inputs = Arc::from([input]);
                                }
                                17 => {
                                    let input = result_tails[index].inputs[0];
                                    result_tails[index].inputs = Arc::from([input, input]);
                                }
                                _ => result_tails.push(result_tails[index].clone()),
                            }
                            origin.result_tails = result_tails.into();
                        }
                        _ => unreachable!(),
                    }
                    origin.input_tails = tails.into();
                    forged.inputs = inputs.into();
                    forged.writes = writes.into();
                    forged.results = results.into();
                    assert!(
                        owned_shared_family_write_source(
                            &forged,
                            &origin,
                            parameter,
                            &mut Budget::new(1_000_000)
                        )
                        .unwrap()
                        .is_none(),
                        "forged owned source {control}"
                    );
                }
                assert!(
                    owned_shared_family_write_source(
                        original,
                        schema,
                        parameter,
                        &mut Budget::new(0)
                    )
                    .is_err()
                );
            }
            assert_eq!(witnesses, 1, "exact original or_insert call exercised");
            if expected {
                let plan = builder.solve().unwrap();
                assert!(plan.problems.is_empty(), "{:?}", plan.problems);
            }
        }
    }

    #[test]
    fn copy_family_reborrow_replays_real_entry_calls_and_rejects_noncopy_sources() {
        for (text, expected, check_key_carrier_forgeries) in [
            (
                r#"fn main(){let mut values:Vec<Vec<&str>>=Vec::new();let _result=values.get_mut(0);}"#,
                1,
                false,
            ),
            (
                r#"fn main(){let mut values:Vec<Vec<&mut i32>>=Vec::new();let _result=values.get_mut(0);}"#,
                0,
                false,
            ),
            (
                r#"use std::collections::BTreeMap;fn main(){let mut m:BTreeMap<usize,Vec<&str>>=BTreeMap::new();let _result=m.get_mut(&0);}"#,
                1,
                false,
            ),
            (
                r#"use std::collections::HashMap;fn main(){let mut m:HashMap<usize,Vec<&str>>=HashMap::new();let _result=m.get_mut(&0);}"#,
                1,
                false,
            ),
            (
                r#"use std::collections::BTreeMap;fn main(){let mut m:BTreeMap<usize,Vec<&mut i32>>=BTreeMap::new();let _result=m.get_mut(&0);}"#,
                0,
                false,
            ),
            (
                r#"use std::collections::HashMap;fn main(){let mut m:HashMap<&str,usize>=HashMap::new();*m.entry("a").or_insert(0)+=1;}"#,
                2,
                true,
            ),
            (
                r#"use std::collections::HashMap;fn main(){let mut m:HashMap<String,usize>=HashMap::new();let _entry=m.entry(String::from("a"));}"#,
                0,
                false,
            ),
            (
                r#"use std::collections::BTreeMap;fn main(){let mut m:BTreeMap<usize,Vec<&str>>=BTreeMap::new();let _entry=m.entry(0);}"#,
                1,
                false,
            ),
            (
                r#"use std::collections::BTreeMap;fn main(){let mut m:BTreeMap<usize,Vec<&mut i32>>=BTreeMap::new();let _entry=m.entry(0);}"#,
                0,
                false,
            ),
        ] {
            let snapshot = Arc::new(crate::compiler::Snapshot {
                input: crate::compiler::Input::Text(text.into()),
                source_options: crate::compiler::source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let ledger = snapshot.budget.clone();
            let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
            let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(0).key();
            let requested = engine.request(key.clone());
            engine.run(1).unwrap();
            requested.completion().unwrap();
            let body = engine
                .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
                .unwrap();
            let publication = body
                .publication
                .as_ref()
                .expect("genuine original main publication");
            let function = Arc::new(cfg::lower_source_closed(publication, &ledger).unwrap());
            let facts = publication.facts().clone();
            let mut budget = Budget::new(32_000_000);
            let mut builder = CarrierBuilder::new(
                &function,
                FactsCtx::Validated(&facts),
                ledger,
                &mut budget,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            builder.function = Some(function.clone());
            builder.authority = Some(facts.clone());
            let mut candidates = 0;
            let mut admitted = 0;
            for (index, block) in function.blocks.iter().enumerate() {
                let Terminator::Call { signature, .. } = &block.terminator.value else {
                    continue;
                };
                let Some(view) = builder.calls[index].clone() else {
                    continue;
                };
                if copy_family_effect_rows(view.original_contract(), &mut Budget::new(1_000_000))
                    .unwrap()
                    .is_none()
                    || view.original_contract().results.is_empty()
                {
                    continue;
                }
                candidates += 1;
                let accepted = builder
                    .copy_family_reborrow_admitted(
                        &signature.params,
                        &signature.result,
                        &view,
                        Some(&facts),
                    )
                    .unwrap();
                admitted += usize::from(accepted);
                if accepted {
                    assert!(
                        builder
                            .affine_contract_admitted(
                                &signature.params,
                                &signature.result,
                                &view,
                                Some(&facts)
                            )
                            .unwrap()
                    );
                    let original = view.original_contract();
                    for row in view.contract().results.iter() {
                        if !copy_family_fixed_result_rows(
                            original,
                            row,
                            view.nominal_mappings(),
                            &mut Budget::new(1_000_000),
                        )
                        .unwrap()
                        {
                            continue;
                        }
                        for (mapping_index, mapping) in view.nominal_mappings().iter().enumerate() {
                            if mapping.original == mapping.instantiated {
                                continue;
                            }
                            let mut relevant = false;
                            for before in original.results.iter() {
                                if fixed_reborrow_location_matches(
                                    &before.location,
                                    &row.location,
                                    view.nominal_mappings(),
                                    &mut Budget::new(1_000_000),
                                )
                                .unwrap()
                                {
                                    relevant |= before.location.path.0.iter().any(|step| {
                                        matches!(step, CarrierStep::VariantField { variant, .. } if variant.owner == mapping.original)
                                    });
                                }
                            }
                            if !relevant {
                                continue;
                            }
                            for control in 0..3 {
                                let mut changed = view.nominal_mappings().to_vec();
                                match control {
                                    0 => {
                                        changed.remove(mapping_index);
                                    }
                                    1 => changed[mapping_index].instantiated = mapping.original,
                                    _ => changed.push(mapping.clone()),
                                }
                                assert!(
                                    !copy_family_fixed_result_rows(
                                        original,
                                        row,
                                        &changed,
                                        &mut Budget::new(1_000_000),
                                    )
                                    .unwrap(),
                                    "forged nominal replay {control}"
                                );
                            }
                        }
                        for control in 0..8 {
                            let mut before = original.clone();
                            let mut after = row.clone();
                            match control {
                                0 => after.sources = Arc::from([]),
                                1 => {
                                    after.sources = Arc::from([
                                        after.sources[0].clone(),
                                        after.sources[0].clone(),
                                    ])
                                }
                                2 => {
                                    let mut source = after.sources[0].clone();
                                    source.param = usize::MAX;
                                    after.sources = Arc::from([source]);
                                }
                                3 => {
                                    after.location.channel =
                                        CarrierChannel::Fixed(ReferencePosition { depth: 1 })
                                }
                                4 => {
                                    after.location.path =
                                        CarrierPath(Arc::from([CarrierStep::Referent]))
                                }
                                5 => before.inputs = Arc::from([]),
                                6 => {
                                    let mut rows = before.results.to_vec();
                                    rows.extend(before.results.iter().cloned());
                                    before.results = rows.into();
                                }
                                _ => {
                                    let mut rows = before.results.to_vec();
                                    for original in &mut rows {
                                        original.sources = Arc::from([]);
                                    }
                                    before.results = rows.into();
                                }
                            }
                            assert!(
                                !copy_family_fixed_result_rows(
                                    &before,
                                    &after,
                                    view.nominal_mappings(),
                                    &mut Budget::new(1_000_000),
                                )
                                .unwrap(),
                                "forged fixed result {control}"
                            );
                        }
                        for (index, step) in row.location.path.0.iter().enumerate() {
                            if !matches!(step, CarrierStep::VariantField { .. }) {
                                continue;
                            }
                            for control in 0..3 {
                                let mut after = row.clone();
                                let mut path = after.location.path.0.to_vec();
                                let CarrierStep::VariantField { variant, field } = &mut path[index]
                                else {
                                    unreachable!()
                                };
                                match control {
                                    0 => variant.index = usize::MAX,
                                    1 => *field = usize::MAX,
                                    _ => path[index] = CarrierStep::Field(usize::MAX),
                                }
                                after.location.path = CarrierPath(path.into());
                                assert!(
                                    !copy_family_fixed_result_rows(
                                        original,
                                        &after,
                                        view.nominal_mappings(),
                                        &mut Budget::new(1_000_000),
                                    )
                                    .unwrap(),
                                    "forged fixed variant {control}"
                                );
                            }
                        }
                        assert!(
                            copy_family_fixed_result_rows(
                                original,
                                row,
                                view.nominal_mappings(),
                                &mut Budget::new(0),
                            )
                            .is_err()
                        );
                    }
                    let mut mapping_witnesses = 0;
                    for write in original.writes.iter() {
                        let CarrierChannel::ParamTail {
                            parameter,
                            prefix: 0,
                        } = write.destination.location.channel
                        else {
                            unreachable!()
                        };
                        for (destination, component) in builder
                            .transport_effect_input(&write.destination, &view, true)
                            .unwrap()
                        {
                            let mapping = view
                                .component_mappings()
                                .iter()
                                .find(|mapping| {
                                    mapping.original_family
                                        == ir::CarrierFamily::Parameter(parameter)
                                        && component.as_ref() == Some(&mapping.original)
                                })
                                .expect("authenticated retained relative component");
                            assert!(
                                builder
                                    .copy_family_component_matches(
                                        parameter,
                                        &write.destination.location,
                                        &destination.location,
                                        mapping,
                                        &view
                                    )
                                    .unwrap()
                            );
                            mapping_witnesses += 1;
                            for control in 0..6 {
                                let mut forged = mapping.clone();
                                let mut occurrence = write.destination.location.clone();
                                let mut expanded = destination.location.clone();
                                match control {
                                    0 => {
                                        forged.original.path =
                                            CarrierPath(Arc::from([CarrierStep::Field(0)]))
                                    }
                                    1 => {
                                        forged.substituted.path =
                                            CarrierPath(Arc::from([CarrierStep::Field(usize::MAX)]))
                                    }
                                    2 => {
                                        forged.original.channel = CarrierChannel::ParamTail {
                                            parameter,
                                            prefix: 1,
                                        }
                                    }
                                    3 => {
                                        forged.original_family =
                                            ir::CarrierFamily::Parameter(ir::ParamId {
                                                owner: parameter.owner,
                                                index: u16::MAX,
                                            })
                                    }
                                    4 => {
                                        occurrence.path =
                                            CarrierPath(Arc::from([CarrierStep::Field(usize::MAX)]))
                                    }
                                    _ => {
                                        expanded.path =
                                            CarrierPath(Arc::from([CarrierStep::Field(usize::MAX)]))
                                    }
                                }
                                assert!(
                                    !builder
                                        .copy_family_component_matches(
                                            parameter,
                                            &occurrence,
                                            &expanded,
                                            &forged,
                                            &view
                                        )
                                        .unwrap(),
                                    "forged relative component {control}"
                                );
                            }
                        }
                    }
                    assert!(mapping_witnesses > 0);
                    if !check_key_carrier_forgeries {
                        let families =
                            copy_family_effect_rows(original, &mut Budget::new(1_000_000))
                                .unwrap()
                                .unwrap();
                        let mut noncopy_identity = false;
                        for (index, row) in original.writes.iter().enumerate() {
                            let CarrierChannel::ParamTail {
                                parameter,
                                prefix: 0,
                            } = row.destination.location.channel
                            else {
                                unreachable!()
                            };
                            let (_, actual) = view.parameter_actual(parameter).unwrap().unwrap();
                            if row.sources.as_ref() != std::slice::from_ref(&row.destination)
                                || builder.copy_permission_in(actual, Some(&facts)).unwrap()
                            {
                                continue;
                            }
                            noncopy_identity = true;
                            assert!(!families.contains(&parameter));
                            // A second same-family source changes identity into
                            // mutation and must restore whole-actual Copy demand.
                            let mut changed = original.clone();
                            let mut inputs = changed.inputs.to_vec();
                            let mut writes = changed.writes.to_vec();
                            let mut other = row.destination.clone();
                            other.param = usize::MAX;
                            inputs.push(other.clone());
                            writes[index].sources = Arc::from([row.destination.clone(), other]);
                            changed.inputs = inputs.into();
                            changed.writes = writes.into();
                            assert!(
                                copy_family_effect_rows(&changed, &mut Budget::new(1_000_000))
                                    .unwrap()
                                    .unwrap()
                                    .contains(&parameter)
                            );
                        }
                        assert!(noncopy_identity, "genuine nested non-Copy identity family");
                    }
                    for control in 0..7 {
                        let mut forged = original.clone();
                        let mut writes = forged.writes.to_vec();
                        let mut inputs = forged.inputs.to_vec();
                        match control {
                            0 => writes[0].sources = Arc::from([]),
                            1 => writes.push(writes[0].clone()),
                            2 => {
                                writes[0].sources = Arc::from([
                                    writes[0].sources[0].clone(),
                                    writes[0].sources[0].clone(),
                                ])
                            }
                            3 => writes[0].destination.param = usize::MAX,
                            4 => {
                                writes[0].destination.location.path =
                                    CarrierPath(Arc::from([CarrierStep::Field(usize::MAX)]))
                            }
                            5 => {
                                let mut source = writes[0].sources[0].clone();
                                source.location.channel =
                                    CarrierChannel::Fixed(ReferencePosition { depth: 0 });
                                writes[0].sources = Arc::from([source]);
                            }
                            _ => {
                                let missing = inputs
                                    .iter()
                                    .position(|input| *input == writes[0].destination)
                                    .unwrap();
                                inputs.remove(missing);
                            }
                        }
                        forged.writes = writes.into();
                        forged.inputs = inputs.into();
                        assert!(
                            copy_family_effect_rows(&forged, &mut Budget::new(1_000_000))
                                .unwrap()
                                .is_none(),
                            "forged shape {control}"
                        );
                    }
                    assert!(copy_family_effect_rows(original, &mut Budget::new(0)).is_err());
                    if check_key_carrier_forgeries
                        && matches!(
                            signature.params.first(),
                            Some(Ty::Ref {
                                kind: BorrowKind::Mut,
                                ..
                            })
                        )
                    {
                        let mut shared = signature.params.clone();
                        let Ty::Ref { referent, .. } = &shared[0] else {
                            unreachable!()
                        };
                        shared[0] = Ty::Ref {
                            kind: BorrowKind::Shared,
                            referent: referent.clone(),
                        };
                        assert!(
                            !builder
                                .copy_family_reborrow_admitted(
                                    &shared,
                                    &signature.result,
                                    &view,
                                    Some(&facts)
                                )
                                .unwrap()
                        );
                        let mut noncopy = signature.params.clone();
                        noncopy[1] = Ty::Ref {
                            kind: BorrowKind::Mut,
                            referent: Arc::new(Ty::Int(ir::IntTy::I32)),
                        };
                        assert!(
                            !builder
                                .copy_family_reborrow_admitted(
                                    &noncopy,
                                    &signature.result,
                                    &view,
                                    Some(&facts)
                                )
                                .unwrap()
                        );
                        let write = &original.writes[0].destination;
                        let expanded = &view.contract().writes[0].destination;
                        let CarrierChannel::ParamTail { parameter, .. } = write.location.channel
                        else {
                            unreachable!()
                        };
                        let mapping = &view.component_mappings()[0];
                        let mut forged = mapping.clone();
                        forged.original.path = CarrierPath(Arc::from([CarrierStep::Field(0)]));
                        assert!(
                            !builder
                                .identity_slice_component_matches(
                                    parameter,
                                    &write.location,
                                    &expanded.location,
                                    &forged,
                                    view.nominal_mappings()
                                )
                                .unwrap()
                        );
                    }
                }
            }
            assert!(
                candidates > 0,
                "real source must exercise the proof boundary"
            );
            assert_eq!(admitted, expected, "{text}");
        }
    }

    #[test]
    fn identity_nested_slot_reborrow_authenticates_original_and_expanded_ownership() {
        for (text, expected) in [
            (
                r#"fn main(){let mut a=1i32;let mut b=5i32;let mut v:Vec<&mut i32>=vec![&mut a,&mut b];*v[0]+=10;*v[1]=7;println!("{} {}",v[0],v[1]);}"#,
                true,
            ),
            (
                r#"fn main(){let mut text=String::from("text");let mut values=vec![&mut text];let _=&mut values[0usize];}"#,
                false,
            ),
        ] {
            let snapshot = Arc::new(crate::compiler::Snapshot {
                input: crate::compiler::Input::Text(text.into()),
                source_options: crate::compiler::source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let ledger = snapshot.budget.clone();
            let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
            let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(0).key();
            let requested = engine.request(key.clone());
            engine.run(1).unwrap();
            requested.completion().unwrap();
            let body = engine
                .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
                .unwrap();
            let publication = body
                .publication
                .as_ref()
                .expect("genuine original main publication");
            let function = Arc::new(cfg::lower_source_closed(publication, &ledger).unwrap());
            let facts = publication.facts().clone();
            let mut budget = Budget::new(32_000_000);
            let mut builder = CarrierBuilder::new(
                &function,
                FactsCtx::Validated(&facts),
                ledger,
                &mut budget,
                128 * 1024 * 1024,
                None,
            )
            .unwrap();
            builder.function = Some(function.clone());
            builder.authority = Some(facts.clone());
            let (signature, view) = function
                .blocks
                .iter()
                .enumerate()
                .find_map(|(index, block)| {
                    let Terminator::Call { signature, .. } = &block.terminator.value else {
                        return None;
                    };
                    let view = builder.calls[index].clone()?;
                    identity_nested_slot_reborrow_rows(
                        view.original_contract(),
                        view.contract(),
                        &mut Budget::new(1_000_000),
                    )
                    .unwrap()
                    .map(|_| (signature.clone(), view))
                })
                .expect("genuine original nested-slot carrier rows");
            assert!(
                !builder
                    .identity_copy_reborrow_admitted(
                        &signature.params,
                        &signature.result,
                        &view,
                        Some(&facts)
                    )
                    .unwrap(),
                "the mutable reference leaf remains non-Copy"
            );
            assert_eq!(
                builder
                    .identity_nested_slot_reborrow_admitted(
                        &signature.params,
                        &signature.result,
                        &view,
                        Some(&facts)
                    )
                    .unwrap(),
                expected
            );
            let Ty::Ref {
                referent: actual, ..
            } = &signature.result
            else {
                unreachable!()
            };
            let Ty::Ref { referent: leaf, .. } = actual.as_ref() else {
                unreachable!()
            };
            assert_eq!(
                builder.copy_permission_in(leaf, Some(&facts)).unwrap(),
                expected,
                "genuine non-Copy String pointee reaches the exclusive leaf boundary"
            );
            if !expected {
                continue;
            }
            assert!(
                builder
                    .affine_contract_admitted(
                        &signature.params,
                        &signature.result,
                        &view,
                        Some(&facts)
                    )
                    .unwrap()
            );
            for side in 0..2 {
                for mutation in 0..10 {
                    let mut original = view.original_contract().clone();
                    let mut expanded = view.contract().clone();
                    let changed = if side == 0 {
                        &mut original
                    } else {
                        &mut expanded
                    };
                    let mut inputs = changed.inputs.to_vec();
                    let mut results = changed.results.to_vec();
                    let mut writes = changed.writes.to_vec();
                    match mutation {
                        0 => writes[0].sources = Arc::from([inputs[0].clone()]),
                        1 => writes[0].destination.param += 1,
                        2 => writes[0].destination.location.path = CarrierPath::default(),
                        3 => results[1].sources = Arc::from([inputs[0].clone()]),
                        4 => results[0].sources = Arc::from([inputs[1].clone()]),
                        5 => {
                            results[1].location.path =
                                CarrierPath(Arc::from([CarrierStep::Field(0)]))
                        }
                        6 => inputs.swap(0, 1),
                        7 => writes.push(writes[0].clone()),
                        8 => results.push(results[1].clone()),
                        9 => writes[0].sources = Arc::from([inputs[1].clone(), inputs[1].clone()]),
                        _ => unreachable!(),
                    }
                    changed.inputs = inputs.into();
                    changed.results = results.into();
                    changed.writes = writes.into();
                    assert!(
                        identity_nested_slot_reborrow_rows(
                            &original,
                            &expanded,
                            &mut Budget::new(1_000_000)
                        )
                        .unwrap()
                        .is_none(),
                        "side={side} mutation={mutation}"
                    );
                }
            }
            let parameter = identity_nested_slot_reborrow_rows(
                view.original_contract(),
                view.contract(),
                &mut Budget::new(1_000_000),
            )
            .unwrap()
            .unwrap();
            let mapping = &view.component_mappings()[0];
            for control in 0..5 {
                let mut forged = mapping.clone();
                match control {
                    0 => forged.original.path = CarrierPath(Arc::from([CarrierStep::Field(0)])),
                    1 => forged.substituted.path = CarrierPath(Arc::from([CarrierStep::Field(0)])),
                    2 => {
                        forged.original_family = ir::CarrierFamily::Parameter(ir::ParamId {
                            index: parameter.index + 1,
                            ..parameter
                        })
                    }
                    3 => {
                        forged.original.channel = CarrierChannel::ParamTail {
                            parameter,
                            prefix: 1,
                        }
                    }
                    4 => {
                        forged.substituted.channel =
                            CarrierChannel::Fixed(ReferencePosition { depth: 1 })
                    }
                    _ => unreachable!(),
                }
                for (original, expanded) in [
                    (
                        &view.original_contract().writes[0].destination.location,
                        &view.contract().writes[0].destination.location,
                    ),
                    (
                        &view.original_contract().results[1].location,
                        &view.contract().results[1].location,
                    ),
                ] {
                    assert!(
                        !builder
                            .identity_slice_component_matches(
                                parameter,
                                original,
                                expanded,
                                &forged,
                                view.nominal_mappings()
                            )
                            .unwrap()
                    );
                }
            }
            let wrong_result = Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(Ty::Ref {
                    kind: BorrowKind::Shared,
                    referent: leaf.clone(),
                }),
            };
            assert!(
                !builder
                    .identity_nested_slot_reborrow_admitted(
                        &signature.params,
                        &wrong_result,
                        &view,
                        Some(&facts)
                    )
                    .unwrap()
            );
            let same_shape = Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(Ty::Ref {
                    kind: BorrowKind::Mut,
                    referent: Arc::new(Ty::Int(ir::IntTy::I64)),
                }),
            };
            assert!(
                !builder
                    .identity_nested_slot_reborrow_admitted(
                        &signature.params,
                        &same_shape,
                        &view,
                        Some(&facts)
                    )
                    .unwrap()
            );
            let owner_index = view.original_contract().writes[0].destination.param;
            let mut shared_owner = signature.params.clone();
            let Ty::Ref {
                referent: sequence, ..
            } = &signature.params[owner_index]
            else {
                unreachable!()
            };
            shared_owner[owner_index] = Ty::Ref {
                kind: BorrowKind::Shared,
                referent: sequence.clone(),
            };
            assert!(
                !builder
                    .identity_nested_slot_reborrow_admitted(
                        &shared_owner,
                        &signature.result,
                        &view,
                        Some(&facts)
                    )
                    .unwrap()
            );
            let mut wrong_owner = signature.params.clone();
            wrong_owner[view.original_contract().writes[0].destination.param] = Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(Ty::Tuple(Arc::from([actual.as_ref().clone()]))),
            };
            assert!(
                !builder
                    .identity_nested_slot_reborrow_admitted(
                        &wrong_owner,
                        &signature.result,
                        &view,
                        Some(&facts)
                    )
                    .unwrap()
            );
        }
    }

    #[test]
    fn identity_slice_reborrow_rejects_noncopy_result_element_at_proof_boundary() {
        let snapshot = Arc::new(crate::compiler::Snapshot {
            input: crate::compiler::Input::Text(
                r#"fn observe(values:&mut[&'static str]){println!("{}",values[0usize]);}fn main(){let mut values=vec!["first","second"];observe(&mut values);println!("{}",values[1usize]);}"#.into(),
            ),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let ledger = snapshot.budget.clone();
        let engine = crate::compiler::Engine::new(crate::compiler::CompilerProvider(snapshot));
        let key = crate::compiler::keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(1).key();
        let requested = engine.request(key.clone());
        engine.run(1).unwrap();
        requested.completion().unwrap();
        let body = engine
            .fact_of::<crate::compiler::TemplateBodyFact>(&key, 0)
            .unwrap();
        let publication = body
            .publication
            .as_ref()
            .expect("genuine original main publication");
        let function = Arc::new(cfg::lower_source_closed(publication, &ledger).unwrap());
        let facts = publication.facts().clone();
        let mut budget = Budget::new(32_000_000);
        let mut builder = CarrierBuilder::new(
            &function,
            FactsCtx::Validated(&facts),
            ledger,
            &mut budget,
            128 * 1024 * 1024,
            None,
        )
        .unwrap();
        builder.function = Some(function.clone());
        builder.authority = Some(facts.clone());
        let mut selected = None;
        for (index, block) in function.blocks.iter().enumerate() {
            let Terminator::Call { signature, .. } = &block.terminator.value else {
                continue;
            };
            let Ty::Ref {
                kind: BorrowKind::Mut,
                referent,
            } = &signature.result
            else {
                continue;
            };
            if !matches!(referent.as_ref(), Ty::Slice { .. }) {
                continue;
            }
            let Some(view) = builder.calls[index].clone() else {
                continue;
            };
            if identity_slice_reborrow_rows(
                view.original_contract(),
                view.contract(),
                &mut Budget::new(1_000_000),
            )
            .unwrap()
            .is_some()
            {
                selected = Some((signature.clone(), view));
                break;
            }
        }
        let (signature, view) =
            selected.expect("exact original owned-sequence descriptor reborrow");
        let Ty::Ref { referent, .. } = &signature.result else {
            unreachable!()
        };
        assert!(
            builder
                .identity_copy_slice_reborrow_admitted(
                    &signature.params,
                    referent,
                    &view,
                    Some(&facts),
                )
                .unwrap(),
            "genuine Copy element proof must reach and pass its leaf checks"
        );
        let parameter = identity_slice_reborrow_rows(
            view.original_contract(),
            view.contract(),
            &mut Budget::new(1_000_000),
        )
        .unwrap()
        .unwrap();
        let original = &view.original_contract().writes[0].destination.location;
        let expanded = &view.contract().writes[0].destination.location;
        let mapping = view
            .component_mappings()
            .iter()
            .find(|mapping| mapping.original_family == ir::CarrierFamily::Parameter(parameter))
            .expect("genuine relative family mapping");
        assert!(mapping.original.path.0.is_empty());
        assert!(mapping.substituted.path.0.is_empty());
        assert_ne!(&mapping.original, original);
        assert!(
            builder
                .identity_slice_component_matches(
                    parameter,
                    original,
                    expanded,
                    mapping,
                    view.nominal_mappings(),
                )
                .unwrap()
        );
        for control in 0..5 {
            let mut forged = mapping.clone();
            match control {
                0 => forged.original.path = CarrierPath(Arc::from([CarrierStep::Field(0)])),
                1 => forged.substituted.path = CarrierPath(Arc::from([CarrierStep::Field(0)])),
                2 => {
                    forged.original_family = ir::CarrierFamily::Parameter(ir::ParamId {
                        index: parameter.index + 1,
                        ..parameter
                    })
                }
                3 => {
                    forged.original.channel = CarrierChannel::ParamTail {
                        parameter,
                        prefix: 1,
                    }
                }
                4 => {
                    forged.substituted.channel =
                        CarrierChannel::Fixed(ReferencePosition { depth: 1 })
                }
                _ => unreachable!(),
            }
            assert!(
                !builder
                    .identity_slice_component_matches(
                        parameter,
                        original,
                        expanded,
                        &forged,
                        view.nominal_mappings(),
                    )
                    .unwrap(),
                "forged relative mapping control {control}"
            );
        }
        // This is a proof-level forged result leaf, not an unsupported source
        // slice annotation. All original call/family/mapping authority is kept.
        let noncopy = Ty::Slice {
            element: Arc::new(Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(Ty::Int(ir::IntTy::I32)),
            }),
        };
        assert!(
            !builder
                .identity_copy_slice_reborrow_admitted(
                    &signature.params,
                    &noncopy,
                    &view,
                    Some(&facts),
                )
                .unwrap()
        );
    }

    #[test]
    fn identity_slice_reborrow_rows_reject_descriptor_family_aliases() {
        let program = frontend::parse("fn view<T>(){}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let parameter = facts.env().ordered_params().next().unwrap();
        let root = InputCarrier {
            param: 0,
            location: fixed_location(ReferencePosition { depth: 0 }),
        };
        let family = InputCarrier {
            param: 0,
            location: CarrierLocation {
                path: CarrierPath(Arc::from([
                    CarrierStep::Referent,
                    CarrierStep::SliceElement,
                ])),
                channel: CarrierChannel::ParamTail {
                    parameter,
                    prefix: 0,
                },
            },
        };
        let mut actual = family.clone();
        actual.location.channel = CarrierChannel::Fixed(ReferencePosition { depth: 0 });
        let original = ir::CarrierContract {
            inputs: Arc::from([root.clone(), family.clone()]),
            results: Arc::from([
                ir::CarrierReturn {
                    location: root.location.clone(),
                    sources: Arc::from([root.clone(), family.clone()]),
                },
                ir::CarrierReturn {
                    location: family.location.clone(),
                    sources: Arc::from([family.clone()]),
                },
            ]),
            writes: Arc::from([ir::CarrierWriteEnvelope {
                destination: family.clone(),
                sources: Arc::from([family.clone()]),
            }]),
        };
        let expanded = ir::CarrierContract {
            inputs: Arc::from([root.clone(), actual.clone()]),
            results: Arc::from([
                ir::CarrierReturn {
                    location: root.location.clone(),
                    sources: Arc::from([root.clone()]),
                },
                ir::CarrierReturn {
                    location: actual.location.clone(),
                    sources: Arc::from([actual.clone()]),
                },
            ]),
            writes: Arc::from([ir::CarrierWriteEnvelope {
                destination: actual.clone(),
                sources: Arc::from([actual.clone()]),
            }]),
        };
        let admitted = |original: &ir::CarrierContract, expanded: &ir::CarrierContract| {
            identity_slice_reborrow_rows(original, expanded, &mut Budget::new(1_000_000))
                .unwrap()
                .is_some()
        };
        assert!(admitted(&original, &expanded));
        let mut forged = original.clone();
        let mut rows = forged.results.to_vec();
        rows[0].sources = Arc::from([root.clone(), root.clone()]);
        forged.results = rows.into();
        assert!(!admitted(&forged, &expanded));
        let mut forged = expanded.clone();
        let mut rows = forged.results.to_vec();
        rows[0].sources = Arc::from([root.clone(), actual.clone()]);
        forged.results = rows.into();
        assert!(!admitted(&original, &forged));
        let mut forged = expanded.clone();
        let mut rows = forged.results.to_vec();
        rows[1].sources = Arc::from([root.clone()]);
        forged.results = rows.into();
        assert!(!admitted(&original, &forged));
        let mut forged = expanded.clone();
        let mut writes = forged.writes.to_vec();
        writes[0].sources = Arc::from([root]);
        forged.writes = writes.into();
        assert!(!admitted(&original, &forged));
        let mut forged = expanded.clone();
        let mut inputs = forged.inputs.to_vec();
        inputs[1].param = 1;
        forged.inputs = inputs.into();
        assert!(!admitted(&original, &forged));
        assert!(identity_slice_reborrow_rows(&original, &expanded, &mut Budget::new(0)).is_err());
    }

    #[test]
    fn identity_copy_reborrow_requires_exact_original_self_sources() {
        let input = |param| InputCarrier {
            param,
            location: CarrierLocation {
                path: ir::CarrierPath(Arc::from([
                    CarrierStep::Referent,
                    CarrierStep::SliceElement,
                ])),
                channel: CarrierChannel::Fixed(ReferencePosition { depth: 0 }),
            },
        };
        let row = ir::CarrierWriteEnvelope {
            destination: input(0),
            sources: Arc::from([input(0)]),
        };
        let original = ir::CarrierContract {
            inputs: Arc::from([input(0), input(1)]),
            results: Arc::from([]),
            writes: Arc::from([row.clone()]),
        };
        let admitted = |contract: &ir::CarrierContract| {
            exact_identity_write_rows(contract, &mut Budget::new(1_000_000)).unwrap()
        };
        assert!(admitted(&original));
        let mut forged = original.clone();
        let mut changed = row.clone();
        // Type and path remain identical. Parameter identity must not change.
        changed.sources = Arc::from([input(1)]);
        forged.writes = Arc::from([changed]);
        assert!(!admitted(&forged));
        let mut changed = row.clone();
        let mut sibling = input(0);
        sibling.location.path = ir::CarrierPath(Arc::from([
            CarrierStep::Referent,
            CarrierStep::ConstantIndex(0),
        ]));
        changed.sources = Arc::from([sibling]);
        forged.writes = Arc::from([changed]);
        assert!(!admitted(&forged));
        let mut changed = row.clone();
        changed.sources = Arc::from([]);
        forged.writes = Arc::from([changed]);
        assert!(!admitted(&forged));
        let mut changed = row.clone();
        changed.sources = Arc::from([input(0), input(1)]);
        forged.writes = Arc::from([changed]);
        assert!(!admitted(&forged));
        forged.writes = Arc::from([row.clone(), row]);
        assert!(!admitted(&forged));
        forged = original.clone();
        forged.inputs = Arc::from([input(1)]);
        assert!(!admitted(&forged));
        forged = original.clone();
        forged.writes = Arc::from([]);
        assert!(!admitted(&forged));
        assert!(exact_identity_write_rows(&original, &mut Budget::new(0)).is_err());
    }

    #[test]
    fn identity_fallback_does_not_admit_affine_results_or_replacement() {
        for source in [
            "fn pair<T,U>(x:T,y:U)->(T,U){(x,y)}fn main(){let mut a=1i32;let mut b=2i32;let (x,y)=pair::<&mut i32,&mut i32>(&mut a,&mut b);*x=10;*y=20;}",
            "fn put_and_view<'a,T>(dst:&'a mut T,x:T)->&'a mut T{*dst=x;dst}fn main(){let mut a=1i32;let mut b=2i32;let mut slot=&mut a;let r=put_and_view::<&mut i32>(&mut slot,&mut b);**r=31;}",
        ] {
            for workers in [1, 8] {
                let error = crate::compiler::compile(source.into(), workers, true)
                    .err()
                    .expect("multi-affine result unexpectedly admitted")
                    .to_string();
                assert!(
                    error.contains("affine carrier") || error.contains("bounded transport/write"),
                    "workers={workers}: {error}"
                );
            }
        }
    }

    #[test]
    fn call_binding_lookup_is_ordered_exact_and_logarithmically_metered() {
        let template = source_template(
            "fn id<T>(x:T)->T{x} fn probe<T>(x:T)->T{id::<T>(id::<T>(x))} fn main(){}",
            1,
        );
        let mut build_budget = Budget::new(32_000_000);
        let plan = Plan::build_validated(
            template.function(),
            template.facts(),
            None,
            &mut build_budget,
            128 * 1024 * 1024,
        )
        .unwrap();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        let effects = plan.effects().unwrap();
        let original = &effects.bindings;
        assert_eq!(original.len(), 2);
        let payload = |bindings: &[CallCarrierBinding]| {
            carrier_effects_payload_bytes(
                &effects.writes,
                &effects.must_overwrite,
                bindings,
                &mut Budget::new(32_000_000),
            )
            .unwrap()
        };
        let bytes = payload(original);
        assert!(bytes >= size_of::<CarrierEffects>() + 2 * size_of::<CallCarrierBinding>());
        // Temporary analysis storage and reservation bookkeeping cannot escape
        // merely because they were paid during this analysis invocation.
        build_budget.reserve_bytes(1024 * 1024).unwrap();
        assert_eq!(payload(original), bytes);
        let exact = ir::PlanBudget::new(ir::PlanLimits {
            bytes,
            ..ir::PlanLimits::default()
        });
        let retain = |ledger: &ir::PlanBudget| {
            retain_carrier_effects_payload(
                &effects.writes,
                &effects.must_overwrite,
                original,
                &mut Budget::new(32_000_000),
                ledger,
            )
        };
        assert_eq!(retain(&exact).unwrap(), bytes);
        assert_eq!(exact.usage_snapshot().bytes, bytes);
        let short = ir::PlanBudget::new(ir::PlanLimits {
            bytes: bytes - 1,
            ..ir::PlanLimits::default()
        });
        assert!(retain(&short).is_err());
        assert_eq!(
            short.usage_snapshot().first_failure.unwrap().kind,
            ir::ResourceBudgetKind::Bytes
        );
        let poisoned = ir::PlanBudget::default();
        poisoned.poison_ledger_for_tests();
        assert!(retain(&poisoned).is_err());
        assert!(poisoned.usage_snapshot().poisoned);
        let mut capacity_rows = original.to_vec();
        let before = payload(&capacity_rows);
        let old_capacity = capacity_rows[0].actual_writes.capacity();
        capacity_rows[0].actual_writes.reserve_exact(9);
        let added_capacity = capacity_rows[0].actual_writes.capacity() - old_capacity;
        assert_eq!(
            payload(&capacity_rows) - before,
            added_capacity * size_of::<Target>()
        );
        assert!(
            carrier_effects_payload_bytes(
                &effects.writes,
                &effects.must_overwrite,
                original,
                &mut Budget::new(0),
            )
            .is_err()
        );
        let subject_ok = |binding: &CallCarrierBinding| {
            call_binding_subject_matches(
                binding,
                template.function(),
                Some(template.function()),
                Some(template.facts()),
            )
        };
        assert!(original.iter().all(subject_ok));
        let mut calls = vec![None; template.function().blocks.len()];
        for binding in original.iter() {
            calls[binding.block.0] = Some(binding.contract.clone());
        }
        assert!(
            original
                .iter()
                .all(|binding| call_binding_view_matches(binding, &calls))
        );
        let mut foreign_view = original[0].clone();
        foreign_view.contract = Arc::new(foreign_view.contract.as_ref().clone());
        assert!(!call_binding_view_matches(&foreign_view, &calls));
        let mut missing_view = calls.clone();
        missing_view[original[0].block.0] = None;
        assert!(!call_binding_view_matches(&original[0], &missing_view));
        let mut stale = original[0].clone();
        stale.block = original[1].block;
        assert!(!subject_ok(&stale));
        stale.block = BlockId(usize::MAX);
        assert!(!subject_ok(&stale));
        let mut stale_call = original[0].clone();
        stale_call.call = original[1].call;
        assert!(!subject_ok(&stale_call));
        let cloned_function = Arc::new(template.function().as_ref().clone());
        assert!(!call_binding_subject_matches(
            &original[0],
            &cloned_function,
            Some(&cloned_function),
            Some(template.facts())
        ));
        assert!(!call_binding_subject_matches(
            &original[0],
            template.function(),
            Some(template.function()),
            None
        ));
        // The authored-result write consumer uses these same exact subject
        // and view guards before it can read any lifetime obligation.
        let slot_region =
            |binding: &CallCarrierBinding,
             function: &Arc<cfg::Function>,
             facts: Option<&Arc<super::super::traits::ValidatedFacts>>,
             calls: &[Option<Arc<super::super::traits::CarrierContractView>>]| {
                call_result_slot_region(
                    binding,
                    function,
                    Some(function),
                    facts,
                    calls,
                    &fixed_location(ReferencePosition { depth: 0 }),
                    &mut Budget::new(1_000_000),
                )
                .unwrap()
            };
        assert!(matches!(
            slot_region(
                &original[0],
                template.function(),
                Some(template.facts()),
                &calls
            ),
            Ok(None)
        ));
        for forged in [&foreign_view, &stale, &stale_call] {
            assert!(
                slot_region(forged, template.function(), Some(template.facts()), &calls).is_err()
            );
        }
        assert!(
            slot_region(
                &original[0],
                &cloned_function,
                Some(template.facts()),
                &calls
            )
            .is_err()
        );
        assert!(slot_region(&original[0], template.function(), None, &calls).is_err());
        assert!(
            slot_region(
                &original[0],
                template.function(),
                Some(template.facts()),
                &missing_view
            )
            .is_err()
        );
        assert!(ordered_call_bindings(original, &mut Budget::new(1)).unwrap());
        for (index, binding) in original.iter().enumerate() {
            assert_eq!(
                call_binding_index(original, binding.block, &mut Budget::new(2)).unwrap(),
                Some(index)
            );
        }
        assert_eq!(
            call_binding_index(original, BlockId(usize::MAX), &mut Budget::new(2)).unwrap(),
            None
        );
        assert_eq!(
            call_binding_index(&[], BlockId(0), &mut Budget::new(0)).unwrap(),
            None
        );
        let duplicates = vec![original[0].clone(), original[0].clone()];
        assert!(!ordered_call_bindings(&duplicates, &mut Budget::new(1)).unwrap());
        let reversed = vec![original[1].clone(), original[0].clone()];
        assert!(!ordered_call_bindings(&reversed, &mut Budget::new(1)).unwrap());
        assert!(ordered_call_bindings(original, &mut Budget::new(0)).is_err());
        // Build adversarial sparse keys from a genuine binding, without
        // publishing the altered occurrence rows as compiler facts.
        for count in [1usize, 2, 3, 4, 7, 8, 31, 32, 1024] {
            let rows: Vec<_> = (0..count)
                .map(|index| {
                    let mut row = original[0].clone();
                    row.block = BlockId(index * 2 + 1);
                    row
                })
                .collect();
            let bound = (usize::BITS - count.leading_zeros()) as usize;
            assert!(ordered_call_bindings(&rows, &mut Budget::new(count - 1)).unwrap());
            for key in [0, 1, count, count * 2 - 1, count * 2, usize::MAX] {
                let expected = rows.iter().position(|row| row.block == BlockId(key));
                assert_eq!(
                    call_binding_index(&rows, BlockId(key), &mut Budget::new(bound)).unwrap(),
                    expected
                );
            }
            // The leftmost failed probe consumes exactly floor(log2(n))+1
            // comparisons, not a rounded estimate paid after the search.
            assert!(call_binding_index(&rows, BlockId(0), &mut Budget::new(bound - 1)).is_err());
        }
    }

    #[test]
    fn parameter_equality_call_guards_cover_empty_and_nonempty_sources() {
        for (body, has_sources) in [
            ("<T as Source>::get(x)", false),
            ("<T as Source>::pass(x, fallback)", true),
        ] {
            let params = if has_sources { "x:T,fallback:U" } else { "x:T" };
            let source = format!(
                "trait Source:Sized{{type Out;fn get(self)->Self::Out;\
                 fn pass(self,value:Self::Out)->Self::Out;}}\
                 fn probe<T,U>({params})->U where T:Source<Out=U>{{{body}}}\
                 fn main(){{}}"
            );
            let template = source_template(&source, 0);
            let facts = template.facts();
            let body_view = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
            assert_eq!(
                !body_view.contract().results[0].sources.is_empty(),
                has_sources
            );
            let function = template.function();
            let (target, signature) = function
                .blocks
                .iter()
                .find_map(|block| match &block.terminator.value {
                    Terminator::Call {
                        target, signature, ..
                    } => Some((target, signature)),
                    _ => None,
                })
                .unwrap();
            let view = facts
                .carrier_contract(ir::SignatureSubject::Call(target))
                .unwrap();
            let row = view.contract().results.first().unwrap();
            assert_eq!(!row.sources.is_empty(), has_sources);
            assert!(matches!(signature.result, Ty::Param(_)));
            assert_ne!(signature.params[0], signature.result);
            assert!(
                result_flow_problem(
                    Some(&view),
                    Some(facts),
                    ir::SignatureSubject::Call(target),
                    signature,
                    &row.location,
                    &mut false,
                )
                .unwrap()
                .is_none()
            );
            assert!(matches!(
                result_flow_problem(
                    None,
                    Some(facts),
                    ir::SignatureSubject::Call(target),
                    signature,
                    &row.location,
                    &mut false,
                )
                .unwrap(),
                Some(Problem::Invalid(_))
            ));
            assert!(matches!(
                result_flow_problem(
                    Some(&view),
                    Some(facts),
                    ir::SignatureSubject::Body,
                    signature,
                    &row.location,
                    &mut false,
                )
                .unwrap(),
                Some(Problem::Invalid(_))
            ));
            let mut wrong_signature = signature.clone();
            wrong_signature.result = Ty::Bool;
            assert!(matches!(
                result_flow_problem(
                    Some(&view),
                    Some(facts),
                    ir::SignatureSubject::Call(target),
                    &wrong_signature,
                    &row.location,
                    &mut false,
                )
                .unwrap(),
                Some(Problem::Invalid(_))
            ));
            let mut wrong_location = row.location.clone();
            wrong_location.path = ir::CarrierPath(Arc::from([ir::CarrierStep::Field(0)]));
            assert!(matches!(
                result_flow_problem(
                    Some(&view),
                    Some(facts),
                    ir::SignatureSubject::Call(target),
                    signature,
                    &wrong_location,
                    &mut false,
                )
                .unwrap(),
                Some(Problem::Invalid(_))
            ));
            // These are universally checked source bodies with no selected impl.
            // The pass case deliberately retains its conservative U input union.
            ownership::verify_template(template).unwrap();
        }
    }

    #[test]
    fn parameter_equality_direct_forwarding_and_aggregate_returns_keep_u_tail() {
        for (result, expression) in [
            ("U", "get::<T,U>(x)"),
            ("(U,i32)", "(get::<T,U>(x),7)"),
            ("[U;1]", "[get::<T,U>(x)]"),
        ] {
            let source = format!(
                "trait Source:Sized{{type Out;fn get(self)->Self::Out;}}\
                 fn get<T,U>(x:T)->U where T:Source<Out=U>{{<T as Source>::get(x)}}\
                 fn forward<T,U>(x:T,unrelated:&i32)->{result} where T:Source<Out=U>{{{expression}}}\
                 fn main(){{}}"
            );
            let template = source_template(&source, 1);
            let facts = template.facts();
            let body = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
            assert_eq!(body.contract().results.len(), 1);
            let result = &body.contract().results[0];
            assert!(matches!(
                result.location.channel,
                CarrierChannel::ParamTail { .. }
            ));
            assert!(result.sources.is_empty());
            let function = template.function();
            let (target, signature) = function
                .blocks
                .iter()
                .find_map(|block| match &block.terminator.value {
                    Terminator::Call {
                        target, signature, ..
                    } => Some((target, signature)),
                    _ => None,
                })
                .unwrap();
            assert!(matches!(target, ir::Callee::Direct(_)));
            let view = facts
                .carrier_contract(ir::SignatureSubject::Call(target))
                .unwrap();
            assert!(view.contract().results[0].sources.is_empty());
            assert!(
                result_flow_problem(
                    Some(&view),
                    Some(facts),
                    ir::SignatureSubject::Call(target),
                    signature,
                    &view.contract().results[0].location,
                    &mut false,
                )
                .unwrap()
                .is_none()
            );
            ownership::verify_template(template).unwrap();
        }
        // The callee itself retains a rigid associated result. Only the
        // caller owns Out=U, so transporting the raw projection family must
        // replay that equality before publishing a U result channel.
        let template = source_template(
            "trait Source:Sized{type Out;fn get(self)->Self::Out;} fn get<T:Source>(x:T)->T::Out{x.get()} fn forward<T:Source<Out=U>,U>(x:T)->U{get(x)} fn main(){}",
            1,
        );
        let facts = template.facts();
        let (target, signature) = template
            .function()
            .blocks
            .iter()
            .find_map(|block| match &block.terminator.value {
                Terminator::Call {
                    target, signature, ..
                } => Some((target, signature)),
                _ => None,
            })
            .unwrap();
        let view = facts
            .source_carrier_contract(ir::SignatureSubject::Call(target))
            .unwrap();
        let row = &view.contract().results[0];
        assert!(matches!(
            row.location.channel,
            CarrierChannel::ParamTail { .. }
        ));
        for _ in 0..3 {
            assert!(
                result_flow_problem(
                    Some(&view),
                    Some(facts),
                    ir::SignatureSubject::Call(target),
                    signature,
                    &row.location,
                    &mut false
                )
                .unwrap()
                .is_none()
            );
        }
        let mut unrelated = row.location.clone();
        let parameter = facts.env().ordered_params().next().unwrap();
        unrelated.channel = CarrierChannel::ParamTail {
            parameter,
            prefix: 0,
        };
        assert!(matches!(
            result_flow_problem(
                Some(&view),
                Some(facts),
                ir::SignatureSubject::Call(target),
                signature,
                &unrelated,
                &mut false
            )
            .unwrap(),
            Some(Problem::Invalid(_))
        ));
        ownership::verify_template(template).unwrap();

        // A different associated family is eligible only through its exact
        // producer bound, not merely because both projections mention S.
        let template = source_template(
            "trait Make{type Out;fn get(&mut self)->Self::Out;} trait Source{type Item;type Producer:Make<Out=Self::Item>;type Other;} struct Wrap<S:Source>{producer:S::Producer,other:S::Other} fn next<S:Source>(x:&mut Wrap<S>)->S::Item{x.producer.get()} fn main(){}",
            0,
        );
        let facts = template.facts();
        let view = facts
            .source_carrier_contract(ir::SignatureSubject::Body)
            .unwrap();
        let sources = &view.contract().results[0].sources;
        assert!(!sources.is_empty());
        assert!(
            sources.iter().any(|source| source
                .location
                .path
                .0
                .contains(&ir::CarrierStep::Field(0)))
        );
        assert!(
            !sources.iter().any(|source| source
                .location
                .path
                .0
                .contains(&ir::CarrierStep::Field(1)))
        );
        view.validate_for(facts).unwrap();
        ownership::verify_template(template).unwrap();
    }

    #[test]
    fn parameter_equality_self_produced_u_does_not_acquire_copy_permission() {
        for (bound, succeeds) in [("U", false), ("U:Copy", true)] {
            let source = format!(
                "trait Source:Sized{{type Out;fn get(self)->Self::Out;}}\
                 fn duplicate<T,{bound}>(x:T)->(U,U) where T:Source<Out=U>{{\
                     let value=<T as Source>::get(x);(value,value)\
                 }}fn main(){{}}"
            );
            let template = source_template(&source, 0);
            let checked = ownership::verify_template(template);
            assert_eq!(checked.is_ok(), succeeds, "{source}: {checked:?}");
            if let Err(analysis) = checked {
                assert!(analysis.complete);
                assert!(
                    analysis.diagnostics.iter().any(|diagnostic| matches!(
                        diagnostic.kind,
                        ownership::ErrorKind::ReadNotDefinitelyInitialized
                    )),
                    "{:?}",
                    analysis.diagnostics
                );
            }
        }
    }
}
