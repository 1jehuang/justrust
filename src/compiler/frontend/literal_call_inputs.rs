//! Independent source evidence for closed literal arguments with nested references.
//! No locals, calls, casts, projections or mutable borrows enter this path.
use super::*;

pub(super) fn copy_arguments(
    arguments: &[Expr],
    budget: &ir::PlanBudget,
) -> Result<Option<Vec<Expr>>, FactsError> {
    if arguments.is_empty() {
        return Ok(None);
    }
    let mut copied = Vec::new();
    for argument in arguments {
        let Some(literal) = copy_literal(argument, budget, 0)? else {
            return Ok(None);
        };
        sr::push(&mut copied, literal, budget)?;
    }
    Ok(Some(copied))
}

pub(super) fn copy_literal(
    source: &Expr,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<Expr>, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    sr::retain(budget, std::mem::size_of::<Expr>() + 32)?;
    Ok(Some(match source {
        Expr::Str(text) => {
            sr::retain(budget, text.len())?;
            Expr::Str(text.clone())
        }
        Expr::Bool(value) => Expr::Bool(*value),
        // A written suffix fixes the scalar input without expected-type help.
        Expr::Integer(literal) if literal.suffix.is_some() => Expr::Integer(literal.clone()),
        Expr::Char(value) => Expr::Char(*value),
        Expr::BorrowValue {
            kind: ir::BorrowKind::Shared,
            value,
        } => {
            let Some(value) = copy_literal(value, budget, depth + 1)? else {
                return Ok(None);
            };
            Expr::BorrowValue {
                kind: ir::BorrowKind::Shared,
                value: Box::new(value),
            }
        }
        Expr::Struct { name, fields } => {
            sr::work(budget, name.len())?;
            let mut copied = Vec::new();
            for (name, value) in fields {
                let Some(value) = copy_literal(value, budget, depth + 1)? else {
                    return Ok(None);
                };
                sr::push(&mut copied, (*name, value), budget)?;
            }
            Expr::Struct {
                name: name.clone(),
                fields: copied,
            }
        }
        _ => return Ok(None),
    }))
}

// Like the original binding mapping, this records storage identities during the
// initial source check. Reconstruction may verify them, never choose new ones.
pub(super) fn capture_temporaries(
    checked: &ir::CheckedFunction,
    id: ir::ExprId,
    output: &mut HashMap<ir::ExprId, ir::LocalId>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    let expr = checked
        .exprs
        .get(id.0)
        .ok_or_else(|| FactsError::invalid(budget, "literal source expression missing"))?;
    match &expr.kind {
        ir::ExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                capture_temporaries(checked, *value, output, budget, depth + 1)?;
            }
        }
        ir::ExprKind::Block(block) => {
            if let [ir::TypedStmt::Assign { local, value }] = block.statements.as_slice() {
                reserve_source_map(output, budget)?;
                if output.insert(id, *local).is_some() {
                    return Err(FactsError::invalid(
                        budget,
                        "duplicate literal temporary occurrence",
                    ));
                }
                capture_temporaries(checked, *value, output, budget, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn source_type_equal(
    left: &ir::Ty,
    right: &ir::Ty,
    facts: &tr::ValidatedFacts,
    budget: &ir::PlanBudget,
) -> Result<bool, FactsError> {
    facts.validate_ty_metadata_with_budget(left, budget)?;
    facts.validate_ty_metadata_with_budget(right, budget)?;
    facts
        .world()
        .types_equal(std::slice::from_ref(left), std::slice::from_ref(right))
}

#[allow(clippy::too_many_arguments)]
fn compare(
    source: &Expr,
    ty: &ir::Ty,
    actual: &ir::CheckedFunction,
    id: ir::ExprId,
    checker: &Checker<'_>,
    facts: &tr::ValidatedFacts,
    temporaries: &HashMap<ir::ExprId, ir::LocalId>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, FactsError> {
    sr::depth(budget, depth)?;
    sr::work(budget, 1)?;
    let right = actual
        .exprs
        .get(id.0)
        .ok_or_else(|| FactsError::invalid(budget, "literal argument expression missing"))?;
    if !source_type_equal(ty, &right.ty, facts, budget)?
        || right.completion != ir::Completion::Returns
    {
        return Ok(false);
    }
    Ok(match (source, &right.kind) {
        (Expr::Str(a), ir::ExprKind::Str(b)) => {
            sr::work(budget, a.len().max(b.len()))?;
            a == b
        }
        (Expr::Bool(a), ir::ExprKind::Bool(b)) => a == b,
        (Expr::Integer(literal), ir::ExprKind::Int(value)) => {
            resolve_integer(literal, false, None).map_err(|e| source_error(budget, &e))? == *value
        }
        (Expr::Char(a), ir::ExprKind::Char(b)) => u32::from(*a) == b.value,
        (Expr::Struct { fields: af, .. }, ir::ExprKind::Struct { ty: b, fields: bf }) => {
            if !source_type_equal(ty, &ir::Ty::Struct(b.clone()), facts, budget)?
                || af.len() != bf.len()
            {
                return Ok(false);
            }
            for ((name, source), (index, value)) in af.iter().zip(bf) {
                let (expected_index, expected_ty) = checker
                    .field_type(ty, &FieldSelector::Named(*name))
                    .map_err(|e| source_error(budget, &e))?;
                if expected_index != *index
                    || !compare(
                        source,
                        &expected_ty,
                        actual,
                        *value,
                        checker,
                        facts,
                        temporaries,
                        budget,
                        depth + 1,
                    )?
                {
                    return Ok(false);
                }
            }
            true
        }
        (
            Expr::BorrowValue {
                kind: ir::BorrowKind::Shared,
                value: source,
            },
            ir::ExprKind::Block(block),
        ) => {
            let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = ty
            else {
                return Ok(false);
            };
            let ([ir::TypedStmt::Assign { local, value }], Some(tail)) =
                (block.statements.as_slice(), block.tail)
            else {
                return Ok(false);
            };
            if temporaries.get(&id) != Some(local) {
                return Ok(false);
            }
            let Some(decl) = actual.locals.get(local.0) else {
                return Ok(false);
            };
            let Some(tail) = actual.exprs.get(tail.0) else {
                return Ok(false);
            };
            let ir::ExprKind::Borrow {
                kind: ir::BorrowKind::Shared,
                place,
                timing: ir::BorrowTiming::Immediate,
            } = &tail.kind
            else {
                return Ok(false);
            };
            place.local == *local
                && place.projection.is_empty()
                && source_type_equal(referent, &decl.ty, facts, budget)?
                && source_type_equal(ty, &tail.ty, facts, budget)?
                && compare(
                    source,
                    referent,
                    actual,
                    *value,
                    checker,
                    facts,
                    temporaries,
                    budget,
                    depth + 1,
                )?
        }
        // Statically promoted `&literal`: the referent must be exactly the
        // replayed scalar, and no frame temporary may stand behind it.
        (
            Expr::BorrowValue {
                kind: ir::BorrowKind::Shared,
                value: source,
            },
            ir::ExprKind::PromotedScalar(scalar),
        ) => {
            let ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } = ty
            else {
                return Ok(false);
            };
            if temporaries.contains_key(&id)
                || !source_type_equal(referent, &scalar.value_ty(), facts, budget)?
            {
                return Ok(false);
            }
            match (source.as_ref(), scalar) {
                (Expr::Bool(a), ir::PromotedScalar::Bool(b)) => *a == *b,
                (Expr::Char(a), ir::PromotedScalar::Char(b)) => u32::from(*a) == b.value,
                (Expr::Integer(literal), ir::PromotedScalar::Int(value)) => {
                    resolve_integer(literal, false, None).map_err(|e| source_error(budget, &e))?
                        == *value
                }
                _ => false,
            }
        }
        _ => false,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate(
    sources: &[Expr],
    actual: &ir::CheckedFunction,
    arguments: &[ir::ExprId],
    target: &ir::CallTarget,
    signature: &ir::Signature,
    view: &tr::OriginalCallableView,
    signatures: &Signatures,
    module: ModuleId,
    facts: &tr::ValidatedFacts,
    temporaries: &HashMap<ir::ExprId, ir::LocalId>,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    if sources.is_empty()
        || signature.params.len() != sources.len()
        || arguments.len() != sources.len()
        || view.signature().params.len() != sources.len()
    {
        return Err(FactsError::invalid(
            budget,
            "closed literal input arity changed",
        ));
    }
    // Returned references are admitted only from static strings, closed
    // reference-free scalars, and statically promoted `&scalar` inputs, never
    // from borrowed temporaries or constructors.
    // Their values and all generic actuals are independently replayed below.
    if !ir::carrier_locations(&signature.result, budget)
        .map_err(|e| source_error(budget, &e))?
        .is_empty()
        && !sources.iter().all(|source| match source {
            Expr::Str(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Integer(_) => true,
            Expr::BorrowValue {
                kind: ir::BorrowKind::Shared,
                value,
            } => matches!(
                value.as_ref(),
                Expr::Bool(_) | Expr::Char(_) | Expr::Integer(_)
            ),
            _ => false,
        })
    {
        return Err(FactsError::unsupported(
            budget,
            "returned literal references require static string inputs",
        ));
    }
    let checker = Checker::new(signatures, ir::Ty::Unit, module);
    let scopes = view.generic_env().scopes();
    if scopes.len() != 1 {
        return Err(FactsError::unsupported(
            budget,
            "closed literal inference requires one generic scope",
        ));
    }
    let mut state = sr::CallInferenceState::new(Some(scopes[0].clone()), budget)?;
    // Obtain source constructor constraints without solving through the legacy
    // checker. The ordinary canonical-source restriction remains unchanged.
    let mut materialize = |decl, args: &[ir::Ty]| {
        signatures
            .plan
            .as_ref()
            .ok_or_else(|| FactsError::invalid(budget, "literal replay lacks source plan"))?
            .intern_nominal(decl, args, ir::FactsCtx::Validated(facts), budget)
            .map_err(|e| source_error(budget, &e))
    };
    for (index, (source, argument)) in sources.iter().zip(arguments).enumerate() {
        sr::work(budget, 1)?;
        // Source evidence supplies no expected result or checked-argument type.
        let evidence = checker
            .inference_evidence(source, 0)
            .map_err(|e| source_error(budget, &e))?;
        let resolver = sr::CallInferenceState::new(None, budget)?;
        let input = resolver
            .resolve(&evidence, false, 0, &mut materialize)?
            .ok_or_else(|| {
                FactsError::unsupported(budget, "literal source type remains unresolved")
            })?;
        if !compare(
            source,
            &input,
            actual,
            *argument,
            &checker,
            facts,
            temporaries,
            budget,
            0,
        )? || !source_type_equal(&input, &signature.params[index], facts, budget)?
        {
            return Err(FactsError::invalid(
                budget,
                "closed literal argument changed source value, storage or type",
            ));
        }
        let schema = state.schema(&view.signature().params[index], 0)?;
        state.constrain(&schema, &evidence, 0)?;
    }
    let actuals = state.finish(false, &mut materialize)?.ok_or_else(|| {
        FactsError::unsupported(
            budget,
            "closed literal input leaves generic arguments unresolved",
        )
    })?;
    if actuals.len() != target.type_args.len() {
        return Err(FactsError::invalid(
            budget,
            "literal inference changed generic argument count",
        ));
    }
    for (expected, actual) in actuals.iter().zip(target.type_args.iter()) {
        if !source_type_equal(expected, actual, facts, budget)? {
            return Err(FactsError::invalid(
                budget,
                "literal inference changed generic argument",
            ));
        }
    }
    ir::validate_callee(
        &ir::Callee::Direct(target.clone()),
        signature,
        ir::FactsCtx::Validated(facts),
    )
    .map_err(|e| source_error(budget, &e))
}
