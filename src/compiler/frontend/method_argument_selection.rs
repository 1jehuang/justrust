//! Canonical receiver ordering shared by fixed and inferred method arguments.
use super::*;
use std::borrow::Cow;

pub(super) enum MethodArguments<'a> {
    Missing,
    Deferred,
    Ready(Cow<'a, [ir::Ty]>),
    /// One application of a trait with several in-scope applications
    /// (`Into<T>`), chosen by the caller from the expected result type.
    /// Selection proves exactly this application instead of re-enumerating.
    Applied(ir::TraitRef, Cow<'a, [ir::Ty]>),
}

impl PlanCtx {
    pub(super) fn select_method_with_arguments<'a>(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
        context: (&ModuleId, &ir::PlanBudget),
        arguments: impl FnMut(&ir::Ty, ReceiverKind, bool) -> Result<MethodArguments<'a>, String>,
        realize_signature: impl FnMut(
            &ir::Callee,
            &ir::Signature,
        ) -> Result<Option<ir::Signature>, String>,
    ) -> Result<Option<MethodSelection>, String> {
        self.select_method_with_arguments_scratch(
            receiver,
            name,
            caller,
            context,
            &std::cell::RefCell::new(tr::TraitScratch::default()),
            arguments,
            realize_signature,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_method_with_arguments_scratch<'a>(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
        context: (&ModuleId, &ir::PlanBudget),
        scratch: &std::cell::RefCell<tr::TraitScratch>,
        mut arguments: impl FnMut(&ir::Ty, ReceiverKind, bool) -> Result<MethodArguments<'a>, String>,
        mut realize_signature: impl FnMut(
            &ir::Callee,
            &ir::Signature,
        ) -> Result<Option<ir::Signature>, String>,
    ) -> Result<Option<MethodSelection>, String> {
        let (module, budget) = context;
        if let Some(selection) = self.closure_method_selection(receiver, name, caller)? {
            return Ok(Some(selection));
        }
        // Call syntax can use a declared callable bound through a reference.
        // Resolve that exact bound before inspecting reference blanket impls,
        // whose Args and Output parameters are not inferable from the self head.
        // Preserve the dereference count so normal receiver checking still
        // enforces shared/mutable access and forbids moving a borrowed FnOnce.
        let mut callable_root = receiver;
        for depth in 1..=2 {
            let ir::Ty::Ref { referent, .. } = callable_root else {
                break;
            };
            callable_root = referent;
            budget.charge_work(1)?;
            if let Some(mut selection) =
                self.closure_method_selection(callable_root, name, caller)?
            {
                selection.depth = depth;
                return Ok(Some(selection));
            }
        }
        let mut root = receiver;
        let mut through_shared = false;
        for depth in 0..=2 {
            budget.charge_work(1)?;
            for adjustment in [
                ReceiverAdjustment::Identity,
                ReceiverAdjustment::SharedAutoref,
                ReceiverAdjustment::MutAutoref,
            ] {
                // Autodereferencing a shared reference cannot grant mutable
                // access to its pointee, even if a later method probe fits.
                if through_shared && matches!(adjustment, ReceiverAdjustment::MutAutoref) {
                    continue;
                }
                budget.retain_bytes(128)?;
                let candidate = match adjustment {
                    ReceiverAdjustment::Identity => root.clone(),
                    ReceiverAdjustment::SharedAutoref => ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: Arc::new(root.clone()),
                    },
                    ReceiverAdjustment::MutAutoref => ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        referent: Arc::new(root.clone()),
                    },
                };
                for traits_only in [false, true] {
                    let borrowed = match &candidate {
                        ir::Ty::Ref { kind, referent } => {
                            Some((ReceiverKind::Borrowed { kind: *kind }, referent.as_ref()))
                        }
                        _ => None,
                    };
                    for (mode, owner) in
                        std::iter::once((ReceiverKind::Value { mutable: false }, &candidate))
                            .chain(borrowed)
                    {
                        let found = match arguments(owner, mode, traits_only)? {
                            MethodArguments::Missing => continue,
                            MethodArguments::Deferred => return Ok(None),
                            MethodArguments::Ready(actuals) => self
                                .lookup_associated_with_scratch(
                                    owner,
                                    name,
                                    actuals.as_ref(),
                                    caller,
                                    module,
                                    true,
                                    Some(mode),
                                    traits_only,
                                    budget,
                                    &mut scratch.borrow_mut(),
                                )?,
                            MethodArguments::Applied(application, actuals) => {
                                if !traits_only {
                                    return Err(
                                        "trait application selected for an inherent probe".into()
                                    );
                                }
                                self.select_trait_filtered_with_scratch(
                                    &application,
                                    name,
                                    actuals.as_ref(),
                                    caller,
                                    true,
                                    Some(mode),
                                    budget,
                                    &mut scratch.borrow_mut(),
                                )?
                            }
                        };
                        if let Some((target, signature, mode)) = found {
                            // Selection must inspect the same representation as the
                            // checked receiver. Keep the declaration-owned original
                            // for final call construction and source receipt replay.
                            let (signature, original_signature) =
                                match realize_signature(&target, &signature)? {
                                    Some(mapped) => (mapped, Some(signature)),
                                    None => (signature, None),
                                };
                            if !signature
                                .params
                                .first()
                                .map(|formal| metered_type_equal(formal, &candidate, budget, 0))
                                .transpose()?
                                .unwrap_or(false)
                            {
                                return Err(format!(
                                    "canonical method receiver formal mismatch for `{name:?}`: formal {:?}, receiver {:?}",
                                    signature.params.first(),
                                    candidate
                                ));
                            }
                            ir::root_reference_layers(&candidate)?;
                            return Ok(Some(MethodSelection {
                                target,
                                signature,
                                original_signature,
                                mode,
                                depth,
                                adjustment,
                            }));
                        }
                    }
                }
            }
            match root {
                ir::Ty::Ref { kind, referent } => {
                    through_shared |= *kind == ir::BorrowKind::Shared;
                    root = referent;
                }
                _ => break,
            }
        }
        Ok(None)
    }
}
