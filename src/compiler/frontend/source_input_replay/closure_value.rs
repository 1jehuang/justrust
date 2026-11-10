//! Closure input types come from the exact anonymous source provider and its
//! sealed recipe, not a generic call's already-selected type arguments.
use super::*;

pub(super) fn provider(
    plan: &PlanCtx,
    source: &Expr,
    budget: &ir::PlanBudget,
) -> Result<Option<(ir::DeclId, u32)>, FactsError> {
    sr::work(budget, 1)?;
    let Expr::Var(path) = source else {
        return Ok(None);
    };
    let Some(syntax) = plan
        .stored_closure_syntax(path)
        .map_err(|error| source_error(budget, &error))?
    else {
        return Ok(None);
    };
    let key = name_scope::path_key(plan.program.names.as_ref(), path, Namespace::Value)
        .map_err(|error| source_error(budget, &error))?;
    let provider =
        *plan.registry.functions.get(&key).ok_or_else(|| {
            FactsError::invalid(budget, "closure input source provider is missing")
        })?;
    Ok(Some((provider, syntax.site)))
}

fn value_place(
    checked: &ir::CheckedFunction,
    mut id: ir::ExprId,
    place: &ir::Place,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    for projection in place.projection.iter().rev() {
        sr::work(budget, 1)?;
        let value = checked.exprs.get(id.0).ok_or_else(|| {
            FactsError::invalid(
                budget,
                "closure capture read is outside checked expressions",
            )
        })?;
        id = match (projection, &value.kind) {
            (
                ir::Projection::Field(index),
                ir::ExprKind::Field {
                    base,
                    index: actual,
                },
            ) if index == actual => *base,
            (
                ir::Projection::ConstantIndex(index),
                ir::ExprKind::Index {
                    base,
                    index: actual,
                },
            ) if index == actual => *base,
            (ir::Projection::DynamicIndex(local), ir::ExprKind::DynamicIndex { base, index })
                if checked.exprs.get(index.0).is_some_and(
                    |value| matches!(value.kind, ir::ExprKind::Local(actual) if actual == *local),
                ) =>
            {
                *base
            }
            _ => {
                return Err(FactsError::invalid(
                    budget,
                    "closure input capture projection changed",
                ));
            }
        };
    }
    if !checked.exprs.get(id.0).is_some_and(
        |value| matches!(value.kind, ir::ExprKind::Local(actual) if actual == place.local),
    ) {
        return Err(FactsError::invalid(
            budget,
            "closure input capture local changed",
        ));
    }
    Ok(())
}

pub(super) fn replay(
    original: &OriginalSource,
    source: &Expr,
    actual: &ir::TypedExpr,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, FactsError> {
    let (provider, site) = provider(&original.plan, source, budget)?.ok_or_else(|| {
        FactsError::invalid(budget, "closure input lost its original anonymous provider")
    })?;
    let (ir::Ty::Closure(closure), ir::ExprKind::Tuple(values)) = (&actual.ty, &actual.kind) else {
        return Err(FactsError::invalid(
            budget,
            "closure input constructor changed",
        ));
    };
    if closure.provider() != provider
        || closure.owner() != original.declaration
        || closure.site() != site
        || original
            .plan
            .stored_closure_owner(provider)
            .map_err(|error| source_error(budget, &error))?
            != Some((original.declaration, site))
    {
        return Err(FactsError::invalid(
            budget,
            "closure input source identity changed",
        ));
    }
    let recipe = original
        .plan
        .registry
        .closure_recipes
        .get(&provider)
        .ok_or_else(|| FactsError::invalid(budget, "closure input source recipe is missing"))?;
    let template = closure
        .shape_seal()
        .and_then(|seal| seal.template_for(recipe))
        .ok_or_else(|| FactsError::invalid(budget, "closure input source shape seal changed"))?;
    let unsealed = closure.as_ref().clone().with_shape_seal(None);
    if !template.try_eq(&unsealed, |left, right| {
        replay_type_equal(left, right, budget)
    })? || values.len() != closure.captures().len()
        || values.len() != closure.sources().len()
    {
        return Err(FactsError::invalid(
            budget,
            "closure input sealed capture shape changed",
        ));
    }
    for ((id, ty), source) in values.iter().zip(closure.captures()).zip(closure.sources()) {
        sr::work(budget, 1)?;
        let value = checked.exprs.get(id.0).ok_or_else(|| {
            FactsError::invalid(budget, "closure input capture expression is missing")
        })?;
        if value.completion != ir::Completion::Returns || !replay_type_equal(&value.ty, ty, budget)?
        {
            return Err(FactsError::invalid(
                budget,
                "closure input capture type changed",
            ));
        }
        match (source.kind, &value.kind) {
            (ir::CaptureKind::Value, _) => value_place(checked, *id, &source.place, budget)?,
            (
                ir::CaptureKind::Shared,
                ir::ExprKind::Borrow {
                    kind: ir::BorrowKind::Shared,
                    place,
                    timing: ir::BorrowTiming::Immediate,
                },
            )
            | (
                ir::CaptureKind::Mut,
                ir::ExprKind::Borrow {
                    kind: ir::BorrowKind::Mut,
                    place,
                    timing: ir::BorrowTiming::Immediate,
                },
            ) if place == &source.place => {}
            _ => {
                return Err(FactsError::invalid(
                    budget,
                    "closure input capture source changed",
                ));
            }
        }
    }
    original
        .facts
        .validate_ty_metadata_with_budget(&actual.ty, budget)?;
    sr::retain(budget, std::mem::size_of::<ir::Ty>())?;
    Ok(actual.ty.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closure_input_replays_exact_provider_and_capture_places() {
        let prepared = crate::compiler::source::prepare_text_with_prelude(
            "closure-input-source.rs",
            "fn main(){let x=1i32;let f=||x;let g=||2i32;}",
            &crate::compiler::source::SourceOptions::default(),
        )
        .unwrap();
        let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_shared_program(Arc::new(program), &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("closure input source publication");
        };
        let original = publication.original().source_for_test();
        let checked = publication.source();
        let function = plan.callable_body(&plan.program, declaration).unwrap();
        let Stmt::Let { value: source, .. } = &function.body.statements[1] else {
            panic!("source closure binding");
        };
        let Stmt::Let { value: other, .. } = &function.body.statements[2] else {
            panic!("other source closure binding");
        };
        let ir::TypedStmt::Let { value, .. } = &checked.body.statements[1] else {
            panic!("checked closure binding");
        };
        let actual = &checked.exprs[value.0];
        assert!(copy_arguments(std::slice::from_ref(source), &budget)
            .unwrap()
            .is_some());
        assert!(replay(&original, source, actual, checked, &budget).is_ok());
        assert!(replay(&original, other, actual, checked, &budget).is_err());
        let ir::ExprKind::Tuple(captures) = &actual.kind else {
            panic!("closure capture constructor");
        };
        assert_eq!(captures.len(), 1);
        let capture = captures[0];
        for mutation in 0..3 {
            let mut forged = checked.as_ref().clone();
            match mutation {
                0 => forged.exprs[capture.0].ty = ir::Ty::Bool,
                1 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[capture.0].kind
                    else {
                        panic!("shared capture source");
                    };
                    place.local = ir::LocalId(place.local.0 + 1);
                }
                _ => {
                    let ir::ExprKind::Borrow { kind, .. } = &mut forged.exprs[capture.0].kind
                    else {
                        panic!("shared capture source");
                    };
                    *kind = ir::BorrowKind::Mut;
                }
            }
            assert!(replay(&original, source, actual, &forged, &budget).is_err());
        }
    }
}
