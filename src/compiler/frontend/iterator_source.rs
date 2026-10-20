//! Implicit IntoIterator::into_iter and Iterator::next occurrences belong to
//! the original for statement, not to fabricated method-call syntax.
use super::*;

/// A candidate only. Receiver typing and ordinary method selection must still
/// establish whether this is the builtin immediate-for array iterator.
pub(super) fn array_iter_candidate<'a>(
    expression: &'a Expr,
    names: &NameStore,
) -> Result<Option<(&'a Expr, AstIdent)>, String> {
    let Expr::MethodCall {
        receiver,
        name,
        type_args,
        args,
    } = expression
    else {
        return Ok(None);
    };
    Ok((type_args.is_empty()
        && args.is_empty()
        && names
            .spelling(*name)
            .map_err(native_adapter::name_diagnostic)?
            == "iter")
        .then_some((receiver.as_ref(), *name)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ArrayIterReceiver {
    Borrow {
        place: ir::Place,
        timing: ir::BorrowTiming,
    },
    Local(ir::LocalId),
    Temporary {
        local: ir::LocalId,
        value: ir::ExprId,
        borrow: ir::ExprId,
        place: ir::Place,
        timing: ir::BorrowTiming,
    },
}

#[derive(Clone, Debug)]
pub(super) struct ArrayIterOccurrence {
    source_index: usize,
    for_index: usize,
    receiver: ir::ExprId,
    shape: ArrayIterReceiver,
}

fn array_iter_receiver_shape(
    checked: &[ir::TypedExpr],
    id: ir::ExprId,
    budget: &ir::PlanBudget,
) -> Result<ArrayIterReceiver, String> {
    budget.charge_work(1)?;
    let expression = checked
        .get(id.0)
        .ok_or("array iterator receiver outside checked arena")?;
    if !matches!(&expression.ty, ir::Ty::Ref { kind: ir::BorrowKind::Shared, referent } if matches!(referent.as_ref(), ir::Ty::Array { .. }))
    {
        return Err("array iterator receiver must retain a shared array reference".into());
    }
    let retain_place = |place: &ir::Place| -> Result<ir::Place, String> {
        budget.charge_work(place.projection.len())?;
        budget.retain_bytes(
            place
                .projection
                .len()
                .checked_mul(std::mem::size_of::<ir::Projection>())
                .ok_or("array iterator place size overflow")?,
        )?;
        Ok(place.clone())
    };
    match &expression.kind {
        ir::ExprKind::Borrow {
            kind: ir::BorrowKind::Shared,
            place,
            timing,
        } => Ok(ArrayIterReceiver::Borrow {
            place: retain_place(place)?,
            timing: *timing,
        }),
        ir::ExprKind::Local(local) => Ok(ArrayIterReceiver::Local(*local)),
        ir::ExprKind::Block(block) => {
            let ([ir::TypedStmt::Assign { local, value }], Some(borrow)) =
                (block.statements.as_slice(), block.tail)
            else {
                return Err("unsupported: array iterator reference transport".into());
            };
            let Some(ir::TypedExpr {
                kind:
                    ir::ExprKind::Borrow {
                        kind: ir::BorrowKind::Shared,
                        place,
                        timing,
                    },
                ..
            }) = checked.get(borrow.0)
            else {
                return Err("array iterator temporary lacks its shared borrow".into());
            };
            if place.local != *local {
                return Err("array iterator temporary changed its borrowed owner".into());
            }
            Ok(ArrayIterReceiver::Temporary {
                local: *local,
                value: *value,
                borrow,
                place: retain_place(place)?,
                timing: *timing,
            })
        }
        _ => Err("unsupported: array iterator reference transport".into()),
    }
}

impl CanonicalSourceCapability {
    pub(super) fn record_array_iter(
        &self,
        source: &ForSource,
        receiver: ir::ExprId,
        checked: &[ir::TypedExpr],
    ) -> Result<(), String> {
        let budget = &self.plan.budget;
        let for_index = *self
            .inventory
            .for_pointers
            .get(&(source as *const ForSource as usize))
            .ok_or("array iterator has no original for occurrence")?;
        let source_index = self.inventory.calls[for_index]
            .array_iter_source
            .ok_or("array iterator has no original direct method occurrence")?;
        let shape = array_iter_receiver_shape(checked, receiver, budget)?;
        sr::push(
            &mut self.array_iters.borrow_mut(),
            ArrayIterOccurrence {
                source_index,
                for_index,
                receiver,
                shape,
            },
            budget,
        )
        .map_err(|error| self.render(error))
    }
}

pub(super) fn replay_array_iters(
    original: &OriginalSource,
    checked: &ir::CheckedFunction,
    ordinary: &[Option<usize>],
    budget: &ir::PlanBudget,
) -> Result<Vec<bool>, FactsError> {
    sr::work(budget, original.calls.len())?;
    sr::retain(budget, original.calls.len())?;
    let mut sources = vec![false; original.calls.len()];
    let mut previous = None;
    for row in &original.array_iter_mapping {
        sr::work(budget, 1)?;
        let site = original
            .calls
            .get(row.for_index)
            .ok_or_else(|| FactsError::invalid(budget, "orphan array iterator for occurrence"))?;
        if !site.for_iterable
            || site.array_iter_source != Some(row.source_index)
            || row.source_index >= row.for_index
            || ordinary.get(row.source_index).copied().flatten().is_some()
            || std::mem::replace(&mut sources[row.source_index], true)
            || previous.is_some_and(|(source, receiver)| {
                source >= row.source_index || receiver >= row.receiver.0
            })
        {
            return Err(FactsError::invalid(
                budget,
                "array iterator source correspondence changed",
            ));
        }
        previous = Some((row.source_index, row.receiver.0));
        sr::work(
            budget,
            original.builtin_len_mapping.len().max(1).ilog2() as usize + 1,
        )?;
        let length = original
            .builtin_len_mapping
            .binary_search_by_key(&row.for_index, |length| length.source_index)
            .ok()
            .map(|index| original.builtin_len_mapping[index].receiver);
        if length != Some(row.receiver) {
            return Err(FactsError::invalid(
                budget,
                "array iterator lost its exact loop receiver",
            ));
        }
        let shape =
            array_iter_receiver_shape(&checked.exprs, row.receiver, budget).map_err(|error| {
                if budget.has_resource_failure() {
                    source_error(budget, &error)
                } else {
                    FactsError::invalid(budget, &error)
                }
            })?;
        if shape != row.shape {
            return Err(FactsError::invalid(
                budget,
                "array iterator shared borrow changed",
            ));
        }
    }
    Ok(sources)
}

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
                    array_iter_source: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn published() -> (ir::PlanBudget, Arc<sr::CanonicalSourcePublication>) {
        let program =
            parse("fn f(){let a=[1i32,2];for e in a.iter(){let x=*e;}}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        match plan.publish_callable_body(declaration, &budget).unwrap() {
            PublishedCallableBody::Source(body) => (budget, body),
            _ => panic!("expected canonical source"),
        }
    }

    fn invalid<T>(result: Result<T, FactsError>) {
        assert!(
            matches!(result, Err(error) if error.kind() == tr::FactsErrorKind::InvalidMetadata)
        );
    }

    #[test]
    fn array_iter_receipt_requires_original_call_and_exact_loop_borrow() {
        let (budget, publication) = published();
        let original = publication.original().source_for_test();
        let replay = |source: &OriginalSource| {
            replay_source_body(source, publication.source(), publication.facts(), &budget)
        };
        replay(&original).unwrap();
        assert_eq!(original.array_iter_mapping.len(), 1);
        let row = &original.array_iter_mapping[0];
        let mut missing = original.clone();
        missing.array_iter_mapping.clear();
        invalid(replay(&missing));
        let mut duplicate = original.clone();
        duplicate.array_iter_mapping.push(row.clone());
        invalid(replay(&duplicate));
        let mut wrong_source = original.clone();
        wrong_source.array_iter_mapping[0].source_index = row.for_index;
        invalid(replay(&wrong_source));
        let mut detached = original.clone();
        detached.calls[row.for_index].array_iter_source = None;
        invalid(replay(&detached));
        let mut receiver = publication.source().as_ref().clone();
        let ir::ExprKind::Borrow { kind, .. } = &mut receiver.exprs[row.receiver.0].kind else {
            panic!("expected shared borrow")
        };
        *kind = ir::BorrowKind::Mut;
        invalid(replay_source_body(
            &original,
            &Arc::new(receiver),
            publication.facts(),
            &budget,
        ));
        let mut wrong_receiver = original.clone();
        wrong_receiver.array_iter_mapping[0].receiver = ir::ExprId(usize::MAX);
        invalid(replay(&wrong_receiver));
    }
}
