//! Reachable-CFG storage and definite-initialization analysis.
//!
//! Includes bounded B1 local-reference checking, not general Rust borrowing or
//! Drop elaboration. Private `Move` operands consume even Copy-typed places so that
//! move-path analysis can be tested before non-Copy syntax is supported.
use super::borrow::{self, Access, CanonicalPlace, CanonicalRoot, Phase, Problem, Target};
use super::cfg::{self, BlockId, Operand, Rvalue, Statement, Terminator};
use super::ir::{ExprId, LocalId, Place, Projection, Ty};
use std::collections::VecDeque;

#[derive(Clone, Debug, Default)]
pub struct AnalysisStats {
    pub blocks_total: usize,
    pub blocks_reachable: usize,
    pub edges_reachable: usize,
    pub locals: usize,
    pub move_paths: usize,
    pub worklist_pops: usize,
    pub state_changes: usize,
    /// Operand evaluations in the final diagnostic pass, including constants.
    pub operands_read: usize,
    pub moves: usize,
    pub writes: usize,
    pub storage_events: usize,
    pub diagnostics_suppressed: usize,
    /// Downcast authorizations in final diagnostic replay, including singleton proofs.
    pub variant_proof_checks: usize,
    /// Reachable SwitchVariant true edges, independent of worklist iteration order.
    pub variant_refinements: usize,
    /// Known descendant proofs invalidated in final diagnostic replay.
    pub variant_proofs_killed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidCfg(String),
    StorageAlreadyLive,
    StorageNotDefinitelyLive,
    ReadNotDefinitelyInitialized,
    VariantNotDefinitelySelected,
    WriteIntoUninitializedAggregate,
    ImmutableReassignment,
    BorrowConflict(String),
    InvalidReference(String),
    UnsupportedBorrowing(String),
    ResourceLimit,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub kind: ErrorKind,
    pub block: BlockId,
    /// None identifies the block terminator.
    pub statement: Option<usize>,
    pub origin: Option<ExprId>,
    pub place: Option<Place>,
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub stats: AnalysisStats,
    pub diagnostics: Box<[Diagnostic]>,
    /// False means resource exhaustion or truncated diagnostics, never success.
    pub complete: bool,
}
impl Analysis {
    pub fn passed(&self) -> bool {
        self.complete && self.diagnostics.is_empty()
    }
}

#[derive(Clone, Copy)]
struct Limits {
    blocks: usize,
    locals: usize,
    paths: usize,
    operations: usize,
    state_bytes: usize,
    work: usize,
    diagnostics: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            blocks: 16_384,
            locals: 65_536,
            paths: 131_072,
            operations: 1_000_000,
            state_bytes: 128 * 1024 * 1024,
            work: 32 * 1024 * 1024,
            diagnostics: 256,
        }
    }
}
#[derive(Debug)]
pub(super) struct Exhausted;
pub(super) struct Budget(pub(super) usize);
impl Budget {
    pub(super) fn spend(&mut self, amount: usize) -> Result<(), Exhausted> {
        self.0 = self.0.checked_sub(amount).ok_or(Exhausted)?;
        Ok(())
    }
}

/// Bound type traversal before calling the full structural verifier, including
/// repeated shared aggregate types that could otherwise expand exponentially.
fn preflight(f: &cfg::Function, limits: Limits) -> Result<(), Exhausted> {
    fn contract_scratch(
        type_work: usize,
        arity: usize,
        loan_sites: usize,
        max_bytes: usize,
        contract: &super::ir::RefContract,
        budget: &mut Budget,
    ) -> Result<(), Exhausted> {
        let candidates = match &contract.returns {
            super::ir::ReturnContract::NoReference => &[][..],
            super::ir::ReturnContract::SharedInputPaths(paths)
            | super::ir::ReturnContract::MutInputPaths(paths) => paths.as_ref(),
        };
        if candidates.len() > 4096 {
            return Err(Exhausted);
        }
        budget.spend(candidates.len() + 1)?;
        let mut steps = 0usize;
        for candidate in candidates {
            if candidate.projection.len() > 128 {
                return Err(Exhausted);
            }
            steps = steps
                .checked_add(candidate.projection.len())
                .ok_or(Exhausted)?;
            if steps > 131072 {
                return Err(Exhausted);
            }
        }
        budget.spend(steps)?;
        let provided_bytes = candidates
            .len()
            .checked_mul(std::mem::size_of::<super::ir::InputPath>())
            .and_then(|n| n.checked_add(steps.checked_mul(std::mem::size_of::<Projection>())?))
            .ok_or(Exhausted)?;
        let generated_bytes = if matches!(contract.returns, super::ir::ReturnContract::NoReference)
        {
            0
        } else {
            // Expanded preceding type visits bound occurrence count. Reserve
            // candidate Arc/Vec growth and full path copies, independently of
            // the supplied list (which may be forged small or empty).
            type_work
                .min(4096)
                .checked_mul(std::mem::size_of::<super::ir::InputPath>())
                .and_then(|n| {
                    n.checked_add(
                        type_work
                            .checked_mul(128)?
                            .min(131072)
                            .checked_mul(std::mem::size_of::<Projection>())?,
                    )
                })
                .and_then(|n| n.checked_mul(4))
                .ok_or(Exhausted)?
        };
        // A structural type visit can retain a comparison pair, depth, Vec
        // growth and hash-table slot. The expanded type walk also bounds
        // occurrence enumeration, independently of helper DAG equality caches.
        let bytes = type_work
            .checked_mul(128)
            .and_then(|n| n.checked_add(arity.checked_mul(std::mem::size_of::<Ty>())?))
            .and_then(|n| n.checked_add(loan_sites.checked_mul(std::mem::size_of::<usize>())?))
            .and_then(|n| n.checked_add(provided_bytes))
            .and_then(|n| n.checked_add(generated_bytes))
            .and_then(|n| n.checked_add(4096))
            .ok_or(Exhausted)?;
        if bytes > max_bytes {
            return Err(Exhausted);
        }
        Ok(())
    }
    fn charge_contract(
        result: Result<usize, String>,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        match result {
            Ok(work) => {
                budget.spend(work)?;
                Ok(true)
            }
            // Shared helpers use explicit limit diagnostics for every work,
            // depth, candidate and region cap. Never turn exhaustion into a
            // complete semantic rejection by replaying it through validate().
            Err(message) if message.contains("limit") || message.contains("exhaust") => {
                Err(Exhausted)
            }
            Err(_) => Ok(false),
        }
    }
    if f.blocks.len() > limits.blocks
        || f.locals.len() > limits.locals
        || f.params.len() > limits.locals
        || f.loan_sites.len() > super::ir::MAX_LOAN_SITES
    {
        return Err(Exhausted);
    }
    let mut budget = Budget(limits.operations);
    budget.spend(f.params.len())?;
    budget.spend(f.blocks.len())?;
    let call_origins = f
        .blocks
        .iter()
        .filter(|block| {
            matches!(&block.terminator.value,
                Terminator::Call { signature, .. } if matches!(signature.result, Ty::Ref { .. })
            )
        })
        .count();
    let formal_origins = f
        .params
        .iter()
        .filter(|local| {
            f.locals
                .get(local.0)
                .is_some_and(|local| matches!(local.ty, Ty::Ref { .. }))
        })
        .count();
    if f.blocks.len() > limits.blocks
        || f.locals.len() > limits.locals
        || f.loan_sites
            .len()
            .checked_add(formal_origins)
            .and_then(|n| n.checked_add(call_origins))
            .ok_or(Exhausted)?
            > super::ir::MAX_LOAN_SITES
    {
        return Err(Exhausted);
    }
    fn place_type(f: &cfg::Function, place: &Place, budget: &mut Budget) -> Result<(), Exhausted> {
        budget.spend(place.projection.len() + 1)?;
        // Invalid references are diagnosed by the verifier after this bounded
        // preflight, rather than indexing untrusted input here.
        let Some(local) = f.locals.get(place.local.0) else {
            return Ok(());
        };
        let mut ty = &local.ty;
        let mut projection = place.projection.iter();
        while let Some(step) = projection.next() {
            ty = match step {
                Projection::Field(index) => match ty.field_ty(*index) {
                    Some(field) => field,
                    None => return Ok(()),
                },
                Projection::Deref => {
                    let Ty::Ref { referent, .. } = ty else {
                        return Ok(());
                    };
                    referent
                }
                Projection::ConstantIndex(index) => {
                    let Ty::Array { element, len } = ty else {
                        return Ok(());
                    };
                    if *index >= *len {
                        return Ok(());
                    }
                    element
                }
                Projection::Downcast(variant) => {
                    let Ty::Enum(enumeration) = ty else {
                        return Ok(());
                    };
                    if enumeration.def().id != variant.owner {
                        return Ok(());
                    }
                    let Some(def) = enumeration.def().variants.get(variant.index) else {
                        return Ok(());
                    };
                    let Some(Projection::Field(index)) = projection.next() else {
                        return Ok(());
                    };
                    let Some(field) = def.fields.get(*index) else {
                        return Ok(());
                    };
                    &field.ty
                }
            };
        }
        preflight_ty(ty, 0, budget)
    }
    let before_types = budget.0;
    for local in &f.locals {
        preflight_ty(&local.ty, 0, &mut budget)?;
    }
    // The contract helper sees formal occurrences, not distinct local slots.
    // Malformed duplicate formal indices are diagnosed later, but must not
    // undercount generated candidates or scratch before that validation.
    for param in &f.params {
        if let Some(local) = f.locals.get(param.0) {
            preflight_ty(&local.ty, 0, &mut budget)?;
        }
    }
    preflight_ty(&f.result, 0, &mut budget)?;
    contract_scratch(
        before_types - budget.0,
        f.params.len(),
        f.loan_sites.len(),
        limits.state_bytes,
        &f.ref_contract,
        &mut budget,
    )?;
    let mut param_types = Vec::with_capacity(f.params.len());
    for param in &f.params {
        let Some(local) = f.locals.get(param.0) else {
            return Ok(());
        };
        param_types.push(local.ty.clone());
    }
    if !charge_contract(
        f.ref_contract.validation_work(&param_types, &f.result),
        &mut budget,
    )? {
        return Ok(()); // Full verifier owns malformed-contract diagnostics.
    }
    drop(param_types);
    for site in &f.loan_sites {
        place_type(f, &site.place, &mut budget)?;
    }
    let operand = |op: &Operand, budget: &mut Budget| -> Result<(), Exhausted> {
        budget.spend(1)?;
        if let Operand::Copy(p) | Operand::Move(p) = op {
            place_type(f, p, budget)?;
        }
        if let Operand::Constant(cfg::Constant::Enum { ty, .. }) = op {
            preflight_ty(&Ty::Enum(ty.clone()), 0, budget)?;
        }
        Ok(())
    };
    for block in &f.blocks {
        budget.spend(1)?;
        for item in &block.statements {
            budget.spend(1)?;
            if let Statement::Inspect(place) = &item.value {
                place_type(f, place, &mut budget)?;
            }
            match &item.value {
                Statement::IndexInspect { array, index } => {
                    place_type(f, array, &mut budget)?;
                    operand(index, &mut budget)?;
                }
                Statement::IndexAssign {
                    array,
                    index,
                    value,
                } => {
                    place_type(f, array, &mut budget)?;
                    operand(value, &mut budget)?;
                    operand(index, &mut budget)?;
                }
                _ => {}
            }
            if let Statement::Assign { destination, value } = &item.value {
                place_type(f, destination, &mut budget)?;
                match value {
                    Rvalue::Borrow(_) => budget.spend(1)?,
                    Rvalue::Discriminant(place) => place_type(f, place, &mut budget)?,
                    Rvalue::IndexCopy { array, index } => {
                        place_type(f, array, &mut budget)?;
                        operand(index, &mut budget)?;
                    }
                    Rvalue::Use(op) | Rvalue::Unary { value: op, .. } => operand(op, &mut budget)?,
                    Rvalue::Repeat { ty, value } => {
                        preflight_ty(ty, 0, &mut budget)?;
                        operand(value, &mut budget)?;
                    }
                    Rvalue::Binary { left, right, .. } => {
                        operand(left, &mut budget)?;
                        operand(right, &mut budget)?;
                    }
                    Rvalue::Aggregate {
                        ty: value_ty,
                        fields,
                    } => {
                        preflight_ty(value_ty, 0, &mut budget)?;
                        for field in fields {
                            operand(field, &mut budget)?;
                        }
                    }
                    Rvalue::EnumVariant { ty, fields, .. } => {
                        preflight_ty(&Ty::Enum(ty.clone()), 0, &mut budget)?;
                        for field in fields {
                            operand(field, &mut budget)?;
                        }
                    }
                    Rvalue::Println(_) => {}
                }
            }
        }
        match &block.terminator.value {
            Terminator::Switch { condition, .. } => operand(condition, &mut budget)?,
            Terminator::SwitchVariant { place, .. } => place_type(f, place, &mut budget)?,
            Terminator::Call {
                signature,
                args,
                destination,
                ..
            } => {
                let before_types = budget.0;
                for param in &signature.params {
                    preflight_ty(param, 0, &mut budget)?;
                }
                preflight_ty(&signature.result, 0, &mut budget)?;
                contract_scratch(
                    before_types - budget.0,
                    signature.params.len(),
                    f.loan_sites.len(),
                    limits.state_bytes,
                    &signature.ref_contract,
                    &mut budget,
                )?;
                if !charge_contract(
                    signature
                        .ref_contract
                        .validation_work(&signature.params, &signature.result),
                    &mut budget,
                )? {
                    return Ok(());
                }
                for arg in args {
                    operand(arg, &mut budget)?;
                }
                place_type(f, destination, &mut budget)?;
            }
            _ => {}
        }
    }
    Ok(())
}
fn preflight_ty(ty: &Ty, depth: usize, budget: &mut Budget) -> Result<(), Exhausted> {
    // Recursive coercion verification can compare each enclosing subtree
    // before descending. Weight by depth to conservatively cover that repeated
    // comparison work, not only the type's leaf count.
    budget.spend(depth + 1)?;
    if depth > 128 {
        return Err(Exhausted);
    }
    if let Ty::Ref { referent, .. } = ty {
        preflight_ty(referent, depth + 1, budget)?;
    }
    if let Ty::Array { element, len } = ty {
        if *len > super::ir::MAX_ARRAY_LEN {
            return Err(Exhausted);
        }
        // Zero arrays still validate their element type and Copy semantics.
        // Nonempty arrays expand before allocation, bounding repeated shared
        // types as well as their distinct element move paths.
        for _ in 0..(*len).max(1) {
            preflight_ty(element, depth + 1, budget)?;
        }
    }
    if let Ty::Enum(ty) = ty {
        budget.spend(ty.def().variants.len())?;
        for variant in ty.def().variants.iter() {
            for field in variant.fields.iter() {
                preflight_ty(&field.ty, depth + 1, budget)?;
            }
        }
    }
    if let Some(count) = ty.field_count() {
        for index in 0..count {
            preflight_ty(ty.field_ty(index).ok_or(Exhausted)?, depth + 1, budget)?;
        }
    }
    Ok(())
}

struct PathNode {
    parent: Option<usize>,
    children: Vec<usize>,
    /// Enum variants select disjoint groups in `children`. Empty for non-enums.
    variant_offsets: Vec<usize>,
    /// Preorder interval [self, end) covers this place's entire subtree.
    end: usize,
}
struct Paths {
    roots: Vec<usize>,
    external: Vec<Option<usize>>,
    nodes: Vec<PathNode>,
}
impl Paths {
    fn new(f: &cfg::Function, limit: usize) -> Result<Self, Exhausted> {
        fn add(
            paths: &mut Paths,
            ty: &Ty,
            parent: Option<usize>,
            limit: usize,
        ) -> Result<usize, Exhausted> {
            if paths.nodes.len() >= limit {
                return Err(Exhausted);
            }
            let id = paths.nodes.len();
            paths.nodes.push(PathNode {
                parent,
                children: Vec::new(),
                variant_offsets: Vec::new(),
                end: 0,
            });
            if let Some(count) = ty.field_count() {
                for index in 0..count {
                    let child = add(paths, ty.field_ty(index).ok_or(Exhausted)?, Some(id), limit)?;
                    paths.nodes[id].children.push(child);
                }
            }
            if let Ty::Array { element, len } = ty {
                if *len > super::ir::MAX_ARRAY_LEN || *len > limit - paths.nodes.len() {
                    return Err(Exhausted);
                }
                for _ in 0..*len {
                    let child = add(paths, element, Some(id), limit)?;
                    paths.nodes[id].children.push(child);
                }
            }
            if let Ty::Enum(enumeration) = ty {
                for variant in enumeration.def().variants.iter() {
                    let offset = paths.nodes[id].children.len();
                    paths.nodes[id].variant_offsets.push(offset);
                    for field in variant.fields.iter() {
                        let child = add(paths, &field.ty, Some(id), limit)?;
                        paths.nodes[id].children.push(child);
                    }
                }
            }
            paths.nodes[id].end = paths.nodes.len();
            Ok(id)
        }
        let mut result = Self {
            roots: Vec::new(),
            external: vec![None; f.locals.len()],
            nodes: Vec::new(),
        };
        for local in &f.locals {
            let root = add(&mut result, &local.ty, None, limit)?;
            result.roots.push(root);
        }
        for local in &f.params {
            if let Ty::Ref { referent, .. } = &f.locals[local.0].ty {
                let root = add(&mut result, referent, None, limit)?;
                result.external[local.0] = Some(root);
            }
        }
        Ok(result)
    }
    fn resolve(&self, place: &Place) -> usize {
        self.resolve_projection(self.roots[place.local.0], &place.projection)
    }
    fn canonical_root(&self, root: &CanonicalRoot) -> usize {
        match root {
            CanonicalRoot::Owned(local) => self.roots[local.0],
            CanonicalRoot::ParameterReferent(local) => {
                self.external[local.0].expect("formal referent path")
            }
        }
    }
    fn resolve_canonical(&self, place: &CanonicalPlace) -> usize {
        self.resolve_projection(self.canonical_root(&place.root), &place.projection)
    }
    fn resolve_projection(&self, mut node: usize, path: &[Projection]) -> usize {
        let mut projection = path.iter();
        while let Some(step) = projection.next() {
            let index = match step {
                Projection::Deref => {
                    unreachable!("deref must be resolved through canonical loan provenance")
                }
                Projection::Field(index) | Projection::ConstantIndex(index) => *index,
                Projection::Downcast(variant) => {
                    let Some(Projection::Field(index)) = projection.next() else {
                        unreachable!("validated downcast has a field")
                    };
                    self.nodes[node].variant_offsets[variant.index] + index
                }
            };
            node = self.nodes[node].children[index];
        }
        node
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Bits(Vec<u64>);
impl Bits {
    fn new(count: usize) -> Self {
        Self(vec![0; count.div_ceil(64)])
    }
    fn get(&self, bit: usize) -> bool {
        self.0[bit / 64] & (1 << (bit % 64)) != 0
    }
    fn set(&mut self, bit: usize, value: bool) {
        let mask = 1 << (bit % 64);
        if value {
            self.0[bit / 64] |= mask;
        } else {
            self.0[bit / 64] &= !mask;
        }
    }
    fn range(
        &mut self,
        start: usize,
        end: usize,
        value: bool,
        budget: &mut Budget,
    ) -> Result<(), Exhausted> {
        if start == end {
            return Ok(());
        }
        for (index, word) in self
            .0
            .iter_mut()
            .enumerate()
            .take(end.div_ceil(64))
            .skip(start / 64)
        {
            budget.spend(1)?;
            let low = start.saturating_sub(index * 64);
            let high = (end - index * 64).min(64);
            let mask = (u64::MAX >> (64 - (high - low))) << low;
            if value {
                *word |= mask;
            } else {
                *word &= !mask;
            }
        }
        Ok(())
    }
    fn all(&self, start: usize, end: usize, budget: &mut Budget) -> Result<bool, Exhausted> {
        if start == end {
            return Ok(true);
        }
        for (index, word) in self
            .0
            .iter()
            .enumerate()
            .take(end.div_ceil(64))
            .skip(start / 64)
        {
            budget.spend(1)?;
            let low = start.saturating_sub(index * 64);
            let high = (end - index * 64).min(64);
            let mask = (u64::MAX >> (64 - (high - low))) << low;
            if *word & mask != mask {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn merge(
        &mut self,
        other: &Self,
        intersection: bool,
        budget: &mut Budget,
    ) -> Result<bool, Exhausted> {
        budget.spend(self.0.len())?;
        let mut changed = false;
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            let merged = if intersection { *a & *b } else { *a | *b };
            changed |= *a != merged;
            *a = merged;
        }
        Ok(changed)
    }
}
#[derive(Clone)]
struct State {
    must_live: Bits,
    may_live: Bits,
    must_init: Bits,
    may_init: Bits,
    ever_written: Bits,
    /// Variant index or usize::MAX (unknown), indexed by actual enum path.
    /// Owner identity is fixed by the structurally verified path type.
    selected: Vec<usize>,
}
impl State {
    fn new(locals: usize, paths: usize) -> Self {
        Self {
            must_live: Bits::new(locals),
            may_live: Bits::new(locals),
            must_init: Bits::new(paths),
            may_init: Bits::new(paths),
            ever_written: Bits::new(paths),
            selected: vec![usize::MAX; paths],
        }
    }
    fn words(&self) -> usize {
        self.must_live.0.len() * 2 + self.must_init.0.len() * 3 + self.selected.len()
    }
    fn merge(&mut self, other: &Self, budget: &mut Budget) -> Result<bool, Exhausted> {
        let mut changed = self.must_live.merge(&other.must_live, true, budget)?;
        changed |= self.may_live.merge(&other.may_live, false, budget)?;
        changed |= self.must_init.merge(&other.must_init, true, budget)?;
        changed |= self.may_init.merge(&other.may_init, false, budget)?;
        changed |= self
            .ever_written
            .merge(&other.ever_written, false, budget)?;
        budget.spend(self.selected.len())?;
        for (ours, theirs) in self.selected.iter_mut().zip(&other.selected) {
            if *ours != *theirs && *ours != usize::MAX {
                *ours = usize::MAX;
                changed = true;
            }
        }
        Ok(changed)
    }
}

struct Transfer<'a> {
    f: &'a cfg::Function,
    paths: &'a Paths,
    budget: &'a mut Budget,
    report: Option<&'a mut Report>,
    block: BlockId,
    statement: Option<usize>,
    origin: Option<ExprId>,
    diagnostic_limit: usize,
    loans: Option<&'a borrow::Plan>,
    phase: Phase,
}
struct Report {
    stats: AnalysisStats,
    diagnostics: Vec<Diagnostic>,
    complete: bool,
}
impl Transfer<'_> {
    fn problem(&mut self, problem: Problem, place: Place) {
        let kind = match problem {
            Problem::Conflict(message) => ErrorKind::BorrowConflict(message),
            Problem::Invalid(message) => ErrorKind::InvalidReference(message),
            Problem::Unsupported(message) => ErrorKind::UnsupportedBorrowing(message),
        };
        self.diagnostic(kind, place);
    }
    fn targets(&mut self, place: &Place) -> Result<Vec<Target>, Exhausted> {
        if let Some(loans) = self.loans {
            match loans.resolve(place, self.block, self.statement, self.phase, self.budget)? {
                Ok(targets) => Ok(targets),
                Err(problem) => {
                    self.problem(problem, place.clone());
                    Ok(vec![])
                }
            }
        } else {
            Ok(vec![Target {
                place: CanonicalPlace::owned(place),
                authority: vec![],
            }])
        }
    }
    fn check_access(&mut self, target: &Target, access: Access) -> Result<(), Exhausted> {
        if let Some(loans) = self.loans
            && let Some(problem) = loans.check(
                target,
                access,
                self.block,
                self.statement,
                self.phase,
                self.budget,
            )?
        {
            self.problem(problem, target.place.diagnostic());
        }
        Ok(())
    }
    fn carrier_read(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        if place.projection.first() == Some(&Projection::Deref) {
            self.read_raw(state, &root_place(place.local))?;
        }
        Ok(())
    }
    fn mut_authority(&mut self, place: &Place) {
        if !matches!(
            self.f.locals[place.local.0].ty,
            Ty::Ref {
                kind: super::ir::BorrowKind::Mut,
                ..
            }
        ) {
            self.problem(
                Problem::Invalid("mutation requires mutable reference authority".into()),
                place.clone(),
            );
        }
    }
    fn read_mode(
        &mut self,
        state: &mut State,
        place: &Place,
        access: Access,
    ) -> Result<(), Exhausted> {
        if self.loans.is_none() {
            return self.read_raw(state, place);
        }
        self.carrier_read(state, place)?;
        for target in self.targets(place)? {
            self.check_access(&target, access)?;
            self.read_canonical(state, &target.place)?;
        }
        Ok(())
    }
    fn read(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        self.read_mode(state, place, Access::Read)
    }
    fn write(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        if self.loans.is_none() {
            return self.write_raw(state, place);
        }
        let deref = place.projection.first() == Some(&Projection::Deref);
        self.carrier_read(state, place)?;
        if deref {
            self.mut_authority(place);
        }
        if deref {
            self.budget.spend(1)?;
            if let Some(report) = &mut self.report {
                report.stats.writes += 1;
            }
        }
        for target in self.targets(place)? {
            self.check_access(&target, Access::Write)?;
            if deref {
                // Every alternative must already be initialized. A may-alias
                // write must never initialize every possible referent.
                self.read_canonical(state, &target.place)?;
                let node = self.paths.resolve_canonical(&target.place);
                self.kill_proofs(state, node)?;
                self.mark_written(state, node)?;
            } else {
                self.write_raw(state, place)?;
            }
        }
        Ok(())
    }
    fn authorize(&mut self, state: &State, place: &Place) -> Result<bool, Exhausted> {
        self.authorize_canonical(state, &CanonicalPlace::owned(place))
    }
    fn authorize_canonical(
        &mut self,
        state: &State,
        place: &CanonicalPlace,
    ) -> Result<bool, Exhausted> {
        // Cover this traversal and the subsequent resolve traversals for reads,
        // moves and writes. Structural typing was checked for every CFG block.
        self.budget
            .spend(place.projection.len().checked_mul(3).ok_or(Exhausted)? + 1)?;
        let mut node = self.paths.canonical_root(&place.root);
        let mut projection = place.projection.iter();
        while let Some(step) = projection.next() {
            let index = match step {
                Projection::Deref => {
                    unreachable!("deref must be canonicalized before authorization")
                }
                Projection::Field(index) | Projection::ConstantIndex(index) => *index,
                Projection::Downcast(variant) => {
                    if let Some(report) = &mut self.report {
                        report.stats.variant_proof_checks += 1;
                    }
                    if self.paths.nodes[node].variant_offsets.len() != 1
                        && state.selected[node] != variant.index
                    {
                        self.diagnostic(
                            ErrorKind::VariantNotDefinitelySelected,
                            place.diagnostic(),
                        );
                        return Ok(false);
                    }
                    let Some(Projection::Field(index)) = projection.next() else {
                        unreachable!("validated downcast has a field")
                    };
                    self.paths.nodes[node].variant_offsets[variant.index] + index
                }
            };
            node = self.paths.nodes[node].children[index];
        }
        Ok(true)
    }
    fn diagnostic(&mut self, kind: ErrorKind, place: Place) {
        if let Some(report) = &mut self.report {
            if report.diagnostics.len() < self.diagnostic_limit {
                report.diagnostics.push(Diagnostic {
                    kind,
                    block: self.block,
                    statement: self.statement,
                    origin: self.origin,
                    place: Some(place),
                });
            } else {
                report.stats.diagnostics_suppressed += 1;
                report.complete = false;
            }
        }
    }
    fn ancestors(&mut self, state: &State, node: usize) -> Result<bool, Exhausted> {
        let mut parent = self.paths.nodes[node].parent;
        while let Some(node) = parent {
            self.budget.spend(1)?;
            if !state.must_init.get(node) {
                return Ok(false);
            }
            parent = self.paths.nodes[node].parent;
        }
        Ok(true)
    }
    fn kill_proofs(&mut self, state: &mut State, node: usize) -> Result<(), Exhausted> {
        let end = self.paths.nodes[node].end;
        self.budget.spend(end - node)?;
        for selected in &mut state.selected[node..end] {
            if *selected != usize::MAX {
                if let Some(report) = &mut self.report {
                    report.stats.variant_proofs_killed += 1;
                }
                *selected = usize::MAX;
            }
        }
        Ok(())
    }
    fn initialize(&mut self, state: &mut State, node: usize, value: bool) -> Result<(), Exhausted> {
        self.kill_proofs(state, node)?;
        let end = self.paths.nodes[node].end;
        state.must_init.range(node, end, value, self.budget)?;
        state.may_init.range(node, end, value, self.budget)
    }
    fn read_raw(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        self.read_canonical(state, &CanonicalPlace::owned(place))
    }
    fn read_canonical(
        &mut self,
        state: &mut State,
        place: &CanonicalPlace,
    ) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if !self.authorize_canonical(state, place)? {
            return Ok(());
        }
        if matches!(place.root, CanonicalRoot::Owned(local) if !state.must_live.get(local.0)) {
            self.diagnostic(ErrorKind::StorageNotDefinitelyLive, place.diagnostic());
        } else {
            let node = self.paths.resolve_canonical(place);
            if !self.ancestors(state, node)?
                || !state
                    .must_init
                    .all(node, self.paths.nodes[node].end, self.budget)?
            {
                self.diagnostic(ErrorKind::ReadNotDefinitelyInitialized, place.diagnostic());
            }
        }
        Ok(())
    }
    fn operand(&mut self, state: &mut State, op: &Operand) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.operands_read += 1;
        }
        if let Operand::Move(place) = op {
            self.budget.spend(place.projection.len() + 1)?;
            if place
                .projection
                .iter()
                .any(|p| matches!(p, Projection::ConstantIndex(_)))
            {
                // The CFG verifier also rejects this, including in dead blocks.
                // Keep transfer fail-closed independently, and meter repeated
                // element Copy checks rather than recursively walking for free.
                let ty = self.f.place_ty(place).expect("structurally verified place");
                preflight_ty(ty, 0, self.budget)?;
                if !ty.is_copy() {
                    self.diagnostic(
                        ErrorKind::InvalidCfg(
                            "non-Copy move through array indexing is unsupported".into(),
                        ),
                        place.clone(),
                    );
                    return Ok(());
                }
            }
        }
        if let Operand::Copy(place) | Operand::Move(place) = op {
            if matches!(op, Operand::Move(_)) {
                if matches!(
                    self.f.locals[place.local.0].ty,
                    Ty::Ref {
                        kind: super::ir::BorrowKind::Mut,
                        ..
                    }
                ) && place.projection.is_empty()
                    && let Some(loans) = self.loans
                    && let Some(problem) = loans.check_handle_move(
                        place.local,
                        self.block,
                        self.statement,
                        self.budget,
                    )?
                {
                    self.problem(problem, place.clone());
                }
                if place.projection.contains(&Projection::Deref) {
                    self.mut_authority(place);
                }
                self.read_mode(state, place, Access::Move)?;
            } else {
                self.read(state, place)?;
            }
            if matches!(op, Operand::Move(_)) {
                if let Some(report) = &mut self.report {
                    report.stats.moves += 1;
                }
                for target in self.targets(place)? {
                    self.initialize(state, self.paths.resolve_canonical(&target.place), false)?;
                }
            }
        }
        Ok(())
    }
    fn write_raw(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.writes += 1;
        }
        self.authorize(state, place)?;
        let node = self.paths.resolve(place);
        let root = self.paths.roots[place.local.0];
        if !state.must_live.get(place.local.0) {
            self.diagnostic(ErrorKind::StorageNotDefinitelyLive, place.clone());
        } else if !self.ancestors(state, node)? {
            self.diagnostic(ErrorKind::WriteIntoUninitializedAggregate, place.clone());
        } else if !self.f.locals[place.local.0].mutable && state.ever_written.get(root) {
            self.diagnostic(ErrorKind::ImmutableReassignment, place.clone());
        }
        // Continue a deterministic transfer even after a diagnostic. No failed
        // analysis is a permission to emit, and this limits cascading errors.
        self.initialize(state, node, true)?;
        self.mark_written(state, node)
    }
    fn mark_written(&mut self, state: &mut State, node: usize) -> Result<(), Exhausted> {
        state
            .ever_written
            .range(node, self.paths.nodes[node].end, true, self.budget)?;
        let mut parent = self.paths.nodes[node].parent;
        while let Some(node) = parent {
            self.budget.spend(1)?;
            state.ever_written.set(node, true);
            parent = self.paths.nodes[node].parent;
        }
        Ok(())
    }
    fn index_write(&mut self, state: &mut State, array: &Place) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.writes += 1;
        }
        // An unknown slot can only replace an element of an already ready
        // array. Never initialize a guessed child or resurrect missing bits.
        self.carrier_read(state, array)?;
        let deref = array.projection.first() == Some(&Projection::Deref);
        if deref {
            self.mut_authority(array);
        }
        for target in self.targets(array)? {
            self.check_access(&target, Access::Write)?;
            self.read_canonical(state, &target.place)?;
            let node = self.paths.resolve_canonical(&target.place);
            if !deref
                && !self.f.locals[array.local.0].mutable
                && state.ever_written.get(self.paths.roots[array.local.0])
            {
                self.diagnostic(ErrorKind::ImmutableReassignment, target.place.diagnostic());
            }
            self.kill_proofs(state, node)?;
            self.mark_written(state, node)?;
        }
        Ok(())
    }
    fn rvalue(&mut self, state: &mut State, value: &Rvalue) -> Result<(), Exhausted> {
        match value {
            Rvalue::Borrow(id) => {
                let site = &self.f.loan_sites[id.0];
                let source = site.place.clone();
                let kind = site.kind;
                self.carrier_read(state, &source)?;
                if kind == super::ir::BorrowKind::Mut {
                    if source.projection.first() == Some(&Projection::Deref) {
                        self.mut_authority(&source);
                    } else if !self.f.locals[source.local.0].mutable {
                        self.diagnostic(ErrorKind::ImmutableReassignment, source.clone());
                    }
                }
                let targets = if let Some(loans) = self.loans {
                    loans.issued(*id, self.budget)?
                } else {
                    vec![]
                };
                if targets.is_empty() {
                    self.problem(
                        Problem::Invalid("borrow issuance has no canonical provenance".into()),
                        source,
                    );
                }
                for target in targets {
                    self.check_access(
                        &target,
                        if kind == super::ir::BorrowKind::Mut {
                            Access::MutBorrow
                        } else {
                            Access::SharedBorrow
                        },
                    )?;
                    self.read_canonical(state, &target.place)?;
                }
            }
            // Reading a tag does not copy or move the non-Copy enum, but does
            // require the entire enum, including active payloads, initialized.
            Rvalue::Discriminant(place) => self.read(state, place)?,
            Rvalue::IndexCopy { array, index } => {
                self.operand(state, index)?;
                self.read(state, array)?;
            }
            Rvalue::Use(op) | Rvalue::Unary { value: op, .. } => self.operand(state, op)?,
            // Repetition evaluates once even for a zero-length destination.
            Rvalue::Repeat { value, .. } => self.operand(state, value)?,
            Rvalue::Binary { left, right, .. } => {
                self.operand(state, left)?;
                self.operand(state, right)?;
            }
            Rvalue::Aggregate { fields, .. } | Rvalue::EnumVariant { fields, .. } => {
                for field in fields {
                    self.operand(state, field)?;
                }
            }
            Rvalue::Println(_) => {}
        }
        Ok(())
    }
    fn block(&mut self, state: &mut State) -> Result<(), Exhausted> {
        for (index, item) in self.f.blocks[self.block.0].statements.iter().enumerate() {
            self.budget.spend(1)?;
            self.statement = Some(index);
            self.origin = item.origin;
            self.phase = Phase::Read;
            match &item.value {
                Statement::Inspect(place) => self.read(state, place)?,
                Statement::IndexInspect { array, index } => {
                    self.operand(state, index)?;
                    self.read(state, array)?;
                }
                Statement::IndexAssign {
                    array,
                    index,
                    value,
                } => {
                    // Lowering has already snapshotted RHS and index effects.
                    // Preserve that owning-value-before-index transfer order.
                    self.operand(state, value)?;
                    self.operand(state, index)?;
                    self.phase = Phase::Write;
                    self.index_write(state, array)?;
                }
                Statement::StorageLive(local) => {
                    self.check_access(
                        &Target {
                            place: CanonicalPlace::owned(&root_place(*local)),
                            authority: vec![],
                        },
                        Access::StorageEnd,
                    )?;
                    if let Some(report) = &mut self.report {
                        report.stats.storage_events += 1;
                    }
                    if state.may_live.get(local.0) {
                        self.diagnostic(ErrorKind::StorageAlreadyLive, root_place(*local));
                    }
                    state.must_live.set(local.0, true);
                    state.may_live.set(local.0, true);
                    let root = self.paths.roots[local.0];
                    self.initialize(state, root, false)?;
                    state.ever_written.range(
                        root,
                        self.paths.nodes[root].end,
                        false,
                        self.budget,
                    )?;
                }
                Statement::StorageDead(local) => {
                    self.check_access(
                        &Target {
                            place: CanonicalPlace::owned(&root_place(*local)),
                            authority: vec![],
                        },
                        Access::StorageEnd,
                    )?;
                    if let Some(report) = &mut self.report {
                        report.stats.storage_events += 1;
                    }
                    if !state.must_live.get(local.0) {
                        self.diagnostic(ErrorKind::StorageNotDefinitelyLive, root_place(*local));
                    }
                    state.must_live.set(local.0, false);
                    state.may_live.set(local.0, false);
                    self.initialize(state, self.paths.roots[local.0], false)?;
                }
                Statement::Assign { destination, value } => {
                    self.rvalue(state, value)?;
                    self.phase = Phase::Write;
                    self.write(state, destination)?;
                }
            }
        }
        self.statement = None;
        self.phase = Phase::Read;
        self.origin = self.f.blocks[self.block.0].terminator.origin;
        match &self.f.blocks[self.block.0].terminator.value {
            Terminator::Switch { condition, .. } => self.operand(state, condition)?,
            Terminator::SwitchVariant { place, .. } => {
                self.budget.spend(place.projection.len() + 1)?;
                let node = self.paths.resolve(place);
                if self.paths.nodes[node].variant_offsets.len() == 1 {
                    // A singleton pattern does not inspect storage or a tag.
                    // Payload bindings still perform their normal full reads.
                    self.authorize(state, place)?;
                } else {
                    self.read(state, place)?;
                }
            }
            Terminator::Call {
                args,
                destination,
                signature,
                ..
            } => {
                // Snapshot effects before consuming reference handles. A bounded
                // node bitmap deduplicates alias targets instead of retaining
                // an argument-count product of cloned canonical target vectors.
                let mut mutations = signature
                    .params
                    .iter()
                    .any(|ty| matches!(ty, Ty::Ref { .. }))
                    .then(|| Bits::new(self.paths.nodes.len()));
                if let Some(mutations) = &mutations {
                    self.budget.spend(mutations.0.len())?;
                }
                for (arg, ty) in args.iter().zip(&signature.params) {
                    if let Ty::Ref { kind, .. } = ty {
                        let (Operand::Copy(carrier) | Operand::Move(carrier)) = arg else {
                            unreachable!("validated reference argument")
                        };
                        let deref = Place {
                            local: carrier.local,
                            projection: vec![Projection::Deref],
                        };
                        self.carrier_read(state, &deref)?;
                        for target in self.targets(&deref)? {
                            self.check_access(
                                &target,
                                if *kind == super::ir::BorrowKind::Mut {
                                    Access::MutBorrow
                                } else {
                                    Access::SharedBorrow
                                },
                            )?;
                            self.read_canonical(state, &target.place)?;
                            if *kind == super::ir::BorrowKind::Mut {
                                mutations
                                    .as_mut()
                                    .expect("reference call effects")
                                    .set(self.paths.resolve_canonical(&target.place), true);
                            }
                        }
                    }
                }
                for arg in args {
                    self.operand(state, arg)?;
                }
                if let Some(mutations) = mutations {
                    self.budget.spend(self.paths.nodes.len())?;
                    for node in 0..self.paths.nodes.len() {
                        if mutations.get(node) {
                            self.kill_proofs(state, node)?;
                        }
                    }
                }
                self.phase = Phase::Write;
                self.write(state, destination)?;
            }
            Terminator::Return => {
                let mutable_reference = matches!(
                    self.f.result,
                    Ty::Ref {
                        kind: super::ir::BorrowKind::Mut,
                        ..
                    }
                );
                if !mutable_reference {
                    self.operand(state, &Operand::Copy(root_place(self.f.return_local)))?;
                }
                if let Some(loans) = self.loans
                    && let Some(problem) = loans.check_return(self.f, self.block, self.budget)?
                {
                    self.problem(problem, root_place(self.f.return_local));
                }
                if mutable_reference {
                    self.operand(state, &Operand::Move(root_place(self.f.return_local)))?;
                }
            }
            Terminator::Goto(_) | Terminator::Unreachable => {}
        }
        Ok(())
    }
}
fn root_place(local: LocalId) -> Place {
    Place {
        local,
        projection: Vec::new(),
    }
}
fn successors(terminator: &Terminator) -> [Option<BlockId>; 2] {
    match terminator {
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

/// Validate every CFG block, then solve reachable states before diagnosing.
/// Structural errors in dead blocks are errors, but dead reads/moves are not.
/// Results are private transfer semantics, not a claim of Rust borrow checking.
pub fn analyze(function: &cfg::Function) -> Analysis {
    analyze_with_limits(function, Limits::default())
}

fn analyze_with_limits(f: &cfg::Function, limits: Limits) -> Analysis {
    let mut report = Report {
        stats: AnalysisStats {
            blocks_total: f.blocks.len(),
            locals: f.locals.len(),
            ..AnalysisStats::default()
        },
        diagnostics: Vec::new(),
        complete: true,
    };
    let result = (|| -> Result<(), Exhausted> {
        preflight(f, limits)?;
        if let Err(error) = f.validate() {
            report.diagnostics.push(Diagnostic {
                kind: ErrorKind::InvalidCfg(error),
                block: f.entry,
                statement: None,
                origin: None,
                place: None,
            });
            return Ok(());
        }
        let paths = Paths::new(f, limits.paths)?;
        report.stats.move_paths = paths.nodes.len();
        let mut budget = Budget(limits.work);
        let mut seed = State::new(f.locals.len(), paths.nodes.len());
        // Account for dense retained entry states and transient states as well
        // as Vec headers. No allocation proportional to blocks*paths precedes
        // this checked bound.
        let per_state = seed
            .words()
            .checked_mul(8)
            .and_then(|n| n.checked_add(std::mem::size_of::<State>()))
            .ok_or(Exhausted)?;
        let bytes = per_state
            .checked_mul(f.blocks.len().checked_add(3).ok_or(Exhausted)?)
            // One call's deduplicated mutable-effect bitmap can coexist with
            // the solver and canonical target scratch, never with another call.
            .and_then(|n| n.checked_add(paths.nodes.len().div_ceil(64).checked_mul(8)?))
            .and_then(|n| n.checked_add(std::mem::size_of::<Bits>()))
            .ok_or(Exhausted)?;
        if bytes > limits.state_bytes {
            return Err(Exhausted);
        }
        let loans = if !f.loan_sites.is_empty()
            || f.locals
                .iter()
                .any(|local| matches!(local.ty, Ty::Ref { .. }))
        {
            Some(borrow::Plan::build(
                f,
                &mut budget,
                limits.state_bytes - bytes,
            )?)
        } else {
            None
        };
        if let Some(loans) = &loans {
            if bytes.checked_add(loans.bytes).ok_or(Exhausted)? > limits.state_bytes {
                return Err(Exhausted);
            }
            for (block, statement, problem) in &loans.problems {
                if report.diagnostics.len() >= limits.diagnostics {
                    return Err(Exhausted);
                }
                let kind = match problem {
                    Problem::Conflict(message) => ErrorKind::BorrowConflict(message.clone()),
                    Problem::Invalid(message) => ErrorKind::InvalidReference(message.clone()),
                    Problem::Unsupported(message) => {
                        ErrorKind::UnsupportedBorrowing(message.clone())
                    }
                };
                report.diagnostics.push(Diagnostic {
                    kind,
                    block: *block,
                    statement: *statement,
                    origin: None,
                    place: None,
                });
            }
        }
        for local in &f.params {
            seed.must_live.set(local.0, true);
            seed.may_live.set(local.0, true);
            let root = paths.roots[local.0];
            seed.must_init
                .range(root, paths.nodes[root].end, true, &mut budget)?;
            seed.may_init
                .range(root, paths.nodes[root].end, true, &mut budget)?;
            seed.ever_written
                .range(root, paths.nodes[root].end, true, &mut budget)?;
            if let Some(root) = paths.external[local.0] {
                seed.must_init
                    .range(root, paths.nodes[root].end, true, &mut budget)?;
                seed.may_init
                    .range(root, paths.nodes[root].end, true, &mut budget)?;
                seed.ever_written
                    .range(root, paths.nodes[root].end, true, &mut budget)?;
            }
        }
        let mut entries: Vec<Option<State>> = (0..f.blocks.len()).map(|_| None).collect();
        entries[f.entry.0] = Some(seed);
        report.stats.blocks_reachable = 1;
        let mut queue = VecDeque::from([f.entry]);
        let mut queued = vec![false; f.blocks.len()];
        queued[f.entry.0] = true;
        while let Some(block) = queue.pop_front() {
            queued[block.0] = false;
            report.stats.worklist_pops += 1;
            budget.spend(1)?;
            let input = entries[block.0].as_ref().expect("queued reachable block");
            budget.spend(input.words())?;
            let mut output = input.clone();
            Transfer {
                f,
                paths: &paths,
                budget: &mut budget,
                report: None,
                block,
                statement: None,
                origin: None,
                diagnostic_limit: limits.diagnostics,
                loans: loans.as_ref(),
                phase: Phase::Read,
            }
            .block(&mut output)?;
            for target in successors(&f.blocks[block.0].terminator.value)
                .into_iter()
                .enumerate()
            {
                let (edge, Some(target)) = target else {
                    continue;
                };
                let previous = if edge == 0 {
                    if let Terminator::SwitchVariant { place, variant, .. } =
                        &f.blocks[block.0].terminator.value
                    {
                        budget.spend(place.projection.len() + 1)?;
                        let node = paths.resolve(place);
                        let old = output.selected[node];
                        output.selected[node] = variant.index;
                        Some((node, old))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let changed = if let Some(entry) = &mut entries[target.0] {
                    entry.merge(&output, &mut budget)?
                } else {
                    budget.spend(output.words())?;
                    entries[target.0] = Some(output.clone());
                    report.stats.blocks_reachable += 1;
                    true
                };
                if let Some((node, old)) = previous {
                    output.selected[node] = old;
                }
                if changed {
                    report.stats.state_changes += 1;
                    if !queued[target.0] {
                        queue.push_back(target);
                        queued[target.0] = true;
                    }
                }
            }
        }
        // Diagnostic order and semantic counters do not depend on queue order.
        for (index, entry) in entries.iter().enumerate() {
            if let Some(entry) = entry {
                report.stats.edges_reachable += successors(&f.blocks[index].terminator.value)
                    .iter()
                    .flatten()
                    .count();
                budget.spend(entry.words())?;
                let mut state = entry.clone();
                Transfer {
                    f,
                    paths: &paths,
                    budget: &mut budget,
                    report: Some(&mut report),
                    block: BlockId(index),
                    statement: None,
                    origin: None,
                    diagnostic_limit: limits.diagnostics,
                    loans: loans.as_ref(),
                    phase: Phase::Read,
                }
                .block(&mut state)?;
                if matches!(
                    f.blocks[index].terminator.value,
                    Terminator::SwitchVariant { .. }
                ) {
                    report.stats.variant_refinements += 1;
                }
            }
        }
        Ok(())
    })();
    if result.is_err() || !report.complete {
        report.complete = false;
        if report.diagnostics.len() >= limits.diagnostics && !report.diagnostics.is_empty() {
            report.diagnostics.pop();
            report.stats.diagnostics_suppressed += 1;
        }
        report.diagnostics.push(Diagnostic {
            kind: ErrorKind::ResourceLimit,
            block: f.entry,
            statement: None,
            origin: None,
            place: None,
        });
    }
    Analysis {
        stats: report.stats,
        diagnostics: report.diagnostics.into_boxed_slice(),
        complete: report.complete,
    }
}

#[cfg(test)]
mod tests {
    use super::super::ir::{
        DefId, EnumDef, EnumTy, FieldDef, LocalDecl, SnapshotId, StructDef, StructTy, VariantDef,
        VariantId, VariantShape,
    };
    use super::*;
    use cfg::{BasicBlock, Constant, Located};

    fn borrowed_source(source: &str) -> cfg::Function {
        let program = super::super::frontend::parse(&format!("{source} fn main(){{}}")).unwrap();
        let signatures = super::super::frontend::signature_table(&program).unwrap();
        let checked =
            super::super::frontend::type_function_with_signatures(&program, 0, &signatures)
                .unwrap();
        cfg::lower(&checked).unwrap()
    }

    #[test]
    fn projected_returns_preserve_nested_places_and_unique_parent_freeze() {
        for source in [
            "fn f<'a>(p:&'a (bool,[i32;2]))->&'a i32{&(*p).1[1]}",
            "struct S{a:bool,b:(i32,[i32;2])} fn f<'a>(p:&'a mut S)->&'a mut i32{&mut (*p).b.1[1]}",
            "fn f()->i32{let mut p=(1,2);let r=&mut p;let q=get(r);*q=3;(*r).1} fn get<'a>(p:&'a mut (i32,i32))->&'a mut i32{&mut (*p).0}",
            "fn f<'a>(p:&'a (i32,bool))->&'a i32{id(get(p))} fn get<'a>(p:&'a (i32,bool))->&'a i32{&(*p).0} fn id<'a>(p:&'a i32)->&'a i32{p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
        let source = "fn f()->i32{let mut p=(1,2);let r=&mut p;let q=get(r);let n=(*r).1;*q+n} fn get<'a>(p:&'a mut (i32,i32))->&'a mut i32{&mut (*p).0}";
        assert!(!analyze(&borrowed_source(source)).passed());
    }

    #[test]
    fn projected_call_candidates_remain_correlated_with_original_arguments() {
        let f = borrowed_source(
            "fn f(p:&(i32,bool),q:&(bool,i32),b:bool)->i32{*pick(b,p,q)} fn pick<'a>(b:bool,p:&'a (i32,bool),q:&'a (bool,i32))->&'a i32{if b{&(*p).0}else{&(*q).1}}",
        );
        assert!(analyze(&f).passed());
        let (block, destination) = f
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match &block.terminator.value {
                Terminator::Call { destination, .. } => Some((BlockId(index), destination.clone())),
                _ => None,
            })
            .unwrap();
        let mut budget = Budget(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let targets = plan
            .resolve(
                &Place {
                    local: destination.local,
                    projection: vec![Projection::Deref],
                },
                block,
                None,
                Phase::Write,
                &mut budget,
            )
            .unwrap()
            .unwrap();
        assert_eq!(targets.len(), 2);
        for (param, field) in [(f.params[0], 0), (f.params[1], 1)] {
            assert!(targets.iter().any(|target| target.place.root
                == CanonicalRoot::ParameterReferent(param)
                && target.place.projection == [Projection::Field(field)]));
        }
        assert!(targets[0].authority != targets[1].authority);
        let f = borrowed_source(
            "fn f()->i32{let x=(1,2);*pick(&x,&x)} fn pick<'a>(p:&'a (i32,i32),q:&'a (i32,i32))->&'a i32{&(*p).0}",
        );
        let (block, destination) = f
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match &block.terminator.value {
                Terminator::Call { destination, .. } => Some((BlockId(index), destination.clone())),
                _ => None,
            })
            .unwrap();
        let mut budget = Budget(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let targets = plan
            .resolve(
                &Place {
                    local: destination.local,
                    projection: vec![Projection::Deref],
                },
                block,
                None,
                Phase::Write,
                &mut budget,
            )
            .unwrap()
            .unwrap();
        assert_eq!(targets.len(), 4);
        assert!(targets.iter().any(|a| {
            targets
                .iter()
                .any(|b| a.place == b.place && a.authority != b.authority)
        }));
        assert!(analyze(&f).passed());
    }

    #[test]
    fn projected_metadata_limits_precede_structural_helper_allocation() {
        use super::super::ir::{InputPath, ReturnContract};
        for (count, depth) in [(4097, 0), (1025, 128), (1, 129)] {
            let mut f = borrowed_source("fn f<'a>(p:&'a i32)->&'a i32{p}");
            let path = InputPath {
                param: 0,
                projection: vec![Projection::Field(0); depth].into(),
            };
            std::sync::Arc::make_mut(&mut f.ref_contract).returns =
                ReturnContract::SharedInputPaths(vec![path; count].into());
            let result = analyze(&f);
            assert!(
                !result.complete,
                "{count}/{depth}: {:?}",
                result.diagnostics
            );
            assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        }
        let mut f = borrowed_source("fn f<'a>(p:&'a i32)->&'a i32{p}");
        std::sync::Arc::make_mut(&mut f.ref_contract).returns = ReturnContract::SharedInputPaths(
            vec![InputPath {
                param: 0,
                projection: vec![Projection::Deref].into(),
            }]
            .into(),
        );
        let result = analyze(&f);
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn projected_candidate_products_and_generated_scratch_fail_closed() {
        let f = borrowed_source(
            "fn f(p:&[i32;32])->i32{*get(p)} fn get<'a>(p:&'a [i32;32])->&'a i32{&(*p)[0]}",
        );
        assert!(analyze(&f).passed());
        let mut budget = Budget(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let mut budget = Budget(32 * 1024 * 1024);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        let result = analyze_with_limits(
            &f,
            Limits {
                state_bytes: 1024,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
    }

    #[test]
    fn mutable_returns_transfer_unique_authority_and_resume_after_last_use() {
        for source in [
            "fn f<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);*q=2;*r=3;x} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let q=id(id(&mut x));let moved=q;*moved=3;x} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let q=id(&mut x);let s=&*q;let n=*s;*q=3;n+x} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;*id(&mut x)} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn mutable_return_freezes_parent_and_preserves_lifetime_dependencies() {
        for source in [
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);let n=*r;*q+n} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);*r=2;*q} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f()->i32{let mut x=1;let q=id(&mut x);let moved=q;let n=*q;*moved+n} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
            "fn f<'a,'b>(s:&'a mut bool,x:&'b mut i32)->&'b mut i32{pick(s,x)} fn pick<'a>(s:&'a mut bool,x:&'a mut i32)->&'a mut i32{x}",
            "fn f()->i32{let mut x=7;let r:&mut i32;{let mut s=true;r=pick(&mut s,&mut x);}*r} fn pick<'a>(s:&'a mut bool,x:&'a mut i32)->&'a mut i32{x}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete, "{source}: {:?}", result.diagnostics);
            assert!(!result.passed(), "accepted {source}");
        }
    }

    #[test]
    fn mutable_return_origins_keep_existing_memory_and_work_caps() {
        let f = borrowed_source(
            "fn f(p:&mut i32)->i32{*id(p)} fn id<'a>(p:&'a mut i32)->&'a mut i32{p}",
        );
        let mut budget = Budget(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let mut budget = Budget(32 * 1024 * 1024);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        for limits in [
            Limits {
                state_bytes: 1024,
                ..Limits::default()
            },
            Limits {
                work: 1,
                ..Limits::default()
            },
        ] {
            let result = analyze_with_limits(&f, limits);
            assert!(!result.complete);
            assert!(!result.passed());
        }
        assert!(analyze(&f).passed());
    }

    #[test]
    fn mutable_normal_return_moves_handle_and_rejects_forged_copy() {
        let f = borrowed_source("fn f<'a>(p:&'a mut i32)->&'a mut i32{p}");
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        let explicit_moves = f
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|item| {
                matches!(
                    item.value,
                    Statement::Assign {
                        value: Rvalue::Use(Operand::Move(_)),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(result.stats.moves, explicit_moves + 1);
        let mut forged = f.clone();
        let value = forged
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|item| match &mut item.value {
                Statement::Assign {
                    value: Rvalue::Use(op @ Operand::Move(_)),
                    ..
                } => Some(op),
                _ => None,
            })
            .unwrap();
        let Operand::Move(source) = value else {
            unreachable!()
        };
        *value = Operand::Copy(source.clone());
        assert!(forged.validate().is_err());
        assert!(!analyze(&forged).passed());
    }

    #[test]
    fn mutable_result_union_write_kills_all_proofs_without_restoring_init() {
        let f = borrowed_source(
            "enum E{A(i32),B(i32)} fn f(b:bool){let mut x=E::A(1);let mut y=E::A(2);let q=pick(b,&mut x,&mut y);*q=E::A(3);} fn pick<'a>(b:bool,x:&'a mut E,y:&'a mut E)->&'a mut E{if b{x}else{y}}",
        );
        assert!(analyze(&f).passed());
        let (block, statement, destination) = f
            .blocks
            .iter()
            .enumerate()
            .find_map(|(block, bb)| {
                bb.statements
                    .iter()
                    .enumerate()
                    .find_map(|(statement, item)| match &item.value {
                        Statement::Assign { destination, .. }
                            if destination.projection.first() == Some(&Projection::Deref) =>
                        {
                            Some((BlockId(block), statement, destination.clone()))
                        }
                        _ => None,
                    })
            })
            .unwrap();
        let paths = Paths::new(&f, 131072).unwrap();
        let mut budget = Budget(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        let targets = plan
            .resolve(
                &destination,
                block,
                Some(statement),
                Phase::Write,
                &mut budget,
            )
            .unwrap()
            .unwrap();
        assert_eq!(targets.len(), 2);
        let nodes: Vec<_> = targets
            .iter()
            .map(|target| paths.resolve_canonical(&target.place))
            .collect();
        for initialized in [true, false] {
            let mut state = State::new(f.locals.len(), paths.nodes.len());
            for local in 0..f.locals.len() {
                state.must_live.set(local, true);
                state.may_live.set(local, true);
            }
            state
                .must_init
                .range(0, paths.nodes.len(), true, &mut budget)
                .unwrap();
            state
                .may_init
                .range(0, paths.nodes.len(), true, &mut budget)
                .unwrap();
            for node in &nodes {
                state.selected[*node] = 0;
            }
            if !initialized {
                state.must_init.set(nodes[0] + 1, false);
            }
            let must = state.must_init.clone();
            let may = state.may_init.clone();
            Transfer {
                f: &f,
                paths: &paths,
                budget: &mut budget,
                report: None,
                block,
                statement: Some(statement),
                origin: None,
                diagnostic_limit: 10,
                loans: Some(&plan),
                phase: Phase::Write,
            }
            .write(&mut state, &destination)
            .unwrap();
            assert_eq!(state.must_init, must);
            assert_eq!(state.may_init, may);
            for node in &nodes {
                assert_eq!(state.selected[*node], usize::MAX);
            }
        }
    }

    #[test]
    fn shared_returns_preserve_transitive_origins_and_parent_authority() {
        for source in [
            "fn f<'a>(p:&'a i32)->&'a i32{p}",
            "fn f<'a:'b,'b>(s:&'a bool,x:&'b i32)->&'b i32{pick(s,x)} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}",
            "fn f()->i32{let x=7;*id(id(&x))} fn id<'a>(x:&'a i32)->&'a i32{x}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);let n=*r;n+*q} fn id<'a>(x:&'a i32)->&'a i32{x}",
            "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);let n=*q;*r=2;n+*r} fn id<'a>(x:&'a i32)->&'a i32{x}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn shared_return_lifetime_dependencies_do_not_launder_universal_bounds() {
        for source in [
            "fn f<'a,'b>(s:&'a bool,x:&'b i32)->&'b i32{pick(s,x)} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}",
            "fn f<'a,'b>(s:&'a bool,x:&'b i32)->&'b i32{id(pick(s,x))} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x} fn id<'a>(x:&'a i32)->&'a i32{x}",
            "fn f<'a>(x:&'a i32)->&'a i32{let s=true;pick(&s,x)} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(!result.passed(), "accepted {source}");
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::InvalidReference(_))),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn shared_returns_pin_non_target_inputs_but_not_independent_regions() {
        let same = "fn f()->i32{let x=7;let r:&i32;{let s=true;r=pick(&s,&x);}*r} fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}";
        let result = analyze(&borrowed_source(same));
        assert!(result.complete);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_)))
        );
        let independent = "fn f()->i32{let x=7;let r:&i32;{let s=true;r=pick(&s,&x);}*r} fn pick<'a,'b>(s:&'a bool,x:&'b i32)->&'b i32{x}";
        let result = analyze(&borrowed_source(independent));
        assert!(result.passed(), "{:?}", result.diagnostics);
        let mutation = "fn f()->i32{let mut x=1;let r=&mut x;let q=id(r);*r=2;*q} fn id<'a>(x:&'a i32)->&'a i32{x}";
        let result = analyze(&borrowed_source(mutation));
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_)))
        );
    }

    #[test]
    fn shared_return_call_defines_real_outgoing_origin_without_sites() {
        let f = borrowed_source("fn f(p:&i32)->i32{*id(p)} fn id<'a>(x:&'a i32)->&'a i32{x}");
        assert!(f.loan_sites.is_empty());
        let (block, destination) = f
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match &block.terminator.value {
                Terminator::Call { destination, .. } => Some((BlockId(index), destination.clone())),
                _ => None,
            })
            .unwrap();
        let deref = Place {
            local: destination.local,
            projection: vec![Projection::Deref],
        };
        let mut budget = Budget(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        assert!(
            plan.resolve(&deref, block, None, Phase::Read, &mut budget)
                .unwrap()
                .is_err()
        );
        let after = plan
            .resolve(&deref, block, None, Phase::Write, &mut budget)
            .unwrap()
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].place.root,
            CanonicalRoot::ParameterReferent(f.params[0])
        );
        let mut budget = Budget(32 * 1024 * 1024);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        assert!(analyze(&f).passed());
    }

    #[test]
    fn shared_return_origin_contract_and_scratch_caps_fail_incomplete() {
        let mut f = borrowed_source("fn f(p:&i32)->i32{*id(p)} fn id<'a>(x:&'a i32)->&'a i32{x}");
        let call = f
            .blocks
            .iter()
            .find(|block| matches!(block.terminator.value, Terminator::Call { .. }))
            .unwrap()
            .clone();
        f.blocks
            .extend(std::iter::repeat_n(call, super::super::ir::MAX_LOAN_SITES));
        let result = analyze(&f);
        assert!(!result.complete);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        for count in [64, 65] {
            let mut f =
                borrowed_source("fn f(p:&i32)->i32{*id(p)} fn id<'a>(x:&'a i32)->&'a i32{x}");
            for block in &mut f.blocks {
                if let Terminator::Call { signature, .. } = &mut block.terminator.value {
                    std::sync::Arc::make_mut(&mut signature.ref_contract).region_count = count;
                }
            }
            let result = analyze_with_limits(
                &f,
                Limits {
                    operations: 512,
                    ..Limits::default()
                },
            );
            assert!(!result.complete, "count={count}: {:?}", result.diagnostics);
            assert!(!result.passed());
        }
        let f = borrowed_source("fn f<'a>(p:&'a i32)->&'a i32{p}");
        let result = analyze_with_limits(
            &f,
            Limits {
                state_bytes: 1024,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
        let mut malformed = f;
        std::sync::Arc::make_mut(&mut malformed.ref_contract).result_region =
            Some(super::super::ir::RegionParamId(4));
        let result = analyze(&malformed);
        assert!(result.complete);
        assert!(matches!(
            result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn shared_return_cycles_fail_closed_and_empty_diverging_contract_can_pass() {
        let f = borrowed_source(
            "fn f(p:&i32)->i32{let mut r=p;loop{r=id(r);if true{break;}}*r} fn id<'a>(x:&'a i32)->&'a i32{x}",
        );
        let result = analyze(&f);
        assert!(result.complete);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::UnsupportedBorrowing(_))),
            "{:?}",
            result.diagnostics
        );
        // Loop-result contextual typing is a separate frontend boundary. This
        // valid private CFG has no normal return and therefore no obligation
        // to fabricate an eligible returned pointer.
        let mut f = borrowed_source("fn f<'a,'b>(p:&'a i32)->&'a i32{p}");
        let contract = std::sync::Arc::make_mut(&mut f.ref_contract);
        contract.result_region = Some(super::super::ir::RegionParamId(1));
        contract.returns = super::super::ir::ReturnContract::SharedInputPaths(vec![].into());
        f.blocks = vec![BasicBlock {
            statements: vec![],
            terminator: located(Terminator::Goto(BlockId(0))),
        }];
        f.entry = BlockId(0);
        f.validate().unwrap();
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn reference_parameters_without_sites_have_independent_external_paths() {
        for source in [
            "fn f(p:&mut i32)->i32{*p=3;*p}",
            "fn f(p:&i32,q:&i32)->i32{*p+*q}",
        ] {
            let f = borrowed_source(source);
            assert!(f.loan_sites.is_empty());
            let result = analyze(&f);
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
            let paths = Paths::new(&f, 131072).unwrap();
            assert!(paths.nodes.len() > f.locals.len());
            let mut budget = Budget(32 * 1024 * 1024);
            let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
            assert!(plan.bytes > 0);
            let mut budget = Budget(32 * 1024 * 1024);
            assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        }
    }

    #[test]
    fn reference_parameter_referent_outlives_moved_carrier_storage() {
        let f =
            borrowed_source("fn f(p:&mut i32)->i32{let c:&mut i32;{let q=p;c=&mut *q;}*c=3;*c}");
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn reference_parameter_bounds_count_formals_and_external_expansion() {
        let mut f = borrowed_source("fn f(p:&i32,q:&i32)->i32{*p+*q}");
        let result = analyze_with_limits(
            &f,
            Limits {
                paths: f.locals.len(),
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
        let p = f.params[0];
        f.loan_sites = vec![
            cfg::LoanSite {
                kind: super::super::ir::BorrowKind::Shared,
                place: Place {
                    local: p,
                    projection: vec![Projection::Deref]
                },
                origin: None
            };
            super::super::ir::MAX_LOAN_SITES - 1
        ];
        let result = analyze(&f);
        assert!(!result.complete);
        assert!(!result.passed());
    }

    #[test]
    fn borrowing_last_shared_read_ends_before_same_assignment_write() {
        let mut f = borrowed_source("fn f()->i32{let mut x=1;let r=&x;x=*r;x}");
        let x = LocalId(f.locals.iter().position(|local| local.name == "x").unwrap());
        let r = LocalId(f.locals.iter().position(|local| local.name == "r").unwrap());
        // Source lowering may snapshot the RHS. Force the private CFG's last
        // reference use into the owner assignment itself to test both phases.
        let value = f
            .blocks
            .iter_mut()
            .rev()
            .flat_map(|block| block.statements.iter_mut().rev())
            .find_map(|statement| match &mut statement.value {
                Statement::Assign { destination, value } if destination.local == x => Some(value),
                _ => None,
            })
            .unwrap();
        *value = Rvalue::Use(Operand::Copy(Place {
            local: r,
            projection: vec![Projection::Deref],
        }));
        f.validate().unwrap();
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn borrowing_unused_issuance_checks_initialization_and_mutability() {
        for source in [
            "fn f(){let x:i32;let _=&x;}",
            "fn f(){let x=1;let _=&mut x;}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(!result.passed(), "accepted {source}");
        }
    }

    #[test]
    fn borrowing_parent_retarget_and_storage_end_preserve_child_target() {
        for source in [
            "fn f()->i32{let mut x=1;let mut y=2;let mut r=&mut x;let c=&mut *r;r=&mut y;*c=3;*r=4;x+y}",
            "fn f()->i32{let mut x=1;let c:&mut i32;{let r=&mut x;c=&mut *r;}*c=2;x}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.passed(), "{source}: {:?}", result.diagnostics);
        }
    }

    #[test]
    fn borrowing_live_child_blocks_parent_move_and_owner_end() {
        for source in [
            "fn f()->i32{let mut x=1;let r=&mut x;let c=&mut *r;let moved=r;*c=2;*moved}",
            "fn f()->i32{let c:&mut i32;{let mut x=1;let r=&mut x;c=&mut *r;}*c=2;0}",
        ] {
            let result = analyze(&borrowed_source(source));
            assert!(result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_))),
                "{source}: {:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn borrowing_child_authority_is_candidate_specific() {
        let shared =
            borrowed_source("fn f()->i32{let mut x=1;let r=&mut x;let c=&*r;let n=*r;n+*c}");
        let result = analyze(&shared);
        assert!(result.passed(), "{:?}", result.diagnostics);
        let unique =
            borrowed_source("fn f()->i32{let mut x=1;let r=&mut x;let c=&mut *r;let n=*r;n+*c}");
        let result = analyze(&unique);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| matches!(d.kind, ErrorKind::BorrowConflict(_)))
        );
    }

    #[test]
    fn borrowing_preflight_and_scratch_caps_fail_closed() {
        let f = borrowed_source("fn f()->i32{let mut x=1;let r=&mut x;let c=&mut *r;*c=2;x}");
        let mut budget = Budget(32 * 1024 * 1024);
        let plan = borrow::Plan::build(&f, &mut budget, 128 * 1024 * 1024).unwrap();
        assert!(plan.bytes > 4096);
        let mut budget = Budget(32 * 1024 * 1024);
        assert!(borrow::Plan::build(&f, &mut budget, plan.bytes - 1).is_err());
        let result = analyze_with_limits(
            &f,
            Limits {
                state_bytes: 1,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
        let result = analyze_with_limits(
            &f,
            Limits {
                work: 1,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
    }

    fn located<T>(value: T) -> Located<T> {
        Located {
            value,
            origin: None,
        }
    }
    fn place(local: usize) -> Place {
        Place {
            local: LocalId(local),
            projection: vec![],
        }
    }
    fn local(ty: Ty) -> LocalDecl {
        LocalDecl {
            name: String::new(),
            ty,
            mutable: true,
        }
    }
    fn nominal(fields: Vec<Ty>) -> Ty {
        Ty::Struct(StructTy::new(StructDef {
            id: DefId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "Paths".into(),
            module: String::new(),
            public: false,
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| FieldDef {
                    name: format!("f{index}"),
                    ty,
                    public: false,
                })
                .collect::<Vec<_>>()
                .into(),
        }))
    }
    fn fixture() -> cfg::Function {
        cfg::Function {
            ref_contract: Default::default(),
            name: "caps".into(),
            params: vec![],
            result: Ty::Unit,
            locals: vec![local(Ty::Unit)],
            entry: BlockId(0),
            return_local: LocalId(0),
            loan_sites: vec![],
            blocks: vec![BasicBlock {
                statements: vec![
                    located(Statement::StorageLive(LocalId(0))),
                    located(Statement::Assign {
                        destination: place(0),
                        value: Rvalue::Use(Operand::Constant(Constant::Unit)),
                    }),
                ],
                terminator: located(Terminator::Return),
            }],
        }
    }

    fn unit_enum() -> EnumTy {
        EnumTy::new(EnumDef {
            id: DefId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "Tag".into(),
            module: String::new(),
            public: false,
            variants: ["A", "B"]
                .into_iter()
                .map(|name| VariantDef {
                    name: name.into(),
                    shape: VariantShape::Unit,
                    fields: vec![].into(),
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    fn enum_fixture() -> cfg::Function {
        let mut f = fixture();
        let ty = Ty::Enum(unit_enum());
        f.locals
            .extend([local(ty.clone()), local(Ty::I32), local(ty)]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
        ]);
        f
    }

    fn discriminant() -> Located<Statement> {
        located(Statement::Assign {
            destination: place(2),
            value: Rvalue::Discriminant(place(1)),
        })
    }

    fn payload_enum(variants: Vec<Vec<Ty>>) -> EnumTy {
        EnumTy::new(EnumDef {
            id: DefId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "Payload".into(),
            module: String::new(),
            public: false,
            variants: variants
                .into_iter()
                .enumerate()
                .map(|(variant, fields)| VariantDef {
                    name: format!("V{variant}"),
                    shape: VariantShape::Tuple,
                    fields: fields
                        .into_iter()
                        .enumerate()
                        .map(|(index, ty)| FieldDef {
                            name: index.to_string(),
                            ty,
                            public: false,
                        })
                        .collect::<Vec<_>>()
                        .into(),
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    fn variant(ty: &EnumTy, index: usize) -> VariantId {
        VariantId {
            owner: ty.def().id,
            index,
        }
    }

    fn payload_field(ty: &EnumTy, selected: usize, field: usize) -> Place {
        Place {
            local: LocalId(1),
            projection: vec![
                Projection::Downcast(variant(ty, selected)),
                Projection::Field(field),
            ],
        }
    }

    fn payload_fixture(ty: &EnumTy) -> cfg::Function {
        let mut f = fixture();
        f.locals
            .extend([local(Ty::Enum(ty.clone())), local(Ty::I32)]);
        f.params.push(LocalId(1));
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(2))));
        f
    }

    fn read_payload(source: Place) -> Located<Statement> {
        located(Statement::Assign {
            destination: place(2),
            value: Rvalue::Use(Operand::Copy(source)),
        })
    }

    fn split_payload(f: &mut cfg::Function, ty: &EnumTy) {
        f.blocks[0].terminator = located(Terminator::SwitchVariant {
            place: place(1),
            variant: variant(ty, 0),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Return),
            },
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Return),
            },
        ]);
    }

    #[test]
    fn reference_calls_capture_effects_before_move_and_preserve_initialization() {
        use super::super::ir::BorrowKind;
        for kind in [BorrowKind::Shared, BorrowKind::Mut] {
            let ty = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
            let mut f = payload_fixture(&ty);
            let reference = Ty::Ref {
                kind,
                referent: std::sync::Arc::new(Ty::Enum(ty.clone())),
            };
            f.locals.push(local(reference.clone()));
            f.loan_sites.push(cfg::LoanSite {
                kind,
                place: place(1),
                origin: None,
            });
            split_payload(&mut f, &ty);
            f.blocks[1].statements.extend([
                located(Statement::StorageLive(LocalId(3))),
                located(Statement::Assign {
                    destination: place(3),
                    value: Rvalue::Borrow(cfg::LoanId(0)),
                }),
            ]);
            f.blocks[1].terminator = located(Terminator::Call {
                name: "callee".into(),
                signature: super::super::ir::Signature {
                    ref_contract: Default::default(),
                    params: vec![reference],
                    result: Ty::Unit,
                },
                args: vec![Operand::Move(place(3))],
                destination: place(0),
                next: BlockId(3),
            });
            f.blocks.push(BasicBlock {
                statements: vec![
                    read_payload(payload_field(&ty, 0, 0)),
                    located(Statement::Inspect(place(1))),
                ],
                terminator: located(Terminator::Return),
            });
            f.validate().unwrap();
            let result = analyze(&f);
            assert!(result.complete);
            if kind == BorrowKind::Shared {
                assert!(result.passed(), "{:?}", result.diagnostics);
                assert_eq!(result.stats.variant_proofs_killed, 0);
            } else {
                assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
                assert_eq!(
                    result.diagnostics[0].kind,
                    ErrorKind::VariantNotDefinitelySelected
                );
                assert_eq!(result.stats.variant_proofs_killed, 1);
            }
        }
    }

    #[test]
    fn reference_call_union_mutation_kills_every_candidate_proof() {
        use super::super::ir::BorrowKind;
        let ty = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        let pair = Ty::Tuple(vec![Ty::Enum(ty.clone()), Ty::Enum(ty.clone())].into());
        let reference = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: std::sync::Arc::new(Ty::Enum(ty.clone())),
        };
        let mut f = fixture();
        f.locals.extend([
            local(pair),
            local(Ty::I32),
            local(reference.clone()),
            local(Ty::Bool),
        ]);
        f.params.extend([LocalId(1), LocalId(4)]);
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
        ]);
        let member = |index| Place {
            local: LocalId(1),
            projection: vec![Projection::Field(index)],
        };
        for index in 0..2 {
            f.loan_sites.push(cfg::LoanSite {
                kind: BorrowKind::Mut,
                place: member(index),
                origin: None,
            });
        }
        f.blocks[0].terminator = located(Terminator::SwitchVariant {
            place: member(0),
            variant: variant(&ty, 0),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::SwitchVariant {
                    place: member(1),
                    variant: variant(&ty, 0),
                    then_block: BlockId(3),
                    else_block: BlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Return),
            },
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Switch {
                    condition: Operand::Copy(place(4)),
                    then_block: BlockId(4),
                    else_block: BlockId(5),
                }),
            },
        ]);
        for loan in 0..2 {
            f.blocks.push(BasicBlock {
                statements: vec![located(Statement::Assign {
                    destination: place(3),
                    value: Rvalue::Borrow(cfg::LoanId(loan)),
                })],
                terminator: located(Terminator::Goto(BlockId(6))),
            });
        }
        f.blocks.push(BasicBlock {
            statements: vec![],
            terminator: located(Terminator::Call {
                name: "callee".into(),
                signature: super::super::ir::Signature {
                    ref_contract: Default::default(),
                    params: vec![reference],
                    result: Ty::Unit,
                },
                args: vec![Operand::Move(place(3))],
                destination: place(0),
                next: BlockId(7),
            }),
        });
        let reads = (0..2)
            .map(|index| {
                let mut field = member(index);
                field
                    .projection
                    .extend([Projection::Downcast(variant(&ty, 0)), Projection::Field(0)]);
                read_payload(field)
            })
            .chain([located(Statement::Inspect(place(1)))])
            .collect();
        f.blocks.push(BasicBlock {
            statements: reads,
            terminator: located(Terminator::Return),
        });
        f.validate().unwrap();
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|d| d.kind == ErrorKind::VariantNotDefinitelySelected)
        );
        assert_eq!(result.stats.variant_proofs_killed, 2);
    }

    fn array(element: Ty, len: usize) -> Ty {
        Ty::Array {
            element: std::sync::Arc::new(element),
            len,
        }
    }

    fn indexed(local: usize, index: usize) -> Place {
        Place {
            local: LocalId(local),
            projection: vec![Projection::ConstantIndex(index)],
        }
    }

    fn dynamic_fixture(element: Ty) -> cfg::Function {
        let mut f = fixture();
        f.locals.extend([
            local(array(element.clone(), 2)),
            local(Ty::Usize),
            local(element),
        ]);
        f.params.extend([LocalId(1), LocalId(2)]);
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(3))));
        f
    }

    fn dynamic_read() -> Located<Statement> {
        located(Statement::Assign {
            destination: place(3),
            value: Rvalue::IndexCopy {
                array: place(1),
                index: Operand::Copy(place(2)),
            },
        })
    }

    fn dynamic_inspect() -> Located<Statement> {
        located(Statement::IndexInspect {
            array: place(1),
            index: Operand::Copy(place(2)),
        })
    }

    #[test]
    fn dynamic_copy_reads_and_ignored_noncopy_values_do_not_consume_array() {
        for element in [
            Ty::I32,
            Ty::Unit,
            Ty::Tuple(vec![Ty::I32, Ty::Usize].into()),
        ] {
            let mut f = dynamic_fixture(element);
            f.blocks[0].statements.extend([
                dynamic_read(),
                dynamic_read(),
                dynamic_inspect(),
                located(Statement::Inspect(place(1))),
            ]);
            let result = analyze(&f);
            assert!(result.passed(), "{:?}", result.diagnostics);
            assert_eq!(result.stats.moves, 0);
        }
        let mut f = dynamic_fixture(nominal(vec![Ty::I32]));
        f.blocks[0]
            .statements
            .extend([dynamic_inspect(), located(Statement::Inspect(place(1)))]);
        assert!(analyze(&f).passed());
        f.blocks[0].statements.push(dynamic_read());
        assert!(matches!(
            analyze(&f).diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn dynamic_index_readiness_follows_ordered_initialization_but_not_earlier_static_checks() {
        let mut f = dynamic_fixture(Ty::I32);
        f.params.retain(|local| *local != LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(1))),
            located(Statement::Assign {
                destination: place(1),
                value: Rvalue::Aggregate {
                    ty: array(Ty::I32, 2),
                    fields: vec![
                        Operand::Constant(Constant::I32(7)),
                        Operand::Constant(Constant::I32(8)),
                    ],
                },
            }),
            dynamic_read(),
        ]);
        assert!(analyze(&f).passed());
        // Represents an earlier static indexed-base readiness check. Its
        // diagnostic cannot be erased by initialization in the later index.
        f.blocks[0]
            .statements
            .insert(4, located(Statement::Inspect(place(1))));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn dynamic_inspect_rejects_unready_array_or_index_without_consuming_either() {
        for missing in [1, 2] {
            let mut f = dynamic_fixture(Ty::I32);
            f.params.retain(|local| *local != LocalId(missing));
            f.blocks[0].statements.extend([
                located(Statement::StorageLive(LocalId(missing))),
                dynamic_inspect(),
            ]);
            let result = analyze(&f);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(result.diagnostics[0].place, Some(place(missing)));
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::ReadNotDefinitelyInitialized
            );
            assert_eq!(result.stats.moves, 0);
        }
    }

    #[test]
    fn dynamic_operations_consume_index_before_array_read_and_rhs_before_index() {
        let mut f = dynamic_fixture(Ty::Usize);
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(3),
            value: Rvalue::IndexCopy {
                array: place(1),
                index: Operand::Move(indexed(1, 0)),
            },
        }));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].place, Some(place(1)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );

        let mut f = dynamic_fixture(Ty::Usize);
        f.blocks[0].statements.push(located(Statement::IndexAssign {
            array: place(1),
            index: Operand::Copy(place(2)),
            value: Operand::Move(place(2)),
        }));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].place, Some(place(2)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn dynamic_noncopy_replacement_consumes_snapshot_but_preserves_array_initialization() {
        let element = nominal(vec![Ty::I32]);
        let mut f = dynamic_fixture(element.clone());
        f.locals.push(local(element));
        f.params.push(LocalId(4));
        f.blocks[0].statements.extend([
            located(Statement::IndexAssign {
                array: place(1),
                index: Operand::Copy(place(2)),
                value: Operand::Move(place(4)),
            }),
            dynamic_inspect(),
            located(Statement::Inspect(place(1))),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.moves, 1);
        f.blocks[0]
            .statements
            .push(located(Statement::Inspect(place(4))));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].place, Some(place(4)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn dynamic_replacement_requires_mutable_initialized_array() {
        for (mutable, initialized, expected) in [
            (true, true, None),
            (false, true, Some(ErrorKind::ImmutableReassignment)),
            (true, false, Some(ErrorKind::ReadNotDefinitelyInitialized)),
        ] {
            let mut f = dynamic_fixture(Ty::I32);
            f.locals[1].mutable = mutable;
            if !initialized {
                f.params.retain(|local| *local != LocalId(1));
                f.blocks[0]
                    .statements
                    .push(located(Statement::StorageLive(LocalId(1))));
            }
            f.blocks[0].statements.push(located(Statement::IndexAssign {
                array: place(1),
                index: Operand::Copy(place(2)),
                value: Operand::Constant(Constant::I32(9)),
            }));
            let result = analyze(&f);
            if let Some(expected) = expected {
                assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
                assert_eq!(result.diagnostics[0].kind, expected);
            } else {
                assert!(result.passed(), "{:?}", result.diagnostics);
            }
        }
    }

    #[test]
    fn dynamic_replacement_never_changes_must_or_may_initialization_bits() {
        let f = dynamic_fixture(Ty::I32);
        let paths = Paths::new(&f, 100).unwrap();
        let mut state = State::new(f.locals.len(), paths.nodes.len());
        let node = paths.resolve(&place(1));
        state.must_live.set(1, true);
        state.must_init.set(node, true);
        state.must_init.set(paths.resolve(&indexed(1, 1)), true);
        state.may_init.set(node, true);
        state.may_init.set(paths.resolve(&indexed(1, 0)), true);
        let must = state.must_init.clone();
        let may = state.may_init.clone();
        let mut budget = Budget(1000);
        Transfer {
            f: &f,
            paths: &paths,
            budget: &mut budget,
            report: None,
            block: BlockId(0),
            statement: None,
            origin: None,
            diagnostic_limit: 10,
            loans: None,
            phase: Phase::Read,
        }
        .index_write(&mut state, &place(1))
        .unwrap();
        assert_eq!(state.must_init, must);
        assert_eq!(state.may_init, may);
        assert!(state.ever_written.get(node));
    }

    #[test]
    fn dynamic_replacement_kills_all_contained_proofs_but_preserves_outer_enum() {
        let element = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        let outer = payload_enum(vec![
            vec![array(Ty::Enum(element.clone()), 2), Ty::I32],
            vec![Ty::I32],
        ]);
        let mut f = fixture();
        f.locals.extend([
            local(Ty::Enum(outer.clone())),
            local(Ty::Usize),
            local(Ty::Enum(element.clone())),
            local(Ty::I32),
        ]);
        f.params.extend([LocalId(1), LocalId(2), LocalId(3)]);
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(4))));
        split_payload(&mut f, &outer);
        let array = payload_field(&outer, 0, 0);
        let item = |index| {
            let mut place = array.clone();
            place.projection.push(Projection::ConstantIndex(index));
            place
        };
        f.blocks[1].terminator = located(Terminator::SwitchVariant {
            place: item(0),
            variant: variant(&element, 0),
            then_block: BlockId(3),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::SwitchVariant {
                    place: item(1),
                    variant: variant(&element, 0),
                    then_block: BlockId(4),
                    else_block: BlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![
                    located(Statement::IndexAssign {
                        array: array.clone(),
                        index: Operand::Copy(place(2)),
                        value: Operand::Move(place(3)),
                    }),
                    located(Statement::Inspect(payload_field(&outer, 0, 1))),
                    located(Statement::Inspect(array.clone())),
                ],
                terminator: located(Terminator::Return),
            },
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.variant_proofs_killed, 2);
        for index in [0, 1] {
            let mut field = item(index);
            field.projection.extend([
                Projection::Downcast(variant(&element, 0)),
                Projection::Field(0),
            ]);
            f.blocks[4].statements.push(located(Statement::Assign {
                destination: place(4),
                value: Rvalue::Use(Operand::Copy(field)),
            }));
        }
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|d| d.kind == ErrorKind::VariantNotDefinitelySelected)
        );
    }

    #[test]
    fn usize_constants_are_scalar_leaves_without_index_narrowing() {
        assert!(Ty::Usize.is_copy());
        let mut f = dynamic_fixture(Ty::Usize);
        f.blocks[0].statements.extend([
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Constant(Constant::Usize(u64::MAX))),
            }),
            dynamic_read(),
            dynamic_inspect(),
        ]);
        let result = analyze(&f);
        // Static ownership does not narrow or guess a runtime index. Bounds
        // failure remains the runtime operation's responsibility, even at MAX.
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.move_paths, 6);
        assert!(preflight_ty(&Ty::Usize, 0, &mut Budget(1)).is_ok());
        f.locals[1].ty = array(Ty::Usize, 0);
        assert!(analyze(&f).passed());
    }

    #[test]
    fn malformed_runtime_indices_are_rejected_in_dead_blocks() {
        for operation in 0..3 {
            let mut f = dynamic_fixture(Ty::I32);
            let index = Operand::Constant(Constant::I32(0));
            let value = match operation {
                0 => Statement::Assign {
                    destination: place(3),
                    value: Rvalue::IndexCopy {
                        array: place(1),
                        index,
                    },
                },
                1 => Statement::IndexInspect {
                    array: place(1),
                    index,
                },
                _ => Statement::IndexAssign {
                    array: place(1),
                    index,
                    value: Operand::Constant(Constant::I32(7)),
                },
            };
            f.blocks.push(BasicBlock {
                statements: vec![located(value)],
                terminator: located(Terminator::Unreachable),
            });
            let result = analyze(&f);
            assert!(!result.passed());
            assert!(matches!(
                result.diagnostics[0].kind,
                ErrorKind::InvalidCfg(_)
            ));
            assert_eq!(result.stats.worklist_pops, 0);
        }
    }

    #[test]
    fn runtime_index_operations_preflight_array_subtrees_before_validation() {
        let mut base = dynamic_fixture(Ty::I32);
        base.locals[1].ty = array(Ty::I32, 64);
        let base_cost = (0..=1024)
            .find(|operations| {
                preflight(
                    &base,
                    Limits {
                        operations: *operations,
                        ..Limits::default()
                    },
                )
                .is_ok()
            })
            .expect("small fixture fits bounded preflight");
        for operation in 0..3 {
            let mut f = dynamic_fixture(Ty::I32);
            f.locals[1].ty = array(Ty::I32, 64);
            let index = Operand::Copy(place(2));
            let value = match operation {
                0 => Statement::Assign {
                    destination: place(3),
                    value: Rvalue::IndexCopy {
                        array: place(1),
                        index,
                    },
                },
                1 => Statement::IndexInspect {
                    array: place(1),
                    index,
                },
                _ => Statement::IndexAssign {
                    array: place(1),
                    index,
                    value: Operand::Constant(Constant::I32(7)),
                },
            };
            f.blocks[0].statements.push(located(value));
            let result = analyze_with_limits(
                &f,
                Limits {
                    operations: base_cost + 128,
                    ..Limits::default()
                },
            );
            assert!(!result.complete, "operation={operation}");
            assert_eq!(result.stats.move_paths, 0);
            assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        }
    }

    #[test]
    fn array_paths_are_distinct_and_zero_arrays_have_only_a_root() {
        let mut f = fixture();
        f.locals
            .extend([local(array(array(Ty::I32, 2), 2)), local(array(Ty::I32, 0))]);
        let paths = Paths::new(&f, 9).unwrap();
        assert_eq!(paths.nodes.len(), 9);
        assert!(Paths::new(&f, 8).is_err());
        assert_ne!(paths.resolve(&indexed(1, 0)), paths.resolve(&indexed(1, 1)));
        let nested = |index| Place {
            local: LocalId(1),
            projection: vec![
                Projection::ConstantIndex(index),
                Projection::ConstantIndex(1),
            ],
        };
        assert_ne!(paths.resolve(&nested(0)), paths.resolve(&nested(1)));
        assert!(paths.nodes[paths.roots[2]].children.is_empty());
    }

    #[test]
    fn repeat_zero_and_one_consume_noncopy_operand_once() {
        for len in [0, 1] {
            let element = nominal(vec![Ty::I32]);
            let ty = array(element.clone(), len);
            let mut f = fixture();
            f.locals.extend([local(element), local(ty.clone())]);
            f.params.push(LocalId(1));
            f.blocks[0].statements.extend([
                located(Statement::StorageLive(LocalId(2))),
                located(Statement::Assign {
                    destination: place(2),
                    value: Rvalue::Repeat {
                        ty,
                        value: Operand::Move(place(1)),
                    },
                }),
                located(Statement::Inspect(place(2))),
            ]);
            let result = analyze(&f);
            assert!(result.passed(), "len={len}: {:?}", result.diagnostics);
            assert_eq!(result.stats.moves, 1);
            assert_eq!(result.stats.operands_read, 3);
            f.blocks[0]
                .statements
                .push(located(Statement::Inspect(place(1))));
            let result = analyze(&f);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::ReadNotDefinitelyInitialized
            );
        }
    }

    #[test]
    fn repeat_copy_operand_is_evaluated_once_and_noncopy_many_is_rejected() {
        let mut f = fixture();
        let ty = array(Ty::I32, 4);
        f.locals.extend([local(Ty::I32), local(ty.clone())]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Repeat {
                    ty,
                    value: Operand::Copy(place(1)),
                },
            }),
            located(Statement::Inspect(place(2))),
            located(Statement::Inspect(place(1))),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.operands_read, 3);
        assert_eq!(result.stats.moves, 0);

        let element = nominal(vec![Ty::I32]);
        f.locals[1].ty = element.clone();
        f.locals[2].ty = array(element, 2);
        let Statement::Assign { value, .. } = &mut f.blocks[0].statements[3].value else {
            unreachable!()
        };
        *value = Rvalue::Repeat {
            ty: f.locals[2].ty.clone(),
            value: Operand::Move(place(1)),
        };
        let result = analyze(&f);
        assert!(matches!(
            &result.diagnostics[0].kind,
            ErrorKind::InvalidCfg(_)
        ));
    }

    #[test]
    fn zero_length_noncopy_array_whole_move_clears_root_marker() {
        let ty = array(nominal(vec![Ty::I32]), 0);
        assert!(!ty.is_copy());
        let mut f = fixture();
        f.locals.extend([local(ty.clone()), local(ty)]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Inspect(place(1))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
            located(Statement::Inspect(place(2))),
            located(Statement::Inspect(place(1))),
        ]);
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].place, Some(place(1)));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn indexed_copy_field_reads_preserve_noncopy_array_for_whole_move() {
        let element = nominal(vec![nominal(vec![Ty::I32]), Ty::I32]);
        let ty = array(element, 2);
        let mut f = fixture();
        f.locals
            .extend([local(ty.clone()), local(Ty::I32), local(ty)]);
        f.params.push(LocalId(1));
        let mut scalar = indexed(1, 0);
        scalar.projection.push(Projection::Field(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
            located(Statement::Inspect(place(1))),
            read_payload(scalar.clone()),
            read_payload(scalar),
            located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.moves, 1);
    }

    #[test]
    fn indexed_noncopy_moves_are_rejected_even_through_projected_subfields() {
        let inner = nominal(vec![Ty::I32]);
        let element = nominal(vec![inner.clone(), Ty::I32]);
        for nested in [false, true] {
            let mut f = fixture();
            f.locals.extend([
                local(array(element.clone(), 2)),
                local(if nested {
                    inner.clone()
                } else {
                    element.clone()
                }),
            ]);
            f.params.push(LocalId(1));
            let mut source = indexed(1, 0);
            if nested {
                source.projection.push(Projection::Field(0));
            }
            f.blocks[0].statements.extend([
                located(Statement::StorageLive(LocalId(2))),
                located(Statement::Assign {
                    destination: place(2),
                    value: Rvalue::Use(Operand::Move(source)),
                }),
            ]);
            let result = analyze(&f);
            assert!(!result.passed());
            assert!(matches!(
                result.diagnostics[0].kind,
                ErrorKind::InvalidCfg(_)
            ));
        }
    }

    #[test]
    fn indexed_private_copy_move_remains_a_consuming_transfer() {
        let mut f = fixture();
        f.locals.extend([local(array(Ty::I32, 2)), local(Ty::I32)]);
        f.params.push(LocalId(1));
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(indexed(1, 0))),
            }),
            located(Statement::Inspect(indexed(1, 1))),
            located(Statement::Inspect(indexed(1, 0))),
        ]);
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.stats.moves, 1);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
        assert_eq!(result.diagnostics[0].place, Some(indexed(1, 0)));
    }

    #[test]
    fn array_base_inspection_rejects_uninitialized_or_moved_wildcard_bases() {
        let ty = array(nominal(vec![Ty::I32]), 2);
        for moved in [false, true] {
            let mut f = fixture();
            f.locals.extend([local(ty.clone()), local(ty.clone())]);
            if moved {
                f.params.push(LocalId(1));
                f.blocks[0].statements.extend([
                    located(Statement::StorageLive(LocalId(2))),
                    located(Statement::Inspect(place(1))),
                    located(Statement::Assign {
                        destination: place(2),
                        value: Rvalue::Use(Operand::Move(place(1))),
                    }),
                ]);
            } else {
                f.blocks[0]
                    .statements
                    .push(located(Statement::StorageLive(LocalId(1))));
            }
            f.blocks[0]
                .statements
                .push(located(Statement::Inspect(place(1))));
            let result = analyze(&f);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::ReadNotDefinitelyInitialized
            );
        }
    }

    #[test]
    fn projected_array_inspection_does_not_read_unrelated_aggregate_siblings() {
        let mut f = fixture();
        f.locals.extend([
            local(Ty::Tuple(vec![array(Ty::I32, 2), Ty::I32].into())),
            local(Ty::I32),
        ]);
        f.params.push(LocalId(1));
        let field = |index| Place {
            local: LocalId(1),
            projection: vec![Projection::Field(index)],
        };
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(field(1))),
            }),
            located(Statement::Inspect(field(0))),
        ]);
        assert!(analyze(&f).passed());
        f.blocks[0]
            .statements
            .push(located(Statement::Inspect(place(1))));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn indexed_assignment_requires_mutable_initialized_ancestors() {
        for (mutable, initialized, expected) in [
            (true, true, None),
            (false, true, Some(ErrorKind::ImmutableReassignment)),
            (
                true,
                false,
                Some(ErrorKind::WriteIntoUninitializedAggregate),
            ),
        ] {
            let mut f = fixture();
            f.locals.push(local(array(Ty::I32, 2)));
            f.locals[1].mutable = mutable;
            if initialized {
                f.params.push(LocalId(1));
            } else {
                f.blocks[0]
                    .statements
                    .push(located(Statement::StorageLive(LocalId(1))));
            }
            f.blocks[0].statements.push(located(Statement::Assign {
                destination: indexed(1, 0),
                value: Rvalue::Use(if initialized {
                    Operand::Copy(indexed(1, 1))
                } else {
                    Operand::Constant(Constant::I32(4))
                }),
            }));
            let result = analyze(&f);
            if let Some(expected) = expected {
                assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
                assert_eq!(result.diagnostics[0].kind, expected);
            } else {
                assert!(result.passed(), "{:?}", result.diagnostics);
            }
        }
    }

    #[test]
    fn array_element_variant_proofs_are_independent_and_inspection_preserves_them() {
        let ty = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        let mut f = fixture();
        f.locals
            .extend([local(array(Ty::Enum(ty.clone()), 2)), local(Ty::I32)]);
        f.params.push(LocalId(1));
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(2))));
        split_payload(&mut f, &ty);
        f.blocks[0].terminator = located(Terminator::SwitchVariant {
            place: indexed(1, 0),
            variant: variant(&ty, 0),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        let field = |index| {
            let mut place = indexed(1, index);
            place
                .projection
                .extend([Projection::Downcast(variant(&ty, 0)), Projection::Field(0)]);
            place
        };
        f.blocks[1].statements.extend([
            located(Statement::Inspect(place(1))),
            read_payload(field(0)),
            located(Statement::Assign {
                destination: indexed(1, 1),
                value: Rvalue::EnumVariant {
                    ty: ty.clone(),
                    variant: 1,
                    fields: vec![Operand::Constant(Constant::I32(9))],
                },
            }),
            read_payload(field(0)),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.variant_proofs_killed, 0);
        f.blocks[1].statements.push(read_payload(field(1)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
        f.blocks[1].statements.pop();
        f.blocks[1].statements.extend([
            located(Statement::Assign {
                destination: indexed(1, 0),
                value: Rvalue::EnumVariant {
                    ty: ty.clone(),
                    variant: 1,
                    fields: vec![Operand::Constant(Constant::I32(8))],
                },
            }),
            read_payload(field(0)),
        ]);
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
        assert_eq!(result.stats.variant_proofs_killed, 1);
    }

    #[test]
    fn array_preflight_bounds_expansion_and_checks_zero_element_types() {
        let ty = array(Ty::I32, super::super::ir::MAX_ARRAY_LEN);
        assert!(preflight_ty(&ty, 0, &mut Budget(8192)).is_err());
        assert!(preflight_ty(&ty, 0, &mut Budget(8193)).is_ok());
        assert!(
            preflight_ty(
                &array(Ty::I32, super::super::ir::MAX_ARRAY_LEN + 1),
                0,
                &mut Budget(1_000_000)
            )
            .is_err()
        );
        let mut huge = Ty::I32;
        for _ in 0..8 {
            huge = array(huge, 16);
        }
        let mut f = fixture();
        f.locals.push(local(huge));
        let result = analyze(&f);
        assert!(!result.complete);
        assert_eq!(result.stats.move_paths, 0);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        let mut deep = Ty::I32;
        for _ in 0..130 {
            deep = array(deep, 0);
        }
        assert!(preflight_ty(&deep, 0, &mut Budget(1_000_000)).is_err());
    }

    #[test]
    fn payload_downcast_requires_successful_edge_proof() {
        let ty = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        let mut f = payload_fixture(&ty);
        f.blocks[0]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );

        let mut f = payload_fixture(&ty);
        split_payload(&mut f, &ty);
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        assert!(analyze(&f).passed());
        f.blocks[2]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(result.diagnostics[0].block, BlockId(2));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
    }

    #[test]
    fn payload_proof_intersects_for_shared_targets_and_joins() {
        let ty = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        for shared_target in [false, true] {
            let mut f = payload_fixture(&ty);
            split_payload(&mut f, &ty);
            f.blocks.push(BasicBlock {
                statements: vec![read_payload(payload_field(&ty, 0, 0))],
                terminator: located(Terminator::Return),
            });
            if shared_target {
                f.blocks[0].terminator = located(Terminator::SwitchVariant {
                    place: place(1),
                    variant: variant(&ty, 0),
                    then_block: BlockId(3),
                    else_block: BlockId(3),
                });
            } else {
                f.blocks[1].terminator = located(Terminator::Goto(BlockId(3)));
                f.blocks[2].terminator = located(Terminator::Goto(BlockId(3)));
            }
            let result = analyze(&f);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(
                result.diagnostics[0].kind,
                ErrorKind::VariantNotDefinitelySelected
            );
        }
    }

    #[test]
    fn whole_enum_assignment_kills_stale_variant_proof() {
        let ty = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        let mut f = payload_fixture(&ty);
        split_payload(&mut f, &ty);
        f.blocks[1].statements.extend([
            located(Statement::Assign {
                destination: place(1),
                value: Rvalue::EnumVariant {
                    ty: ty.clone(),
                    variant: 1,
                    fields: vec![Operand::Constant(Constant::I32(9))],
                },
            }),
            read_payload(payload_field(&ty, 0, 0)),
        ]);
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
        assert_eq!(result.stats.variant_proofs_killed, 1);
        assert_eq!(result.stats.variant_proof_checks, 1);
        assert_eq!(result.stats.variant_refinements, 1);
    }

    #[test]
    fn payload_destination_requires_proof_even_when_inactive_bits_are_true() {
        let ty = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        let mut f = payload_fixture(&ty);
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: payload_field(&ty, 0, 0),
            value: Rvalue::Use(Operand::Constant(Constant::I32(3))),
        }));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::VariantNotDefinitelySelected
        );
    }

    #[test]
    fn singleton_switch_does_not_read_partially_moved_or_uninitialized_root() {
        let ty = payload_enum(vec![vec![Ty::I32, Ty::I32]]);
        let mut f = payload_fixture(&ty);
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(2),
            value: Rvalue::Use(Operand::Move(payload_field(&ty, 0, 0))),
        }));
        split_payload(&mut f, &ty);
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 1)));
        assert!(analyze(&f).passed(), "{:?}", analyze(&f).diagnostics);
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );

        let mut f = payload_fixture(&ty);
        f.params.clear();
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(1))));
        split_payload(&mut f, &ty);
        assert!(analyze(&f).passed());
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn selected_variant_refinement_never_restores_initialization() {
        let ty = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        let mut f = payload_fixture(&ty);
        f.params.clear();
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(1))));
        split_payload(&mut f, &ty);
        f.blocks[1]
            .statements
            .push(read_payload(payload_field(&ty, 0, 0)));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|d| d.kind == ErrorKind::ReadNotDefinitelyInitialized)
        );
    }

    #[test]
    fn payload_move_retains_outer_proof_but_full_enum_read_fails() {
        let ty = payload_enum(vec![vec![Ty::I32, Ty::I32], vec![Ty::I32]]);
        let mut f = payload_fixture(&ty);
        split_payload(&mut f, &ty);
        f.blocks[1].statements.extend([
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Move(payload_field(&ty, 0, 0))),
            }),
            read_payload(payload_field(&ty, 0, 1)),
        ]);
        assert!(analyze(&f).passed());
        f.blocks[1].statements.push(discriminant());
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn nested_proof_invalidation_is_subtree_scoped() {
        let inner = payload_enum(vec![vec![Ty::I32], vec![Ty::I32]]);
        let outer = payload_enum(vec![vec![Ty::Enum(inner), Ty::I32], vec![Ty::I32]]);
        let f = payload_fixture(&outer);
        let paths = Paths::new(&f, 100).unwrap();
        let outer_node = paths.resolve(&place(1));
        let inner_node = paths.resolve(&payload_field(&outer, 0, 0));
        let sibling = paths.resolve(&payload_field(&outer, 0, 1));
        let mut state = State::new(f.locals.len(), paths.nodes.len());
        state.selected[outer_node] = 0;
        state.selected[inner_node] = 0;
        let mut budget = Budget(1000);
        let mut transfer = Transfer {
            f: &f,
            paths: &paths,
            budget: &mut budget,
            report: None,
            block: BlockId(0),
            statement: None,
            origin: None,
            diagnostic_limit: 10,
            loans: None,
            phase: Phase::Read,
        };
        transfer.initialize(&mut state, sibling, true).unwrap();
        assert_eq!(state.selected[outer_node], 0);
        assert_eq!(state.selected[inner_node], 0);
        transfer.initialize(&mut state, inner_node, false).unwrap();
        assert_eq!(state.selected[outer_node], 0);
        assert_eq!(state.selected[inner_node], usize::MAX);
        state.selected[inner_node] = 0;
        transfer.initialize(&mut state, outer_node, true).unwrap();
        assert_eq!(state.selected[outer_node], usize::MAX);
        assert_eq!(state.selected[inner_node], usize::MAX);
    }

    #[test]
    fn payload_metadata_and_proof_storage_are_resource_bounded() {
        let ty = payload_enum(vec![vec![Ty::I32; 10], vec![Ty::I32; 10]]);
        assert!(preflight_ty(&Ty::Enum(ty.clone()), 0, &mut Budget(40)).is_err());
        let f = payload_fixture(&ty);
        let paths = Paths::new(&f, 23).unwrap();
        assert_eq!(paths.nodes.len(), 23);
        assert!(Paths::new(&f, 22).is_err());
        let old_words =
            Bits::new(f.locals.len()).0.len() * 2 + Bits::new(paths.nodes.len()).0.len() * 3;
        let old_state_bytes = old_words * 8 + std::mem::size_of::<State>();
        let result = analyze_with_limits(
            &f,
            Limits {
                state_bytes: old_state_bytes * (f.blocks.len() + 3),
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert_eq!(result.stats.worklist_pops, 0);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
    }

    #[test]
    fn enum_discriminant_reads_do_not_consume_noncopy_root() {
        let mut f = enum_fixture();
        f.blocks[0].statements.extend([
            discriminant(),
            discriminant(),
            located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(place(1))),
            }),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
        assert_eq!(result.stats.moves, 1);
        assert_eq!(result.stats.move_paths, 4);
    }

    #[test]
    fn enum_discriminant_requires_live_initialized_root() {
        for (setup, expected) in [
            (
                Statement::Assign {
                    destination: place(3),
                    value: Rvalue::Use(Operand::Move(place(1))),
                },
                ErrorKind::ReadNotDefinitelyInitialized,
            ),
            (
                Statement::StorageDead(LocalId(1)),
                ErrorKind::StorageNotDefinitelyLive,
            ),
        ] {
            let mut f = enum_fixture();
            f.blocks[0].statements.push(located(setup));
            f.blocks[0].statements.push(discriminant());
            let result = analyze(&f);
            assert!(result.complete);
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(result.diagnostics[0].kind, expected);
            assert_eq!(result.diagnostics[0].place, Some(place(1)));
        }

        let mut f = enum_fixture();
        f.params.clear();
        f.blocks[0]
            .statements
            .push(located(Statement::StorageLive(LocalId(1))));
        f.blocks[0].statements.push(discriminant());
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn unreachable_discriminant_does_not_read_moved_enum() {
        let mut f = enum_fixture();
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(3),
            value: Rvalue::Use(Operand::Move(place(1))),
        }));
        f.blocks.push(BasicBlock {
            statements: vec![discriminant()],
            terminator: located(Terminator::Unreachable),
        });
        assert!(analyze(&f).passed());
    }

    #[test]
    fn enum_metadata_is_budgeted_without_adding_child_paths() {
        let ty = Ty::Enum(unit_enum());
        assert!(preflight_ty(&ty, 0, &mut Budget(2)).is_err());
        assert!(preflight_ty(&ty, 0, &mut Budget(3)).is_ok());
        let f = enum_fixture();
        let paths = Paths::new(&f, 4).unwrap();
        assert!(paths.nodes[paths.roots[1]].children.is_empty());
    }

    #[test]
    fn constant_switch_false_edge_still_checks_moved_enum_reads() {
        let mut f = enum_fixture();
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(3),
            value: Rvalue::Use(Operand::Move(place(1))),
        }));
        f.blocks[0].terminator = located(Terminator::Switch {
            condition: Operand::Constant(Constant::Bool(true)),
            then_block: BlockId(1),
            else_block: BlockId(2),
        });
        f.blocks.extend([
            BasicBlock {
                statements: vec![],
                terminator: located(Terminator::Return),
            },
            BasicBlock {
                statements: vec![discriminant()],
                terminator: located(Terminator::Return),
            },
        ]);
        let result = analyze(&f);
        assert!(result.complete);
        assert_eq!(result.stats.blocks_reachable, 3);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].block, BlockId(2));
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
    }

    #[test]
    fn enum_constant_initializes_root_for_discriminant() {
        let mut f = enum_fixture();
        f.params.clear();
        let Ty::Enum(ty) = &f.locals[1].ty else {
            unreachable!()
        };
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(1))),
            located(Statement::Assign {
                destination: place(1),
                value: Rvalue::Use(Operand::Constant(Constant::Enum {
                    ty: ty.clone(),
                    variant: 0,
                })),
            }),
            discriminant(),
        ]);
        let result = analyze(&f);
        assert!(result.passed(), "{:?}", result.diagnostics);
    }

    #[test]
    fn projected_enum_discriminant_checks_its_own_initialization() {
        let mut f = fixture();
        let ty = Ty::Enum(unit_enum());
        f.locals.extend([
            local(Ty::Tuple(vec![ty.clone(), ty.clone()].into())),
            local(Ty::I32),
            local(ty),
        ]);
        f.params.push(LocalId(1));
        let field = |index| Place {
            local: LocalId(1),
            projection: vec![Projection::Field(index)],
        };
        f.blocks[0].statements.extend([
            located(Statement::StorageLive(LocalId(2))),
            located(Statement::StorageLive(LocalId(3))),
            located(Statement::Assign {
                destination: place(3),
                value: Rvalue::Use(Operand::Move(field(0))),
            }),
            located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Discriminant(field(1)),
            }),
        ]);
        assert!(analyze(&f).passed());
        f.blocks[0].statements.push(located(Statement::Assign {
            destination: place(2),
            value: Rvalue::Discriminant(field(0)),
        }));
        let result = analyze(&f);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0].kind,
            ErrorKind::ReadNotDefinitelyInitialized
        );
        assert_eq!(result.diagnostics[0].place, Some(field(0)));
    }

    #[test]
    fn bit_ranges_cover_empty_intervals_and_word_boundaries() {
        for start in 0..=131 {
            for end in start..=131 {
                let mut bits = Bits::new(131);
                let mut budget = Budget(1000);
                bits.range(start, end, true, &mut budget).unwrap();
                for index in 0..131 {
                    assert_eq!(bits.get(index), (start..end).contains(&index));
                }
                assert!(bits.all(start, end, &mut budget).unwrap());
                if start < end {
                    bits.set(end - 1, false);
                    assert!(!bits.all(start, end, &mut budget).unwrap());
                }
                bits.range(start, end, false, &mut budget).unwrap();
                assert!((0..131).all(|index| !bits.get(index)));
            }
        }
    }

    #[test]
    fn every_private_resource_cap_fails_closed() {
        let f = fixture();
        assert!(analyze(&f).passed());
        for limits in [
            Limits {
                blocks: 0,
                ..Limits::default()
            },
            Limits {
                locals: 0,
                ..Limits::default()
            },
            Limits {
                paths: 0,
                ..Limits::default()
            },
            Limits {
                operations: 0,
                ..Limits::default()
            },
            Limits {
                state_bytes: 0,
                ..Limits::default()
            },
            Limits {
                work: 0,
                ..Limits::default()
            },
        ] {
            let result = analyze_with_limits(&f, limits);
            assert!(!result.passed());
            assert!(!result.complete);
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == ErrorKind::ResourceLimit)
            );
        }
        assert!(
            analyze_with_limits(
                &f,
                Limits {
                    diagnostics: 0,
                    ..Limits::default()
                }
            )
            .passed()
        );
    }

    #[test]
    fn diagnostic_truncation_is_explicit_and_never_success() {
        let mut f = fixture();
        f.locals.push(local(Ty::Unit));
        f.blocks[0].statements = vec![
            located(Statement::StorageLive(LocalId(0))),
            located(Statement::StorageLive(LocalId(1))),
        ];
        for _ in 0..5 {
            f.blocks[0].statements.push(located(Statement::Assign {
                destination: place(0),
                value: Rvalue::Use(Operand::Copy(place(1))),
            }));
        }
        let result = analyze_with_limits(
            &f,
            Limits {
                diagnostics: 2,
                ..Limits::default()
            },
        );
        assert!(!result.complete);
        assert!(!result.passed());
        assert_eq!(result.diagnostics.len(), 2);
        assert_eq!(result.diagnostics[0].statement, Some(2));
        assert_eq!(result.diagnostics[1].kind, ErrorKind::ResourceLimit);
        assert!(result.stats.diagnostics_suppressed >= 3);
        let zero = analyze_with_limits(
            &f,
            Limits {
                diagnostics: 0,
                ..Limits::default()
            },
        );
        assert!(!zero.complete);
        assert_eq!(zero.diagnostics.len(), 1);
        assert_eq!(zero.diagnostics[0].kind, ErrorKind::ResourceLimit);
    }

    #[test]
    fn diagnostic_replay_exhaustion_cannot_report_success() {
        // Search a small fixed bound rather than pinning an internal operation
        // count. Extra proof accounting must not turn this into a solver-only
        // exhaustion test or let replay exhaustion masquerade as success.
        let result = (0..=256)
            .map(|work| {
                analyze_with_limits(
                    &fixture(),
                    Limits {
                        work,
                        ..Limits::default()
                    },
                )
            })
            .find(|result| {
                !result.complete && result.stats.worklist_pops == 1 && result.stats.writes == 1
            })
            .expect("bounded work budget must exercise mid-replay exhaustion");
        assert!(!result.complete);
        assert!(!result.passed());
        assert_eq!(result.stats.worklist_pops, 1);
        assert_eq!(
            result.stats.writes, 1,
            "final diagnostic replay must have begun"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.kind == ErrorKind::ResourceLimit)
        );
    }

    #[test]
    fn repeated_wide_type_comparisons_are_bounded_before_validation() {
        let mut f = fixture();
        // Distinct allocations defeat pointer-equality shortcuts in Eq.
        let first = Ty::Tuple(vec![Ty::I32; 20_000].into());
        let second = Ty::Tuple(vec![Ty::I32; 20_000].into());
        f.locals.extend([local(first), local(second)]);
        f.params.push(LocalId(1));
        f.blocks[0].statements = vec![located(Statement::StorageLive(LocalId(2)))];
        for _ in 0..64 {
            f.blocks[0].statements.push(located(Statement::Assign {
                destination: place(2),
                value: Rvalue::Use(Operand::Copy(place(1))),
            }));
        }
        f.blocks[0].terminator = located(Terminator::Unreachable);
        let result = analyze(&f);
        assert!(!result.complete);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        assert_eq!(result.stats.move_paths, 0);
        assert_eq!(result.stats.worklist_pops, 0);
    }

    #[test]
    fn nested_coercion_preflight_charges_revisited_subtrees() {
        let mut ty = Ty::I32;
        for _ in 0..64 {
            ty = Ty::Tuple(vec![ty].into());
        }
        assert!(preflight_ty(&ty, 0, &mut Budget(1000)).is_err());
        assert!(preflight_ty(&ty, 0, &mut Budget(3000)).is_ok());
    }

    #[test]
    fn nominal_fields_get_distinct_nested_move_paths() {
        let inner = nominal(vec![Ty::Unit, Ty::Bool]);
        let outer = nominal(vec![Ty::I32, Ty::Tuple(vec![inner.clone(), inner].into())]);
        let mut f = fixture();
        f.locals.push(local(outer.clone()));
        let paths = Paths::new(&f, 10).unwrap();
        assert_eq!(paths.nodes.len(), 10);
        assert!(Paths::new(&f, 9).is_err());
        let projected = |fields: Vec<usize>| Place {
            local: LocalId(1),
            projection: fields.into_iter().map(Projection::Field).collect(),
        };
        assert_eq!(paths.resolve(&projected(vec![1, 0, 0])), 5);
        assert_eq!(paths.resolve(&projected(vec![1, 1, 0])), 8);
        assert_eq!(paths.nodes[8].parent, Some(7));
        assert_eq!(paths.nodes[7].end, 10);
        assert!(preflight_ty(&outer, 0, &mut Budget(15)).is_err());
        assert!(preflight_ty(&outer, 0, &mut Budget(30)).is_ok());
    }

    #[test]
    fn shared_nominal_dag_expansion_is_bounded_before_paths() {
        let mut ty = Ty::I32;
        for _ in 0..20 {
            ty = nominal(vec![ty.clone(), ty]);
        }
        let mut f = fixture();
        f.locals.push(local(ty));
        let result = analyze(&f);
        assert!(!result.complete);
        assert_eq!(result.diagnostics[0].kind, ErrorKind::ResourceLimit);
        assert_eq!(result.stats.move_paths, 0);
        assert_eq!(result.stats.worklist_pops, 0);
    }
}
