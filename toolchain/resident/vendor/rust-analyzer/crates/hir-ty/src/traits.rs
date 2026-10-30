//! Trait solving using next trait solver.

use std::{cell::OnceCell, hash::Hash};

use base_db::Crate;
use hir_def::{
    AdtId, AssocItemId, ExpressionStoreOwnerId, GenericDefId, HasModule, ImplId, Lookup, TraitId,
    expr_store::ExpressionStore,
    hir::generics::WherePredicate,
    lang_item::LangItems,
    nameres::DefMap,
    resolver::Resolver,
    signatures::{
        ConstFlags, ConstSignature, EnumFlags, EnumSignature, FnFlags, FunctionSignature,
        StructFlags, StructSignature, TraitFlags, TraitSignature, TypeAliasFlags,
        TypeAliasSignature, UnionSignature,
    },
};
use hir_expand::name::Name;
use intern::sym;
use rustc_type_ir::{
    TypeVisitableExt, TypingMode,
    inherent::{BoundExistentialPredicates, IntoKind, Ty as _},
};

use crate::{
    LifetimeElisionKind, LifetimeLoweringMode, Span, TyLoweringContext,
    db::HirDatabase,
    generics::Generics,
    lower::LoweringMode,
    next_solver::{
        DbInterner, GenericArgs, ParamEnv, StoredClauses, Ty, TyKind,
        infer::{
            DbInternerInferExt, InferCtxt,
            select::EvaluationResult,
            traits::{Obligation, ObligationCause},
        },
        obligation_ctxt::ObligationCtxt,
    },
};

/// Type for `hir`, because commonly we want both param env and a crate in an exported API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParamEnvAndCrate<'db> {
    pub param_env: ParamEnv<'db>,
    pub krate: Crate,
}

impl<'db> ParamEnvAndCrate<'db> {
    #[inline]
    pub fn store(self) -> StoredParamEnvAndCrate {
        StoredParamEnvAndCrate { param_env: self.param_env.clauses.store(), krate: self.krate }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoredParamEnvAndCrate {
    param_env: StoredClauses,
    pub krate: Crate,
}

impl StoredParamEnvAndCrate {
    #[inline]
    pub fn param_env<'db>(&self, _db: &'db dyn HirDatabase) -> ParamEnv<'db> {
        ParamEnv { clauses: self.param_env.as_ref() }
    }

    #[inline]
    pub fn as_ref<'db>(&self, db: &'db dyn HirDatabase) -> ParamEnvAndCrate<'db> {
        ParamEnvAndCrate { param_env: self.param_env(db), krate: self.krate }
    }
}

/// This should be used in `hir` only.
pub fn structurally_normalize_ty<'db>(
    infcx: &InferCtxt<'db>,
    ty: Ty<'db>,
    env: ParamEnv<'db>,
) -> Ty<'db> {
    let TyKind::Alias(..) = ty.kind() else { return ty };
    let mut ocx = ObligationCtxt::new(infcx);
    let ty = ocx.structurally_normalize_ty(&ObligationCause::dummy(), env, ty).unwrap_or(ty);
    ty.replace_infer_with_error(infcx.interner)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, salsa::SalsaValue)]
pub enum FnTrait {
    // Warning: Order is important. If something implements `x` it should also implement
    // `y` if `y <= x`.
    FnOnce,
    FnMut,
    Fn,

    AsyncFnOnce,
    AsyncFnMut,
    AsyncFn,
}

impl FnTrait {
    pub fn method_name(self) -> Name {
        match self {
            FnTrait::FnOnce => Name::new_symbol_root(sym::call_once),
            FnTrait::FnMut => Name::new_symbol_root(sym::call_mut),
            FnTrait::Fn => Name::new_symbol_root(sym::call),
            FnTrait::AsyncFnOnce => Name::new_symbol_root(sym::async_call_once),
            FnTrait::AsyncFnMut => Name::new_symbol_root(sym::async_call_mut),
            FnTrait::AsyncFn => Name::new_symbol_root(sym::async_call),
        }
    }

    pub fn get_id(self, lang_items: &LangItems) -> Option<TraitId> {
        match self {
            FnTrait::FnOnce => lang_items.FnOnce,
            FnTrait::FnMut => lang_items.FnMut,
            FnTrait::Fn => lang_items.Fn,
            FnTrait::AsyncFnOnce => lang_items.AsyncFnOnce,
            FnTrait::AsyncFnMut => lang_items.AsyncFnMut,
            FnTrait::AsyncFn => lang_items.AsyncFn,
        }
    }
}

/// This should not be used in `hir-ty`, only in `hir`.
pub fn implements_trait_unique<'db>(
    ty: Ty<'db>,
    db: &'db dyn HirDatabase,
    env: ParamEnvAndCrate<'db>,
    trait_: TraitId,
) -> bool {
    implements_trait_unique_with_infcx(db, env, trait_, &mut |infcx| {
        infcx.fill_rest_fresh_args(Span::Dummy, trait_.into(), [ty.into()])
    })
}

/// This should not be used in `hir-ty`, only in `hir`.
pub fn implements_trait_unique_with_args<'db>(
    db: &'db dyn HirDatabase,
    env: ParamEnvAndCrate<'db>,
    trait_: TraitId,
    args: GenericArgs<'db>,
) -> bool {
    implements_trait_unique_with_infcx(db, env, trait_, &mut |_| args)
}

pub fn implements_trait_unique_with_infcx<'db>(
    db: &'db dyn HirDatabase,
    env: ParamEnvAndCrate<'db>,
    trait_: TraitId,
    create_args: &mut dyn FnMut(&InferCtxt<'db>) -> GenericArgs<'db>,
) -> bool {
    let interner = DbInterner::new_with(db, env.krate);
    // FIXME(next-solver): I believe this should be `PostAnalysis`.
    let infcx = interner.infer_ctxt().build(TypingMode::non_body_analysis());

    let args = create_args(&infcx);
    let trait_ref = rustc_type_ir::TraitRef::new_from_args(interner, trait_.into(), args);
    if trait_ref.self_ty().is_ty_error() {
        return false;
    }

    let obligation = Obligation::new(interner, ObligationCause::dummy(), env.param_env, trait_ref);
    infcx.predicate_must_hold_modulo_regions(&obligation)
}

/// Whether two types unify once `create_tys` has replaced their generic parameters with
/// inference variables: `[u8]` and an impl's `[T]` do, `[u8]` and `[MaybeUninit<T>]` do not.
pub fn could_unify_with_infcx<'db>(
    db: &'db dyn HirDatabase,
    krate: Crate,
    create_tys: &mut dyn FnMut(&InferCtxt<'db>) -> (Ty<'db>, Ty<'db>),
) -> bool {
    let interner = DbInterner::new_with(db, krate);
    let infcx = interner.infer_ctxt().build(TypingMode::non_body_analysis());
    let (a, b) = create_tys(&infcx);
    infcx.at(&ObligationCause::dummy(), ParamEnv::empty()).eq(a, b).is_ok()
}

/// Whether the trait impl `impl_` may apply to the type `create_ty` makes, as rustdoc's
/// `synthesize_blanket_impls` asks: the impl's parameters are inference variables, its self
/// type is equated with the type, and its where-clauses and trait ref may each hold.
pub fn impl_may_apply<'db>(
    db: &'db dyn HirDatabase,
    krate: Crate,
    impl_: ImplId,
    create_ty: &mut dyn FnMut(&InferCtxt<'db>) -> Ty<'db>,
) -> bool {
    let interner = DbInterner::new_with(db, krate);
    let infcx = interner.infer_ctxt().build(TypingMode::non_body_analysis());
    let ty = create_ty(&infcx);
    let Some(trait_ref) = db.impl_trait(impl_) else { return false };
    let args = infcx.fresh_args_for_item(Span::Dummy, impl_.into());
    let trait_ref = trait_ref.instantiate(interner, args).skip_norm_wip();
    if infcx.at(&ObligationCause::dummy(), ParamEnv::empty()).eq(trait_ref.self_ty(), ty).is_err() {
        return false;
    }
    let may_hold = |obligation| infcx.predicate_may_hold(&obligation);
    let clauses = crate::lower::GenericPredicates::query_all(db, impl_.into())
        .iter_instantiated(interner, args.as_slice())
        .map(crate::next_solver::Unnormalized::skip_norm_wip);
    crate::next_solver::util::clauses_as_obligations(
        clauses,
        ObligationCause::dummy(),
        ParamEnv::empty(),
    )
    .all(may_hold)
        && may_hold(Obligation::new(
            interner,
            ObligationCause::dummy(),
            ParamEnv::empty(),
            trait_ref,
        ))
}

/// Whether the trait's item `item` cannot be used through `impl_`: one of its own where-clauses
/// that names only the trait's parameters (`Self::Item: Ord` of `Iterator::cmp`) cannot hold
/// there. rustc's `is_impossible_associated_item` (`compiler/rustc_trait_selection/src/traits/mod.rs`
/// at rust 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0), which rustdoc asks before it lists an item
/// the impl leaves as the trait provides it. `krate` is the crate asking, rustc's local crate:
/// a clause it could not know the answer to holds as far as it can tell.
pub fn is_impossible_associated_item(
    db: &dyn HirDatabase,
    krate: Crate,
    impl_: ImplId,
    item: GenericDefId,
) -> bool {
    use std::ops::ControlFlow;

    use rustc_type_ir::{TypeSuperVisitable, TypeVisitable, TypeVisitor};

    use crate::next_solver::{Const, ConstKind, EarlyBinder, Region, RegionKind};

    /// Breaks at a parameter of the item's own, numbered after the trait's.
    struct OwnParam(u32);
    impl<'db> TypeVisitor<DbInterner<'db>> for OwnParam {
        type Result = ControlFlow<()>;
        fn visit_ty(&mut self, ty: Ty<'db>) -> ControlFlow<()> {
            match ty.kind() {
                TyKind::Param(param) if param.index >= self.0 => ControlFlow::Break(()),
                _ => ty.super_visit_with(self),
            }
        }
        fn visit_region(&mut self, region: Region<'db>) -> ControlFlow<()> {
            match region.kind() {
                RegionKind::ReEarlyParam(param) if param.index >= self.0 => ControlFlow::Break(()),
                _ => ControlFlow::Continue(()),
            }
        }
        fn visit_const(&mut self, konst: Const<'db>) -> ControlFlow<()> {
            match konst.kind() {
                ConstKind::Param(param) if param.index >= self.0 => ControlFlow::Break(()),
                _ => konst.super_visit_with(self),
            }
        }
    }

    let Some(trait_ref) = db.impl_trait(impl_) else { return false };
    let interner = DbInterner::new_with(db, impl_.krate(db));
    // As coherence asks: the impl's parameters are inference variables, so a clause that holds
    // for some of their values (`T: Ord` of an `IntoIter<T>`) is ambiguous, not an error.
    let infcx = interner.infer_ctxt().build(TypingMode::Coherence);
    let fresh = infcx.fresh_args_for_item(Span::Dummy, impl_.into());
    let args = trait_ref.instantiate(interner, fresh).skip_norm_wip().args;
    let parent = crate::generics::generics(db, item).len_parent() as u32;
    let mut ocx = ObligationCtxt::new(&infcx);
    for clause in crate::lower::GenericPredicates::query_own_explicit(db, item)
        .skip_binder()
        .filter(|clause| clause.visit_with(&mut OwnParam(parent)).is_continue())
    {
        let clause = EarlyBinder::bind(clause).instantiate(interner, args).skip_norm_wip();
        // Coherence answers a trait goal only a crate upstream of `krate` could implement
        // (`io::Error: Ord`) as ambiguous, since that crate may add the impl; rust-analyzer's
        // solver takes every crate as local, so the goal is left out here instead.
        // A projection is normalized as the solver constrains it, though the impl's own bounds
        // on its inference variables leave the goal ambiguous (`Self::Item` of a
        // `FindIter<'a, 'h, A: Automaton>` is `Match`); one under a binder (`for<'a>`) is
        // judged as written, the normalizer taking no bound vars. Each projection has a
        // fulfillment context of its own, so the clauses already registered are answered once,
        // at the end, and not with it.
        if let rustc_type_ir::ClauseKind::Trait(pred) = clause.kind().skip_binder() {
            let known = knowable(db, krate, pred.trait_ref, |ty| {
                if !matches!(ty.kind(), TyKind::Alias(..)) || ty.has_escaping_bound_vars() {
                    return ty;
                }
                let mut own = crate::next_solver::fulfill::FulfillmentCtxt::new(&infcx);
                infcx
                    .at(&ObligationCause::dummy(), ParamEnv::empty())
                    .structurally_normalize_ty(ty, &mut own)
                    .ok()
                    .filter(|it| !matches!(it.kind(), TyKind::Infer(_)))
                    .unwrap_or(ty)
            });
            if !known {
                continue;
            }
        }
        ocx.register_obligation(Obligation::new(
            interner,
            ObligationCause::dummy(),
            ParamEnv::empty(),
            clause,
        ));
    }
    !ocx.try_evaluate_obligations().is_empty()
}

/// Whether `krate` can know every impl of `trait_ref`: the trait is its own or `#[fundamental]`,
/// or one of the types is its own, under references and `#[fundamental]` types. rustc's
/// `trait_ref_is_knowable` and `orphan_check_trait_ref`, leaving out where parameters may appear.
/// Each type looked at goes through `normalize` first.
fn knowable<'db>(
    db: &'db dyn HirDatabase,
    krate: Crate,
    trait_ref: crate::next_solver::TraitRef<'db>,
    mut normalize: impl FnMut(Ty<'db>) -> Ty<'db>,
) -> bool {
    let trait_id = trait_ref.def_id.0;
    if trait_id.module(db).krate(db) == krate
        || TraitSignature::of(db, trait_id).flags.contains(TraitFlags::FUNDAMENTAL)
    {
        return true;
    }
    trait_ref.args.types().any(|mut ty| {
        loop {
            ty = normalize(ty);
            match ty.kind() {
                TyKind::Ref(_, inner, _) => ty = inner,
                TyKind::Adt(adt_def, args) => {
                    let fundamental = match adt_def.def_id() {
                        AdtId::StructId(id) => {
                            StructSignature::of(db, id).flags.contains(StructFlags::FUNDAMENTAL)
                        }
                        _ => false,
                    };
                    match args.types().next() {
                        Some(inner) if fundamental => ty = inner,
                        _ => break adt_def.def_id().module(db).krate(db) == krate,
                    }
                }
                TyKind::Dynamic(bounds, _) => {
                    break bounds
                        .principal_def_id()
                        .is_some_and(|it| it.0.module(db).krate(db) == krate);
                }
                _ => break false,
            }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WherePredicateEvaluation {
    Holds,
    NotProven,
    HasErrors,
    NoObligations,
}

/// This should not be used in `hir-ty`, only in `hir`.
/// This is exposed to allow the IDE to evaluate arbitrary predicates.
pub fn where_predicate_must_hold<'db>(
    db: &'db dyn HirDatabase,
    resolver: &Resolver<'db>,
    store: &'db ExpressionStore,
    def: ExpressionStoreOwnerId,
    generic_def: GenericDefId,
    env: ParamEnvAndCrate<'db>,
    predicate: &WherePredicate,
) -> WherePredicateEvaluation {
    let interner = DbInterner::new_with(db, env.krate);
    let infcx = interner.infer_ctxt().build(TypingMode::PostAnalysis);
    let generics = OnceCell::<Generics<'db>>::new();
    let mut ctx = TyLoweringContext::new(
        db,
        resolver,
        store,
        def,
        generic_def,
        &generics,
        LifetimeElisionKind::Infer,
        LifetimeLoweringMode::Bound,
    )
    .with_interning_mode(LoweringMode::Ide);
    let clauses =
        ctx.lower_where_predicate(predicate, false).map(|(clause, _)| clause).collect::<Vec<_>>();

    if !ctx.diagnostics.is_empty()
        || clauses.iter().any(|clause| clause.as_predicate().references_error())
    {
        return WherePredicateEvaluation::HasErrors;
    }

    if clauses.is_empty() {
        return if ctx.unsized_types.is_empty() {
            WherePredicateEvaluation::HasErrors
        } else {
            WherePredicateEvaluation::NoObligations
        };
    }

    let result = infcx.probe(|snapshot| {
        let mut ocx = ObligationCtxt::new(&infcx);
        for clause in clauses {
            let obligation = Obligation::new(
                interner,
                ObligationCause::dummy(),
                env.param_env,
                clause.as_predicate(),
            );
            ocx.register_obligation(obligation);
        }

        let mut result = EvaluationResult::EvaluatedToOk;
        for error in ocx.evaluate_obligations_error_on_ambiguity() {
            if error.is_true_error() {
                return EvaluationResult::EvaluatedToErr;
            }
            result = result.max(EvaluationResult::EvaluatedToAmbig);
        }
        if infcx.opaque_types_added_in_snapshot(snapshot) {
            result.max(EvaluationResult::EvaluatedToOkModuloOpaqueTypes)
        } else if infcx.region_constraints_added_in_snapshot(snapshot) {
            result.max(EvaluationResult::EvaluatedToOkModuloRegions)
        } else {
            result
        }
    });

    if result.must_apply_modulo_regions() {
        WherePredicateEvaluation::Holds
    } else {
        WherePredicateEvaluation::NotProven
    }
}

pub fn is_inherent_impl_coherent(db: &dyn HirDatabase, def_map: &DefMap, impl_id: ImplId) -> bool {
    let self_ty = db.impl_self_ty(impl_id).instantiate_identity().skip_norm_wip();
    let self_ty = self_ty.kind();
    let impl_allowed = match self_ty {
        TyKind::Tuple(_)
        | TyKind::FnDef(_, _)
        | TyKind::Array(_, _)
        | TyKind::Never
        | TyKind::RawPtr(_, _)
        | TyKind::Ref(_, _, _)
        | TyKind::Slice(_)
        | TyKind::Str
        | TyKind::Bool
        | TyKind::Char
        | TyKind::Int(_)
        | TyKind::Uint(_)
        | TyKind::Float(_) => def_map.is_rustc_coherence_is_core(),

        TyKind::Adt(adt_def, _) => adt_def.def_id().module(db).krate(db) == def_map.krate(),
        TyKind::Dynamic(it, _) => it
            .principal_def_id()
            .is_some_and(|trait_id| trait_id.0.module(db).krate(db) == def_map.krate()),

        _ => true,
    };
    impl_allowed || {
        let rustc_has_incoherent_inherent_impls = match self_ty {
            TyKind::Tuple(_)
            | TyKind::FnDef(_, _)
            | TyKind::Array(_, _)
            | TyKind::Never
            | TyKind::RawPtr(_, _)
            | TyKind::Ref(_, _, _)
            | TyKind::Slice(_)
            | TyKind::Str
            | TyKind::Bool
            | TyKind::Char
            | TyKind::Int(_)
            | TyKind::Uint(_)
            | TyKind::Float(_) => true,

            TyKind::Adt(adt_def, _) => match adt_def.def_id() {
                hir_def::AdtId::StructId(id) => StructSignature::of(db, id)
                    .flags
                    .contains(StructFlags::RUSTC_HAS_INCOHERENT_INHERENT_IMPLS),
                hir_def::AdtId::UnionId(id) => UnionSignature::of(db, id)
                    .flags
                    .contains(StructFlags::RUSTC_HAS_INCOHERENT_INHERENT_IMPLS),
                hir_def::AdtId::EnumId(it) => EnumSignature::of(db, it)
                    .flags
                    .contains(EnumFlags::RUSTC_HAS_INCOHERENT_INHERENT_IMPLS),
            },
            TyKind::Dynamic(it, _) => it.principal_def_id().is_some_and(|trait_id| {
                TraitSignature::of(db, trait_id.0)
                    .flags
                    .contains(TraitFlags::RUSTC_HAS_INCOHERENT_INHERENT_IMPLS)
            }),

            _ => false,
        };
        let items = impl_id.impl_items(db);
        rustc_has_incoherent_inherent_impls
            && !items.items.is_empty()
            && items.items.iter().all(|&(_, assoc)| match assoc {
                AssocItemId::FunctionId(it) => FunctionSignature::of(db, it)
                    .flags
                    .contains(FnFlags::RUSTC_ALLOW_INCOHERENT_IMPL),
                AssocItemId::ConstId(it) => ConstSignature::of(db, it)
                    .flags
                    .contains(ConstFlags::RUSTC_ALLOW_INCOHERENT_IMPL),
                AssocItemId::TypeAliasId(it) => TypeAliasSignature::of(db, it)
                    .flags
                    .contains(TypeAliasFlags::RUSTC_ALLOW_INCOHERENT_IMPL),
            })
    }
}

/// Checks whether the impl satisfies the orphan rules.
///
/// Given `impl<P1..=Pn> Trait<T1..=Tn> for T0`, an `impl` is valid only if at least one of the following is true:
/// - Trait is a local trait
/// - All of
///   - At least one of the types `T0..=Tn` must be a local type. Let `Ti` be the first such type.
///   - No uncovered type parameters `P1..=Pn` may appear in `T0..Ti` (excluding `Ti`)
pub fn check_orphan_rules<'db>(db: &'db dyn HirDatabase, impl_: ImplId) -> bool {
    let Some(impl_trait) = db.impl_trait(impl_) else {
        // not a trait impl
        return true;
    };

    let local_crate = impl_.lookup(db).container.krate(db);
    let is_local = |tgt_crate| tgt_crate == local_crate;

    let trait_ref = impl_trait.instantiate_identity().skip_norm_wip();
    let trait_id = trait_ref.def_id.0;
    if is_local(trait_id.module(db).krate(db)) {
        // trait to be implemented is local
        return true;
    }

    let interner = DbInterner::new_with(db, local_crate);
    let infcx = interner.infer_ctxt().build(TypingMode::non_body_analysis());
    let param_env = db.trait_environment(impl_.into());
    let trait_ref = infcx
        .at(&ObligationCause::dummy(), param_env)
        .deeply_normalize(trait_ref)
        .unwrap_or(trait_ref);

    let unwrap_fundamental = |mut ty: Ty<'db>| {
        // Unwrap all layers of fundamental types with a loop.
        loop {
            match ty.kind() {
                TyKind::Ref(_, referenced, _) => ty = referenced,
                TyKind::Adt(adt_def, subs) => {
                    let AdtId::StructId(s) = adt_def.def_id() else {
                        break ty;
                    };
                    let struct_signature = StructSignature::of(db, s);
                    if struct_signature.flags.contains(StructFlags::FUNDAMENTAL) {
                        let next = subs.types().next();
                        match next {
                            Some(it) => ty = it,
                            None => break ty,
                        }
                    } else {
                        break ty;
                    }
                }
                _ => break ty,
            }
        }
    };
    //   - At least one of the types `T0..=Tn` must be a local type. Let `Ti` be the first such type.

    // FIXME: param coverage
    //   - No uncovered type parameters `P1..=Pn` may appear in `T0..Ti` (excluding `Ti`)
    let is_not_orphan = trait_ref.args.types().any(|ty| match unwrap_fundamental(ty).kind() {
        TyKind::Adt(adt_def, _) => is_local(adt_def.def_id().module(db).krate(db)),
        TyKind::Error(_) => true,
        TyKind::Dynamic(it, _) => {
            it.principal_def_id().is_some_and(|trait_id| is_local(trait_id.0.module(db).krate(db)))
        }
        _ => false,
    });
    #[allow(clippy::let_and_return, reason = "the name clarifies the meaning of the boolean")]
    is_not_orphan
}
