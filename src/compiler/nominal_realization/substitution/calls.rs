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
    if facts.phase() != crate::compiler::traits::FactsPhase::Closed {
        return Err("closed call realization requires closed caller authority".into());
    }
    for signature in [canonical, mapped] {
        facts
            .validate_signature_with(ir::SignatureSubject::Call(target), signature, scratch)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
    }
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
        pair(context, engine, source, output, caller, budget, 0)?;
    }
    let mut output = mapped.clone();
    // The resolved canonical Call proof owns the authoritative closed carrier
    // contract. The substituted carrier view above has independently replayed
    // the same complete rows, but may allocate another Arc. Install the proof's
    // contract before issuing the strict original/mapped receipt.
    output.ref_contract = canonical.ref_contract.clone();
    facts
        .validate_signature_with(ir::SignatureSubject::Call(target), &output, scratch)
        .map_err(|e| e.into_legacy_diagnostic(budget))?;
    Ok(output)
}

fn pair(
    context: Option<&frontend::PlanCtx>,
    engine: EngineAuthority,
    source: &ir::Ty,
    mapped: &ir::Ty,
    caller: ir::FactsCtx<'_>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    caller.validate_ty_metadata_with_budget(source, budget)?;
    caller.validate_ty_metadata_with_budget(mapped, budget)?;
    let recurse = |a, b| pair(context, engine, a, b, caller, budget, depth + 1);
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
        if matches!(mapped_id.index, ir::NominalIndex::GraphRealization(_)) {
            let capability = capability.ok_or("closed call realization omitted typed read")?;
            capability.retain_for_substitution(mapped, engine, caller, budget)?;
            if matches!(source_id.index, ir::NominalIndex::GraphRealization(_)) {
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
            // A closed application realized through the graph (e.g. a closure
            // result `Item`) and its canonical source slot are the same
            // nominal: declaration, arity and every ordered actual were
            // paired above, and a source slot carries no graph capability.
            if matches!(source_id.index, ir::NominalIndex::GraphRealization(_))
                && matches!(mapped_id.index, ir::NominalIndex::SourceSlot(_))
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
        _ if source == mapped => Ok(()),
        _ => Err("closed call realization changed structural type".into()),
    }
}
