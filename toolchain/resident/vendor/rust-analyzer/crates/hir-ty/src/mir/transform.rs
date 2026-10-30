//! The passes that take a body from [`MirPhase::Built`] to [`MirPhase::Runtime`], as rustc's
//! `run_runtime_lowering_passes` does, in the order they run:
//!
//! 1. [`asserts::add_asserts`]: rustc's `Assert`s before what panics, with their own unwind
//!    edges into the scopes' unwind drop tree, which lowering builds.
//! 2. [`upvars::upvars_in_place`]: an `async` body uses its captures in place.
//! 3. [`cleanup::remove_noop_landing_pads`]: an unwind edge to nothing but storage ends goes.
//! 4. [`elaborate_drops::elaborate_drops`]: every drop drops; drop flags are locals.
//! 5. [`packed_drops::add_moves_for_packed_drops`]: what a packed struct leaves misaligned is
//!    moved out before it is dropped.
//!
//! None of them optimizes. The IDE never asks for these queries, so it never pays for them;
//! the code generator and the interpreter do. `docs/mir-passes.md` has the design.

mod asserts;
mod cleanup;
mod elaborate_drops;
mod move_paths;
mod packed_drops;
mod upvars;

pub(super) use cleanup::remove_noop_landing_pads;

use std::cell::RefCell;

use base_db::Crate;
use hir_def::HasModule;
use rustc_hash::FxHashMap;
use rustc_type_ir::inherent::{IntoKind, Ty as _};

use crate::{
    InferBodyId,
    db::{HirDatabase, InternedClosureId, InternedCoroutineId},
    drop::{DropGlue, has_drop_glue},
    next_solver::{
        DbInterner, ErrorGuaranteed, ParamEnv, StoredGenericArgs, Ty, TyKind, TypingMode,
        infer::{DbInternerInferExt, InferCtxt, traits::ObligationCause},
        obligation_ctxt::ObligationCtxt,
    },
    traits::{ParamEnvAndCrate, StoredParamEnvAndCrate},
};

use super::{MirBody, MirLowerError, MirPhase, TerminatorKind, monomorphization::monomorphized};

type Result<'db, T> = std::result::Result<T, MirLowerError<'db>>;

/// What the passes ask of the types of a body.
pub(super) struct Ctx<'db> {
    pub(super) db: &'db dyn HirDatabase,
    pub(super) infcx: InferCtxt<'db>,
    pub(super) env: ParamEnv<'db>,
    krate: Crate,
    needs_drop: RefCell<FxHashMap<Ty<'db>, bool>>,
}

impl<'db> Ctx<'db> {
    pub(super) fn new(db: &'db dyn HirDatabase, owner: InferBodyId<'db>) -> Self {
        let krate = owner.krate(db);
        let env = db.trait_environment(owner.generic_def(db));
        let interner = DbInterner::new_with(db, krate);
        let infcx = interner.infer_ctxt().build(TypingMode::non_body_analysis());
        Ctx { db, infcx, env, krate, needs_drop: RefCell::default() }
    }

    /// Whether a value of `ty` may have something to drop: a type parameter may.
    pub(super) fn needs_drop(&self, ty: Ty<'db>) -> bool {
        if let Some(&known) = self.needs_drop.borrow().get(&ty) {
            return known;
        }
        let needs = has_drop_glue(&self.infcx, ty, self.env) != DropGlue::None;
        self.needs_drop.borrow_mut().insert(ty, needs);
        needs
    }

    pub(super) fn normalize(&self, ty: Ty<'db>) -> Ty<'db> {
        if !matches!(ty.kind(), TyKind::Alias(..)) {
            return ty;
        }
        let mut ocx = ObligationCtxt::new(&self.infcx);
        // One the solver can't decide comes back as a variable, and stays the alias, which
        // `needs_drop` takes as maybe having something to drop.
        match ocx.structurally_normalize_ty(&ObligationCause::dummy(), self.env, ty) {
            Ok(normalized) if matches!(normalized.kind(), TyKind::Infer(_)) => ty,
            Ok(normalized) => normalized,
            Err(_) => Ty::new_error(self.infcx.interner, ErrorGuaranteed),
        }
    }

    fn has_destructor(&self, adt: hir_def::AdtId) -> bool {
        crate::drop::destructor(self.db, adt).is_some()
    }

    /// Whether a value of `ty` has nothing to drop and no cell in it.
    fn is_plain(&self, ty: Ty<'db>) -> bool {
        !self.needs_drop(ty) && crate::drop::is_freeze(&self.infcx, ty, self.env)
    }

    fn bool_ty(&self) -> Ty<'db> {
        Ty::new_bool(self.infcx.interner)
    }

    /// The size of `ty` in bytes, if its layout is known where the body is generic.
    fn size_of(&self, ty: Ty<'db>) -> Option<u64> {
        let env = ParamEnvAndCrate { param_env: self.env, krate: self.krate };
        let layout = self.db.layout_of_ty(ty.store(), env.store()).ok()?;
        Some(layout.size.bytes())
    }

    /// The alignment of `ty`, if its layout is known where the body is generic.
    fn layout_align(&self, ty: Ty<'db>) -> Option<u64> {
        let env = ParamEnvAndCrate { param_env: self.env, krate: self.krate };
        let layout = self.db.layout_of_ty(ty.store(), env.store()).ok()?;
        Some(layout.align.abi.bytes())
    }
}

/// `built` after the passes.
fn run_passes<'db>(db: &'db dyn HirDatabase, built: &MirBody<'db>) -> Result<'db, MirBody<'db>> {
    let mut body = built.clone();
    let ctx = Ctx::new(db, body.owner);
    // rustc's `CleanupPostBorrowck` before these: a false edge, which only borrow checking
    // follows, is a jump to its real target.
    for (_, block) in body.basic_blocks.iter_mut() {
        if let Some(terminator) = &mut block.terminator
            && let TerminatorKind::FalseEdge { real_target, .. } = terminator.kind
        {
            terminator.kind = TerminatorKind::Goto { target: real_target };
        }
    }
    // `Assert`s first: lowering says where each panicking statement unwinds by its position.
    asserts::add_asserts(&ctx, &mut body);
    upvars::upvars_in_place(&mut body);
    body.promoted_constants = body
        .promoted_locals
        .iter()
        .map(|&(local, _)| local)
        .filter(|&local| ctx.is_plain(body.locals[local].ty.as_ref()))
        .collect();
    cleanup::remove_noop_landing_pads(&mut body);
    elaborate_drops::elaborate_drops(&ctx, &mut body)?;
    packed_drops::add_moves_for_packed_drops(&ctx, &mut body);
    body.phase = MirPhase::Runtime;
    body.shrink_to_fit();
    Ok(body)
}

/// How many of the runtime bodies (and their monomorphized copies) salsa keeps the value of once
/// a revision is over; 0 keeps every one. A body let go is lowered again when it is next asked
/// for, and what depends on it cannot be backdated then.
pub fn set_runtime_mir_capacity(db: &mut dyn HirDatabase, capacity: usize) {
    runtime_mir_body_query::set_lru_capacity(db, capacity);
    runtime_mir_body_for_closure_query::set_lru_capacity(db, capacity);
    runtime_mir_body_for_coroutine_query::set_lru_capacity(db, capacity);
    runtime_mir_body_for_coroutine_by_move_query::set_lru_capacity(db, capacity);
    monomorphized_runtime_mir_body_query::set_lru_capacity(db, capacity);
    monomorphized_runtime_mir_body_for_closure_query::set_lru_capacity(db, capacity);
    monomorphized_runtime_mir_body_for_coroutine_query::set_lru_capacity(db, capacity);
    monomorphized_runtime_mir_body_for_coroutine_by_move_query::set_lru_capacity(db, capacity);
}

#[salsa::tracked(returns(as_ref), cycle_result = runtime_mir_body_cycle_result, lru = 0)]
pub fn runtime_mir_body_query<'db>(
    db: &'db dyn HirDatabase,
    def: InferBodyId<'db>,
) -> Result<'db, MirBody<'db>> {
    run_passes(db, db.mir_body(def)?)
}

/// The constant behind a promoted `&expr` of `def` after the passes: what the code generator
/// evaluates for it with the interpreter.
#[salsa::tracked(returns(as_ref))]
pub fn runtime_promoted_mir_body_query<'db>(
    db: &'db dyn HirDatabase,
    def: InferBodyId<'db>,
    expr: hir_def::hir::ExprId,
) -> Result<'db, MirBody<'db>> {
    let built = super::promoted_mir_body_query(db, def, expr).map_err(Clone::clone)?;
    run_passes(db, built)
}

#[salsa::tracked(returns(as_ref), cycle_result = runtime_mir_body_for_closure_cycle_result, lru = 0)]
pub fn runtime_mir_body_for_closure_query<'db>(
    db: &'db dyn HirDatabase,
    closure: InternedClosureId<'db>,
) -> Result<'db, MirBody<'db>> {
    run_passes(db, db.mir_body_for_closure(closure)?)
}

#[salsa::tracked(returns(as_ref), cycle_result = runtime_mir_body_for_coroutine_cycle_result, lru = 0)]
pub fn runtime_mir_body_for_coroutine_query<'db>(
    db: &'db dyn HirDatabase,
    coroutine: InternedCoroutineId<'db>,
) -> Result<'db, MirBody<'db>> {
    run_passes(db, db.mir_body_for_coroutine(coroutine)?)
}

#[salsa::tracked(returns(as_ref), cycle_result = runtime_mir_body_for_coroutine_cycle_result, lru = 0)]
pub fn runtime_mir_body_for_coroutine_by_move_query<'db>(
    db: &'db dyn HirDatabase,
    coroutine: InternedCoroutineId<'db>,
) -> Result<'db, MirBody<'db>> {
    run_passes(db, db.mir_body_for_coroutine_by_move(coroutine)?)
}

#[salsa::tracked(returns(as_ref), cycle_result = monomorphized_runtime_mir_body_cycle_result, lru = 0)]
pub fn monomorphized_runtime_mir_body_query<'db>(
    db: &'db dyn HirDatabase,
    def: InferBodyId<'db>,
    subst: StoredGenericArgs,
    trait_env: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    monomorphized(db, db.runtime_mir_body(def)?, &subst, &trait_env)
}

#[salsa::tracked(
    returns(as_ref),
    cycle_result = monomorphized_runtime_mir_body_for_closure_cycle_result,
    lru = 0
)]
pub fn monomorphized_runtime_mir_body_for_closure_query<'db>(
    db: &'db dyn HirDatabase,
    closure: InternedClosureId<'db>,
    subst: StoredGenericArgs,
    trait_env: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    monomorphized(db, db.runtime_mir_body_for_closure(closure)?, &subst, &trait_env)
}

#[salsa::tracked(
    returns(as_ref),
    cycle_result = monomorphized_runtime_mir_body_for_coroutine_cycle_result,
    lru = 0
)]
pub fn monomorphized_runtime_mir_body_for_coroutine_query<'db>(
    db: &'db dyn HirDatabase,
    coroutine: InternedCoroutineId<'db>,
    subst: StoredGenericArgs,
    trait_env: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    monomorphized(db, db.runtime_mir_body_for_coroutine(coroutine)?, &subst, &trait_env)
}

#[salsa::tracked(
    returns(as_ref),
    cycle_result = monomorphized_runtime_mir_body_for_coroutine_cycle_result,
    lru = 0
)]
pub fn monomorphized_runtime_mir_body_for_coroutine_by_move_query<'db>(
    db: &'db dyn HirDatabase,
    coroutine: InternedCoroutineId<'db>,
    subst: StoredGenericArgs,
    trait_env: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    monomorphized(db, db.runtime_mir_body_for_coroutine_by_move(coroutine)?, &subst, &trait_env)
}

fn runtime_mir_body_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InferBodyId<'db>,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

fn runtime_mir_body_for_closure_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InternedClosureId<'db>,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

fn runtime_mir_body_for_coroutine_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InternedCoroutineId<'db>,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

fn monomorphized_runtime_mir_body_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InferBodyId<'db>,
    _: StoredGenericArgs,
    _: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

fn monomorphized_runtime_mir_body_for_closure_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InternedClosureId<'db>,
    _: StoredGenericArgs,
    _: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}

fn monomorphized_runtime_mir_body_for_coroutine_cycle_result<'db>(
    _db: &'db dyn HirDatabase,
    _: salsa::Id,
    _: InternedCoroutineId<'db>,
    _: StoredGenericArgs,
    _: StoredParamEnvAndCrate,
) -> Result<'db, MirBody<'db>> {
    Err(MirLowerError::Loop)
}
