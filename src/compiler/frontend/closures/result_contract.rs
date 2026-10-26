//! Exact straight-line required-call result receipts, owned by the parent.
//! This slice deliberately does not infer fixed or nested reference contracts.
use super::super::type_check::Checker;
use super::*;
#[cfg(test)]
#[path = "result_contract_tests.rs"]
mod tests;

fn audit_arguments(
    exprs: &[ir::TypedExpr],
    tail: ir::ExprId,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    budget.charge_work(exprs.len())?;
    let edges = exprs.iter().try_fold(1usize, |total, expression| {
        let count = match &expression.kind {
            ir::ExprKind::Call { args, .. } | ir::ExprKind::Tuple(args) => args.len(),
            ir::ExprKind::Field { .. } | ir::ExprKind::Deref { .. } => 1,
            _ => 0,
        };
        total
            .checked_add(count)
            .ok_or("closure result source-flow resource limit exceeded")
    })?;
    let bytes = edges
        .checked_mul(std::mem::size_of::<ir::ExprId>())
        .and_then(|bytes| bytes.checked_add(exprs.len()))
        .ok_or("closure result source-flow resource limit exceeded")?;
    budget.retain_bytes(bytes)?;
    let mut visited = vec![false; exprs.len()];
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(edges)
        .map_err(|_| "closure result source-flow allocation resource limit")?;
    pending.push(tail);
    while let Some(id) = pending.pop() {
        budget.charge_work(1)?;
        let seen = visited
            .get_mut(id.0)
            .ok_or("closure result source has an invalid expression")?;
        if *seen {
            continue;
        }
        *seen = true;
        let expression = &exprs[id.0];
        match &expression.kind {
            ir::ExprKind::Call { args, .. } | ir::ExprKind::Tuple(args) => {
                budget.charge_work(args.len())?;
                pending.extend(args.iter().copied());
            }
            ir::ExprKind::Field { base, .. } => pending.push(*base),
            ir::ExprKind::Deref { value } => pending.push(*value),
            ir::ExprKind::Local(_) | ir::ExprKind::Borrow { .. } => {},
            _ => return Err("unsupported: closure restricted result requires straight-line call argument source flow".into()),
        }
        if pending.len() > edges {
            return Err("closure result source-flow resource limit exceeded".into());
        }
    }
    Ok(())
}

impl Checker<'_> {
    pub(super) fn restricted_closure_result(
        &self,
        owner: ir::DeclId,
        provider: ir::DeclId,
        site: u32,
        body: &ir::TypedBlock,
        result: &ir::Ty,
        prefix: &[ir::TypedStmt],
        tuple: ir::LocalId,
    ) -> Result<Option<Arc<ir::ClosureResultReceipt>>, String> {
        let plan = self.signatures.plan()?;
        let locations = ir::carrier_locations(result, &plan.budget)?;
        if locations.is_empty() {
            return Ok(None);
        }
        if matches!(result, ir::Ty::Ref { kind: ir::BorrowKind::Shared, referent } if matches!(referent.as_ref(), ir::Ty::Str))
        {
            let tail = body.tail.ok_or("closure argument result has no tail")?;
            let facts = self
                .facts()
                .validated()
                .ok_or("closure argument result requires canonical facts")?;
            let (declaration, borrow, place, argument, depth) = argument_str_origin(
                &self.exprs,
                &self.locals,
                prefix,
                &body.statements,
                tuple,
                tail,
                facts,
            )?;
            if !body.statements.is_empty() {
                return Err(
                    "unsupported: closure argument reference result requires a straight-line body"
                        .into(),
                );
            }
            plan.budget.charge_work(self.call_evidence.len())?;
            if !self.call_evidence.iter().any(|trace| trace.call == tail) {
                return Err(
                    "closure argument result lacks original call occurrence evidence".into(),
                );
            }
            return Ok(Some(Arc::new(ir::ClosureResultReceipt::argument_str(
                owner,
                provider,
                site,
                tail,
                declaration,
                argument,
                place.local,
                tuple,
                borrow,
                place,
                depth,
                &plan.budget,
            )?)));
        }
        if !body.statements.is_empty()
            || !matches!(result, ir::Ty::Param(_))
            || locations.len() != 1
            || !locations[0].path.0.is_empty()
            || !matches!(
                locations[0].channel,
                ir::CarrierChannel::ParamTail { prefix: 0, .. }
            )
        {
            return Err(
                "unsupported: closure reference results require checked capture carrier contracts"
                    .into(),
            );
        }
        let tail = body
            .tail
            .ok_or("closure restricted result has no source tail")?;
        let ir::ExprKind::Call {
            target: ir::Callee::TraitItem(call),
            args,
            signature,
            ..
        } = &self
            .exprs
            .get(tail.0)
            .ok_or("closure result tail is invalid")?
            .kind
        else {
            return Err(
                "unsupported: closure symbolic result needs an exact required-call result origin"
                    .into(),
            );
        };
        audit_arguments(&self.exprs, tail, &plan.budget)?;
        if !self.call_evidence.iter().any(|trace| trace.call == tail) {
            return Err("closure result call has no original argument occurrence evidence".into());
        }
        let facts = self
            .facts()
            .validated()
            .ok_or("closure result lacks canonical caller facts")?;
        let callee = ir::Callee::TraitItem(call.clone());
        let view = facts
            .carrier_contract(ir::SignatureSubject::Call(&callee))
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
        view.validate_restricted_result(
            facts,
            ir::SignatureSubject::Call(&callee),
            signature,
            &locations[0],
        )
        .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
        plan.budget.retain_bytes(
            std::mem::size_of::<ir::ClosureResultReceipt>()
                + args.len() * std::mem::size_of::<ir::ExprId>(),
        )?;
        Ok(Some(Arc::new(ir::ClosureResultReceipt::new(
            owner,
            provider,
            site,
            tail,
            call.clone(),
            args.clone().into(),
            signature.clone(),
            &plan.budget,
        )?)))
    }
}

/// Replay the original occurrence against the actual mode-specific body. The
/// surrounding publication factory still checks the whole typed source.
pub(in crate::compiler) fn validate_source(
    receipt: &ir::ClosureResultReceipt,
    source: &ir::CheckedFunction,
    facts: &tr::ValidatedFacts,
) -> Result<(), String> {
    let budget = facts.budget();
    if source.params.len() != 2 || source.body.tail != Some(receipt.tail()) {
        return Err("closure result changed its original parameter or tail occurrence".into());
    }
    if let Some(origin) = receipt.argument_origin() {
        let (closure, _) = facts
            .closure_lowering_identity()
            .ok_or("closure argument result lacks actual closure body authority")?;
        if closure.owner() != receipt.owner()
            || closure.provider() != receipt.provider()
            || closure.site() != receipt.site()
            || closure
                .result_receipt()
                .is_none_or(|original| !original.same_source_shape(receipt))
        {
            return Err(
                "closure argument result has foreign parent, site or receipt authority".into(),
            );
        }
        budget.charge_work(source.call_evidence.len())?;
        let (declaration, borrow, place, argument, depth) = argument_str_origin(
            &source.exprs,
            &source.locals,
            &source.body.statements,
            &source.body.statements,
            source.params[1],
            receipt.tail(),
            facts,
        )?;
        let Some(ir::Callee::Direct(expected_call)) = receipt.argument_callee(budget)? else {
            return Err("closure argument result lost its exact direct callee".into());
        };
        if expected_call.declaration != declaration
            || !expected_call.type_args.is_empty()
            || borrow != origin.borrow
            || place != origin.place
            || place.local != origin.local
            || argument != origin.argument
            || depth != origin.depth
            || source.params[1] != origin.tuple
            || !matches!(&source.result, ir::Ty::Ref { kind: ir::BorrowKind::Shared, referent } if matches!(referent.as_ref(), ir::Ty::Str))
            || !source
                .call_evidence
                .iter()
                .any(|trace| trace.call == receipt.tail())
        {
            return Err("closure argument result changed its exact source or tuple channel".into());
        }
        return Ok(());
    }
    // The source slice itself has no statements. Only the ordinary generated
    // parameter-entry lets may precede its original tail after capture rewrite.
    for statement in &source.body.statements {
        let value = match statement {
            ir::TypedStmt::Let { value, .. } | ir::TypedStmt::LetPattern { value, .. } => *value,
            _ => {
                return Err(
                    "unsupported: closure result source has an unproved early return or statement"
                        .into(),
                );
            }
        };
        let Some(ir::TypedExpr {
            kind: ir::ExprKind::Field { base, .. },
            ..
        }) = source.exprs.get(value.0)
        else {
            return Err(
                "closure result parameter entry changed its original tuple projection".into(),
            );
        };
        if !matches!(source.exprs.get(base.0).map(|expr| &expr.kind), Some(ir::ExprKind::Local(local)) if *local == source.params[1])
        {
            return Err("closure result parameter entry belongs to another argument tuple".into());
        }
    }
    let Some(ir::TypedExpr {
        kind:
            ir::ExprKind::Call {
                target: ir::Callee::TraitItem(call),
                args,
                signature,
                ..
            },
        ..
    }) = source.exprs.get(receipt.tail().0)
    else {
        return Err("closure result lost its original required-call occurrence".into());
    };
    let (expected_call, expected_result) = facts
        .closure_result_witness(receipt)
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    if call != &expected_call
        || source.result != expected_result
        || args.as_slice() != receipt.arguments()
        || !source
            .call_evidence
            .iter()
            .any(|trace| trace.call == receipt.tail())
    {
        return Err(
            "closure result changed its original required call or argument occurrences".into(),
        );
    }
    audit_arguments(&source.exprs, receipt.tail(), budget)?;
    let callee = ir::Callee::TraitItem(call.clone());
    let view = facts
        .carrier_contract(ir::SignatureSubject::Call(&callee))
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    for location in ir::carrier_locations(&source.result, budget)?.iter() {
        view.validate_restricted_result(
            facts,
            ir::SignatureSubject::Call(&callee),
            signature,
            location,
        )
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    }
    Ok(())
}

/// Deliberately bounded: a checked direct shared-reference call whose receiver
/// borrows a field reached exclusively through a real shared input parameter.
/// No captured storage, local-owned temporary, static or inferred type match
/// can provide this origin.
fn argument_str_origin(
    exprs: &[ir::TypedExpr],
    locals: &[ir::LocalDecl],
    prefix: &[ir::TypedStmt],
    statements: &[ir::TypedStmt],
    tuple: ir::LocalId,
    tail: ir::ExprId,
    facts: &tr::ValidatedFacts,
) -> Result<(ir::DeclId, ir::ExprId, ir::Place, usize, u8), String> {
    let budget = facts.budget();
    budget.charge_work(
        prefix
            .len()
            .checked_add(1)
            .ok_or("closure argument origin work overflow")?,
    )?;
    let Some(ir::TypedExpr {
        kind:
            ir::ExprKind::Call {
                target: target @ ir::Callee::Direct(call),
                args,
                signature,
                ..
            },
        ..
    }) = exprs.get(tail.0)
    else {
        return Err(
            "unsupported: closure reference result needs an exact direct call origin".into(),
        );
    };
    if !call.type_args.is_empty()
        || args.len() != 1
        || !matches!(&signature.result, ir::Ty::Ref { kind: ir::BorrowKind::Shared, referent } if matches!(referent.as_ref(), ir::Ty::Str))
    {
        return Err("unsupported: closure argument reference result call shape".into());
    }
    facts
        .validate_signature(ir::SignatureSubject::Call(target), signature)
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    let view = facts
        .carrier_contract(ir::SignatureSubject::Call(target))
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    view.validate_for(facts)
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    budget.retain_bytes(2 * std::mem::size_of::<usize>())?;
    let root = ir::CarrierLocation {
        path: ir::CarrierPath(Arc::from([])),
        channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
    };
    let expected = ir::InputCarrier {
        param: 0,
        location: root.clone(),
    };
    if view.contract().results.len() != 1
        || view.contract().results[0].location != root
        || view.contract().results[0].sources.as_ref() != [expected]
    {
        return Err(
            "closure argument result lacks an exact checked receiver return contract".into(),
        );
    }
    let borrow = args[0];
    let Some(ir::TypedExpr {
        kind:
            ir::ExprKind::Borrow {
                kind: ir::BorrowKind::Shared,
                place,
                timing: ir::BorrowTiming::Immediate,
            },
        ..
    }) = exprs.get(borrow.0)
    else {
        return Err("unsupported: closure argument result must borrow its input place".into());
    };
    let mut argument = None;
    for statement in prefix {
        let ir::TypedStmt::Let { local, value } = statement else {
            return Err("unsupported: closure argument reference result parameter pattern".into());
        };
        let Some(ir::TypedExpr {
            kind: ir::ExprKind::Field { base, index },
            ..
        }) = exprs.get(value.0)
        else {
            return Err("closure argument result changed parameter entry".into());
        };
        if !matches!(exprs.get(base.0).map(|e| &e.kind), Some(ir::ExprKind::Local(actual)) if *actual == tuple)
        {
            return Err("closure argument result parameter belongs to another tuple".into());
        }
        if *local == place.local {
            if argument.replace(*index).is_some() {
                return Err("closure argument result repeats parameter entry".into());
            }
        }
    }
    let fields_only = place
        .projection
        .iter()
        .all(|part| matches!(part, ir::Projection::Field(_)));
    budget.charge_work(
        statements
            .len()
            .checked_add(place.projection.len())
            .ok_or("closure argument escape lookup overflow")?,
    )?;
    let argument = match argument {
        Some(argument) => argument,
        None if fields_only && statements.iter().any(|statement| matches!(statement,
            ir::TypedStmt::Let { local, .. } | ir::TypedStmt::Assign { local, .. } if *local == place.local)) => {
                return Err("closure reference result borrows local-owned storage".into());
            }
        None => return Err("unsupported: closure reference result originates in capture or local storage".into()),
    };
    let mut ty = locals
        .get(place.local.0)
        .ok_or("closure argument local is invalid")?
        .ty
        .clone();
    if fields_only {
        return Err("closure reference result borrows owned argument storage".into());
    }
    let mut derefs = 0usize;
    for (position, projection) in place.projection.iter().enumerate() {
        budget.charge_work(1)?;
        match (projection, &ty) {
            (
                ir::Projection::Deref,
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent,
                },
            ) => {
                if derefs != position {
                    return Err(
                        "unsupported: closure argument result reference inside field".into(),
                    );
                }
                derefs = derefs
                    .checked_add(1)
                    .ok_or("closure argument reference depth overflow")?;
                ty = referent.as_ref().clone();
            }
            (ir::Projection::Field(index), _) if derefs != 0 => {
                ty = ty
                    .field_ty(*index, budget)?
                    .ok_or("closure argument field is invalid")?;
                if matches!(ty, ir::Ty::Ref { .. }) {
                    return Err(
                        "unsupported: closure argument result reference-valued field".into(),
                    );
                }
            }
            _ => return Err("unsupported: closure argument result projection".into()),
        }
    }
    let depth = u8::try_from(
        derefs
            .checked_sub(1)
            .ok_or("closure reference result cannot borrow an owned argument")?,
    )
    .map_err(|_| "closure argument reference depth overflow")?;
    budget.retain_bytes(
        place
            .projection
            .len()
            .checked_mul(std::mem::size_of::<ir::Projection>())
            .ok_or("closure argument projection allocation overflow")?,
    )?;
    Ok((call.declaration, borrow, place.clone(), argument, depth))
}
