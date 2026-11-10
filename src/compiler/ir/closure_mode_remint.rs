//! Type-only remint of a source-checked closure body for its final receiver mode.
//! ExprIds, places, statements and original call-argument evidence stay fixed.
use super::*;

pub(in crate::compiler) fn remint_closure_mode_body(
    source: &CheckedFunction,
    source_facts: &super::super::traits::ValidatedFacts,
    facts: &super::super::traits::ValidatedFacts,
    cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
) -> Result<CheckedFunction, String> {
    facts
        .validate_parent_local_closure_source(source_facts)
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    if source.body_substitution.is_some() || !facts.budget().same_authority(budget) {
        return Err(
            "closure mode remint requires original source and exact planning authority".into(),
        );
    }
    let env = facts.env();
    budget.retain_bytes(
        env.total_arity()
            .checked_mul(std::mem::size_of::<Ty>())
            .ok_or("closure mode identity argument storage overflow")?,
    )?;
    let mut args = Vec::new();
    args.try_reserve_exact(env.total_arity())
        .map_err(|_| "closure mode identity arguments allocation failed")?;
    for parameter in env.ordered_params() {
        // ParamId preserves each source-ordered slot. Mixed type/const binders
        // also accept their own symbolic slot as the identity actual.
        args.push(Ty::Param(parameter));
    }
    budget.retain_bytes(
        source
            .locals
            .len()
            .checked_mul(std::mem::size_of::<LocalDecl>())
            .and_then(|n| {
                source
                    .exprs
                    .len()
                    .checked_mul(std::mem::size_of::<TypedExpr>())
                    .and_then(|m| n.checked_add(m))
            })
            .ok_or("closure mode body storage overflow")?,
    )?;
    let mut output = source.clone();
    let mut visitor = Instantiate {
        facts: FactsCtx::Validated(facts),
        env,
        args: &args,
        cx,
        budget,
        owners: std::collections::HashMap::new(),
        component_owners_seen: std::collections::HashSet::new(),
    };
    output.result = visitor.ty(&output.result)?;
    for local in &mut output.locals {
        budget.charge_work(1)?;
        local.ty = visitor.ty(&local.ty)?;
    }
    for expression in &mut output.exprs {
        budget.charge_work(1)?;
        expression.ty = visitor.ty(&expression.ty)?;
        match &mut expression.kind {
            ExprKind::CellNew { ty, .. }
            | ExprKind::CellGet { ty, .. }
            | ExprKind::CellSet { ty, .. } => {
                let Ty::Cell(mapped) = visitor.ty(&Ty::Cell(ty.clone()))? else {
                    return Err("closure mode remint changed Cell kind".into());
                };
                *ty = mapped;
            }
            ExprKind::ManuallyDrop { ty, .. }
            | ExprKind::ManuallyDropIntoInner { ty, .. }
            | ExprKind::ManuallyDropDrop { ty, .. } => {
                let Ty::ManuallyDrop(mapped) = visitor.ty(&Ty::ManuallyDrop(ty.clone()))? else {
                    return Err("closure mode remint changed ManuallyDrop kind".into());
                };
                *ty = mapped;
            }
            ExprKind::PhantomData(marker) => {
                let Ty::PhantomData(mapped) = visitor.ty(&Ty::PhantomData(marker.clone()))? else {
                    return Err("closure mode remint changed PhantomData kind".into());
                };
                *marker = mapped;
            }
            ExprKind::OpaqueWrap { opaque, .. } => {
                let Ty::Opaque(mapped) = visitor.ty(&Ty::Opaque(opaque.clone()))? else {
                    return Err("closure mode remint changed opaque kind".into());
                };
                *opaque = mapped;
            }
            ExprKind::Borrow { place, .. } | ExprKind::BoundsCheck { array: place, .. } => {
                visitor.path(&mut place.projection)?;
            }
            ExprKind::Struct { ty, .. } => {
                let Ty::Struct(mapped) = visitor.ty(&Ty::Struct(ty.clone()))? else {
                    return Err("closure mode remint changed struct kind".into());
                };
                *ty = mapped;
            }
            ExprKind::EnumVariant { ty, .. } => visitor.enum_ty(ty)?,
            ExprKind::Match { arms, .. } => {
                for arm in arms {
                    visitor.pattern(&mut arm.pattern)?;
                }
            }
            ExprKind::Call {
                target, signature, ..
            } => {
                *target = map_callee_metadata(target, budget, |ty| visitor.ty(ty))?;
                *signature = visitor.signature(signature)?;
            }
            ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                visitor.block(then_block, 0)?;
                visitor.block(else_block, 0)?;
            }
            ExprKind::Block(block)
            | ExprKind::Loop { body: block, .. }
            | ExprKind::LabeledBlock { body: block, .. } => visitor.block(block, 0)?,
            ExprKind::TypeAlignment(query) => {
                let queried = visitor.ty(query.queried_type())?;
                visitor
                    .cx
                    .realize_alignment_query(query, queried, visitor.facts, budget)?;
            }
            // These operands have no nested parent-local nominal metadata.
            // Associated constants keep their original symbolic operand since
            // the substitution above is the identity on every generic slot.
            ExprKind::ConstParameter(_)
            | ExprKind::AssociatedConst(_)
            | ExprKind::RuntimeQuery(_)
            | ExprKind::FnItem
            | ExprKind::CharToInt { .. }
            | ExprKind::U8ToChar { .. }
            | ExprKind::PtrCast { .. }
            | ExprKind::Intrinsic { .. }
            | ExprKind::LeadingZeros { .. }
            | ExprKind::IntCast { .. }
            | ExprKind::FloatCast { .. }
            | ExprKind::Char(_)
            | ExprKind::Str(_)
            | ExprKind::ByteStr(_)
            | ExprKind::ByteRepeat { .. }
            | ExprKind::PromotedArray(_)
            | ExprKind::PromotedScalar(_)
            | ExprKind::Float(_)
            | ExprKind::Deref { .. }
            | ExprKind::DynamicIndex { .. }
            | ExprKind::ArrayToSlice { .. }
            | ExprKind::RefToDyn { .. }
            | ExprKind::SliceDataPointer { .. }
            | ExprKind::SliceLen { .. }
            | ExprKind::SliceIndex { .. }
            | ExprKind::Subslice { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::Index { .. }
            | ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Local(_)
            | ExprKind::NativeSyscall(_)
            | ExprKind::RawSlice { .. }
            | ExprKind::Tuple(_)
            | ExprKind::Field { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Break { .. }
            | ExprKind::Continue { .. }
            | ExprKind::Output(_)
            | ExprKind::Format { .. }
            | ExprKind::Println(_)
            | ExprKind::Panic(_) => {}
        }
    }
    visitor.block(&mut output.body, 0)?;
    // Re-prove calls under the final facts; copied argument traces alone never
    // confer admission to the new facts context.
    let mut canonical = canonicalize_call_metadata(&output, facts, visitor.cx, None, budget)
        .map_err(|error| error.into_legacy_diagnostic(budget))?;
    if canonical.exprs.len() != output.exprs.len() {
        return Err("closure mode canonicalization changed expression occurrences".into());
    }
    let mut scratch = super::super::traits::TraitScratch::default();
    for (mapped, expression) in output.exprs.iter().zip(canonical.exprs.iter_mut()) {
        budget.charge_work(1)?;
        let ExprKind::Call {
            signature: mapped, ..
        } = &mapped.kind
        else {
            continue;
        };
        let ExprKind::Call {
            target, signature, ..
        } = &mut expression.kind
        else {
            return Err("closure mode canonicalization changed call occurrence".into());
        };
        // Canonical discharge proves the selected call, but its source-interner
        // handles do not replace the parent's exact mapped storage handles.
        // Authenticate both COMPLETE signatures against this same call/facts,
        // including every input, ordered actual, result and reference contract.
        facts
            .validate_source_flow_signatures_with_scratch(
                SignatureSubject::Call(target),
                &[signature, mapped],
                &mut scratch,
            )
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        budget.retain_bytes(std::mem::size_of_val(mapped.params.as_slice()))?;
        *signature = mapped.clone();
    }
    canonical.validate_with_facts(FactsCtx::Validated(facts))?;
    Ok(canonical)
}
