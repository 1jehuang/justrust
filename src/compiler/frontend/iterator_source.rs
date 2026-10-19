//! Implicit IntoIterator::into_iter and Iterator::next occurrences belong to
//! the original for statement, not to fabricated method-call syntax.
use super::*;

impl SourceInventory {
    pub(super) fn for_iterator_sites(
        &mut self,
        source: &ForSource,
        node: &sr::SourceNode,
        budget: &ir::PlanBudget,
    ) -> Result<(), FactsError> {
        let first = self.calls.len();
        for operation in [5, 6] {
            sr::push(
                &mut self.calls,
                SourceCallSite {
                    node: node.child(24, operation, budget)?,
                    len_candidate: false,
                    for_iterable: false,
                    leading_zeros_candidate: false,
                    intrinsic_candidate: false,
                    operator_candidate: true,
                    ordinary: None,
                    application: None,
                    argument_binding: None,
                    argument_borrow: None,
                    direct_let: None,
                    args_pointer: 0,
                },
                budget,
            )?;
        }
        insert_source_pointer(
            &mut self.iterator_pointers,
            source as *const ForSource as usize,
            first,
            budget,
        )
    }
}

/// A genuine for occurrence has exactly one complete desugaring. Optional
/// implicit sites must not let a forged body omit its length or either call.
pub(super) fn validate_for_alternatives(
    original: &OriginalSource,
    positions: &[Option<usize>],
    checked: &ir::CheckedFunction,
    builtin_lens: &[bool],
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    for (index, site) in original.calls.iter().enumerate() {
        sr::work(budget, 1)?;
        if !site.for_iterable {
            continue;
        }
        let first = index.checked_sub(2).ok_or_else(|| {
            FactsError::invalid(budget, "for occurrence lost implicit iterator sites")
        })?;
        let into = positions[first];
        let next = positions[first + 1];
        match (builtin_lens[index], into, next) {
            (true, None, None) => {}
            (false, Some(into), Some(next)) => {
                let canonical_into = original
                    .plan
                    .into_iterator_method()
                    .map_err(|error| source_error(budget, &error))?
                    .ok_or_else(|| {
                        FactsError::invalid(budget, "for call has no canonical IntoIterator")
                    })?;
                let canonical_next = original
                    .plan
                    .iterator_next_method()
                    .map_err(|error| source_error(budget, &error))?
                    .ok_or_else(|| {
                        FactsError::invalid(budget, "for call has no canonical Iterator")
                    })?;
                let into_expr = original.call_mapping[into].1;
                let Some(ir::TypedExpr {
                    kind: ir::ExprKind::Call { signature, .. },
                    ..
                }) = checked.exprs.get(into_expr.0)
                else {
                    return Err(FactsError::invalid(
                        budget,
                        "for into_iter is not a checked call",
                    ));
                };
                let [receiver] = signature.params.as_slice() else {
                    return Err(FactsError::invalid(
                        budget,
                        "for into_iter changed receiver arity",
                    ));
                };
                let iterator = original
                    .plan
                    .iterator_into_iter_type(receiver, ir::FactsCtx::Validated(&original.facts))
                    .map_err(|error| source_error(budget, &error))?
                    .ok_or_else(|| {
                        FactsError::invalid(budget, "for receiver has no authoritative IntoIter")
                    })?;
                for (position, (owner, _, method), self_ty) in [
                    (into, canonical_into, receiver.clone()),
                    (next, canonical_next, iterator),
                ] {
                    let call = ir::TraitCall {
                        trait_ref: ir::TraitRef {
                            declaration: owner,
                            self_ty,
                            args: Arc::from([]),
                        },
                        item: method,
                        method_args: Arc::from([]),
                    };
                    let resolved = original
                        .facts
                        .service()
                        .resolve_trait_call(&call, &original.facts)?;
                    validate_original_callee(
                        resolved.callee(),
                        &original.call_mapping[position].2,
                        budget,
                    )?;
                }
            }
            _ => {
                return Err(FactsError::invalid(
                    budget,
                    "for occurrence has incomplete or mixed desugaring",
                ));
            }
        }
    }
    Ok(())
}

impl CanonicalSourceCapability {
    pub(super) fn record_for_iterator_call(
        &self,
        source: &ForSource,
        next: bool,
        id: ir::ExprId,
        kind: &ir::ExprKind,
    ) -> Result<(), String> {
        let budget = &self.plan.budget;
        sr::work(budget, 1).map_err(|error| self.render(error))?;
        let first = self
            .inventory
            .iterator_pointers
            .get(&(source as *const ForSource as usize))
            .copied()
            .ok_or("implicit iterator call has no original for occurrence")?;
        let index = first
            .checked_add(usize::from(next))
            .ok_or("implicit iterator call occurrence overflow")?;
        let ir::ExprKind::Call { target, .. } = kind else {
            return Err("implicit iterator operation did not produce a call".into());
        };
        sr::retain(budget, std::mem::size_of::<ir::Callee>())
            .map_err(|error| self.render(error))?;
        sr::push(
            &mut self.calls.borrow_mut(),
            (index, id, target.clone()),
            budget,
        )
        .map_err(|error| self.render(error))
    }
}
