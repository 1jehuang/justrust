//! Bounded, nonescaping B1 reference provenance and last-use loan analysis.
//! Runtime pointers are never used as provenance. Canonical targets and parent
//! authority are frozen at issuance, independently of later carrier rebinding.
use super::cfg::{self, BlockId, LoanId, Operand, Rvalue, Statement, Terminator};
use super::ir::{
    BorrowKind, FactsCtx, InputReference, LocalId, Place, Projection, ReferencePosition,
    ReturnContract, Ty,
};
use super::ownership::{Budget, Exhausted};
use std::collections::VecDeque;
use std::mem::size_of;

pub(super) const MAX_LOANS: usize = 4096;
pub(super) const MAX_MEMBERSHIPS: usize = 1_000_000;

#[derive(Clone, Copy, Debug)]
pub(super) enum Phase {
    Read,
    Write,
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
#[derive(Clone, Debug)]
pub(super) enum Problem {
    Conflict(String),
    Invalid(String),
    Unsupported(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CanonicalRoot {
    Owned(LocalId),
    ParameterReferent(LocalId, ReferencePosition),
}
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Default, PartialEq, Eq)]
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
#[derive(Clone, Copy)]
struct CarrierSlot {
    local: LocalId,
    position: ReferencePosition,
}

pub(super) fn dereference_count(place: &Place) -> usize {
    place
        .projection
        .iter()
        .take_while(|p| **p == Projection::Deref)
        .count()
}

fn carrier_index(
    refs: &[[Option<usize>; 2]],
    place: &Place,
    position: ReferencePosition,
) -> Option<usize> {
    let depth = dereference_count(place);
    if depth != place.projection.len() {
        return None;
    }
    refs.get(place.local.0)?
        .get(depth.checked_add(usize::from(position.depth))?)
        .copied()
        .flatten()
}

struct Point {
    origins: Origins,
    live: Set,
    activated: Set,
}

pub(super) struct Plan {
    refs: Vec<[Option<usize>; 2]>,
    offsets: Vec<usize>,
    points: Vec<Option<Point>>,
    outgoing: Vec<Option<Point>>,
    parents: Vec<Set>,
    lifetime_deps: Vec<Set>,
    targets: Vec<Vec<Target>>,
    kinds: Vec<BorrowKind>,
    two_phase: Set,
    activations: Vec<Vec<LoanId>>,
    pub problems: Vec<(BlockId, Option<usize>, Problem)>,
    pub bytes: usize,
}

fn add(a: usize, b: usize) -> Result<usize, Exhausted> {
    a.checked_add(b).ok_or(Exhausted)
}
fn mul(a: usize, b: usize) -> Result<usize, Exhausted> {
    a.checked_mul(b).ok_or(Exhausted)
}
fn successors(term: &Terminator) -> [Option<BlockId>; 2] {
    match term {
        Terminator::Goto(next) | Terminator::Call { next, .. } => [Some(*next), None],
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
        Terminator::Return | Terminator::Unreachable => [None, None],
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
        Rvalue::Use(op)
        | Rvalue::Unary { value: op, .. }
        | Rvalue::Repeat { value: op, .. }
        | Rvalue::IntCast { value: op, .. } => visit_operand(op, &mut visit),
        Rvalue::Binary { left, right, .. } => {
            visit_operand(left, &mut visit);
            visit_operand(right, &mut visit);
        }
        Rvalue::Aggregate { fields, .. } | Rvalue::EnumVariant { fields, .. } => {
            for op in fields {
                visit_operand(op, &mut visit);
            }
        }
        Rvalue::Discriminant(p) => visit(p, false),
        Rvalue::IndexCopy { array, index } => {
            visit(array, false);
            visit_operand(index, &mut visit);
        }
        Rvalue::Println(_) => {}
    }
}
fn point_uses(
    f: &cfg::Function,
    refs: &[[Option<usize>; 2]],
    block: usize,
    statement: usize,
    r: usize,
) -> (Set, Set) {
    let mut uses = Set::new(r);
    let mut defs = Set::new(r);
    let mut visit = |place: &Place, moved: bool| {
        for index in refs[place.local.0].iter().flatten().copied() {
            uses.insert(index);
            if moved && place.projection.is_empty() {
                defs.insert(index);
            }
        }
    };
    if let Some(item) = f.blocks[block].statements.get(statement) {
        match &item.value {
            Statement::Assign { destination, value } => {
                visit_rvalue(f, value, &mut visit);
                if destination.projection.contains(&Projection::Deref) {
                    visit(destination, false);
                }
                if destination.projection.is_empty() {
                    for index in refs[destination.local.0].iter().flatten() {
                        defs.insert(*index);
                    }
                }
            }
            Statement::StorageLive(local) | Statement::StorageDead(local) => {
                for index in refs[local.0].iter().flatten() {
                    defs.insert(*index);
                }
            }
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
                visit_operand(value, &mut visit);
                visit_operand(index, &mut visit);
            }
        }
    } else {
        match &f.blocks[block].terminator.value {
            Terminator::Switch { condition, .. } => visit_operand(condition, &mut visit),
            Terminator::SwitchVariant { place, .. } => visit(place, false),
            Terminator::Call {
                args, destination, ..
            } => {
                for op in args {
                    visit_operand(op, &mut visit);
                }
                if destination.projection.contains(&Projection::Deref) {
                    visit(destination, false);
                }
                if destination.projection.is_empty() {
                    for carrier in refs[destination.local.0].iter().flatten() {
                        defs.insert(*carrier);
                    }
                }
            }
            Terminator::Return => visit(
                &Place {
                    local: f.return_local,
                    projection: vec![],
                },
                false,
            ),
            _ => {}
        }
    }
    (uses, defs)
}
fn origin_step(
    f: &cfg::Function,
    refs: &[[Option<usize>; 2]],
    state: &mut Origins,
    item: &Statement,
    loans: usize,
    budget: &mut Budget,
) -> Result<(), Exhausted> {
    budget.spend(mul(state.len(), loans.div_ceil(64))? + 1)?;
    match item {
        Statement::StorageLive(local) | Statement::StorageDead(local) => {
            for index in refs[local.0].iter().flatten() {
                state[*index].clear();
            }
        }
        Statement::Assign { destination, value } => {
            if let Rvalue::Aggregate { fields, .. } | Rvalue::EnumVariant { fields, .. } = value {
                budget.spend(fields.len())?;
            }
            // Freeze the value before any explicit Move clears its source. Each
            // nested position transports existing origins, not an outer loan.
            let mut saved: [Option<Set>; 2] = [None, None];
            if destination.projection.is_empty() {
                for (depth, slot) in refs[destination.local.0].iter().enumerate() {
                    if slot.is_none() {
                        continue;
                    }
                    saved[depth] = match value {
                        Rvalue::Borrow(id) if depth == 0 => {
                            let mut set = Set::new(loans);
                            set.insert(id.0);
                            Some(set)
                        }
                        Rvalue::Borrow(id) => carrier_index(
                            refs,
                            &f.loan_sites[id.0].place,
                            ReferencePosition {
                                depth: (depth - 1) as u8,
                            },
                        )
                        .map(|index| state[index].clone()),
                        Rvalue::Use(Operand::Copy(source) | Operand::Move(source)) => {
                            carrier_index(refs, source, ReferencePosition { depth: depth as u8 })
                                .map(|index| state[index].clone())
                        }
                        _ => None,
                    };
                }
            }
            visit_rvalue(f, value, |source, moved| {
                if moved && source.projection.is_empty() {
                    for index in refs[source.local.0].iter().flatten() {
                        state[*index].clear();
                    }
                }
            });
            if destination.projection.is_empty() {
                for (index, saved) in refs[destination.local.0].iter().zip(saved) {
                    if let Some(index) = index {
                        state[*index] = saved.unwrap_or_else(|| Set::new(loans));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
fn origin_terminator(
    refs: &[[Option<usize>; 2]],
    state: &mut Origins,
    term: &Terminator,
    loans: usize,
    call_returns: &[Option<usize>; 2],
    budget: &mut Budget,
) -> Result<(), Exhausted> {
    if let Terminator::Call {
        args, destination, ..
    } = term
    {
        budget.spend(add(args.len(), mul(state.len(), loans.div_ceil(64))?)?)?;
        for arg in args {
            if let Operand::Move(place) = arg
                && place.projection.is_empty()
            {
                for carrier in refs[place.local.0].iter().flatten() {
                    state[*carrier].clear();
                }
            }
        }
        for (depth, origin) in call_returns.iter().enumerate() {
            if let Some(origin) = origin {
                let carrier =
                    carrier_index(refs, destination, ReferencePosition { depth: depth as u8 })
                        .expect("reference result carrier");
                state[carrier].clear();
                state[carrier].insert(*origin);
            }
        }
    }
    Ok(())
}
fn overlap(a: &CanonicalPlace, b: &CanonicalPlace, budget: &mut Budget) -> Result<bool, Exhausted> {
    budget.spend(a.projection.len().min(b.projection.len()) + 1)?;
    if a.root != b.root {
        return Ok(false);
    }
    for (a, b) in a.projection.iter().zip(&b.projection) {
        // Safe Rust's ordinary indexed borrows overlap the entire array in B1.
        if matches!(a, Projection::ConstantIndex(_)) || matches!(b, Projection::ConstantIndex(_)) {
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
    add(mul(locals, 96)?, mul(add(blocks, 1)?, 96)?)
}

pub(super) fn plan_storage_bytes(
    locals: usize,
    blocks: usize,
    points: usize,
    slots: usize,
    origins: usize,
) -> Result<usize, Exhausted> {
    let metadata = plan_metadata_bytes(locals, blocks)?;
    let origin_size = mul(slots, add(size_of::<Set>(), mul(origins.div_ceil(64), 8)?)?)?;
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
    mul(
        4,
        add(
            mul(allocations, size_of::<Target>())?,
            add(
                mul(
                    add(projection_steps, mul(allocations, 128)?)?,
                    size_of::<Projection>(),
                )?,
                mul(add(authority_entries, overhead_words)?, size_of::<usize>())?,
            )?,
        )?,
    )
}

fn append_target(
    next: &mut Vec<Target>,
    target: &Target,
    suffix: &[Projection],
    precision: TargetPrecision,
    parent: usize,
    allocation: (&mut usize, &mut usize, usize),
    budget: &mut Budget,
) -> Result<(), Exhausted> {
    let (bytes, memberships, max_bytes) = allocation;
    let precision = if target.precision == TargetPrecision::Subtree {
        TargetPrecision::Subtree
    } else {
        precision
    };
    let path_len = add(
        target.place.projection.len(),
        if precision == TargetPrecision::Exact {
            suffix.len()
        } else {
            0
        },
    )?;
    if path_len > 128 {
        return Err(Exhausted);
    }
    budget.spend(path_len + target.authority.len() + 1)?;
    let added_bytes = target_payload_bytes(1, path_len, target.authority.len(), 3)?;
    *bytes = add(*bytes, added_bytes)?;
    *memberships = add(*memberships, target.authority.len() + 2)?;
    if *bytes > max_bytes || *memberships > MAX_MEMBERSHIPS {
        return Err(Exhausted);
    }
    budget.reserve_bytes(added_bytes)?;
    let mut candidate = target.clone();
    candidate.precision = precision;
    if candidate.precision == TargetPrecision::Exact {
        candidate.place.projection.extend_from_slice(suffix);
    }
    if let Err(position) = candidate.authority.binary_search(&parent) {
        candidate.authority.insert(position, parent);
    }
    for existing in next.iter() {
        budget.spend(path_len + candidate.authority.len() + 1)?;
        if existing == &candidate {
            return Ok(());
        }
    }
    next.push(candidate);
    Ok(())
}

impl Plan {
    pub fn build(
        f: &cfg::Function,
        budget: &mut Budget,
        max_bytes: usize,
    ) -> Result<Self, Exhausted> {
        let metadata_bytes = plan_metadata_bytes(f.locals.len(), f.blocks.len())?;
        if metadata_bytes > max_bytes {
            return Err(Exhausted);
        }
        budget.reserve_bytes(metadata_bytes)?;
        budget.spend(f.locals.len() + f.params.len() + f.blocks.len())?;
        let mut parameter_origins = Vec::new();
        for &local in &f.params {
            for depth in
                0..super::ir::reference_layers(&f.locals[local.0].ty).map_err(|_| Exhausted)?
            {
                parameter_origins.push(CarrierSlot {
                    local,
                    position: ReferencePosition { depth: depth as u8 },
                });
            }
        }
        let base = add(f.loan_sites.len(), parameter_origins.len())?;
        let mut call_count = 0;
        for block in &f.blocks {
            if let Terminator::Call { signature, .. } = &block.terminator.value {
                call_count = add(
                    call_count,
                    super::ir::reference_layers(&signature.result).map_err(|_| Exhausted)?,
                )?;
            }
        }
        let loans = add(base, call_count)?;
        if loans > MAX_LOANS {
            return Err(Exhausted);
        }
        let mut refs = vec![[None; 2]; f.locals.len()];
        let mut r = 0;
        for (index, local) in f.locals.iter().enumerate() {
            for slot in refs[index]
                .iter_mut()
                .take(super::ir::reference_layers(&local.ty).map_err(|_| Exhausted)?)
            {
                *slot = Some(r);
                r += 1;
            }
        }
        let b = f.blocks.len();
        let mut next_call = base;
        let mut call_rows = vec![[None; 2]; b];
        for (block, body) in f.blocks.iter().enumerate() {
            if let Terminator::Call { signature, .. } = &body.terminator.value {
                for slot in call_rows[block]
                    .iter_mut()
                    .take(super::ir::reference_layers(&signature.result).map_err(|_| Exhausted)?)
                {
                    *slot = Some(next_call);
                    next_call += 1;
                }
            }
        }
        let mut offsets = Vec::with_capacity(b + 1);
        offsets.push(0usize);
        for block in &f.blocks {
            offsets.push(add(
                *offsets.last().unwrap(),
                add(block.statements.len(), 1)?,
            )?);
        }
        let p = *offsets.last().unwrap();
        // The same full dense/scratch accounting governs recipe admission.
        let mut bytes = plan_storage_bytes(f.locals.len(), b, p, r, loans)?;
        if bytes > max_bytes {
            return Err(Exhausted);
        }
        budget.reserve_bytes(bytes - metadata_bytes)?;
        budget.spend(plan_initial_work(b, p, r, loans)?)?;
        let mut reachable = vec![false; b];
        let mut queue = VecDeque::from([f.entry.0]);
        reachable[f.entry.0] = true;
        while let Some(block) = queue.pop_front() {
            budget.spend(1)?;
            for next in successors(&f.blocks[block].terminator.value)
                .into_iter()
                .flatten()
            {
                if !reachable[next.0] {
                    reachable[next.0] = true;
                    queue.push_back(next.0);
                }
            }
        }
        let mut uses = Vec::with_capacity(p);
        let mut defs = Vec::with_capacity(p);
        for (block, body) in f.blocks.iter().enumerate() {
            for statement in 0..=body.statements.len() {
                budget.spend(1)?;
                let (u, d) = point_uses(f, &refs, block, statement, r);
                uses.push(u);
                defs.push(d);
            }
        }
        let mut ins = vec![Set::new(r); b];
        let mut exits = vec![Set::new(r); b];
        loop {
            let mut changed = false;
            for block in (0..b).filter(|index| reachable[*index]).rev() {
                budget.spend(1)?;
                let mut out = Set::new(r);
                for next in successors(&f.blocks[block].terminator.value)
                    .into_iter()
                    .flatten()
                {
                    out.union(&ins[next.0], budget)?;
                }
                exits[block] = out.clone();
                for point in (offsets[block]..offsets[block + 1]).rev() {
                    budget.spend(out.0.len())?;
                    for ((live, def), used) in
                        out.0.iter_mut().zip(&defs[point].0).zip(&uses[point].0)
                    {
                        *live = (*live & !def) | used;
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
        let empty = || vec![Set::new(loans); r];
        let mut entries = vec![None; b];
        let mut entry = empty();
        for (index, slot) in parameter_origins.iter().enumerate() {
            entry[refs[slot.local.0][usize::from(slot.position.depth)]
                .expect("reference parameter carrier")]
            .insert(f.loan_sites.len() + index);
        }
        entries[f.entry.0] = Some(entry);
        let mut queue = VecDeque::from([f.entry.0]);
        let mut queued = vec![false; b];
        queued[f.entry.0] = true;
        let mut memberships = 0usize;
        while let Some(block) = queue.pop_front() {
            queued[block] = false;
            budget.spend(add(1, mul(r, loans.div_ceil(64))?)?)?;
            let mut out = entries[block].as_ref().unwrap().clone();
            for item in &f.blocks[block].statements {
                origin_step(f, &refs, &mut out, &item.value, loans, budget)?;
            }
            origin_terminator(
                &refs,
                &mut out,
                &f.blocks[block].terminator.value,
                loans,
                &call_rows[block],
                budget,
            )?;
            for next in successors(&f.blocks[block].terminator.value)
                .into_iter()
                .flatten()
            {
                let changed = if let Some(entry) = &mut entries[next.0] {
                    let mut changed = false;
                    for (a, b) in entry.iter_mut().zip(&out) {
                        let before = a.count();
                        changed |= a.union(b, budget)?;
                        memberships = add(memberships, a.count() - before)?;
                    }
                    changed
                } else {
                    memberships = add(memberships, out.iter().map(Set::count).sum())?;
                    entries[next.0] = Some(out.clone());
                    true
                };
                if memberships > MAX_MEMBERSHIPS {
                    return Err(Exhausted);
                }
                if changed && !queued[next.0] {
                    queue.push_back(next.0);
                    queued[next.0] = true;
                }
            }
        }
        let mut points: Vec<Option<Point>> = (0..p).map(|_| None).collect();
        let mut outgoing: Vec<Option<Point>> = (0..b).map(|_| None).collect();
        let mut births = vec![None; loans];
        for block in 0..b {
            if !reachable[block] {
                continue;
            }
            let mut state = entries[block].as_ref().unwrap().clone();
            let mut lives = vec![Set::new(r); f.blocks[block].statements.len() + 1];
            let mut live = exits[block].clone();
            for point in (offsets[block]..offsets[block + 1]).rev() {
                budget.spend(live.0.len())?;
                for ((word, def), used) in live.0.iter_mut().zip(&defs[point].0).zip(&uses[point].0)
                {
                    *word = (*word & !def) | used;
                }
                lives[point - offsets[block]] = live.clone();
            }
            for (statement, live) in lives.into_iter().enumerate() {
                budget.spend(mul(r, loans.div_ceil(64))? + 1)?;
                memberships = add(memberships, state.iter().map(Set::count).sum())?;
                if memberships > MAX_MEMBERSHIPS {
                    return Err(Exhausted);
                }
                points[offsets[block] + statement] = Some(Point {
                    origins: state.clone(),
                    live,
                    activated: Set::new(loans),
                });
                if let Some(item) = f.blocks[block].statements.get(statement) {
                    if let Statement::Assign {
                        value: Rvalue::Borrow(id),
                        ..
                    } = item.value
                    {
                        births[id.0] = Some((block, statement));
                    }
                    origin_step(f, &refs, &mut state, &item.value, loans, budget)?;
                }
            }
            for &row in call_rows[block].iter().flatten() {
                births[row] = Some((block, f.blocks[block].statements.len()));
            }
            origin_terminator(
                &refs,
                &mut state,
                &f.blocks[block].terminator.value,
                loans,
                &call_rows[block],
                budget,
            )?;
            memberships = add(memberships, state.iter().map(Set::count).sum())?;
            if memberships > MAX_MEMBERSHIPS {
                return Err(Exhausted);
            }
            outgoing[block] = Some(Point {
                origins: state,
                live: exits[block].clone(),
                activated: Set::new(loans),
            });
        }
        budget.spend(mul(loans, mul(loans.div_ceil(64), 3)?)?)?;
        let mut parents = vec![Set::new(loans); loans];
        for (loan, site) in f.loan_sites.iter().enumerate() {
            if site.place.projection.first() == Some(&Projection::Deref)
                && let (Some((block, statement)), Some(carrier)) = (
                    births[loan],
                    refs[site.place.local.0][dereference_count(&site.place) - 1],
                )
            {
                parents[loan] =
                    points[offsets[block] + statement].as_ref().unwrap().origins[carrier].clone();
                memberships = add(memberships, parents[loan].count())?;
            }
        }
        let mut lifetime_deps = parents.clone();
        for (block, rows) in call_rows.iter().enumerate() {
            let Some(point) = &points[offsets[block + 1] - 1] else {
                continue;
            };
            let Terminator::Call {
                args, signature, ..
            } = &f.blocks[block].terminator.value
            else {
                continue;
            };
            let contract = &signature.ref_contract;
            let regions = usize::from(contract.region_count);
            budget.spend(
                regions * regions
                    + contract.input_regions.len()
                    + contract.outlives.len() * 2
                    + args.len()
                    + 1,
            )?;
            let closure = contract.closure().expect("validated reference contract");
            for (depth, &row) in rows.iter().enumerate() {
                let Some(row) = row else { continue };
                let result = &contract.results[depth];
                for input in contract.input_regions.iter() {
                    let (Operand::Copy(place) | Operand::Move(place)) = &args[input.input.param]
                    else {
                        unreachable!()
                    };
                    let actual = &point.origins[carrier_index(&refs, place, input.input.position)
                        .expect("reference actual position")];
                    let mut selected = false;
                    for (candidate, _, _) in return_inputs(&result.returns) {
                        budget.spend(1)?;
                        selected |= candidate == input.input;
                    }
                    if selected {
                        parents[row].union(actual, budget)?;
                    }
                    if closure[input.region.0 as usize] & (1u64 << result.region.0) != 0 {
                        lifetime_deps[row].union(actual, budget)?;
                    }
                }
                memberships = add(
                    memberships,
                    parents[row].count() + lifetime_deps[row].count(),
                )?;
            }
        }
        memberships = add(
            memberships,
            lifetime_deps
                .iter()
                .take(f.loan_sites.len())
                .map(Set::count)
                .sum(),
        )?;
        if memberships > MAX_MEMBERSHIPS {
            return Err(Exhausted);
        }
        let mut targets: Vec<Vec<Target>> = vec![vec![]; loans];
        let mut ready = vec![false; loans];
        for (block, rows) in call_rows.iter().enumerate() {
            if !reachable[block] {
                for &row in rows.iter().flatten() {
                    ready[row] = true;
                }
            }
        }
        for (index, slot) in parameter_origins.iter().enumerate() {
            let row = f.loan_sites.len() + index;
            let old_bytes = bytes;
            bytes = add(bytes, target_payload_bytes(1, 0, 0, 2)?)?;
            memberships = add(memberships, 2)?;
            if bytes > max_bytes || memberships > MAX_MEMBERSHIPS {
                return Err(Exhausted);
            }
            budget.reserve_bytes(bytes - old_bytes)?;
            targets[row].push(Target {
                place: CanonicalPlace {
                    root: CanonicalRoot::ParameterReferent(slot.local, slot.position),
                    projection: vec![],
                },
                precision: TargetPrecision::Exact,
                authority: vec![],
            });
            ready[row] = true;
        }
        let mut problems = vec![];
        loop {
            let mut changed = false;
            for (loan, site) in f.loan_sites.iter().enumerate() {
                if ready[loan] {
                    continue;
                }
                budget.spend(1 + parents[loan].0.len())?;
                let reborrow = site.place.projection.first() == Some(&Projection::Deref);
                if reborrow && lifetime_deps[loan].members().any(|parent| !ready[parent]) {
                    continue;
                }
                let mut next = vec![];
                if reborrow {
                    for parent in parents[loan].members() {
                        for target in &targets[parent] {
                            budget.spend(
                                target.place.projection.len()
                                    + target.authority.len()
                                    + site.place.projection.len()
                                    + 1,
                            )?;
                            let path_len = add(
                                target.place.projection.len(),
                                if target.precision == TargetPrecision::Exact {
                                    site.place.projection.len() - dereference_count(&site.place)
                                } else {
                                    0
                                },
                            )?;
                            if path_len > 128 {
                                return Err(Exhausted);
                            }
                            // Include the same growth and resolved/issued peak
                            // used by descriptor admission.
                            let old_bytes = bytes;
                            bytes = add(
                                bytes,
                                target_payload_bytes(1, path_len, target.authority.len(), 3)?,
                            )?;
                            memberships = add(memberships, target.authority.len() + 2)?;
                            if bytes > max_bytes || memberships > MAX_MEMBERSHIPS {
                                return Err(Exhausted);
                            }
                            budget.reserve_bytes(bytes - old_bytes)?;
                            let mut place = target.place.clone();
                            if target.precision == TargetPrecision::Exact {
                                place.projection.extend_from_slice(
                                    &site.place.projection[dereference_count(&site.place)..],
                                );
                            }
                            let mut authority = target.authority.clone();
                            if let Err(position) = authority.binary_search(&parent) {
                                authority.insert(position, parent);
                            }
                            let candidate = Target {
                                place,
                                precision: target.precision,
                                authority,
                            };
                            let mut duplicate = false;
                            for existing in &next {
                                budget.spend(path_len + candidate.authority.len() + 1)?;
                                if existing == &candidate {
                                    duplicate = true;
                                    break;
                                }
                            }
                            if !duplicate {
                                next.push(candidate);
                            }
                        }
                    }
                } else {
                    let old_bytes = bytes;
                    bytes = add(
                        bytes,
                        target_payload_bytes(1, site.place.projection.len(), 0, 2)?,
                    )?;
                    memberships = add(memberships, 1)?;
                    if bytes > max_bytes || memberships > MAX_MEMBERSHIPS {
                        return Err(Exhausted);
                    }
                    budget.reserve_bytes(bytes - old_bytes)?;
                    next.push(Target {
                        place: CanonicalPlace::owned(&site.place),
                        precision: TargetPrecision::Exact,
                        authority: vec![],
                    });
                }
                targets[loan] = next;
                ready[loan] = true;
                changed = true;
            }
            budget.spend(call_rows.len())?;
            for (block, rows) in call_rows.iter().enumerate() {
                let Terminator::Call {
                    args, signature, ..
                } = &f.blocks[block].terminator.value
                else {
                    continue;
                };
                for (depth, &row) in rows.iter().enumerate() {
                    let Some(row) = row else { continue };
                    if ready[row] || lifetime_deps[row].members().any(|parent| !ready[parent]) {
                        continue;
                    }
                    budget.spend(lifetime_deps[row].0.len() + parents[row].0.len() + 1)?;
                    let Some(point) = &points[offsets[block + 1] - 1] else {
                        continue;
                    };
                    let mut next = vec![];
                    for (input, projection, precision) in
                        return_inputs(&signature.ref_contract.results[depth].returns)
                    {
                        budget.spend(projection.len() + 1)?;
                        let (Operand::Copy(actual) | Operand::Move(actual)) = &args[input.param]
                        else {
                            unreachable!()
                        };
                        let actual = &point.origins[carrier_index(&refs, actual, input.position)
                            .expect("reference actual position")];
                        budget.spend(actual.0.len())?;
                        for parent in actual.members() {
                            for target in &targets[parent] {
                                append_target(
                                    &mut next,
                                    target,
                                    projection,
                                    precision,
                                    parent,
                                    (&mut bytes, &mut memberships, max_bytes),
                                    budget,
                                )?;
                            }
                        }
                    }
                    targets[row] = next;
                    ready[row] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for (loan, ready) in ready.into_iter().enumerate() {
            if !ready && let Some((block, statement)) = births[loan] {
                problems.push((
                    BlockId(block),
                    (statement < f.blocks[block].statements.len()).then_some(statement),
                    Problem::Unsupported(
                        "cyclic reference origin dependencies are unsupported".into(),
                    ),
                ));
            }
        }
        // Deriving CallReturn kinds visits all blocks, including no-result calls.
        budget.spend(f.blocks.len())?;
        let mut two_phase = Set::new(loans);
        let mut activations = vec![vec![]; b];
        for (loan, site) in f.loan_sites.iter().enumerate() {
            budget.spend(1)?;
            if let cfg::LoanTiming::TwoPhase { activation, .. } = site.timing {
                two_phase.insert(loan);
                activations[activation.0].push(LoanId(loan));
            }
        }
        let mut plan = Self {
            refs,
            offsets,
            points,
            outgoing,
            parents,
            lifetime_deps,
            targets,
            kinds: f
                .loan_sites
                .iter()
                .map(|site| site.kind)
                .chain(parameter_origins.iter().map(|slot| {
                    super::ir::reference_at(&f.locals[slot.local.0].ty, slot.position)
                        .expect("validated formal position")
                        .0
                }))
                .chain(f.blocks.iter().flat_map(|block| {
                    let mut kinds = [None; 2];
                    if let Terminator::Call { signature, .. } = &block.terminator.value {
                        for result in signature.ref_contract.results.iter() {
                            kinds[usize::from(result.position.depth)] = Some(
                                super::ir::reference_at(&signature.result, result.position)
                                    .expect("validated result position")
                                    .0,
                            );
                        }
                    }
                    kinds.into_iter().flatten()
                }))
                .collect(),
            two_phase,
            activations,
            problems,
            bytes,
        };
        plan.compute_activation(f, &mut memberships, budget)?;
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
            let point = plan.outgoing[block].as_ref().unwrap();
            let active = plan.active(&point.origins, &ins[next], budget)?;
            for loan in active.members() {
                if let Some((birth, _)) = births[loan]
                    && component[birth] == component[block]
                {
                    plan.problems.push((
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
        if plan.problems.len() > 256 {
            return Err(Exhausted);
        }
        Ok(plan)
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
        budget.spend(place.projection.len() + 1)?;
        if place.projection.first() != Some(&Projection::Deref) {
            return Ok(Ok(vec![Target {
                place: CanonicalPlace::owned(place),
                precision: TargetPrecision::Exact,
                authority: vec![],
            }]));
        }
        let depth = dereference_count(place);
        let Some(carrier) = self.refs[place.local.0].get(depth - 1).copied().flatten() else {
            return Ok(Err(Problem::Invalid(
                "dereference has no reference carrier".into(),
            )));
        };
        let (origins, _) = self.point(block, statement, phase);
        budget.spend(origins[carrier].0.len())?;
        let mut resolved = vec![];
        for loan in origins[carrier].members() {
            for target in &self.targets[loan] {
                budget.spend(
                    target.place.projection.len()
                        + target.authority.len()
                        + place.projection.len()
                        + 1,
                )?;
                let mut next = target.clone();
                if next.precision == TargetPrecision::Exact {
                    next.place
                        .projection
                        .extend_from_slice(&place.projection[depth..]);
                }
                if next.place.projection.len() > 128 {
                    return Err(Exhausted);
                }
                next.authority.push(loan);
                resolved.push(next);
                if resolved.len() > MAX_MEMBERSHIPS {
                    return Err(Exhausted);
                }
            }
        }
        if resolved.is_empty() {
            if origins[carrier].count() != 0 {
                Ok(Err(Problem::Unsupported(
                    "reference has no supported whole-input target".into(),
                )))
            } else {
                Ok(Err(Problem::Invalid(
                    "reference has no issued canonical provenance".into(),
                )))
            }
        } else {
            Ok(Ok(resolved))
        }
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
        for result in f.ref_contract.results.iter() {
            if let Some(problem) = self.check_return_position(f, block, result, facts, budget)? {
                return Ok(Some(problem));
            }
        }
        Ok(None)
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
        let (origins, _) = self.point(block, None, Phase::Read);
        let carrier = self.refs[f.return_local.0][usize::from(result.position.depth)]
            .expect("reference return position");
        budget.spend(origins[carrier].0.len())?;
        if origins[carrier].count() == 0 {
            return Ok(Some(Problem::Invalid(
                "returned reference has no issued provenance".into(),
            )));
        }
        let mut dependencies = origins[carrier].clone();
        let mut queue: Vec<_> = dependencies.members().collect();
        for origin in origins[carrier].members() {
            if self.targets[origin].is_empty() {
                return Ok(Some(Problem::Unsupported(
                    "reference result has no supported whole-input target".into(),
                )));
            }
            for target in &self.targets[origin] {
                budget.spend(f.params.len() + target.place.projection.len() + 1)?;
                let CanonicalRoot::ParameterReferent(local, position) = target.place.root else {
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
                    if f.params[input.param] == local && input.position == position {
                        declared_input = true;
                        contained |= precision == TargetPrecision::Subtree
                            || ((target.precision == TargetPrecision::Exact || singleton)
                                && projection == target.place.projection.as_slice());
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
                let CanonicalRoot::ParameterReferent(local, position) = target.place.root else {
                    return Ok(Some(Problem::Invalid(
                        "returned reference lifetime depends on owned frame storage".into(),
                    )));
                };
                let param = f
                    .params
                    .iter()
                    .position(|param| *param == local)
                    .expect("formal canonical root");
                budget.spend(f.ref_contract.input_regions.len())?;
                let region = f
                    .ref_contract
                    .input_regions
                    .iter()
                    .find(|row| row.input == InputReference { param, position })
                    .expect("named reference input position")
                    .region;
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
        let CanonicalRoot::ParameterReferent(local, position) = target.place.root else {
            return Ok(false);
        };
        let Ok((_, input)) = super::ir::reference_at(&f.locals[local.0].ty, position) else {
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
            (Ty::Bool, Ty::Bool) => true,
            _ => false,
        })
    }
    pub fn check(
        &self,
        target: &Target,
        access: Access,
        block: BlockId,
        statement: Option<usize>,
        phase: Phase,
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let point = self.point_data(block, statement, phase);
        let active = self.active(&point.origins, &point.live, budget)?;
        for loan in active.members() {
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
        let Some(carrier) = self.refs[local.0][0] else {
            return Ok(None);
        };
        let (origins, live) = self.point(block, statement, Phase::Read);
        let active = self.active(origins, live, budget)?;
        for child in active.members() {
            let mut seen = self.parents[child].clone();
            let mut queue: Vec<usize> = seen.members().collect();
            while let Some(parent) = queue.pop() {
                budget.spend(1 + self.parents[parent].0.len())?;
                if origins[carrier].has(parent) {
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
