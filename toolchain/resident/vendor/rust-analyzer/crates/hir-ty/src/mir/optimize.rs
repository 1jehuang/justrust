//! rustc's MIR optimizations, run on a [`MirPhase::Runtime`] body as rustc's
//! `run_optimization_passes` runs them on its runtime MIR (`rustc_mir_transform` at 1.99.0,
//! `compiler/rustc_mir_transform/src/lib.rs`). They are optional: the IDE and the interpreter
//! never ask for them, and the code generator asks only when told to (`RA_JIT_MIR_OPT`).
//!
//! The level is rustc's `-Zmir-opt-level`: [`MirOptLevel::Debug`] is what rustc runs at
//! `opt-level=0` (its level 1) and [`MirOptLevel::Release`] what it runs otherwise (its
//! level 2). Each pass is a port of rustc's pass of the same name; where it does less than
//! rustc's, its module says what. Passes rustc runs at a level and these do not are listed in
//! [`passes_not_ported`].
//!
//! The passes run on the generic body, as rustc's do, once per function: monomorphization
//! substitutes after. An edit to one function optimizes that one again.
//!
//! [`MirPhase::Runtime`]: super::MirPhase::Runtime

mod cfg;
mod copy_prop;
mod dse;
mod inst_simplify;
mod locals;

use la_arena::{Arena, ArenaMap, Idx, RawIdx};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use std::cell::Cell;

use crate::{
    InferBodyId,
    db::{HirDatabase, InternedClosureId},
    traits::StoredParamEnvAndCrate,
};

use crate::next_solver::StoredGenericArgs;

use super::{
    BasicBlock, BasicBlockId, LocalId, MirBody, MirLowerError, OperandKind, PlaceElem,
    ProjectionElem, Rvalue, StatementKind, StoredPlace, TerminatorKind,
    monomorphization::monomorphized, transform::Ctx,
};

type Result<'db, T> = std::result::Result<T, MirLowerError<'db>>;

/// How much of rustc's optimization pipeline runs: rustc's `-Zmir-opt-level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, salsa::SalsaValue)]
pub enum MirOptLevel {
    /// None: the body is the runtime body.
    Off,
    /// rustc's level 1, what a build at `opt-level=0` gets.
    Debug,
    /// rustc's level 2, what a build at any other `opt-level` gets.
    Release,
}

impl MirOptLevel {
    pub fn from_number(level: u8) -> Option<MirOptLevel> {
        match level {
            0 => Some(MirOptLevel::Off),
            1 => Some(MirOptLevel::Debug),
            2 => Some(MirOptLevel::Release),
            _ => None,
        }
    }

    pub fn number(self) -> u8 {
        match self {
            MirOptLevel::Off => 0,
            MirOptLevel::Debug => 1,
            MirOptLevel::Release => 2,
        }
    }
}

/// The passes of rustc's pipeline at `level` that this module does not run, in rustc's order.
/// `harness/mirparity.py` prints them next to what it measured.
pub fn passes_not_ported(level: MirOptLevel) -> &'static [&'static str] {
    match level {
        MirOptLevel::Off => &[],
        MirOptLevel::Debug => &[
            "CheckAlignment",
            "CheckNull",
            "CheckEnums",
            "ForceInline",
            "RemoveZsts",
            "UnreachableEnumBranching",
        ],
        MirOptLevel::Release => &[
            "CheckAlignment",
            "CheckNull",
            "CheckEnums",
            "ForceInline",
            "Inline",
            "RemoveZsts",
            "UnreachableEnumBranching",
            "ReferencePropagation",
            "ScalarReplacementOfAggregates",
            "GVN",
            "SsaRangePropagation",
            "MatchBranchSimplification",
            "JumpThreading",
            "EarlyOtherwiseBranch",
            "SimplifyComparisonIntegral",
            "DestinationPropagation",
        ],
    }
}

/// `body` after rustc's optimizations at `level`. Coroutine bodies are left as they are: what
/// a future keeps across a yield is computed from the runtime body.
pub(super) fn optimize<'db>(
    db: &'db dyn HirDatabase,
    body: &MirBody<'db>,
    level: MirOptLevel,
) -> MirBody<'db> {
    let mut body = body.clone();
    if level == MirOptLevel::Off || body.coroutine.is_some() {
        return body;
    }
    let ctx = Ctx::new(db, body.owner);
    let release = level >= MirOptLevel::Release;
    // Each pass, and in a debug build a check that it left a body the next can read.
    macro_rules! run {
        ($pass:literal, $($run:tt)*) => {{
            $($run)*;
            #[cfg(debug_assertions)]
            if let Err(error) = validate(&body) {
                panic!("MIR of {:?} after {}: {error}", body.owner, $pass);
            }
        }};
    }
    // rustc's runtime cleanup, which its runtime MIR has had before any of these:
    // `CleanupPostBorrowck` and `SimplifyCfg-pre-optimizations`.
    run!("CleanupPostBorrowck", cfg::cleanup_post_borrowck(&mut body));
    run!("SimplifyCfg", cfg::simplify_cfg(&mut body));

    run!("LowerSliceLenCalls", inst_simplify::lower_slice_len_calls(&ctx, &mut body));
    run!("InstSimplify", inst_simplify::inst_simplify(&ctx, &mut body));
    // rustc keeps storage markers only where it gives LLVM lifetimes: at `opt-level > 0`.
    if !release {
        run!("RemoveStorageMarkers", locals::remove_storage_markers(&mut body));
    }
    run!("RemoveUnneededDrops", cfg::remove_unneeded_drops(&ctx, &mut body));
    if release {
        run!("UnreachablePropagation", cfg::unreachable_propagation(&mut body));
    }
    run!("SimplifyCfg", cfg::simplify_cfg(&mut body));
    run!("InstSimplify", inst_simplify::inst_simplify(&ctx, &mut body));
    run!("SimplifyConstCondition", cfg::simplify_const_condition(&mut body));
    run!("SimplifyLocals", locals::simplify_locals(&mut body));
    if release {
        run!("DeadStoreElimination", dse::dead_store_elimination(&mut body));
    }
    run!("SimplifyLocals", locals::simplify_locals(&mut body));
    run!("SingleUseConsts", inst_simplify::single_use_consts(&mut body));
    run!("SimplifyConstCondition", cfg::simplify_const_condition(&mut body));
    run!("SimplifyConstCondition", cfg::simplify_const_condition(&mut body));
    run!("RemoveNoopLandingPads", super::transform::remove_noop_landing_pads(&mut body));
    run!("SimplifyCfg", cfg::simplify_cfg(&mut body));
    run!("CopyProp", copy_prop::copy_prop(&ctx, &mut body, release));
    if release {
        run!("DeadStoreElimination", dse::dead_store_elimination(&mut body));
    }
    run!("SimplifyLocals", locals::simplify_locals(&mut body));
    run!("CriticalCallEdges", cfg::critical_call_edges(&mut body));
    body
}

#[salsa::tracked(returns(as_ref), cycle_result = optimized_mir_body_cycle_result)]
pub fn optimized_mir_body_query<'db>(
    db: &'db dyn HirDatabase,
    def: InferBodyId<'db>,
    level: MirOptLevel,
) -> Result<'db, MirBody<'db>> {
    Ok(optimize(db, db.runtime_mir_body(def)?, level))
}

#[salsa::tracked(returns(as_ref), cycle_result = optimized_mir_body_for_closure_cycle_result)]
pub fn optimized_mir_body_for_closure_query<'db>(
    db: &'db dyn HirDatabase,
    closure: InternedClosureId<'db>,
    level: MirOptLevel,
) -> Result<'db, MirBody<'db>> {
    Ok(optimize(db, db.runtime_mir_body_for_closure(closure)?, level))
}

#[salsa::tracked(returns(as_ref), cycle_result = monomorphized_optimized_mir_body_cycle_result)]
pub fn monomorphized_optimized_mir_body_query<'db>(
    db: &'db dyn HirDatabase,
    def: InferBodyId<'db>,
    level: MirOptLevel,
    subst: StoredGenericArgs,
    trait_env: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    monomorphized(db, db.optimized_mir_body(def, level)?, &subst, &trait_env)
}

#[salsa::tracked(
    returns(as_ref),
    cycle_result = monomorphized_optimized_mir_body_for_closure_cycle_result
)]
pub fn monomorphized_optimized_mir_body_for_closure_query<'db>(
    db: &'db dyn HirDatabase,
    closure: InternedClosureId<'db>,
    level: MirOptLevel,
    subst: StoredGenericArgs,
    trait_env: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    monomorphized(db, db.optimized_mir_body_for_closure(closure, level)?, &subst, &trait_env)
}

fn monomorphized_optimized_mir_body_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InferBodyId<'db>,
    _: MirOptLevel,
    _: StoredGenericArgs,
    _: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

fn monomorphized_optimized_mir_body_for_closure_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InternedClosureId<'db>,
    _: MirOptLevel,
    _: StoredGenericArgs,
    _: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

thread_local! {
    /// The level the interpreter runs functions and closures at on this thread, which is
    /// [`MirOptLevel::Off`] unless a test of these passes sets it.
    static INTERPRETER_LEVEL: Cell<MirOptLevel> = const { Cell::new(MirOptLevel::Off) };
}

#[doc(hidden)]
pub fn set_interpreter_mir_opt_level(level: MirOptLevel) {
    INTERPRETER_LEVEL.with(|it| it.set(level));
}

fn interpreter_level() -> MirOptLevel {
    INTERPRETER_LEVEL.with(Cell::get)
}

/// The body the interpreter runs of a function: the runtime body, or that body optimized at
/// the level [`set_interpreter_mir_opt_level`] set.
pub(super) fn interpreted_body<'db>(
    db: &'db dyn HirDatabase,
    def: InferBodyId<'db>,
    subst: StoredGenericArgs,
    env: StoredParamEnvAndCrate,
) -> Result<'db, &'db MirBody<'db>> {
    match interpreter_level() {
        MirOptLevel::Off => db.monomorphized_runtime_mir_body(def, subst, env),
        level => db.monomorphized_optimized_mir_body(def, level, subst, env),
    }
}

pub(super) fn interpreted_closure_body<'db>(
    db: &'db dyn HirDatabase,
    closure: InternedClosureId<'db>,
    subst: StoredGenericArgs,
    env: StoredParamEnvAndCrate,
) -> Result<'db, &'db MirBody<'db>> {
    match interpreter_level() {
        MirOptLevel::Off => db.monomorphized_runtime_mir_body_for_closure(closure, subst, env),
        level => db.monomorphized_optimized_mir_body_for_closure(closure, level, subst, env),
    }
}

fn optimized_mir_body_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InferBodyId<'db>,
    _: MirOptLevel,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

fn optimized_mir_body_for_closure_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InternedClosureId<'db>,
    _: MirOptLevel,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

/// What every pass leaves: each local and block a body names is one it has, and each block
/// ends in a terminator.
#[cfg(debug_assertions)]
fn validate(body: &MirBody<'_>) -> std::result::Result<(), String> {
    let locals = body.locals.len();
    let blocks = body.basic_blocks.len();
    let mut error = None;
    visit_body(body, |local, _, at| {
        if local_index(local) >= locals && error.is_none() {
            error = Some(format!(
                "local _{} of {locals} in bb{}",
                local_index(local),
                block_index(at.block)
            ));
        }
    });
    if let Some(error) = error {
        return Err(error);
    }
    let named = body.param_locals.iter().chain(body.binding_locals.values());
    let named = named.chain(body.promoted_locals.iter().map(|it| &it.0));
    if let Some(local) = named.into_iter().find(|it| local_index(**it) >= locals) {
        return Err(format!(
            "parameter, binding or promoted local _{} of {locals}",
            local_index(*local)
        ));
    }
    if block_index(body.start_block) >= blocks {
        return Err("start block".to_owned());
    }
    let reachable: rustc_hash::FxHashSet<BasicBlockId> =
        reverse_postorder(body).into_iter().collect();
    for (id, block) in body.basic_blocks.iter() {
        let Some(terminator) = &block.terminator else {
            if reachable.contains(&id) {
                return Err(format!("bb{} has no terminator", block_index(id)));
            }
            continue;
        };
        if let Some(target) =
            successors(&terminator.kind).into_iter().find(|it| block_index(*it) >= blocks)
        {
            return Err(format!(
                "bb{} goes to bb{} of {blocks}",
                block_index(id),
                block_index(target)
            ));
        }
    }
    Ok(())
}

/// Every block a terminator can go to, unwinding and coroutine drops included, as rustc's
/// `Terminator::successors_mut`.
fn successors_mut(kind: &mut TerminatorKind, mut f: impl FnMut(&mut BasicBlockId)) {
    match kind {
        TerminatorKind::Goto { target } => f(target),
        TerminatorKind::SwitchInt { targets, .. } => targets.targets.iter_mut().for_each(f),
        TerminatorKind::Drop { target, unwind, .. }
        | TerminatorKind::DropAndReplace { target, unwind, .. }
        | TerminatorKind::FalseUnwind { real_target: target, unwind } => {
            f(target);
            unwind.iter_mut().for_each(f);
        }
        TerminatorKind::Call { target, cleanup, .. } => {
            target.iter_mut().for_each(&mut f);
            cleanup.iter_mut().for_each(f);
        }
        TerminatorKind::Assert { target, cleanup, .. } => {
            f(target);
            cleanup.iter_mut().for_each(f);
        }
        TerminatorKind::Yield { resume, drop, .. } => {
            f(resume);
            drop.iter_mut().for_each(f);
        }
        TerminatorKind::FalseEdge { real_target, imaginary_target } => {
            f(real_target);
            f(imaginary_target);
        }
        TerminatorKind::UnwindResume
        | TerminatorKind::Abort
        | TerminatorKind::Return
        | TerminatorKind::Unreachable
        | TerminatorKind::CoroutineDrop => (),
    }
}

fn successors(kind: &TerminatorKind) -> SmallVec<[BasicBlockId; 4]> {
    let mut out = SmallVec::new();
    match kind {
        TerminatorKind::Goto { target } => out.push(*target),
        TerminatorKind::SwitchInt { targets, .. } => out.extend_from_slice(targets.all_targets()),
        TerminatorKind::Drop { target, unwind, .. }
        | TerminatorKind::DropAndReplace { target, unwind, .. }
        | TerminatorKind::FalseUnwind { real_target: target, unwind } => {
            out.push(*target);
            out.extend(*unwind);
        }
        TerminatorKind::Call { target, cleanup, .. } => {
            out.extend(*target);
            out.extend(*cleanup);
        }
        TerminatorKind::Assert { target, cleanup, .. } => {
            out.push(*target);
            out.extend(*cleanup);
        }
        TerminatorKind::Yield { resume, drop, .. } => {
            out.push(*resume);
            out.extend(*drop);
        }
        TerminatorKind::FalseEdge { real_target, imaginary_target } => {
            out.push(*real_target);
            out.push(*imaginary_target);
        }
        TerminatorKind::UnwindResume
        | TerminatorKind::Abort
        | TerminatorKind::Return
        | TerminatorKind::Unreachable
        | TerminatorKind::CoroutineDrop => (),
    }
    out
}

fn block_index(block: BasicBlockId) -> usize {
    block.into_raw().into_u32() as usize
}

fn block_id(index: usize) -> BasicBlockId {
    Idx::from_raw(RawIdx::from_u32(index as u32))
}

fn local_index(local: LocalId) -> usize {
    local.into_raw().into_u32() as usize
}

fn local_id(index: usize) -> LocalId {
    Idx::from_raw(RawIdx::from_u32(index as u32))
}

/// The blocks reachable from the start, in reverse postorder, unwinding included.
fn reverse_postorder(body: &MirBody<'_>) -> Vec<BasicBlockId> {
    let count = body.basic_blocks.len();
    let mut visited = vec![false; count];
    let mut post = Vec::with_capacity(count);
    let mut stack: Vec<(BasicBlockId, SmallVec<[BasicBlockId; 4]>, usize)> = Vec::new();
    let start = body.start_block;
    visited[block_index(start)] = true;
    let succ = |block: BasicBlockId| {
        body.basic_blocks[block]
            .terminator
            .as_ref()
            .map(|it| successors(&it.kind))
            .unwrap_or_default()
    };
    stack.push((start, succ(start), 0));
    while let Some((block, next, at)) = stack.last_mut() {
        if let Some(&target) = next.get(*at) {
            *at += 1;
            if !visited[block_index(target)] {
                visited[block_index(target)] = true;
                let more = succ(target);
                stack.push((target, more, 0));
            }
        } else {
            post.push(*block);
            stack.pop();
        }
    }
    post.reverse();
    post
}

/// Immediate dominators of the reachable blocks (Cooper, Harvey and Kennedy).
struct Dominators {
    /// By block index: the immediate dominator, `None` for a block that is not reachable.
    idom: Vec<Option<usize>>,
}

impl Dominators {
    fn new(body: &MirBody<'_>) -> Dominators {
        let rpo = reverse_postorder(body);
        let count = body.basic_blocks.len();
        let mut order = vec![usize::MAX; count];
        for (at, &block) in rpo.iter().enumerate() {
            order[block_index(block)] = at;
        }
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); count];
        for &block in &rpo {
            if let Some(terminator) = &body.basic_blocks[block].terminator {
                for target in successors(&terminator.kind) {
                    preds[block_index(target)].push(block_index(block));
                }
            }
        }
        let start = block_index(body.start_block);
        let mut idom: Vec<Option<usize>> = vec![None; count];
        idom[start] = Some(start);
        let intersect = |idom: &[Option<usize>], mut a: usize, mut b: usize| {
            while a != b {
                while order[a] > order[b] {
                    a = idom[a].expect("processed");
                }
                while order[b] > order[a] {
                    b = idom[b].expect("processed");
                }
            }
            a
        };
        let mut changed = true;
        while changed {
            changed = false;
            for &block in rpo.iter().skip(1) {
                let block = block_index(block);
                let mut new = None;
                for &pred in &preds[block] {
                    if idom[pred].is_none() {
                        continue;
                    }
                    new = Some(match new {
                        None => pred,
                        Some(it) => intersect(&idom, pred, it),
                    });
                }
                if new.is_some() && idom[block] != new {
                    idom[block] = new;
                    changed = true;
                }
            }
        }
        Dominators { idom }
    }

    /// Whether `a` dominates `b`; a block dominates itself.
    fn dominates(&self, a: BasicBlockId, b: BasicBlockId) -> bool {
        let (a, mut b) = (block_index(a), block_index(b));
        if self.idom[a].is_none() || self.idom[b].is_none() {
            return false;
        }
        loop {
            if a == b {
                return true;
            }
            let up = self.idom[b].expect("reachable");
            // Only the start block is its own dominator.
            if up == b {
                return false;
            }
            b = up;
        }
    }
}

/// How a statement or terminator mentions a local, as rustc's `PlaceContext` says it for the
/// distinctions these passes make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mention {
    /// The local as a whole is the left-hand side of an assignment.
    Assigned,
    /// A part of the local, not behind a pointer, is the left-hand side of an assignment.
    AssignedPart,
    /// The local as a whole is where a call returns to.
    CallReturned,
    /// Read: copied, moved, inspected, or the pointer a place goes through.
    Read,
    /// Shared borrow of the local or a part of it.
    SharedBorrow,
    /// Anything else that may change it: a `&mut` of it, a drop of it, a part of it a call
    /// returns to, an `asm!` output.
    Mutated,
    /// `StorageLive` or `StorageDead`.
    Storage,
}

/// Where in a body: a block and a statement in it, the terminator at `statements.len()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Location {
    block: BasicBlockId,
    index: usize,
}

fn visit_place(
    place: &StoredPlace,
    outer: Mention,
    at: Location,
    f: &mut impl FnMut(LocalId, Mention, Location),
) {
    let projection = place.projection.as_slice();
    for elem in projection {
        if let ProjectionElem::Index(local) = elem {
            f(*local, Mention::Read, at);
        }
    }
    let base = match projection.first() {
        None => outer,
        Some(ProjectionElem::Deref) => Mention::Read,
        Some(_) => match outer {
            Mention::Assigned => Mention::AssignedPart,
            Mention::CallReturned => Mention::Mutated,
            other => other,
        },
    };
    f(place.local, base, at);
}

fn visit_operand(
    operand: &super::Operand,
    at: Location,
    f: &mut impl FnMut(LocalId, Mention, Location),
) {
    if let OperandKind::Copy(place) | OperandKind::Move(place) = &operand.kind {
        visit_place(place, Mention::Read, at, f);
    }
}

fn visit_rvalue(rvalue: &Rvalue, at: Location, f: &mut impl FnMut(LocalId, Mention, Location)) {
    match rvalue {
        Rvalue::Ref(kind, place) => {
            let mention = match kind {
                super::BorrowKind::Shared | super::BorrowKind::Shallow => Mention::SharedBorrow,
                super::BorrowKind::Mut { .. } => Mention::Mutated,
            };
            visit_place(place, mention, at, f);
        }
        Rvalue::Len(place) | Rvalue::Discriminant(place) | Rvalue::CopyForDeref(place) => {
            visit_place(place, Mention::Read, at, f)
        }
        _ => rvalue.for_each_operand(&mut |it| visit_operand(it, at, f)),
    }
}

/// Calls `f` with every mention of a local in the statement at `at`.
fn visit_statement(
    statement: &StatementKind,
    at: Location,
    f: &mut impl FnMut(LocalId, Mention, Location),
) {
    match statement {
        StatementKind::Assign(place, rvalue) => {
            visit_rvalue(rvalue, at, f);
            visit_place(place, Mention::Assigned, at, f);
        }
        StatementKind::FakeRead(place) => visit_place(place, Mention::Read, at, f),
        StatementKind::Deinit(place) => visit_place(place, Mention::Mutated, at, f),
        StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
            f(*local, Mention::Storage, at)
        }
        StatementKind::InlineAsm(asm) => {
            for operand in &asm.operands {
                match operand {
                    super::AsmOperand::Reg { input, output, .. } => {
                        if let Some(input) = input {
                            visit_operand(input, at, f);
                        }
                        if let Some(output) = output {
                            visit_place(output, Mention::Mutated, at, f);
                        }
                    }
                    super::AsmOperand::Const(it) | super::AsmOperand::Sym(it) => {
                        visit_operand(it, at, f)
                    }
                    super::AsmOperand::Label => (),
                }
            }
            if let Some(taken) = &asm.taken {
                visit_place(taken, Mention::Mutated, at, f);
            }
        }
        StatementKind::Nop => (),
    }
}

fn visit_terminator(
    terminator: &TerminatorKind,
    at: Location,
    f: &mut impl FnMut(LocalId, Mention, Location),
) {
    match terminator {
        TerminatorKind::Drop { place, .. } => visit_place(place, Mention::Mutated, at, f),
        TerminatorKind::DropAndReplace { place, value, .. } => {
            visit_operand(value, at, f);
            visit_place(place, Mention::Mutated, at, f);
        }
        TerminatorKind::Call { func, args, destination, .. } => {
            visit_operand(func, at, f);
            for arg in args.iter() {
                visit_operand(arg, at, f);
            }
            visit_place(destination, Mention::CallReturned, at, f);
        }
        TerminatorKind::Yield { value, resume_arg, .. } => {
            visit_operand(value, at, f);
            visit_place(resume_arg, Mention::Mutated, at, f);
        }
        _ => terminator.for_each_operand(&mut |it| visit_operand(it, at, f)),
    }
}

/// Calls `f` with every mention of a local in the body.
fn visit_body(body: &MirBody<'_>, mut f: impl FnMut(LocalId, Mention, Location)) {
    for (block, data) in body.basic_blocks.iter() {
        for (index, statement) in data.statements.iter().enumerate() {
            visit_statement(&statement.kind, Location { block, index }, &mut f);
        }
        if let Some(terminator) = &data.terminator {
            let at = Location { block, index: data.statements.len() };
            visit_terminator(&terminator.kind, at, &mut f);
        }
    }
}

/// `place` with each local of it, the index locals too, replaced by what `map` says.
fn map_place(place: &mut StoredPlace, map: &mut impl FnMut(LocalId) -> LocalId) {
    place.local = map(place.local);
    if place.projection.as_slice().iter().any(|it| matches!(it, ProjectionElem::Index(_))) {
        let elems: Vec<PlaceElem> = place
            .projection
            .as_slice()
            .iter()
            .map(|it| match *it {
                ProjectionElem::Index(local) => ProjectionElem::Index(map(local)),
                other => other,
            })
            .collect();
        place.projection = super::Projection::new_from_iter(elems).store();
    }
}

/// Every local of the body replaced by what `map` says, in its statements and terminators and
/// in what the body says of its locals. `None` drops the local from those lists; a place
/// must not name it.
fn renumber_locals(
    body: &mut MirBody<'_>,
    new_locals: Arena<super::Local>,
    map: &[Option<LocalId>],
) {
    let mut lookup = |local: LocalId| map[local_index(local)].expect("a removed local is used");
    body.walk_places(|place| map_place(place, &mut lookup));
    for (_, block) in body.basic_blocks.iter_mut() {
        for statement in &mut block.statements {
            if let StatementKind::StorageLive(local) | StatementKind::StorageDead(local) =
                &mut statement.kind
            {
                *local = lookup(*local);
            }
        }
    }
    let get = |local: LocalId| map.get(local_index(local)).copied().flatten();
    body.locals = new_locals;
    let mut bindings = ArenaMap::default();
    for (binding, &local) in body.binding_locals.iter() {
        if let Some(local) = get(local) {
            bindings.insert(binding, local);
        }
    }
    body.binding_locals = bindings;
    body.param_locals = body.param_locals.iter().filter_map(|&it| get(it)).collect();
    body.promoted_locals = body
        .promoted_locals
        .iter()
        .filter_map(|&(local, expr)| Some((get(local)?, expr)))
        .collect();
    body.promoted_constants = body.promoted_constants.iter().filter_map(|&it| get(it)).collect();
    body.scope_ends = body.scope_ends.iter().filter_map(|&it| get(it)).collect();
    body.upvars_in_place =
        body.upvars_in_place.iter().filter_map(|&(it, field)| Some((get(it)?, field))).collect();
    let upvars: FxHashMap<_, _> = std::mem::take(&mut body.upvar_locals)
        .into_iter()
        .map(|(binding, locals)| {
            let locals = locals
                .into_iter()
                .filter_map(|(local, place)| Some((get(local)?, place)))
                .collect::<Vec<_>>();
            (binding, locals)
        })
        .filter(|(_, locals)| !locals.is_empty())
        .collect();
    body.upvar_locals = upvars;
}

/// Replaces the blocks of the body by `blocks`, `map` saying where each old block went.
fn renumber_blocks(body: &mut MirBody<'_>, blocks: Vec<BasicBlock>, map: &[Option<usize>]) {
    let mut arena = Arena::with_capacity(blocks.len());
    for mut block in blocks {
        if let Some(terminator) = &mut block.terminator {
            successors_mut(&mut terminator.kind, |it| {
                *it = block_id(map[block_index(*it)].expect("a removed block is a target"))
            });
        }
        arena.alloc(block);
    }
    body.basic_blocks = arena;
    body.start_block = block_id(map[block_index(body.start_block)].expect("start is kept"));
    body.unreached =
        body.unreached.iter().filter_map(|&it| map[block_index(it)].map(block_id)).collect();
    body.panic_unwinds = body
        .panic_unwinds
        .iter()
        .filter_map(|&(block, at, to)| {
            Some((block_id(map[block_index(block)]?), at, block_id(map[block_index(to)]?)))
        })
        .collect();
}
