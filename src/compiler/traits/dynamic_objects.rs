//! Exact instantiated supertrait applications and vtable slot identities.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    #[test]
    fn dynamic_metadata_and_unsizing_replay_exact_equalities() {
        let source = frontend::parse("struct S;trait Has<A>{type Out;fn get(&self,a:A)->Self::Out;}impl Has<i32> for S{type Out=i32;fn get(&self,a:i32)->i32{a}}fn main(){}").unwrap();
        let plan = PlanCtx::from_program(&source, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        let header = world
            .headers
            .traits
            .iter()
            .find(|header| !header.associated_types.is_empty())
            .unwrap();
        let item = header.associated_types[0];
        let object = ir::DynTy {
            principal: header.declaration,
            args: Arc::from([Ty::Int(ir::IntTy::I32)]),
            associated_types: Arc::from([(item, Ty::Int(ir::IntTy::I32))]),
        };
        let validate = |object: &ir::DynTy| {
            world.validate_ty_metadata_in(&Ty::Dyn(Arc::new(object.clone())), &world.empty.env, 0)
        };
        validate(&object).unwrap();
        let mut missing = object.clone();
        missing.associated_types = Arc::from([]);
        assert!(validate(&missing).is_err());
        let mut duplicate = object.clone();
        duplicate.associated_types = Arc::from([
            (item, Ty::Int(ir::IntTy::I32)),
            (item, Ty::Int(ir::IntTy::I32)),
        ]);
        assert!(validate(&duplicate).is_err());
        let mut wrong_arity = object.clone();
        wrong_arity.args = Arc::from([]);
        assert!(validate(&wrong_arity).is_err());
        let mut foreign = object.clone();
        foreign.associated_types = Arc::from([(header.declaration, Ty::Int(ir::IntTy::I32))]);
        assert!(validate(&foreign).is_err());
        let implementation = world
            .headers
            .impls
            .iter()
            .find(|implementation| implementation.trait_ref.declaration == header.declaration)
            .unwrap();
        let concrete = implementation.trait_ref.self_ty.clone();
        let target = world
            .headers
            .callables
            .iter()
            .find(|callable| {
                callable.schema.env.total_arity() == 0
                    && world.info(callable.declaration).unwrap().owner.is_none()
            })
            .unwrap();
        let facts = world
            .instance_facts(&CallTarget {
                declaration: target.declaration,
                type_args: Arc::from([]),
            })
            .unwrap();
        facts.prove_dynamic_unsize(&concrete, &object).unwrap();
        assert!(facts.prove_dynamic_unsize(&concrete, &missing).is_err());
        let mut wrong_value = object.clone();
        wrong_value.associated_types = Arc::from([(item, Ty::Bool)]);
        validate(&wrong_value).unwrap();
        assert!(facts.prove_dynamic_unsize(&concrete, &wrong_value).is_err());
        assert!(
            !world
                .eq_ty(
                    &Ty::Dyn(Arc::new(object.clone())),
                    &Ty::Dyn(Arc::new(wrong_value)),
                    0
                )
                .unwrap()
        );
        assert!(
            world
                .eq_ty(
                    &Ty::Dyn(Arc::new(object.clone())),
                    &Ty::Dyn(Arc::new(object)),
                    0
                )
                .unwrap()
        );
    }
}

impl ValidatedTraitWorld {
    pub(super) fn dynamic_associated_binding(
        &self,
        projection: &ir::AssociatedTypeProjection,
        caller: &GenericEnv,
    ) -> Result<Option<Ty>> {
        let Ty::Dyn(object) = &projection.trait_ref.self_ty else {
            return Ok(None);
        };
        let mut reachable = false;
        for application in
            self.object_applications(object, &projection.trait_ref.self_ty, caller)?
        {
            if self.eq_predicate(
                &Predicate::Implements(application),
                &Predicate::Implements(projection.trait_ref.clone()),
            )? {
                reachable = true;
                break;
            }
        }
        if !reachable {
            return Err(FactsError::invalid(
                &self.budget,
                "dynamic projection uses an unreachable trait application",
            ));
        }
        let item = self.associated_header(projection.item)?;
        if item.trait_decl != projection.trait_ref.declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "dynamic projection changed exact associated owner",
            ));
        }
        object
            .associated_types
            .iter()
            .find(|(item, _)| *item == projection.item)
            .map(|(_, value)| Some(value.clone()))
            .ok_or_else(|| {
                FactsError::unproved(&self.budget, "E0191: dynamic associated binding is missing")
            })
    }

    pub(super) fn object_applications(
        &self,
        object: &ir::DynTy,
        owner: &Ty,
        caller: &GenericEnv,
    ) -> Result<Vec<TraitRef>> {
        fn visit(
            world: &ValidatedTraitWorld,
            current: TraitRef,
            caller: &GenericEnv,
            seen: &mut Vec<TraitRef>,
            depth: usize,
        ) -> Result<()> {
            charged(&world.budget, world.budget.check_depth(depth))?;
            charged(&world.budget, world.budget.charge_work(1))?;
            for previous in seen.iter() {
                if world.eq_predicate(
                    &Predicate::Implements(previous.clone()),
                    &Predicate::Implements(current.clone()),
                )? {
                    return Ok(());
                }
            }
            let header = world.trait_header(current.declaration)?;
            let mut actuals = Vec::new();
            push(&world.budget, &mut actuals, current.self_ty.clone())?;
            for arg in current.args.iter() {
                push(&world.budget, &mut actuals, arg.clone())?;
            }
            push(&world.budget, seen, current)?;
            for parent in header.supertraits.iter() {
                let mut raw = Vec::new();
                push(&world.budget, &mut raw, parent.self_ty.clone())?;
                for arg in parent.args.iter() {
                    push(&world.budget, &mut raw, arg.clone())?;
                }
                let schema = GenericSchema {
                    env: header.env.clone(),
                    signature: Signature {
                        ref_contract: Arc::new(ir::RefContract {
                            region_count: 0,
                            input_regions: Arc::from([]),
                            results: Arc::from([]),
                            outlives: Arc::from([]),
                        }),
                        params: Vec::new(),
                        result: Ty::Tuple(raw.into()),
                    },
                    predicates: Arc::from([]),
                };
                let specialized = world.substitute_projected_schema(&schema, &actuals, caller)?;
                let Ty::Tuple(values) = specialized.signature.result else {
                    return Err(FactsError::invalid(
                        &world.budget,
                        "dynamic supertrait application changed shape",
                    ));
                };
                let Some((self_ty, args)) = values.split_first() else {
                    return Err(FactsError::invalid(
                        &world.budget,
                        "dynamic supertrait application lost Self",
                    ));
                };
                visit(
                    world,
                    TraitRef {
                        declaration: parent.declaration,
                        self_ty: self_ty.clone(),
                        args: Arc::from(args),
                    },
                    caller,
                    seen,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        let mut applications = Vec::new();
        visit(
            self,
            TraitRef {
                declaration: object.principal,
                self_ty: owner.clone(),
                args: object.args.clone(),
            },
            caller,
            &mut applications,
            0,
        )?;
        Ok(applications)
    }
}

impl ValidatedFacts {
    pub fn dynamic_applications(&self, object: &ir::DynTy) -> Result<Vec<TraitRef>> {
        self.world.check_facts(self, true)?;
        self.world
            .validate_ty_metadata_in(&Ty::Dyn(Arc::new(object.clone())), self.env(), 0)?;
        self.world
            .object_applications(object, &Ty::Dyn(Arc::new(object.clone())), self.env())
    }

    pub fn prove_dynamic_unsize(&self, concrete: &Ty, object: &ir::DynTy) -> Result<()> {
        self.world.check_facts(self, true)?;
        self.world
            .validate_ty_metadata_in(&Ty::Dyn(Arc::new(object.clone())), self.env(), 0)?;
        self.validate_ty(concrete)?;
        let root = Predicate::Implements(TraitRef {
            declaration: object.principal,
            self_ty: concrete.clone(),
            args: object.args.clone(),
        });
        if !matches!(self.world.prove_goal(&root, self)?, ProofOutcome::Proved(_)) {
            return Err(FactsError::unproved(
                self.budget(),
                "E0277: concrete type does not implement dynamic principal",
            ));
        }
        let applications = self
            .world
            .object_applications(object, concrete, self.env())?;
        for (item, expected) in object.associated_types.iter() {
            let owner = self.world.associated_header(*item)?.trait_decl;
            let mut matching = applications
                .iter()
                .filter(|application| application.declaration == owner);
            let application = matching.next().ok_or_else(|| {
                FactsError::invalid(self.budget(), "unreachable dynamic associated owner")
            })?;
            if matching.next().is_some() {
                return Err(FactsError::invalid(
                    self.budget(),
                    "ambiguous dynamic associated owner application",
                ));
            }
            let goal = Predicate::ProjectionEq {
                projection: ir::AssociatedTypeProjection {
                    trait_ref: application.clone(),
                    item: *item,
                },
                ty: expected.clone(),
            };
            if !matches!(self.world.prove_goal(&goal, self)?, ProofOutcome::Proved(_)) {
                return Err(FactsError::unproved(
                    self.budget(),
                    "E0271: concrete associated type differs from dynamic binding",
                ));
            }
        }
        Ok(())
    }

    pub fn object_vtable_targets(
        &self,
        concrete: &Ty,
        object: &ir::DynTy,
    ) -> Result<Vec<CallTarget>> {
        self.prove_dynamic_unsize(concrete, object)?;
        if self.phase() != FactsPhase::Closed
            || matches!(concrete, Ty::Dyn(_) | Ty::Slice { .. } | Ty::Param(_))
        {
            return Err(FactsError::invalid(
                self.budget(),
                "dynamic vtable requires closed sized concrete type",
            ));
        }
        let applications = self
            .world
            .object_applications(object, concrete, self.env())?;
        for (item, expected) in object.associated_types.iter() {
            let header = self.world.associated_header(*item)?;
            let mut selected = None;
            for application in &applications {
                if application.declaration == header.trait_decl {
                    if selected.is_some() {
                        return Err(FactsError::invalid(
                            self.budget(),
                            "ambiguous dynamic associated type application",
                        ));
                    }
                    selected = Some(application.clone());
                }
            }
            let trait_ref = selected.ok_or_else(|| {
                FactsError::invalid(
                    self.budget(),
                    "dynamic associated type is not owned by a reachable trait",
                )
            })?;
            let predicate = Predicate::ProjectionEq {
                projection: ir::AssociatedTypeProjection {
                    trait_ref,
                    item: *item,
                },
                ty: expected.clone(),
            };
            if !matches!(
                self.world.prove_goal(&predicate, self)?,
                ProofOutcome::Proved(_)
            ) {
                return Err(FactsError::invalid(
                    self.budget(),
                    "E0271: concrete associated type does not match dynamic equality",
                ));
            }
        }
        let mut targets = Vec::new();
        for application in applications.into_iter().rev() {
            let header = self.world.trait_header(application.declaration)?;
            for item in header.items.iter() {
                let call = TraitCall {
                    trait_ref: application.clone(),
                    item: *item,
                    method_args: Arc::from([]),
                };
                let resolved = self.world.resolve_trait_call(&call, self)?;
                let Callee::Direct(target) = resolved.callee() else {
                    return Err(FactsError::unsupported(
                        self.budget(),
                        "dynamic vtable slot has no concrete implementation",
                    ));
                };
                push(self.budget(), &mut targets, target.clone())?;
            }
        }
        Ok(targets)
    }

    pub fn object_vtable_methods(&self, object: &ir::DynTy) -> Result<Vec<(TraitRef, DeclId)>> {
        self.world.check_facts(self, true)?;
        self.world
            .validate_ty_metadata_in(&Ty::Dyn(Arc::new(object.clone())), self.env(), 0)?;
        let mut out = Vec::new();
        let owner = Ty::Dyn(Arc::new(object.clone()));
        // Reverse application order keeps ancestors before their principal.
        for application in self
            .world
            .object_applications(object, &owner, self.env())?
            .into_iter()
            .rev()
        {
            let header = self.world.trait_header(application.declaration)?;
            for item in header.items.iter() {
                push(self.budget(), &mut out, (application.clone(), *item))?;
            }
        }
        Ok(out)
    }
}
