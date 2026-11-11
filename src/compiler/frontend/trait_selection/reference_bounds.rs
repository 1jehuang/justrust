//! Infer reference-impl applications from their original referent prerequisite.
//! This discovers arguments only. Canonical selection proves every predicate.
use super::*;
use crate::compiler::{expansion_budget::Storage, token_session::BudgetVec};

impl PlanCtx {
    pub(super) fn reference_bound_applications(
        &self,
        implementation: &tr::ImplHeader,
        owner: &ir::Ty,
        bindings: &BudgetVec<(ir::ParamId, ir::Ty)>,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Option<Vec<ir::TraitRef>>, String> {
        let (
            ir::Ty::Ref {
                referent: pattern, ..
            },
            ir::Ty::Ref { referent, .. },
        ) = (&implementation.trait_ref.self_ty, owner)
        else {
            return Ok(None);
        };
        let ir::Ty::Param(parameter) = pattern.as_ref() else {
            return Ok(None);
        };
        if !implementation.schema_env.contains(*parameter) {
            return Ok(None);
        }
        // Recursive discovery consumes exactly one reference layer. Check the
        // complete reference spine before descending, so each recursive call
        // has a strictly smaller, already depth-bounded input.
        let mut spine = owner;
        let mut depth = 0;
        while let ir::Ty::Ref { referent, .. } = spine {
            budget.charge_work(1)?;
            budget.check_depth(depth)?;
            depth = depth.checked_add(1).ok_or("reference depth overflow")?;
            spine = referent;
        }
        budget.check_depth(depth)?;
        let Some((_, original)) = bindings.as_slice().iter().find(|(id, _)| id == parameter) else {
            return Ok(None);
        };
        if !metered_type_equal(original, referent, budget, 0)? {
            return Err("reference application changed its matched referent".into());
        }
        // These have implicit callable implementations rather than a complete
        // source impl-head enumeration. Their existing callable producer owns
        // that evidence, so absence here cannot exclude them. Dyn is different:
        // inferred_trait_applications first replays dynamic_applications, which
        // expands its authenticated public principal and supertraits rather
        // than mistaking the lack of source impl heads for an empty frontier.
        if matches!(
            referent.as_ref(),
            ir::Ty::Closure(_) | ir::Ty::FnItem(_) | ir::Ty::FnPointer(_)
        ) {
            return Ok(None);
        }
        'predicates: for predicate in implementation.predicates.iter() {
            budget.charge_work(1)?;
            let ir::Predicate::Implements(bound) = predicate else {
                continue;
            };
            if !matches!(&bound.self_ty, ir::Ty::Param(id) if id == parameter) {
                continue;
            }
            // Marker bounds do not constrain an open trait argument.
            if bound.args.is_empty() {
                continue;
            }
            // Unsupported projection patterns are not evidence of absence.
            for argument in bound.args.iter() {
                if ir::contains_associated_types(argument, budget)? {
                    continue 'predicates;
                }
            }
            let (applications, pending) =
                self.inferred_trait_applications(referent, bound.declaration, facts, budget)?;
            if pending {
                return Ok(None);
            }
            let mut discovered = Vec::new();
            for application in applications.as_slice() {
                budget.charge_work(1)?;
                if application.args.len() != bound.args.len() {
                    return Err("reference prerequisite trait arity changed".into());
                }
                let mut inferred = BudgetVec::new(self.program.names.budget(), Storage::Temporary)
                    .map_err(selection_token_error)?;
                for (id, actual) in bindings.as_slice() {
                    inferred
                        .push((*id, actual.clone()))
                        .map_err(selection_token_error)?;
                }
                let mut fits = true;
                for (pattern, actual) in bound.args.iter().zip(application.args.iter()) {
                    fits &= match_trait_self(
                        pattern,
                        actual,
                        &implementation.schema_env,
                        &mut inferred,
                        budget,
                        0,
                    )?;
                }
                if !fits {
                    continue;
                }
                let mut args =
                    self.selection_arguments(implementation.trait_ref.args.len(), budget)?;
                for argument in implementation.trait_ref.args.iter() {
                    budget.charge_work(1)?;
                    let actual = match argument {
                        ir::Ty::Param(id) if implementation.schema_env.contains(*id) => {
                            let Some((_, actual)) =
                                inferred.as_slice().iter().find(|(bound, _)| bound == id)
                            else {
                                continue 'predicates;
                            };
                            actual.clone()
                        }
                        argument if !mentions_param(argument) => argument.clone(),
                        _ => continue 'predicates,
                    };
                    args.push(actual);
                }
                budget.retain_bytes(std::mem::size_of::<ir::TraitRef>() + 32)?;
                discovered
                    .try_reserve(1)
                    .map_err(|_| "reference application allocation failed")?;
                discovered.push(ir::TraitRef {
                    declaration: implementation.trait_ref.declaration,
                    self_ty: owner.clone(),
                    args: args.into(),
                });
            }
            // A complete empty prerequisite frontier excludes this impl.
            // Output equalities and all other predicates remain untouched and
            // are discharged by the canonical selector for each candidate.
            return Ok(Some(discovered));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_application_frontier_retains_exact_original_authority() {
        let program = parse("struct Base;struct Missing;trait Inner<A>{type Output;}trait Outer<A>{fn get(&self)->A;}impl Inner<i32> for Base{type Output=bool;}impl Inner<bool> for Base{type Output=i32;}impl<A,R,F:Inner<A,Output=R>> Outer<A> for &F{fn get(&self)->A{panic!(\"unused\")}}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let mut plan = PlanCtx::from_program(&program, &budget).unwrap();
        let callable = plan.function_decl(0).unwrap();
        let facts = plan.callable_facts(callable).unwrap();
        let implementation = facts
            .world()
            .impl_headers()
            .iter()
            .find(|header| header.trait_ref.declaration == plan.registry.trait_ids[1])
            .unwrap()
            .clone();
        let ir::Ty::Ref {
            referent: pattern, ..
        } = &implementation.trait_ref.self_ty
        else {
            panic!("original self pattern must be a reference")
        };
        let ir::Ty::Param(parameter) = pattern.as_ref() else {
            panic!("original referent must be its declared parameter")
        };
        let base = plan.nominal_declaration(0).unwrap();
        let value = plan
            .intern_nominal(base, &[], ir::FactsCtx::Validated(&facts), &budget)
            .unwrap();
        let owner = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(value.clone()),
        };
        let mut bindings = BudgetVec::new(plan.program.names.budget(), Storage::Temporary).unwrap();
        bindings.push((*parameter, value.clone())).unwrap();
        let applications = plan
            .reference_bound_applications(&implementation, &owner, &bindings, &facts, &budget)
            .unwrap()
            .unwrap();
        assert_eq!(
            applications.len(),
            2,
            "competing original applications must not collapse"
        );
        for application in &applications {
            assert_eq!(
                application.declaration,
                implementation.trait_ref.declaration
            );
            assert!(metered_type_equal(&application.self_ty, &owner, &budget, 0).unwrap());
            assert!(matches!(
                facts
                    .service()
                    .prove_goal(&ir::Predicate::Implements(application.clone()), &facts)
                    .unwrap(),
                tr::ProofOutcome::Proved(_)
            ));
        }
        assert!(
            applications
                .iter()
                .any(|a| a.args.as_ref() == [ir::Ty::Int(ir::IntTy::I32)])
        );
        assert!(
            applications
                .iter()
                .any(|a| a.args.as_ref() == [ir::Ty::Bool])
        );

        let mut foreign = implementation.clone();
        foreign.trait_ref.self_ty = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Param(ir::ParamId {
                owner: ir::BinderId {
                    declaration: callable,
                },
                index: parameter.index,
            })),
        };
        assert!(
            plan.reference_bound_applications(&foreign, &owner, &bindings, &facts, &budget)
                .unwrap()
                .is_none()
        );
        let changed = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Bool),
        };
        assert!(
            plan.reference_bound_applications(
                &implementation,
                &changed,
                &bindings,
                &facts,
                &budget
            )
            .unwrap_err()
            .contains("changed its matched referent")
        );

        let mut unsupported = implementation.clone();
        let mut predicates = unsupported.predicates.to_vec();
        for predicate in &mut predicates {
            if let ir::Predicate::Implements(bound) = predicate
                && bound.declaration == plan.registry.trait_ids[0]
            {
                bound.args = Arc::from([ir::Ty::AssociatedType(Arc::new(
                    ir::AssociatedTypeProjection {
                        trait_ref: bound.clone(),
                        item: plan.registry.trait_associated_types[0][0],
                    },
                ))]);
            }
        }
        unsupported.predicates = predicates.into();
        assert!(
            plan.reference_bound_applications(&unsupported, &owner, &bindings, &facts, &budget)
                .unwrap()
                .is_none(),
            "unsupported projection patterns are not negative evidence"
        );

        let missing = plan.nominal_declaration(1).unwrap();
        let missing = plan
            .intern_nominal(missing, &[], ir::FactsCtx::Validated(&facts), &budget)
            .unwrap();
        let receiver = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(missing.clone()),
        };
        let mut missing_bindings =
            BudgetVec::new(plan.program.names.budget(), Storage::Temporary).unwrap();
        missing_bindings
            .push((*parameter, missing.clone()))
            .unwrap();
        assert!(
            plan.reference_bound_applications(
                &implementation,
                &receiver,
                &missing_bindings,
                &facts,
                &budget
            )
            .unwrap()
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn reference_application_incomplete_prerequisite_remains_pending() {
        let program = parse("struct Base;trait Inner<A>{}trait Outer<A>{}impl<A> Inner<A> for Base{}impl<A,F:Inner<A>> Outer<A> for &F{}fn main(){}").unwrap();
        let budget = ir::PlanBudget::default();
        let mut plan = PlanCtx::from_program(&program, &budget).unwrap();
        let callable = plan.function_decl(0).unwrap();
        let facts = plan.callable_facts(callable).unwrap();
        let base = plan.nominal_declaration(0).unwrap();
        let value = plan
            .intern_nominal(base, &[], ir::FactsCtx::Validated(&facts), &budget)
            .unwrap();
        let receiver = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(value),
        };
        let (applications, pending) = plan
            .inferred_trait_applications(&receiver, plan.registry.trait_ids[1], &facts, &budget)
            .unwrap();
        assert!(
            pending,
            "unconstrained original prerequisite is not an empty proof"
        );
        assert!(applications.as_slice().is_empty());
        assert!(
            plan.trait_applications(&receiver, plan.registry.trait_ids[1], &facts, &budget)
                .err()
                .expect("strict discovery must retain the explicit inference boundary")
                .contains("unsupported:")
        );
    }
}
