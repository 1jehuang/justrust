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

mod correlation;
mod renewal;

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
    fn walk(
        ty: &Ty,
        storage: &mut CanonicalPlace,
        cell_index: &HashMap<CellKey, usize>,
        output: &mut Vec<usize>,
        depth: usize,
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
            Ty::Param(_) => Ok(false),
            Ty::Int(_)
            | Ty::Float(_)
            | Ty::Char
            | Ty::Str
            | Ty::Bool
            | Ty::Unit
            | Ty::Never
            | Ty::RawPtr { .. } => Ok(true),
            _ => {
                if let Ty::Enum(value) = ty {
                    budget.spend(value.def().variants.len())?;
                }
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
                    let result = walk(ty, storage, cell_index, output, depth + 1, budget);
                    storage.projection.truncate(saved);
                    result
                };
                match ty {
                    Ty::Tuple(fields) => {
                        for (index, field) in fields.iter().enumerate() {
                            if !child(&[Projection::Field(index)], field)? {
                                return Ok(false);
                            }
                        }
                    }
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
                    Ty::Struct(value) => {
                        for (index, field) in value.def().fields.iter().enumerate() {
                            if !child(&[Projection::Field(index)], &field.ty)? {
                                return Ok(false);
                            }
                        }
                    }
                    Ty::Enum(value) => {
                        for (variant, definition) in value.def().variants.iter().enumerate() {
                            for (field, ty) in definition.fields.iter().enumerate() {
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
        let Some(&index) = cell_index.get(&key) else {
            return Ok(None);
        };
        let mut output = Vec::new();
        checked_vec_growth(&mut output, budget)?;
        output.push(index);
        return Ok(Some(output));
    }
    if *channel != CellChannel::ReferenceValue {
        return Ok(None);
    }
    let (local, location) = match &target.place.root {
        CanonicalRoot::Owned(local) => (*local, None),
        CanonicalRoot::ParameterReferent(local, location) => (*local, Some(location)),
    };
    let Some(mut ty) = original_locals.get(local.0) else {
        return Ok(None);
    };
    if let Some(location) = location {
        let depth = match location.channel {
            CarrierChannel::Fixed(position) => position.depth,
            CarrierChannel::ParamTail { prefix, .. }
            | CarrierChannel::ProjectionTail { prefix, .. }
            | CarrierChannel::OpaqueTail { prefix, .. } => prefix,
        };
        let length = add(mul(location.path.0.len(), 2)?, usize::from(depth))?;
        budget.reserve_bytes(mul(length, size_of::<Projection>())?)?;
        budget.spend(add(length, 1)?)?;
        let path = value_projection(location);
        let Ok(Ty::Ref { referent, .. }) = ir::projected_ty(ty, &path) else {
            return Ok(None);
        };
        ty = referent;
    }
    // A canonical object's physical coordinates never traverse another pointer.
    if target.place.projection.contains(&Projection::Deref) {
        return Ok(None);
    }
    let Ok(ty) = ir::projected_ty(ty, &target.place.projection) else {
        return Ok(None);
    };
    budget.reserve_bytes(mul(target.place.projection.len(), size_of::<Projection>())?)?;
    let mut storage = target.place.clone();
    let mut output = Vec::new();
    if !walk(ty, &mut storage, cell_index, &mut output, 0, budget)? {
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

fn resolve_cells<'a>(
    original_locals: &[Ty],
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
        && ir::raw_deref_index(local, &place.projection).is_some()
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
    for step in &place.projection {
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
                let Some(mut ty) = original_locals.get(local.0) else {
                    return Err(Exhausted);
                };
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
                    ty = match ir::projected_ty(ty, &path) {
                        Ok(Ty::Ref { referent, .. }) => referent,
                        _ => return Err(Exhausted),
                    };
                }
                ty = ir::projected_ty(ty, &candidate.place.projection).map_err(|_| Exhausted)?;
                if matches!(ty, Ty::Array { element, .. } if !matches!(element.as_ref(), Ty::Ref { kind: BorrowKind::Shared, .. }))
                    && *step != Projection::DynamicIndex(LocalId(usize::MAX))
                {
                    let mut selected = candidate;
                    selected.place.projection.push(*step);
                    checked_vec_growth(&mut expanded, budget)?;
                    expanded.push(selected);
                    continue;
                }
                let count = match ty {
                    Ty::Array { len, .. } => *len,
                    Ty::Slice { .. } => 1,
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
                    selected
                        .place
                        .projection
                        .push(if matches!(ty, Ty::Slice { .. }) {
                            Projection::DynamicIndex(LocalId(usize::MAX))
                        } else {
                            Projection::ConstantIndex(index)
                        });
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
        let mut next = Vec::new();
        for candidate in candidates {
            let Some(cover) = read_cell_cover(
                original_locals,
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

/// A root generic result instantiated to one reference retains the element's
/// declared family lifetime, not the lifetime of the slice descriptor. This is
/// shape selection for the existing authenticated result-flow and origin checks.
fn borrowed_slice_family_result(
    view: &super::traits::CarrierContractView,
    location: &CarrierLocation,
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    let fixed = CarrierChannel::Fixed(ReferencePosition { depth: 0 });
    if !location.path.0.is_empty() || location.channel != fixed {
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
                    budget.spend(1)?;
                    exact &= source.param == original_source.param
                        && source.location.path == original_source.location.path
                        && source.location.channel == fixed;
                }
                if exact {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
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
    initial: FlowState,
    reference_inputs: Vec<(InputCarrier, ReferenceOriginId)>,
    /// Issued only for literal data, never inferred from a reference type.
    static_origin: Option<ReferenceOriginId>,
    family_inputs: Vec<FamilyOrigin>,
    targets: Vec<Vec<Target>>,
    kinds: Vec<BorrowKind>,
    parents: Vec<Set>,
    dependencies: Vec<Set>,
    calls: Vec<Option<Arc<super::traits::CarrierContractView>>>,
    // Each block has one immutable callee/view and this builder has one fixed
    // facts/table pair. Cache only successful lookups, including no summary,
    // so dataflow revisits do not replay identical authority proofs.
    call_effects: Vec<Option<Option<Arc<CarrierEffects>>>>,
    body: Option<Arc<super::traits::CarrierContractView>>,
    body_signature: Option<ir::Signature>,
    call_rows: Vec<Vec<(CarrierLocation, ReferenceOriginId)>>,
    raw_slice_rows: Vec<(BlockId, usize, ReferenceOriginId)>,
    function: Option<Arc<cfg::Function>>,
    authority: Option<Arc<super::traits::ValidatedFacts>>,
    /// Exact `(function, facts)` pair whose provenance already validated in
    /// this plan. Validation is a pure function of these immutable Arcs, so a
    /// repeated per-call check would only redo O(blocks * evidence) work.
    validated_provenance: Option<(Arc<cfg::Function>, Arc<super::traits::ValidatedFacts>)>,
    bindings: Vec<CallCarrierBinding>,
    regions: RegionRelations,
    effect_table: Option<Arc<super::ownership::CarrierEffectTable>>,
    input_regions: Vec<(InputCarrier, AnalysisRegionId)>,
    family_regions: Vec<(ir::CarrierFamily, CarrierLocation, AnalysisRegionId)>,
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
                proof.validate_cfg(function, facts, &self.ledger)?;
            }
        } else {
            // Canonical provenance includes the exact body-substitution proof.
            function.validate_provenance(facts, &self.ledger)?;
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
                        ir::projected_ty(&signature.params[input.input.param], &prefix[..depth]),
                        Ok(Ty::Ref {
                            kind: BorrowKind::Mut,
                            ..
                        })
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
                let prefix = value_projection(&original.location);
                for (index, step) in prefix.iter().enumerate() {
                    if *step == Projection::Deref
                        && matches!(
                            ir::projected_ty(&signature.params[original.param], &prefix[..index]),
                            Ok(Ty::Ref {
                                kind: BorrowKind::Mut,
                                ..
                            })
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
        &self,
        view: &super::traits::CarrierContractView,
        family: &ir::CarrierFamily,
    ) -> Result<bool, Exhausted> {
        match family {
            ir::CarrierFamily::Parameter(parameter) => Ok(view
                .parameter_actual(*parameter)
                .map_err(|_| Exhausted)?
                .is_none()),
            ir::CarrierFamily::Projection(_) | ir::CarrierFamily::Opaque(_) => {
                let facts = self.authority.as_ref().ok_or(Exhausted)?;
                match view.family_actual(family, facts).map_err(|_| Exhausted)? {
                    Some(Ty::AssociatedType(_)) | Some(Ty::Opaque(_)) => Ok(true),
                    Some(_) => Ok(false),
                    None => Err(Exhausted),
                }
            }
        }
    }
    fn transport_effect_input(
        &mut self,
        input: &InputCarrier,
        view: &super::traits::CarrierContractView,
        owned_destination: bool,
    ) -> Result<Vec<(InputCarrier, Option<CarrierLocation>)>, Exhausted> {
        let mut result = Vec::new();
        match &input.location.channel {
            CarrierChannel::ParamTail { .. }
            | CarrierChannel::ProjectionTail { .. }
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
                if result.is_empty() && self.family_is_open(view, &family)? {
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
        for row in source.carriers.iter().filter(|row| {
            row.parameter.is_some() && row.location.path.0.contains(&CarrierStep::SliceElement)
        }) {
            let input = InputCarrier {
                param: row.parameter.ok_or(Exhausted)?,
                location: row.location.clone(),
            };
            for (_, region) in self.body_term_regions(&row.region)? {
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
            | ir::CarrierRegionTerm::OpaqueFamily { .. } => {
                let family = match term {
                    ir::CarrierRegionTerm::Family { parameter } => {
                        ir::CarrierFamily::Parameter(*parameter)
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
            for target in &self.targets[origin] {
                if let CanonicalRoot::ParameterReferent(local, location) = &target.place.root {
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
            if !mapped {
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
        target: Target,
    ) -> Result<bool, Exhausted> {
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
            &self.cells,
            &self.cell_index,
            &self.targets,
            |index| &state.cells[index].references.0,
            place,
            self.budget,
        )
    }
    fn read_value(
        &mut self,
        state: &FlowState,
        place: &Place,
        ty: &Ty,
    ) -> Result<Vec<(CarrierLocation, CarrierOrigins)>, Exhausted> {
        let locations = ir::carrier_locations(ty, &self.ledger).map_err(|_| Exhausted)?;
        let mut output = Vec::new();
        if ir::raw_deref_index(&self.f.locals[place.local.0].ty, &place.projection).is_some() {
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
            let Some(channel) = key_channel(semantic) else {
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
                if let Some(cover) = read_cell_cover(
                    &self.original_locals,
                    &self.cell_index,
                    &target,
                    &channel,
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
    fn place_ty<'a>(&self, f: &'a cfg::Function, place: &Place) -> Result<&'a Ty, Exhausted> {
        ir::projected_ty(&f.locals[place.local.0].ty, &place.projection).map_err(|_| Exhausted)
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
                    if let Some(channel) = key_channel(semantic)
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
        physical: Option<&'a super::opaque_physical::PhysicalValidation<'a>>,
    ) -> Result<Self, Exhausted> {
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
                        kinds.push(*kind);
                    }
                    CarrierChannel::ParamTail { .. }
                    | CarrierChannel::ProjectionTail { .. }
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
                    f.validate_provenance(authority, &ledger)
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
                    kinds.push(*kind);
                }
            }
            call_rows.push(rows);
        }
        let static_origin = if f.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.value,
                    Statement::Assign {
                        value: Rvalue::ByteStr(_),
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
            calls: vec![None; f.blocks.len()],
            call_effects: vec![None; f.blocks.len()],
            body: None,
            body_signature: None,
            call_rows,
            raw_slice_rows,
            function: None,
            authority: None,
            validated_provenance: None,
            bindings: vec![],
            regions: RegionRelations::default(),
            effect_table: None,
            input_regions: vec![],
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
                let channel = key_channel(ty).ok_or(Exhausted)?;
                builder.add_cell(
                    CellKey {
                        storage: CanonicalPlace {
                            root: CanonicalRoot::Owned(LocalId(local)),
                            projection: path,
                        },
                        channel,
                    },
                    ty,
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
                        channel: key_channel(semantic).ok_or(Exhausted)?,
                    },
                    semantic,
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
            let signature = body_signature(f, builder.budget)?;
            let body_view = if let Some(physical) = physical {
                physical
                    .body_carriers(f, authority)
                    .and_then(|boundary| boundary.realize())
                    .map_err(|message| {
                        super::traits::FactsError::from_message(authority.budget(), &message)
                    })
            } else {
                super::traits::NominalCarrierSource::new(f, authority).body()
            };
            match body_view {
                Ok(view) => {
                    let mut valid = true;
                    let mut view_validated = false;
                    for result in view.contract().results.iter() {
                        if let Some(problem) = result_flow_problem(
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
            let mut nominal_calls = super::traits::NominalCarrierSource::new(f, authority);
            for (index, block) in f.blocks.iter().enumerate() {
                if let Terminator::Call {
                    target, signature, ..
                } = &block.terminator.value
                {
                    let call_view = if let Some(physical) = physical {
                        physical
                            .call_carriers(f, authority, BlockId(index))
                            .and_then(|boundary| boundary.realize())
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
                                if let Some(problem) = result_flow_problem(
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
            builder.body_signature = Some(signature);
        }
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
            if let Some(cover) = read_cell_cover(
                &self.original_locals,
                &self.cell_index,
                &target,
                &channel,
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
        )
        .is_some()
        {
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
            if diagnose && indices.len() != targets.len() {
                self.problem(
                    block,
                    statement,
                    Problem::Unsupported(
                        "carrier write has no complete original typed cell cover".into(),
                    ),
                )?;
            }
            let copy = self.copy_permission(&semantic)?;
            if diagnose && !copy && indices.len() != 1 {
                self.problem(
                    block,
                    statement,
                    Problem::Unsupported(
                        "affine carrier write requires one exact canonical destination".into(),
                    ),
                )?;
            }
            for index in indices {
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
                if definite && targets.len() == 1 {
                    state.cells[index] = origins.clone();
                    state.overwritten.insert(index);
                } else {
                    state.cells[index].union(origins, self.budget)?;
                }
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
            Rvalue::ByteStr(_) => {
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
                // The CFG gate has replayed the exact original publication and
                // wrapper. Read the actual operand once, then require complete
                // ordered correspondence before merging any of its origins.
                let carriers = self.operand_value(state, value)?;
                let sources = proof.captured_sources();
                self.budget.spend(carriers.len() + sources.len() + 1)?;
                if carriers.len() != sources.len()
                    || carriers
                        .iter()
                        .zip(sources)
                        .any(|((actual, _), expected)| actual != expected)
                {
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
                for (_, origins) in carriers {
                    merged.union(&origins, self.budget)?;
                }
                Ok(vec![(proof.captured_destination(), merged)])
            }
            Rvalue::Use(operand) => self.operand_value(state, operand),
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
                        Rvalue::EnumVariant { ty, variant, .. } => ty
                            .def()
                            .variants
                            .get(*variant)
                            .and_then(|v| v.fields.get(index))
                            .is_some_and(|f| f.ty.is_shared_str() && !f.lifetime_bound),
                        Rvalue::Aggregate {
                            ty: Ty::Struct(ty), ..
                        } => ty
                            .def()
                            .fields
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
                let len = *len;
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
            }
            Statement::IndexAssign {
                array,
                index: subscript,
                value,
            } => {
                // `(*s)[i] = v` through a slice carrier (MUTABLE-SLICES.md):
                // slice elements are reference-free (slice element boundary),
                // so the write cannot create or replace a carrier component.
                if matches!(
                    ir::indexed_array_ty(
                        &self.f.locals[array.local.0].ty,
                        &array.projection,
                        false
                    ),
                    Ok(Ty::Slice { .. })
                ) {
                    self.consume(state, value)?;
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
                let count = *count;
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
            | Statement::Inspect(_) => {}
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
        if facts.is_some() && view.is_none() {
            self.problem(
                block,
                None,
                Problem::Invalid("call result has no validated declaration view".into()),
            )?;
            return Ok(());
        }
        if let Some(view) = &view {
            let mut view_validated = false;
            for result in view.contract().results.iter() {
                if let Some(problem) = result_flow_problem(
                    Some(view),
                    facts,
                    ir::SignatureSubject::Call(&target),
                    &signature,
                    &result.location,
                    &mut view_validated,
                )? {
                    self.problem(block, None, problem)?;
                    return Ok(());
                }
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
        let mut view_validated = false;
        for (location, row) in rows {
            if let Some(problem) = result_flow_problem(
                view.as_deref(),
                facts,
                ir::SignatureSubject::Call(&target),
                &signature,
                &location,
                &mut view_validated,
            )? {
                self.problem(block, None, problem)?;
                return Ok(());
            }
            let mut value = self.empty_origins()?;
            let mut fixed_contract = false;
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
                for (input, suffix, precision) in return_inputs(&result.returns) {
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
                for source in result
                    .sources
                    .iter()
                    .filter(|source| !source.location.path.0.is_empty())
                {
                    let input = self.input_value(&entry, &args, source)?;
                    for parent in input.references.0.members() {
                        self.parents[row.0].insert(parent);
                        self.dependencies[row.0].insert(parent);
                        for target in self.targets[parent].clone() {
                            let mut target = target;
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
                for parent in input.references.0.members() {
                    self.parents[row.0].insert(parent);
                    self.dependencies[row.0].insert(parent);
                    if fixed_sourceless {
                        // Liveness only: the structural source keeps the
                        // input loans alive but is not an exact place target.
                        continue;
                    }
                    for target in self.targets[parent].clone() {
                        let mut target = target;
                        if !target.authority.contains(&parent) {
                            target.authority.push(parent);
                            target.authority.sort_unstable();
                        }
                        self.push_target(row, target)?;
                    }
                }
            }
            value.references.0.insert(row.0);
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
                match table.lookup(&target, self.f, &facts, view, caller_validated) {
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
                for (destination, component) in
                    self.transport_effect_input(&write.destination, view, true)?
                {
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
                        for (source, source_component) in
                            self.transport_effect_input(source, view, false)?
                        {
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
                if diagnose && (indices.len() != targets.len() || (!copy && indices.len() != 1)) {
                    self.problem(block, None, Problem::Unsupported("call carrier effect lacks one affine target or a complete typed copy-cell cover".into()))?;
                }
                let mutable = destination
                    .projection
                    .iter()
                    .rposition(|step| *step == Projection::Deref)
                    .and_then(|depth| {
                        ir::projected_ty(
                            &self.f.locals[destination.local.0].ty,
                            &destination.projection[..depth],
                        )
                        .ok()
                    })
                    .is_some_and(|ty| {
                        matches!(
                            ty,
                            Ty::Ref {
                                kind: BorrowKind::Mut,
                                ..
                            }
                        )
                    });
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
                    self.used_place(state, cleanup.owner(), &mut used, &mut defs, true)?;
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
            match facts.must_copy(ty) {
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
            match self.facts.must_copy(ty) {
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
                self.metadata_value(ir::carrier_at(result, location, &self.ledger).cloned())?
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
            for (destination, _) in self.transport_effect_input(&write.destination, view, true)? {
                write_channels = add(write_channels, 1)?;
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
                let Some(semantic) = self.metadata_value(
                    ir::carrier_at(parameter, &destination.location, &self.ledger).cloned(),
                )?
                else {
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
        Ok((affine_results == 0 && affine_writes == 0)
            || (affine_results <= 1 && write_channels == 0)
            || (affine_writes <= 1 && result_channels == 0)
            || independent_pair)
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
    fn solve(mut self) -> Result<Plan, Exhausted> {
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
                let binding_validation = if let Some(physical) = self.physical {
                    physical
                        .validate_carrier_view(
                            self.f,
                            &binding.facts,
                            Some(binding.call),
                            &binding.contract,
                        )
                        .map_err(|message| {
                            super::traits::FactsError::from_message(&self.ledger, &message)
                        })
                } else {
                    binding.contract.validate_for(&binding.facts)
                };
                if let Err(error) = binding_validation {
                    if error.is_resource() {
                        return Err(Exhausted);
                    }
                    invalid_binding = true;
                }
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
            if (family_admitted || identity_admitted) && self.problems.is_empty() {
                checked_arc_transfer::<ir::CarrierWriteEnvelope>(writes.len(), self.budget)?;
                checked_arc_transfer::<InputCarrier>(must_overwrite.len(), self.budget)?;
                checked_arc_transfer::<CallCarrierBinding>(self.bindings.len(), self.budget)?;
                self.retain(size_of::<CarrierEffects>() + size_of::<[usize; 8]>(), 1)?;
                // All effect rows, occurrence snapshots, paths and component sets
                // are subsets of this analysis's paid allocations. Retain that
                // conservative whole-allocation bound in the SAME planning ledger
                // before publishing an Arc that outlives the local Budget tokens.
                let retained_payload = self
                    .budget
                    .reserved_bytes()
                    .checked_sub(self.allocation_start)
                    .ok_or(Exhausted)?;
                self.ledger
                    .retain_bytes(retained_payload)
                    .map_err(|_| Exhausted)?;
                Some(Arc::new(CarrierEffects {
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
            targets: self.targets,
            kinds: self.kinds,
            two_phase,
            activations,
            cells: self.cells,
            cell_index: self.cell_index,
            original_locals: self.original_locals,
            family_inputs: self.family_inputs,
            body: self.body,
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
        Statement::Inspect(place) => visit(place, false),
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
    // Lengths of each runtime step, and the total concretization count.
    let mut lens = Vec::new();
    let mut total: usize = 1;
    for step in target {
        budget.spend(1)?;
        ty = match step {
            Projection::Field(index) => match ty.field_ty(*index) {
                Some(ty) => ty,
                None => return Ok(false),
            },
            Projection::ConstantIndex(_) | Projection::DynamicIndex(_) => {
                let Ty::Array { element, len } = ty else {
                    return Ok(false);
                };
                let len = *len;
                if matches!(step, Projection::DynamicIndex(_)) {
                    lens.push(len);
                    total = match total.checked_mul(len) {
                        Some(total) if total <= 4096 => total,
                        _ => return Ok(false),
                    };
                }
                element
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

pub(super) struct Plan {
    offsets: Vec<usize>,
    points: Vec<Option<Point>>,
    outgoing: Vec<Option<Point>>,
    parents: Vec<Set>,
    lifetime_deps: Vec<Set>,
    targets: Vec<Vec<Target>>,
    kinds: Vec<BorrowKind>,
    two_phase: Set,
    activations: Vec<Vec<LoanId>>,
    cells: Vec<CellSlot>,
    cell_index: HashMap<CellKey, usize>,
    original_locals: Vec<Ty>,
    family_inputs: Vec<FamilyOrigin>,
    body: Option<Arc<super::traits::CarrierContractView>>,
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
                    CarrierStep::Referent | CarrierStep::Field(_) | CarrierStep::ConstantIndex(_)
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
        Rvalue::Borrow(id) => visit(&f.loan_sites[id.0].place, false),
        Rvalue::OpaquePack { value: op, .. }
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
        Rvalue::TypeAlignment(_)
        | Rvalue::Println(_)
        | Rvalue::Output(_)
        | Rvalue::Str(_)
        | Rvalue::ByteStr(_) => {}
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
            Some(physical),
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
        builder.solve()
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
                if let Some(birth) = births[loan]
                    && component[birth] == component[block]
                    && !self.killed_before_rebirth(f, BlockId(next), loan, budget)?
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
            budget.spend(self.lifetime_deps[loan].0.len() + 1)?;
            for parent in self.lifetime_deps[loan].members() {
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
                if let Some(problem) = result_flow_problem(
                    Some(body),
                    authority,
                    ir::SignatureSubject::Body,
                    signature,
                    &row.location,
                    &mut view_validated,
                )? {
                    return Ok(Some(problem));
                }
                let projection = value_projection(&row.location);
                // Returning a handle transports its owned carrier components,
                // not the current contents of its pointee. In particular an
                // opaque T behind &T is not read as a reference-valued cell.
                // Positioned reference origins are checked independently below.
                if projection.contains(&Projection::Deref) {
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
                            return Ok(Some(Problem::Invalid(
                                "returned carrier escapes owned frame storage".into(),
                            )));
                        };
                        if !row.sources.iter().any(|input| {
                            f.params.get(input.param) == Some(local) && input.location == *location
                        }) {
                            return Ok(Some(Problem::Invalid(
                                "returned carrier violates its declaration-owned input occurrence"
                                    .into(),
                            )));
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
                                return Ok(Some(Problem::Invalid(
                                    "returned carrier lifetime depends on owned frame storage"
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
            if let Some(problem) = self.check_return_position(f, block, result, facts, budget)? {
                return Ok(Some(problem));
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
                    &self.original_locals,
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
        let anchor = match authority.projected_ty(input, &target.place.projection) {
            Ok(anchor) => anchor,
            Err(error) if error.is_resource() => return Err(Exhausted),
            Err(_) => return Ok(false),
        };
        Ok(match (anchor, output) {
            (Ty::Int(a), Ty::Int(b)) => a == b,
            (Ty::Float(a), Ty::Float(b)) => a == b,
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
            if matches!(access, Access::Read | Access::SharedBorrow)
                && (self.kinds[loan] == BorrowKind::Shared || reserved)
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
                    return Ok(Some(Problem::Conflict(format!(
                        "access conflicts with live {:?} loan {}",
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
        let subject_ok = |binding: &CallCarrierBinding| {
            call_binding_subject_matches(
                binding,
                template.function(),
                Some(template.function()),
                Some(template.facts()),
            )
        };
        assert!(original.iter().all(subject_ok));
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
