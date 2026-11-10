//! Closed call signature conversion over exact current-engine nominal reads.
use super::*;

pub(super) fn realize(
    context: Option<&frontend::PlanCtx>,
    engine: EngineAuthority,
    target: &ir::Callee,
    signatures: (&ir::Signature, &ir::Signature),
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<ir::Signature, String> {
    let (canonical, mapped) = signatures;
    let facts = caller
        .validated()
        .ok_or("closed call realization requires caller facts")?;
    if facts.phase() != crate::compiler::traits::FactsPhase::Closed
        || !facts.budget().same_authority(budget)
    {
        return Err("closed call realization requires closed caller authority".into());
    }
    for signature in [canonical, mapped] {
        facts
            .validate_source_flow_signature_with_scratch(
                ir::SignatureSubject::Call(target),
                signature,
                scratch,
            )
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
    }
    // Source schema authentication is not executable authority. Every mapped
    // root must independently have legal physical storage, and pair below
    // replays each graph capability against this exact current engine.
    for ty in mapped.params.iter().chain(std::iter::once(&mapped.result)) {
        caller.validate_ty_with_budget_scratch(ty, budget, scratch)?;
    }
    target.for_each_value_metadata(caller, |ty| {
        caller.validate_ty_with_budget_scratch(ty, budget, scratch)?;
        pair(context, engine, ty, ty, caller, budget, scratch, 0)
    })?;
    if canonical.params.len() != mapped.params.len()
        || canonical.ref_contract != mapped.ref_contract
    {
        return Err("closed call realization changed contract or arity".into());
    }
    for (source, output) in canonical
        .params
        .iter()
        .zip(&mapped.params)
        .chain(std::iter::once((&canonical.result, &mapped.result)))
    {
        pair(context, engine, source, output, caller, budget, scratch, 0)?;
    }
    let mut output = mapped.clone();
    // The resolved canonical Call proof owns the authoritative closed carrier
    // contract. The substituted carrier view above has independently replayed
    // the same complete rows, but may allocate another Arc. Install the proof's
    // contract before issuing the strict original/mapped receipt.
    output.ref_contract = canonical.ref_contract.clone();
    facts
        .validate_source_flow_signature_with_scratch(
            ir::SignatureSubject::Call(target),
            &output,
            scratch,
        )
        .map_err(|e| e.into_legacy_diagnostic(budget))?;
    Ok(output)
}

pub(super) fn replay_type_mapping(
    context: Option<&frontend::PlanCtx>,
    engine: EngineAuthority,
    source: &ir::Ty,
    mapped: &ir::Ty,
    facts: &crate::compiler::traits::ValidatedFacts,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(), String> {
    if facts.phase() != crate::compiler::traits::FactsPhase::Closed
        || !facts.budget().same_authority(budget)
    {
        return Err("typed root mapping changed closed caller authority".into());
    }
    let caller = ir::FactsCtx::from_validated(facts);
    caller.validate_ty_with_budget_scratch(mapped, budget, scratch)?;
    pair(context, engine, source, mapped, caller, budget, scratch, 0)
}

pub(super) fn replay_storage(
    engine: EngineAuthority,
    ty: &ir::Ty,
    facts: &crate::compiler::traits::ValidatedFacts,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(), String> {
    if facts.phase() != crate::compiler::traits::FactsPhase::Closed
        || !facts.budget().same_authority(budget)
    {
        return Err("typed CFG storage changed closed caller authority".into());
    }
    let caller = ir::FactsCtx::from_validated(facts);
    caller.validate_ty_with_budget_scratch(ty, budget, scratch)?;
    pair(None, engine, ty, ty, caller, budget, scratch, 0)
}

pub(super) fn replay_body(
    engine: EngineAuthority,
    mapped: &ir::Signature,
    facts: &crate::compiler::traits::ValidatedFacts,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
) -> Result<(), String> {
    if facts.phase() != crate::compiler::traits::FactsPhase::Closed
        || !facts.budget().same_authority(budget)
    {
        return Err("typed body signature changed closed caller authority".into());
    }
    let view = facts
        .source_carrier_contract_with_scratch(ir::SignatureSubject::Body, scratch)
        .map_err(|e| e.into_legacy_diagnostic(budget))?;
    let canonical = view.signature_for_replay();
    facts
        .validate_source_flow_signature_with_scratch(ir::SignatureSubject::Body, mapped, scratch)
        .map_err(|e| e.into_legacy_diagnostic(budget))?;
    if canonical.params.len() != mapped.params.len()
        || canonical.ref_contract != mapped.ref_contract
    {
        return Err("typed body signature changed canonical contract or arity".into());
    }
    for (source, output) in canonical
        .params
        .iter()
        .zip(&mapped.params)
        .chain(std::iter::once((&canonical.result, &mapped.result)))
    {
        ir::FactsCtx::from_validated(facts)
            .validate_ty_with_budget_scratch(output, budget, scratch)?;
        pair(
            None,
            engine,
            source,
            output,
            ir::FactsCtx::from_validated(facts),
            budget,
            scratch,
            0,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn pair(
    context: Option<&frontend::PlanCtx>,
    engine: EngineAuthority,
    source: &ir::Ty,
    mapped: &ir::Ty,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
    scratch: &mut crate::compiler::traits::TraitScratch,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let facts = caller
        .validated()
        .ok_or("closed pair requires genuine caller facts")?;
    for ty in [source, mapped] {
        facts
            .validate_ty_metadata_scratch(ty, budget, scratch)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
    }
    let mut recurse = |a, b| pair(context, engine, a, b, caller, budget, scratch, depth + 1);
    let nominal = match (source, mapped) {
        (ir::Ty::Struct(a), ir::Ty::Struct(b)) => Some((
            a.id(),
            a.def().source_decl,
            a.def().type_args.as_ref(),
            b.id(),
            b.def().source_decl,
            b.def().type_args.as_ref(),
            b.realization(),
        )),
        (ir::Ty::Enum(a), ir::Ty::Enum(b)) => Some((
            a.id(),
            a.def().source_decl,
            a.def().type_args.as_ref(),
            b.id(),
            b.def().source_decl,
            b.def().type_args.as_ref(),
            b.realization(),
        )),
        _ => None,
    };
    if let Some((
        source_id,
        source_decl,
        source_args,
        mapped_id,
        mapped_decl,
        mapped_args,
        capability,
    )) = nominal
    {
        if source_decl != mapped_decl || source_args.len() != mapped_args.len() {
            return Err("closed call realization changed nominal declaration or arity".into());
        }
        for (a, b) in source_args.iter().zip(mapped_args) {
            recurse(a, b)?;
        }
        if mapped_id.index.is_graph() {
            let capability = capability.ok_or("closed call realization omitted typed read")?;
            capability
                .retain_for_substitution_with_scratch(mapped, engine, caller, budget, scratch)?;
            if source_id.index.is_graph() {
                if source_id != mapped_id {
                    return Err("closed call realization changed existing graph identity".into());
                }
            } else {
                if let Some(context) = context {
                    let (_, template) = context.nominal_schema(source_decl, budget)?;
                    let template_id = match template {
                        ir::Ty::Struct(n) => n.id(),
                        ir::Ty::Enum(n) => n.id(),
                        _ => {
                            return Err(
                                "closed call realization source schema is not nominal".into()
                            );
                        }
                    };
                    if capability.source_application() != template_id {
                        return Err(
                            "closed call realization changed source schema application".into()
                        );
                    }
                }
            }
            return Ok(());
        }
        if let ir::NominalIndex::ParentLocal { .. } = mapped_id.index {
            // Parent-local closure application: declaration, arity and every
            // ordered actual were paired above; no graph capability exists.
            return Ok(());
        }
        if source_id != mapped_id {
            // Both metadata trees were independently validated above and every
            // ordered actual was paired recursively. A source-slot digest can
            // differ when an actual uses its checked graph representation on
            // only one side, even though the declaration and actuals agree.
            if matches!(
                source_id.index,
                ir::NominalIndex::GraphRealization(_)
                    | ir::NominalIndex::GraphComponentMember(..)
                    | ir::NominalIndex::SourceSlot(_)
            ) && matches!(mapped_id.index, ir::NominalIndex::SourceSlot(_))
                && source_id.snapshot == mapped_id.snapshot
            {
                return Ok(());
            }
            return Err("closed call realization changed source nominal identity".into());
        }
        return Ok(());
    }
    match (source, mapped) {
        (
            ir::Ty::Ref {
                kind: a,
                referent: x,
            },
            ir::Ty::Ref {
                kind: b,
                referent: y,
            },
        )
        | (
            ir::Ty::RawPtr {
                kind: a,
                pointee: x,
            },
            ir::Ty::RawPtr {
                kind: b,
                pointee: y,
            },
        ) if a == b => recurse(x, y),
        (ir::Ty::Array { element: x, len: a }, ir::Ty::Array { element: y, len: b }) if a == b => {
            recurse(x, y)
        }
        (ir::Ty::Slice { element: x }, ir::Ty::Slice { element: y }) => recurse(x, y),
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b.iter()) {
                recurse(a, b)?;
            }
            Ok(())
        }
        (ir::Ty::Closure(a), ir::Ty::Closure(b)) => {
            if a.try_eq(b, |x, y| recurse(x, y).map(|_| true))? {
                Ok(())
            } else {
                Err("closed signature changed canonical closure metadata".into())
            }
        }
        (ir::Ty::Opaque(a), ir::Ty::Opaque(b)) => {
            if a.try_eq(b, |x, y| recurse(x, y).map(|_| true))? {
                Ok(())
            } else {
                Err("closed signature changed canonical opaque metadata".into())
            }
        }
        (ir::Ty::Cell(a), ir::Ty::Cell(b)) => {
            if a.try_eq(b, |x, y| recurse(x, y).map(|_| true))? {
                Ok(())
            } else {
                Err("closed signature changed canonical Cell metadata".into())
            }
        }
        (ir::Ty::ManuallyDrop(a), ir::Ty::ManuallyDrop(b)) => {
            if a.try_eq(b, |x, y| recurse(x, y).map(|_| true))? {
                Ok(())
            } else {
                Err("closed signature changed canonical ManuallyDrop metadata".into())
            }
        }
        (ir::Ty::PhantomData(a), ir::Ty::PhantomData(b)) => {
            if a.try_eq(b, |x, y| recurse(x, y).map(|_| true))? {
                Ok(())
            } else {
                Err("closed signature changed canonical PhantomData metadata".into())
            }
        }
        (ir::Ty::FnPointer(a), ir::Ty::FnPointer(b)) => {
            if a.try_eq(b, |x, y| recurse(x, y).map(|_| true))? {
                Ok(())
            } else {
                Err("closed signature changed canonical function pointer metadata".into())
            }
        }
        (ir::Ty::FnItem(a), ir::Ty::FnItem(b))
            if a.declaration == b.declaration && a.type_args.len() == b.type_args.len() =>
        {
            for (x, y) in a.type_args.iter().zip(b.type_args.iter()) {
                recurse(x, y)?;
            }
            Ok(())
        }
        (ir::Ty::Dyn(a), ir::Ty::Dyn(b))
            if a.principal == b.principal
                && a.args.len() == b.args.len()
                && a.associated_types.len() == b.associated_types.len()
                && a.associated_types
                    .iter()
                    .zip(b.associated_types.iter())
                    .all(|(x, y)| x.0 == y.0) =>
        {
            for (x, y) in a.types().zip(b.types()) {
                recurse(x, y)?;
            }
            Ok(())
        }
        _ if source == mapped => Ok(()),
        _ => Err("closed call realization changed structural type".into()),
    }
}
