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
    ) -> Result<Option<Arc<ir::ClosureResultReceipt>>, String> {
        let plan = self.signatures.plan()?;
        let locations = ir::carrier_locations(result, &plan.budget)?;
        if locations.is_empty() {
            return Ok(None);
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
