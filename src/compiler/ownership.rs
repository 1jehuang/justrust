//! Reachable-CFG storage and definite-initialization analysis.
//!
//! This is not borrow checking, Drop elaboration, or permission to admit new
//! source types. Private `Move` operands consume even Copy-typed places so that
//! move-path analysis can be tested before non-Copy syntax is supported.
use super::cfg::{self, BlockId, Operand, Rvalue, Statement, Terminator};
use super::ir::{ExprId, LocalId, Place, Ty};
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidCfg(String),
    StorageAlreadyLive,
    StorageNotDefinitelyLive,
    ReadNotDefinitelyInitialized,
    WriteIntoUninitializedAggregate,
    ImmutableReassignment,
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
struct Exhausted;
struct Budget(usize);
impl Budget {
    fn spend(&mut self, amount: usize) -> Result<(), Exhausted> {
        self.0 = self.0.checked_sub(amount).ok_or(Exhausted)?;
        Ok(())
    }
}

/// Bound type traversal before calling the full structural verifier, including
/// repeated shared aggregate types that could otherwise expand exponentially.
fn preflight(f: &cfg::Function, limits: Limits) -> Result<(), Exhausted> {
    if f.blocks.len() > limits.blocks || f.locals.len() > limits.locals {
        return Err(Exhausted);
    }
    fn place_type(f: &cfg::Function, place: &Place, budget: &mut Budget) -> Result<(), Exhausted> {
        budget.spend(place.fields.len() + 1)?;
        // Invalid references are diagnosed by the verifier after this bounded
        // preflight, rather than indexing untrusted input here.
        let Some(local) = f.locals.get(place.local.0) else {
            return Ok(());
        };
        let mut ty = &local.ty;
        for &index in &place.fields {
            let Some(field) = ty.field_ty(index) else {
                return Ok(());
            };
            ty = field;
        }
        preflight_ty(ty, 0, budget)
    }
    let mut budget = Budget(limits.operations);
    budget.spend(f.params.len())?;
    for local in &f.locals {
        preflight_ty(&local.ty, 0, &mut budget)?;
    }
    preflight_ty(&f.result, 0, &mut budget)?;
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
            if let Statement::Assign { destination, value } = &item.value {
                place_type(f, destination, &mut budget)?;
                match value {
                    Rvalue::Discriminant(place) => place_type(f, place, &mut budget)?,
                    Rvalue::Use(op) | Rvalue::Unary { value: op, .. } => operand(op, &mut budget)?,
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
                    Rvalue::Println(_) => {}
                }
            }
        }
        match &block.terminator.value {
            Terminator::Switch { condition, .. } => operand(condition, &mut budget)?,
            Terminator::Call {
                signature,
                args,
                destination,
                ..
            } => {
                for param in &signature.params {
                    preflight_ty(param, 0, &mut budget)?;
                }
                preflight_ty(&signature.result, 0, &mut budget)?;
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
    if let Ty::Enum(ty) = ty {
        // Enums have no move-path children, but validation examines their
        // variant metadata, including when supplied only by a constant.
        budget.spend(ty.def().variants.len())?;
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
    /// Preorder interval [self, end) covers this place's entire subtree.
    end: usize,
}
struct Paths {
    roots: Vec<usize>,
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
                end: 0,
            });
            if let Some(count) = ty.field_count() {
                for index in 0..count {
                    let child = add(paths, ty.field_ty(index).ok_or(Exhausted)?, Some(id), limit)?;
                    paths.nodes[id].children.push(child);
                }
            }
            paths.nodes[id].end = paths.nodes.len();
            Ok(id)
        }
        let mut result = Self {
            roots: Vec::new(),
            nodes: Vec::new(),
        };
        for local in &f.locals {
            let root = add(&mut result, &local.ty, None, limit)?;
            result.roots.push(root);
        }
        Ok(result)
    }
    fn resolve(&self, place: &Place) -> usize {
        let mut node = self.roots[place.local.0];
        for field in &place.fields {
            node = self.nodes[node].children[*field];
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
}
impl State {
    fn new(locals: usize, paths: usize) -> Self {
        Self {
            must_live: Bits::new(locals),
            may_live: Bits::new(locals),
            must_init: Bits::new(paths),
            may_init: Bits::new(paths),
            ever_written: Bits::new(paths),
        }
    }
    fn words(&self) -> usize {
        self.must_live.0.len() * 2 + self.must_init.0.len() * 3
    }
    fn merge(&mut self, other: &Self, budget: &mut Budget) -> Result<bool, Exhausted> {
        let mut changed = self.must_live.merge(&other.must_live, true, budget)?;
        changed |= self.may_live.merge(&other.may_live, false, budget)?;
        changed |= self.must_init.merge(&other.must_init, true, budget)?;
        changed |= self.may_init.merge(&other.may_init, false, budget)?;
        changed |= self
            .ever_written
            .merge(&other.ever_written, false, budget)?;
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
}
struct Report {
    stats: AnalysisStats,
    diagnostics: Vec<Diagnostic>,
    complete: bool,
}
impl Transfer<'_> {
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
    fn initialize(&mut self, state: &mut State, node: usize, value: bool) -> Result<(), Exhausted> {
        let end = self.paths.nodes[node].end;
        state.must_init.range(node, end, value, self.budget)?;
        state.may_init.range(node, end, value, self.budget)
    }
    fn read(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if !state.must_live.get(place.local.0) {
            self.diagnostic(ErrorKind::StorageNotDefinitelyLive, place.clone());
        } else {
            let node = self.paths.resolve(place);
            if !self.ancestors(state, node)?
                || !state
                    .must_init
                    .all(node, self.paths.nodes[node].end, self.budget)?
            {
                self.diagnostic(ErrorKind::ReadNotDefinitelyInitialized, place.clone());
            }
        }
        Ok(())
    }
    fn operand(&mut self, state: &mut State, op: &Operand) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.operands_read += 1;
        }
        if let Operand::Copy(place) | Operand::Move(place) = op {
            self.read(state, place)?;
            if matches!(op, Operand::Move(_)) {
                if let Some(report) = &mut self.report {
                    report.stats.moves += 1;
                }
                self.initialize(state, self.paths.resolve(place), false)?;
            }
        }
        Ok(())
    }
    fn write(&mut self, state: &mut State, place: &Place) -> Result<(), Exhausted> {
        self.budget.spend(1)?;
        if let Some(report) = &mut self.report {
            report.stats.writes += 1;
        }
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
    fn rvalue(&mut self, state: &mut State, value: &Rvalue) -> Result<(), Exhausted> {
        match value {
            // Reading a tag does not copy or move the non-Copy enum. Unit
            // enums still require their root to be definitely initialized.
            Rvalue::Discriminant(place) => self.read(state, place)?,
            Rvalue::Use(op) | Rvalue::Unary { value: op, .. } => self.operand(state, op)?,
            Rvalue::Binary { left, right, .. } => {
                self.operand(state, left)?;
                self.operand(state, right)?;
            }
            Rvalue::Aggregate { fields, .. } => {
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
            match &item.value {
                Statement::StorageLive(local) => {
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
                    self.write(state, destination)?;
                }
            }
        }
        self.statement = None;
        self.origin = self.f.blocks[self.block.0].terminator.origin;
        match &self.f.blocks[self.block.0].terminator.value {
            Terminator::Switch { condition, .. } => self.operand(state, condition)?,
            Terminator::Call {
                args, destination, ..
            } => {
                for arg in args {
                    self.operand(state, arg)?;
                }
                self.write(state, destination)?;
            }
            Terminator::Return => {
                self.operand(state, &Operand::Copy(root_place(self.f.return_local)))?
            }
            Terminator::Goto(_) | Terminator::Unreachable => {}
        }
        Ok(())
    }
}
fn root_place(local: LocalId) -> Place {
    Place {
        local,
        fields: Vec::new(),
    }
}
fn successors(terminator: &Terminator) -> [Option<BlockId>; 2] {
    match terminator {
        Terminator::Goto(next) | Terminator::Call { next, .. } => [Some(*next), None],
        Terminator::Switch {
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
            .ok_or(Exhausted)?;
        if bytes > limits.state_bytes {
            return Err(Exhausted);
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
            }
            .block(&mut output)?;
            for target in successors(&f.blocks[block.0].terminator.value)
                .into_iter()
                .flatten()
            {
                let changed = if let Some(entry) = &mut entries[target.0] {
                    entry.merge(&output, &mut budget)?
                } else {
                    budget.spend(output.words())?;
                    entries[target.0] = Some(output.clone());
                    report.stats.blocks_reachable += 1;
                    true
                };
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
                }
                .block(&mut state)?;
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
        DefId, EnumDef, EnumTy, FieldDef, LocalDecl, SnapshotId, StructDef, StructTy,
    };
    use super::*;
    use cfg::{BasicBlock, Constant, Located};

    fn located<T>(value: T) -> Located<T> {
        Located {
            value,
            origin: None,
        }
    }
    fn place(local: usize) -> Place {
        Place {
            local: LocalId(local),
            fields: vec![],
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
            name: "caps".into(),
            params: vec![],
            result: Ty::Unit,
            locals: vec![local(Ty::Unit)],
            entry: BlockId(0),
            return_local: LocalId(0),
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
            variants: vec!["A".into(), "B".into()].into(),
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
            fields: vec![index],
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
        let result = analyze_with_limits(
            &fixture(),
            Limits {
                work: 31,
                ..Limits::default()
            },
        );
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
        let projected = |fields| Place {
            local: LocalId(1),
            fields,
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
