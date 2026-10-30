//! Handling of trait solver errors and converting them to errors `hir` can pass to `ide-diagnostics`.
//!
//! Note that we also have [`crate::next_solver::infer::errors`], which takes the raw [`NextSolverError`],
//! and converts it into [`FulfillmentError`] that contains more details.
//!
//! [`NextSolverError`]: crate::next_solver::fulfill::NextSolverError

use macros::{TypeFoldable, TypeVisitable};
use rustc_type_ir::{
    ClosureKind, Interner, PredicatePolarity,
    inherent::{GenericArgs as _, IntoKind, SliceLike},
    lang_items::SolverTraitLangItem,
};

use crate::{
    Span,
    next_solver::{
        ClauseKind, DbInterner, PolyTraitPredicate, PredicateKind, StoredTraitRef, TraitPredicate,
        TyKind,
        infer::{
            InferCtxt,
            errors::{FulfillmentError, FulfillmentErrorCode},
            select::SelectionError,
        },
    },
};

#[derive(Debug, Clone, PartialEq, Eq, TypeVisitable, TypeFoldable)]
pub struct SolverDiagnostic {
    pub span: Span,
    pub kind: SolverDiagnosticKind,
}

#[derive(Debug, Clone, PartialEq, Eq, TypeVisitable, TypeFoldable)]
pub enum SolverDiagnosticKind {
    TraitUnimplemented {
        trait_predicate: StoredTraitPredicate,
        parent_trait_predicates: Vec<StoredTraitPredicate>,
    },
    /// A closure does not implement the `Fn*` or `AsyncFn*` trait asked of it because of its
    /// closure kind (E0525).
    ClosureKindMismatch {
        #[type_visitable(ignore)]
        expected: ClosureKind,
        #[type_visitable(ignore)]
        found: ClosureKind,
        #[type_visitable(ignore)]
        is_async: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, TypeVisitable, TypeFoldable)]
pub struct StoredTraitPredicate {
    pub trait_ref: StoredTraitRef,
    pub polarity: PredicatePolarity,
}

impl StoredTraitPredicate {
    #[inline]
    pub fn get<'db>(&'db self, interner: DbInterner<'db>) -> TraitPredicate<'db> {
        TraitPredicate { polarity: self.polarity, trait_ref: self.trait_ref.get(interner) }
    }
}

impl SolverDiagnostic {
    pub fn from_fulfillment_error<'db>(
        infcx: &InferCtxt<'db>,
        error: &FulfillmentError<'db>,
    ) -> Option<Self> {
        let span = error.obligation.cause.span();
        if span.is_dummy() {
            return None;
        }

        // FIXME: Handle more error kinds.
        let kind = match &error.code {
            FulfillmentErrorCode::Select(SelectionError::Unimplemented) => {
                match error.obligation.predicate.kind().skip_binder() {
                    PredicateKind::Clause(ClauseKind::Trait(trait_pred)) => {
                        let poly_trait_pred = error.obligation.predicate.kind().rebind(trait_pred);
                        match closure_kind_mismatch(infcx, error, poly_trait_pred) {
                            Some(kind) => kind,
                            None => handle_trait_unimplemented(error, trait_pred)?,
                        }
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(SolverDiagnostic { span, kind })
    }
}

/// rustc's `emit_specialized_closure_kind_error`: the closure kind of a closure that does not
/// implement a `Fn*` or `AsyncFn*` trait, when that is why.
fn closure_kind_mismatch<'db>(
    infcx: &InferCtxt<'db>,
    error: &FulfillmentError<'db>,
    trait_pred: PolyTraitPredicate<'db>,
) -> Option<SolverDiagnosticKind> {
    let interner = infcx.interner;
    let mut trait_pred = infcx.resolve_vars_if_possible(trait_pred);
    // An `AsyncFnKindHelper` goal comes from the `AsyncFn*` goal it is required by.
    if interner.is_trait_lang_item(trait_pred.def_id(), SolverTraitLangItem::AsyncFnKindHelper)
        && let Some(parent) =
            error.parent_trait_obligations.last().and_then(|parent| parent.as_trait_clause())
    {
        trait_pred = infcx.resolve_vars_if_possible(parent);
    }

    let original_self_ty = trait_pred.self_ty().skip_binder();
    let self_ty = match original_self_ty.kind() {
        TyKind::Ref(_, inner, _) if matches!(inner.kind(), TyKind::Closure(..)) => inner,
        _ => original_self_ty,
    };

    let (expected, is_async) = match interner.as_trait_lang_item(trait_pred.def_id())? {
        SolverTraitLangItem::Fn => (ClosureKind::Fn, false),
        SolverTraitLangItem::FnMut => (ClosureKind::FnMut, false),
        SolverTraitLangItem::FnOnce => (ClosureKind::FnOnce, false),
        SolverTraitLangItem::AsyncFn => (ClosureKind::Fn, true),
        SolverTraitLangItem::AsyncFnMut => (ClosureKind::FnMut, true),
        SolverTraitLangItem::AsyncFnOnce => (ClosureKind::FnOnce, true),
        _ => return None,
    };

    let found_args = match self_ty.kind() {
        TyKind::Closure(_, args) => {
            args.as_closure().sig().map_bound(|sig| sig.inputs().as_slice()[0])
        }
        TyKind::CoroutineClosure(_, args) => args
            .as_coroutine_closure()
            .coroutine_closure_sig()
            .map_bound(|sig| sig.tupled_inputs_ty),
        _ => return None,
    };
    let expected_args = trait_pred.map_bound(|trait_pred| trait_pred.trait_ref.args.type_at(1));
    // With arguments of other types, the closure is not what was asked for at all.
    if infcx.enter_forall(found_args, |found_args| {
        infcx.enter_forall(expected_args, |expected_args| {
            !infcx.can_eq(error.obligation.param_env, expected_args, found_args)
        })
    }) {
        return None;
    }

    let found = infcx.closure_kind(self_ty)?;
    (!found.extends(expected)).then_some(SolverDiagnosticKind::ClosureKindMismatch {
        expected,
        found,
        is_async,
    })
}

fn handle_trait_unimplemented<'db>(
    error: &FulfillmentError<'db>,
    trait_pred: TraitPredicate<'db>,
) -> Option<SolverDiagnosticKind> {
    let trait_predicate = StoredTraitPredicate {
        trait_ref: StoredTraitRef::new(trait_pred.trait_ref),
        polarity: trait_pred.polarity,
    };

    let mut parent_trait_predicates = error
        .parent_trait_obligations
        .iter()
        .filter_map(|predicate| predicate.as_trait_clause())
        .map(|trait_predicate| {
            let trait_predicate = trait_predicate.skip_binder();
            StoredTraitPredicate {
                trait_ref: StoredTraitRef::new(trait_predicate.trait_ref),
                polarity: trait_predicate.polarity,
            }
        })
        .collect::<Vec<_>>();
    parent_trait_predicates.reverse();

    Some(SolverDiagnosticKind::TraitUnimplemented { trait_predicate, parent_trait_predicates })
}
