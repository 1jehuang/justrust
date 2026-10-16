//! Bounded, nonescaping B1 reference provenance and last-use loan analysis.
//! Runtime pointers are never used as provenance. Canonical targets and parent
//! authority are frozen at issuance, independently of later carrier rebinding.
use super::cfg::{self, BlockId, LoanId, Operand, Rvalue, Statement, Terminator};
use super::ir::{BorrowKind, LocalId, Place, Projection, ReturnContract, Ty};
use super::ownership::{Budget, Exhausted};
use std::collections::VecDeque;
use std::mem::size_of;

const MAX_LOANS: usize = 4096;
const MAX_MEMBERSHIPS: usize = 1_000_000;

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
    ParameterReferent(LocalId),
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
            CanonicalRoot::Owned(local) | CanonicalRoot::ParameterReferent(local) => local,
        };
        Place {
            local,
            projection: self.projection.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub place: CanonicalPlace,
    /// Only this candidate's ancestors, never a union of alternative exemptions.
    pub authority: Vec<usize>,
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
    live: Set,
}

pub(super) struct Plan {
    refs: Vec<Option<usize>>,
    offsets: Vec<usize>,
    points: Vec<Option<Point>>,
    outgoing: Vec<Option<Point>>,
    parents: Vec<Set>,
    lifetime_deps: Vec<Set>,
    targets: Vec<Vec<Target>>,
    kinds: Vec<BorrowKind>,
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
        Rvalue::Use(op) | Rvalue::Unary { value: op, .. } | Rvalue::Repeat { value: op, .. } => {
            visit_operand(op, &mut visit)
        }
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
    refs: &[Option<usize>],
    block: usize,
    statement: usize,
    r: usize,
) -> (Set, Set) {
    let mut uses = Set::new(r);
    let mut defs = Set::new(r);
    let mut visit = |place: &Place, moved: bool| {
        if let Some(index) = refs[place.local.0] {
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
                if destination.projection.is_empty()
                    && let Some(index) = refs[destination.local.0]
                {
                    defs.insert(index);
                }
            }
            Statement::StorageLive(local) | Statement::StorageDead(local) => {
                if let Some(index) = refs[local.0] {
                    defs.insert(index);
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
                if destination.projection.is_empty()
                    && let Some(carrier) = refs[destination.local.0]
                {
                    defs.insert(carrier);
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
    refs: &[Option<usize>],
    state: &mut Origins,
    item: &Statement,
    loans: usize,
    budget: &mut Budget,
) -> Result<(), Exhausted> {
    budget.spend(mul(state.len(), loans.div_ceil(64))? + 1)?;
    match item {
        Statement::StorageLive(local) | Statement::StorageDead(local) => {
            if let Some(index) = refs[local.0] {
                state[index].clear();
            }
        }
        Statement::Assign { destination, value } => {
            if let Rvalue::Aggregate { fields, .. } | Rvalue::EnumVariant { fields, .. } = value {
                budget.spend(fields.len())?;
            }
            let target = if destination.projection.is_empty() {
                refs[destination.local.0]
            } else {
                None
            };
            let saved = if target.is_some() {
                match value {
                    Rvalue::Borrow(id) => {
                        let mut set = Set::new(loans);
                        set.insert(id.0);
                        Some(set)
                    }
                    Rvalue::Use(Operand::Copy(source) | Operand::Move(source))
                        if source.projection.is_empty() =>
                    {
                        refs[source.local.0].map(|index| state[index].clone())
                    }
                    _ => None,
                }
            } else {
                None
            };
            visit_rvalue(f, value, |source, moved| {
                if moved
                    && source.projection.is_empty()
                    && let Some(index) = refs[source.local.0]
                {
                    state[index].clear();
                }
            });
            if let Some(target) = target {
                state[target] = saved.unwrap_or_else(|| Set::new(loans));
            }
        }
        _ => {}
    }
    Ok(())
}
fn origin_terminator(
    refs: &[Option<usize>],
    state: &mut Origins,
    term: &Terminator,
    loans: usize,
    call_return: Option<usize>,
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
                && let Some(carrier) = refs[place.local.0]
            {
                state[carrier].clear();
            }
        }
        if let Some(origin) = call_return {
            let carrier = refs[destination.local.0].expect("reference result carrier");
            state[carrier].clear();
            state[carrier].insert(origin);
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

fn append_target(
    next: &mut Vec<Target>,
    target: &Target,
    suffix: &[Projection],
    parent: usize,
    allocation: (&mut usize, &mut usize, usize),
    budget: &mut Budget,
) -> Result<(), Exhausted> {
    let (bytes, memberships, max_bytes) = allocation;
    let path_len = add(target.place.projection.len(), suffix.len())?;
    if path_len > 128 {
        return Err(Exhausted);
    }
    budget.spend(path_len + target.authority.len() + 1)?;
    let size = add(
        size_of::<Target>(),
        add(
            mul(path_len + 128, size_of::<Projection>())?,
            mul(target.authority.len() + 3, size_of::<usize>())?,
        )?,
    )?;
    *bytes = add(*bytes, mul(4, size)?)?;
    *memberships = add(*memberships, target.authority.len() + 2)?;
    if *bytes > max_bytes || *memberships > MAX_MEMBERSHIPS {
        return Err(Exhausted);
    }
    let mut candidate = target.clone();
    candidate.place.projection.extend_from_slice(suffix);
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
        let parameter_origins: Vec<_> = f
            .params
            .iter()
            .copied()
            .filter(|local| matches!(f.locals[local.0].ty, Ty::Ref { .. }))
            .collect();
        let base = add(f.loan_sites.len(), parameter_origins.len())?;
        let call_count = f
            .blocks
            .iter()
            .filter(|block| {
                matches!(&block.terminator.value,
                    Terminator::Call { signature, .. } if matches!(signature.result, Ty::Ref { .. })
                )
            })
            .count();
        let loans = add(base, call_count)?;
        if loans > MAX_LOANS {
            return Err(Exhausted);
        }
        let mut refs = vec![None; f.locals.len()];
        let mut r = 0;
        for (index, local) in f.locals.iter().enumerate() {
            if matches!(local.ty, Ty::Ref { .. }) {
                refs[index] = Some(r);
                r += 1;
            }
        }
        let b = f.blocks.len();
        let mut next_call = base;
        let call_rows: Vec<_> = f.blocks.iter().map(|block| {
            if matches!(&block.terminator.value, Terminator::Call { signature, .. } if matches!(signature.result, Ty::Ref { .. })) {
                let row = next_call;
                next_call += 1;
                Some(row)
            } else { None }
        }).collect();
        let mut offsets = Vec::with_capacity(b + 1);
        offsets.push(0usize);
        for block in &f.blocks {
            offsets.push(add(
                *offsets.last().unwrap(),
                add(block.statements.len(), 1)?,
            )?);
        }
        let p = *offsets.last().unwrap();
        let origin_size = mul(r, add(size_of::<Set>(), mul(loans.div_ceil(64), 8)?)?)?;
        let live_size = add(size_of::<Set>(), mul(r.div_ceil(64), 8)?)?;
        // Peak includes solver entry states, point snapshots, scratch and sets
        // used by the backward solver. Check before the dense products allocate.
        let mut bytes = add(
            mul(add(add(p, mul(b, 2)?)?, 2)?, origin_size)?,
            mul(add(mul(p, 3)?, add(mul(b, 3)?, 4)?)?, live_size)?,
        )?;
        bytes = add(
            bytes,
            add(
                mul(b, 256)?,
                add(
                    mul(p, 128)?,
                    mul(f.locals.len(), size_of::<Option<usize>>())?,
                )?,
            )?,
        )?;
        bytes = add(bytes, mul(loans, add(256, mul(loans.div_ceil(64), 16)?)?)?)?;
        // Reserve bounded access scratch alongside persistent targets. Two copies
        // cover the caller's resolved targets while active-loan checks run.
        bytes = add(bytes, mul(loans, 64)?)?;
        bytes = add(
            bytes,
            2 * (size_of::<Target>() + 128 * size_of::<Projection>()),
        )?;
        if bytes > max_bytes {
            return Err(Exhausted);
        }
        budget.spend(add(
            mul(add(p, b)?, mul(r, loans.div_ceil(64))?)?,
            mul(add(mul(p, 3)?, mul(b, 2)?)?, r.div_ceil(64))?,
        )?)?;
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
        for (index, local) in parameter_origins.iter().enumerate() {
            entry[refs[local.0].expect("reference parameter carrier")]
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
                call_rows[block],
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
            if let Some(row) = call_rows[block] {
                births[row] = Some((block, f.blocks[block].statements.len()));
            }
            origin_terminator(
                &refs,
                &mut state,
                &f.blocks[block].terminator.value,
                loans,
                call_rows[block],
                budget,
            )?;
            memberships = add(memberships, state.iter().map(Set::count).sum())?;
            if memberships > MAX_MEMBERSHIPS {
                return Err(Exhausted);
            }
            outgoing[block] = Some(Point {
                origins: state,
                live: exits[block].clone(),
            });
        }
        budget.spend(mul(loans, mul(loans.div_ceil(64), 3)?)?)?;
        let mut parents = vec![Set::new(loans); loans];
        for (loan, site) in f.loan_sites.iter().enumerate() {
            if site.place.projection.first() == Some(&Projection::Deref)
                && let (Some((block, statement)), Some(carrier)) =
                    (births[loan], refs[site.place.local.0])
            {
                parents[loan] =
                    points[offsets[block] + statement].as_ref().unwrap().origins[carrier].clone();
                memberships = add(memberships, parents[loan].count())?;
            }
        }
        let mut lifetime_deps = parents.clone();
        for (block, row) in call_rows.iter().enumerate() {
            let Some(row) = *row else { continue };
            let Some(point) = &points[offsets[block + 1] - 1] else {
                continue;
            };
            let Terminator::Call {
                args, signature, ..
            } = &f.blocks[block].terminator.value
            else {
                unreachable!()
            };
            let contract = &signature.ref_contract;
            let regions = usize::from(contract.region_count);
            budget.spend(
                regions * regions
                    + contract.param_regions.len()
                    + contract.outlives.len() * 2
                    + args.len()
                    + 1,
            )?;
            let closure = contract.closure().expect("validated reference contract");
            let result = contract.result_region.expect("reference result region");
            let (ReturnContract::SharedInputPaths(candidates)
            | ReturnContract::MutInputPaths(candidates)) = &contract.returns
            else {
                unreachable!()
            };
            for (index, arg) in args.iter().enumerate() {
                let Some(region) = contract.param_regions[index] else {
                    continue;
                };
                let (Operand::Copy(place) | Operand::Move(place)) = arg else {
                    unreachable!()
                };
                let actual = &point.origins[refs[place.local.0].expect("reference actual")];
                budget.spend(candidates.len())?;
                if candidates.iter().any(|candidate| candidate.param == index) {
                    parents[row].union(actual, budget)?;
                }
                if closure[region.0 as usize] & (1u64 << result.0) != 0 {
                    lifetime_deps[row].union(actual, budget)?;
                }
            }
            memberships = add(
                memberships,
                parents[row].count() + lifetime_deps[row].count(),
            )?;
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
        for (block, row) in call_rows.iter().enumerate() {
            if let Some(row) = row
                && !reachable[block]
            {
                ready[*row] = true;
            }
        }
        for (index, local) in parameter_origins.iter().enumerate() {
            let row = f.loan_sites.len() + index;
            bytes = add(
                bytes,
                4 * (size_of::<Target>() + 128 * size_of::<Projection>() + 2 * size_of::<usize>()),
            )?;
            memberships = add(memberships, 2)?;
            if bytes > max_bytes || memberships > MAX_MEMBERSHIPS {
                return Err(Exhausted);
            }
            targets[row].push(Target {
                place: CanonicalPlace {
                    root: CanonicalRoot::ParameterReferent(*local),
                    projection: vec![],
                },
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
                                site.place.projection.len() - 1,
                            )?;
                            if path_len > 128 {
                                return Err(Exhausted);
                            }
                            let size = add(
                                size_of::<Target>(),
                                add(
                                    mul(path_len, size_of::<Projection>())?,
                                    mul(target.authority.len() + 1, size_of::<usize>())?,
                                )?,
                            )?;
                            // Include Vec growth and peak resolved/issued clones,
                            // allowing the longest legal projection at access time.
                            bytes = add(
                                bytes,
                                mul(
                                    4,
                                    add(
                                        size,
                                        128 * size_of::<Projection>() + 2 * size_of::<usize>(),
                                    )?,
                                )?,
                            )?;
                            memberships = add(memberships, target.authority.len() + 2)?;
                            if bytes > max_bytes || memberships > MAX_MEMBERSHIPS {
                                return Err(Exhausted);
                            }
                            let mut place = target.place.clone();
                            place
                                .projection
                                .extend_from_slice(&site.place.projection[1..]);
                            let mut authority = target.authority.clone();
                            if let Err(position) = authority.binary_search(&parent) {
                                authority.insert(position, parent);
                            }
                            let candidate = Target { place, authority };
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
                    bytes = add(
                        bytes,
                        mul(
                            4,
                            add(
                                size_of::<Target>()
                                    + 128 * size_of::<Projection>()
                                    + 2 * size_of::<usize>(),
                                mul(site.place.projection.len(), size_of::<Projection>())?,
                            )?,
                        )?,
                    )?;
                    memberships = add(memberships, 1)?;
                    if bytes > max_bytes || memberships > MAX_MEMBERSHIPS {
                        return Err(Exhausted);
                    }
                    next.push(Target {
                        place: CanonicalPlace::owned(&site.place),
                        authority: vec![],
                    });
                }
                targets[loan] = next;
                ready[loan] = true;
                changed = true;
            }
            budget.spend(call_rows.len())?;
            for (block, row) in call_rows.iter().enumerate() {
                let Some(row) = *row else { continue };
                if ready[row] || lifetime_deps[row].members().any(|parent| !ready[parent]) {
                    continue;
                }
                budget.spend(lifetime_deps[row].0.len() + parents[row].0.len() + 1)?;
                let Terminator::Call {
                    args, signature, ..
                } = &f.blocks[block].terminator.value
                else {
                    unreachable!()
                };
                let (ReturnContract::SharedInputPaths(candidates)
                | ReturnContract::MutInputPaths(candidates)) = &signature.ref_contract.returns
                else {
                    unreachable!()
                };
                let Some(point) = &points[offsets[block + 1] - 1] else {
                    continue;
                };
                let mut next = vec![];
                for candidate in candidates.iter() {
                    budget.spend(candidate.projection.len() + 1)?;
                    let (Operand::Copy(actual) | Operand::Move(actual)) = &args[candidate.param]
                    else {
                        unreachable!()
                    };
                    let actual = &point.origins[refs[actual.local.0].expect("reference actual")];
                    budget.spend(actual.0.len())?;
                    for parent in actual.members() {
                        for target in &targets[parent] {
                            append_target(
                                &mut next,
                                target,
                                &candidate.projection,
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
                .chain(parameter_origins.iter().map(|local| {
                    let Ty::Ref { kind, .. } = f.locals[local.0].ty else {
                        unreachable!()
                    };
                    kind
                }))
                .chain(f.blocks.iter().filter_map(|block| {
                    let Terminator::Call { signature, .. } = &block.terminator.value else {
                        return None;
                    };
                    let Ty::Ref { kind, .. } = &signature.result else {
                        return None;
                    };
                    Some(*kind)
                }))
                .collect(),
            problems,
            bytes,
        };
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
    fn point(&self, block: BlockId, statement: Option<usize>, phase: Phase) -> (&Origins, &Set) {
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
            let outgoing = self.outgoing[block.0]
                .as_ref()
                .expect("reachable outgoing point");
            (&outgoing.origins, &outgoing.live)
        } else {
            (&data.origins, &data.live)
        }
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
                authority: vec![],
            }]));
        }
        let Some(carrier) = self.refs[place.local.0] else {
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
                next.place
                    .projection
                    .extend_from_slice(&place.projection[1..]);
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
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let (ReturnContract::SharedInputPaths(allowed) | ReturnContract::MutInputPaths(allowed)) =
            &f.ref_contract.returns
        else {
            return Ok(None);
        };
        let regions = usize::from(f.ref_contract.region_count);
        budget.spend(
            regions * regions
                + f.ref_contract.param_regions.len()
                + f.ref_contract.outlives.len() * 2
                + 1,
        )?;
        let closure = f
            .ref_contract
            .closure()
            .expect("validated universal contract");
        let result = f
            .ref_contract
            .result_region
            .expect("reference result region");
        let (origins, _) = self.point(block, None, Phase::Read);
        let carrier = self.refs[f.return_local.0].expect("reference return carrier");
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
                for candidate in allowed.iter() {
                    budget.spend(candidate.projection.len() + 1)?;
                }
                let CanonicalRoot::ParameterReferent(local) = target.place.root else {
                    return Ok(Some(Problem::Invalid(
                        "cannot return a reference to owned frame storage".into(),
                    )));
                };
                if !allowed.iter().any(|candidate| {
                    f.params[candidate.param] == local
                        && candidate.projection.as_ref() == target.place.projection.as_slice()
                }) {
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
                let CanonicalRoot::ParameterReferent(local) = target.place.root else {
                    return Ok(Some(Problem::Invalid(
                        "returned reference lifetime depends on owned frame storage".into(),
                    )));
                };
                let position = f
                    .params
                    .iter()
                    .position(|param| *param == local)
                    .expect("formal canonical root");
                let region =
                    f.ref_contract.param_regions[position].expect("named reference input region");
                if closure[region.0 as usize] & (1u64 << result.0) == 0 {
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
    pub fn check(
        &self,
        target: &Target,
        access: Access,
        block: BlockId,
        statement: Option<usize>,
        phase: Phase,
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let (origins, live) = self.point(block, statement, phase);
        let active = self.active(origins, live, budget)?;
        for loan in active.members() {
            budget.spend(target.authority.len() + 1)?;
            if target.authority.contains(&loan) {
                continue;
            }
            if matches!(access, Access::Read | Access::SharedBorrow)
                && self.kinds[loan] == BorrowKind::Shared
            {
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
        let Some(carrier) = self.refs[local.0] else {
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
