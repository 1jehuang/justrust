//! Canonical receiver ordering shared by fixed and inferred method arguments.
use super::*;
use std::borrow::Cow;

pub(super) enum MethodArguments<'a> {
    Missing,
    Deferred,
    Ready(Cow<'a, [ir::Ty]>),
}

impl PlanCtx {
    pub(super) fn select_method_with_arguments<'a>(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
        module: &ModuleId,
        budget: &ir::PlanBudget,
        mut arguments: impl FnMut(&ir::Ty, ReceiverKind, bool) -> Result<MethodArguments<'a>, String>,
        mut realize_signature: impl FnMut(
            &ir::Callee,
            &ir::Signature,
        ) -> Result<Option<ir::Signature>, String>,
    ) -> Result<Option<MethodSelection>, String> {
        if let Some(selection) = self.closure_method_selection(receiver, name, caller)? {
            return Ok(Some(selection));
        }
        let mut root = receiver;
        for depth in 0..=2 {
            budget.charge_work(1)?;
            for adjustment in [
                ReceiverAdjustment::Identity,
                ReceiverAdjustment::SharedAutoref,
                ReceiverAdjustment::MutAutoref,
            ] {
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
                        let actuals = match arguments(owner, mode, traits_only)? {
                            MethodArguments::Missing => continue,
                            MethodArguments::Deferred => return Ok(None),
                            MethodArguments::Ready(actuals) => actuals,
                        };
                        if let Some((target, signature, mode)) = self.lookup_associated(
                            owner,
                            name,
                            actuals.as_ref(),
                            caller,
                            module,
                            true,
                            Some(mode),
                            traits_only,
                            budget,
                        )? {
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
                                return Err("canonical method receiver formal mismatch".into());
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
                ir::Ty::Ref { referent, .. } => root = referent,
                _ => break,
            }
        }
        Ok(None)
    }
}
