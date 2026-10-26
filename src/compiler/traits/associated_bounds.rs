//! Declared associated bounds are predicates on an exact owning projection.
//! Their source-backed applications are specialized together, preserving
//! ordered trait arguments and associated equalities through generic callers.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn associated_bound_predicates(
        &self,
        associated: DeclId,
    ) -> Result<Arc<[Predicate]>> {
        let header = self.associated_header(associated)?;
        let predicates = self.schema.associated_type_predicates(associated)?;
        if predicates.len() > MAX_PREDICATES {
            return Err(FactsError::resource(
                &self.budget,
                "associated bound predicate limit exceeded",
            ));
        }
        let mut memberships = Vec::new();
        for predicate in predicates.iter() {
            self.validate_predicate_metadata(predicate, &header.env)?;
            if let Predicate::Implements(goal) = predicate {
                if self.info(goal.declaration)?.kind != DeclKind::Trait
                    || goal.args.len().checked_add(1)
                        != Some(self.trait_header(goal.declaration)?.env.total_arity())
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated bound trait identity or ordered argument arity changed",
                    ));
                }
                let Ty::AssociatedType(projection) = &goal.self_ty else {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated bound subject is not its owning projection",
                    ));
                };
                if projection.item != associated
                    || projection.trait_ref.declaration != header.trait_decl
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated bound subject has foreign owner or item",
                    ));
                }
                let owner = self.trait_header(header.trait_decl)?;
                let arguments = self.trait_arguments(&projection.trait_ref)?;
                if arguments.len() != owner.env.total_arity() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated bound owner arity changed",
                    ));
                }
                for (actual, parameter) in arguments.iter().zip(owner.env.ordered_params()) {
                    if !matches!(actual, Ty::Param(id) if *id == parameter) {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "associated bound changed its symbolic owning application",
                        ));
                    }
                }
                push(&self.budget, &mut memberships, goal.declaration)?;
            }
        }
        for predicate in predicates.iter() {
            if let Predicate::ProjectionEq { projection, .. } = predicate {
                let mut owning_bound = false;
                for candidate in predicates.iter() {
                    if let Predicate::Implements(goal) = candidate {
                        owning_bound |= self.eq_predicate(
                            &Predicate::Implements(goal.clone()),
                            &Predicate::Implements(projection.trait_ref.clone()),
                        )?;
                    }
                }
                if !owning_bound {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "associated declaration equality has no exact declared owning bound",
                    ));
                }
            }
        }
        if memberships.as_slice() != header.bounds.as_ref() {
            return Err(FactsError::invalid(
                &self.budget,
                "associated bound applications disagree with ordered source identities",
            ));
        }
        Ok(predicates)
    }

    pub(super) fn associated_bounds_for(
        &self,
        associated: DeclId,
        owner: &TraitRef,
        caller: &GenericEnv,
    ) -> Result<Arc<[Predicate]>> {
        self.associated_bounds_for_inner(associated, owner, caller, None)
    }

    fn associated_bounds_for_with(
        &self,
        associated: DeclId,
        owner: &TraitRef,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Arc<[Predicate]>> {
        self.check_context(context)?;
        self.associated_bounds_for_inner(associated, owner, context.env(), Some((context, active)))
    }

    fn associated_bounds_for_inner(
        &self,
        associated: DeclId,
        owner: &TraitRef,
        caller: &GenericEnv,
        proof: Option<(ProofContext<'_>, &mut ProofScratch)>,
    ) -> Result<Arc<[Predicate]>> {
        let header = self.associated_header(associated)?;
        if header.trait_decl != owner.declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "associated bound specialization changed owning trait",
            ));
        }
        let predicates = self.associated_bound_predicates(associated)?;
        if predicates.is_empty() {
            return Ok(predicates);
        }
        let schema = GenericSchema {
            env: header.env.clone(),
            signature: Signature {
                params: Vec::new(),
                result: Ty::Unit,
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates,
        };
        let arguments = self.trait_arguments(owner)?;
        let value = match proof {
            Some((context, active)) => {
                self.substitute_projected_schema_with(&schema, &arguments, context, active)?
            }
            None => self.substitute_projected_schema(&schema, &arguments, caller)?,
        };
        Ok(value.predicates)
    }

    pub(super) fn discharge_associated_bound_predicates(
        &self,
        associated: DeclId,
        owner: &TraitRef,
        normal: &Ty,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
        implementation: Option<&projected_methods::OwnerNormalizationContext<'_>>,
    ) -> Result<()> {
        let predicates = self.associated_bounds_for_with(associated, owner, context, active)?;
        for predicate in predicates.iter() {
            let mut reduce = |projection: &ir::AssociatedTypeProjection| -> Result<Ty> {
                if projection.item == associated
                    && self.eq_predicate(
                        &Predicate::Implements(projection.trait_ref.clone()),
                        &Predicate::Implements(owner.clone()),
                    )?
                {
                    return Ok(normal.clone());
                }
                let raw = Ty::AssociatedType(Arc::new(projection.clone()));
                if let Some(implementation) = implementation {
                    return self.normalize_owner_type(&raw, implementation, &mut Vec::new(), 0);
                }
                if self.goal_is_closed_with(&projection.trait_ref, context.env(), active)? {
                    let (normal, dependencies) = self.normalize_closed_projected_type_with(
                        &raw,
                        context,
                        !self.owner_normalization_finalized,
                        active,
                    )?;
                    for dependency in dependencies.iter() {
                        self.validate_projected_dependency_with(dependency, context, active)?;
                    }
                    return Ok(normal);
                }
                Ok(raw)
            };
            let normalized = match predicate {
                Predicate::Implements(goal) => {
                    let mut args = Vec::new();
                    for argument in goal.args.iter() {
                        push(
                            &self.budget,
                            &mut args,
                            self.map_universal_projection_type(
                                argument,
                                context.env(),
                                0,
                                &mut reduce,
                            )?,
                        )?;
                    }
                    Predicate::Implements(TraitRef {
                        declaration: goal.declaration,
                        self_ty: self.map_universal_projection_type(
                            &goal.self_ty,
                            context.env(),
                            0,
                            &mut reduce,
                        )?,
                        args: args.into(),
                    })
                }
                Predicate::ProjectionEq { projection, ty } => {
                    let mut args = Vec::new();
                    for argument in projection.trait_ref.args.iter() {
                        push(
                            &self.budget,
                            &mut args,
                            self.map_universal_projection_type(
                                argument,
                                context.env(),
                                0,
                                &mut reduce,
                            )?,
                        )?;
                    }
                    Predicate::ProjectionEq {
                        projection: ir::AssociatedTypeProjection {
                            trait_ref: TraitRef {
                                declaration: projection.trait_ref.declaration,
                                self_ty: self.map_universal_projection_type(
                                    &projection.trait_ref.self_ty,
                                    context.env(),
                                    0,
                                    &mut reduce,
                                )?,
                                args: args.into(),
                            },
                            item: projection.item,
                        },
                        ty: self.map_universal_projection_type(
                            ty,
                            context.env(),
                            0,
                            &mut reduce,
                        )?,
                    }
                }
            };
            self.require_in(&normalized, context, active)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    fn fixture() -> (
        Arc<ValidatedTraitWorld>,
        Arc<ValidatedFacts>,
        TraitRef,
        DeclId,
    ) {
        let program = frontend::parse("trait Read<A,B>{type Item;}trait Factory<A,B>{type Output:Read<A,B,Item=A>;}fn caller<F:Factory<i32,bool>>(){}fn other<F:Factory<bool,i32>>(){}fn main(){}").unwrap();
        let ctx = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = ctx.trait_world().unwrap();
        let facts = world.callable_facts(ctx.function_decl(0).unwrap()).unwrap();
        let owner = facts
            .predicates
            .predicates
            .iter()
            .find_map(|row| match row {
                Predicate::Implements(goal)
                    if goal.args.len() == 2 && matches!(goal.self_ty, Ty::Param(_)) =>
                {
                    Some(goal.clone())
                }
                _ => None,
            })
            .unwrap();
        let item = world
            .headers
            .associated_types
            .iter()
            .find(|header| header.trait_decl == owner.declaration)
            .unwrap()
            .declaration;
        (world, facts, owner, item)
    }

    #[test]
    fn applied_bounds_preserve_source_order_and_caller_obligations() {
        let (world, facts, owner, item) = fixture();
        let rows = world
            .associated_bounds_for(item, &owner, facts.env())
            .unwrap();
        let context = ProofContext::from_facts(&facts);
        for row in rows.iter() {
            world
                .require_in(row, context, &mut ProofScratch::default())
                .unwrap();
        }
        assert!(rows.iter().any(|row| matches!(
            row,
            Predicate::ProjectionEq {
                ty: Ty::Int(ir::IntTy::I32),
                ..
            }
        )));
        let mut wrong = owner.clone();
        wrong.args = Arc::from([Ty::Bool, Ty::Int(ir::IntTy::I32)]);
        assert!(matches!(
            world
                .prove_in(
                    &Predicate::Implements(wrong),
                    context,
                    &mut ProofScratch::default()
                )
                .unwrap(),
            ProofOutcome::NotProven
        ));
        let (_, foreign, _, _) = fixture();
        assert!(world.check_facts(&foreign, false).is_err());
        assert!(
            world
                .associated_bounds_for(item, &owner, foreign.env())
                .is_err()
        );
        let mut foreign_item = item;
        foreign_item.snapshot = foreign.world.registry;
        assert!(world.associated_bound_predicates(foreign_item).is_err());
    }

    #[test]
    fn reflexive_projection_equality_requires_exact_membership_capability() {
        let (world, facts, owner, item) = fixture();
        let projection = ir::AssociatedTypeProjection {
            trait_ref: owner.clone(),
            item,
        };
        let ty = Ty::AssociatedType(Arc::new(projection.clone()));
        let goal = Predicate::ProjectionEq {
            projection: projection.clone(),
            ty: ty.clone(),
        };
        let context = ProofContext::from_facts(&facts);
        assert!(matches!(
            world
                .prove_projection_equality(&projection, &ty, context, &mut ProofScratch::default())
                .unwrap(),
            ProofOutcome::Proved(_)
        ));
        let (_, foreign, foreign_owner, _) = fixture();
        let membership = foreign
            .world
            .require_in(
                &Predicate::Implements(foreign_owner),
                ProofContext::from_facts(&foreign),
                &mut ProofScratch::default(),
            )
            .unwrap();
        assert!(
            world
                .evidence(
                    &goal,
                    context,
                    ProofOrigin::ReflexiveEquality { membership }
                )
                .is_err()
        );
        let mut wrong = projection.clone();
        wrong.trait_ref.args = Arc::from([Ty::Bool, Ty::Int(ir::IntTy::I32)]);
        assert!(matches!(
            world.prove_projection_equality(
                &wrong,
                &Ty::AssociatedType(Arc::new(wrong.clone())),
                context,
                &mut ProofScratch::default()
            ),
            Err(_) | Ok(ProofOutcome::NotProven)
        ));
    }

    #[test]
    fn cyclic_symbolic_rhs_is_not_a_successful_normalization() {
        let (world, facts, owner, item) = fixture();
        let first = ir::AssociatedTypeProjection {
            trait_ref: owner.clone(),
            item,
        };
        let rows = world
            .associated_bounds_for(item, &owner, facts.env())
            .unwrap();
        let second = rows
            .iter()
            .find_map(|row| match row {
                Predicate::ProjectionEq { projection, .. } => Some(projection.clone()),
                _ => None,
            })
            .unwrap();
        let cycle = [
            Predicate::ProjectionEq {
                projection: first.clone(),
                ty: Ty::AssociatedType(Arc::new(second.clone())),
            },
            Predicate::ProjectionEq {
                projection: second,
                ty: Ty::AssociatedType(Arc::new(first.clone())),
            },
        ];
        let error = world
            .reduce_declared_equality_rhs(&first, &cycle, facts.env(), &mut Vec::new())
            .unwrap_err();
        assert!(format!("{error}").contains("cyclic declared projection equalities"));
    }
}
