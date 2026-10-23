//! Rigid associated projections are opaque canonical families, not concrete
//! guesses. Membership comes from the exact declaration's retained bounds.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn validate_universal_projection(
        &self,
        projection: &ir::AssociatedTypeProjection,
        env: &GenericEnv,
    ) -> Result<()> {
        self.validate_ty_metadata_in(&Ty::AssociatedType(Arc::new(projection.clone())), env, 0)?;
        if self.raw_associated_ty_is_closed(&Ty::AssociatedType(Arc::new(projection.clone())), 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "closed associated projection must normalize before execution",
            ));
        }
        let declaration = env
            .scopes()
            .last()
            .ok_or_else(|| {
                FactsError::invalid(&self.budget, "rigid projection has no canonical binder")
            })?
            .binder()
            .declaration;
        let info = self.info(declaration)?;
        if !self.same_env(&info.env, env)? {
            return Err(FactsError::invalid(
                &self.budget,
                "projection changed its canonical declaration environment",
            ));
        }
        let mut predicates = Vec::new();
        match info.kind {
            DeclKind::Trait => {
                self.append_predicate(
                    &mut predicates,
                    &Predicate::Implements(self.symbolic_trait_ref(declaration)?),
                )?;
            }
            DeclKind::TraitItem => {
                let item = self.item_header(declaration)?;
                for predicate in item.schema.predicates.iter() {
                    self.append_predicate(&mut predicates, predicate)?;
                }
                self.append_predicate(
                    &mut predicates,
                    &Predicate::Implements(self.symbolic_trait_ref(item.trait_decl)?),
                )?;
            }
            DeclKind::Function | DeclKind::AssociatedFunction => {
                let callable = self.callable_header(declaration)?;
                for predicate in callable.schema.predicates.iter() {
                    self.append_predicate(&mut predicates, predicate)?;
                }
                if let Some(item) = self.source_default_item(declaration)? {
                    self.append_predicate(
                        &mut predicates,
                        &Predicate::Implements(self.symbolic_trait_ref(item.trait_decl)?),
                    )?;
                }
                if let Some(owner) = info.owner {
                    for implementation in self.headers.impls.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        if implementation.declaration == owner {
                            for predicate in implementation.predicates.iter() {
                                self.append_predicate(&mut predicates, predicate)?;
                            }
                            self.append_predicate(
                                &mut predicates,
                                &Predicate::Implements(implementation.trait_ref.clone()),
                            )?;
                            break;
                        }
                    }
                }
            }
            DeclKind::TraitImpl => {
                let implementation = self
                    .headers
                    .impls
                    .iter()
                    .find(|implementation| implementation.declaration == declaration)
                    .ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "projection impl binder has no canonical header",
                        )
                    })?;
                for predicate in implementation.predicates.iter() {
                    self.append_predicate(&mut predicates, predicate)?;
                }
                self.append_predicate(
                    &mut predicates,
                    &Predicate::Implements(implementation.trait_ref.clone()),
                )?;
            }
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "projection binder is not a canonical trait or callable",
                ));
            }
        }
        // Membership discovery must not recursively execute type validation on
        // unrelated projected supertrait actuals. Every row is substituted from
        // an authenticated declaration and checked as canonical metadata here.
        // Executable consumers still validate the selected projection separately.
        let mut index = 0;
        while index < predicates.len() {
            if let Predicate::Implements(goal) = &predicates[index] {
                let goal = goal.clone();
                self.validate_predicate_metadata(&predicates[index], env)?;
                let mut inherited = self.substitute_supertrait_predicates(&goal, env, false)?;
                for associated in self.trait_header(goal.declaration)?.associated_types.iter() {
                    for row in self.associated_bounds_for(*associated, &goal, env)?.iter() {
                        self.validate_predicate_metadata(row, env)?;
                        self.append_predicate(&mut inherited, row)?;
                    }
                }
                for row in inherited {
                    self.append_predicate(&mut predicates, &row)?;
                }
            }
            index += 1;
        }
        let expected = Predicate::Implements(projection.trait_ref.clone());
        for predicate in predicates.iter() {
            if self.eq_predicate(predicate, &expected)? {
                return Ok(());
            }
        }
        if matches!(projection.trait_ref.self_ty, Ty::Struct(_) | Ty::Enum(_)) {
            let caller = self.predicate_env(declaration)?;
            if matches!(
                self.prove_in(
                    &expected,
                    ProofContext {
                        predicates: &caller,
                        phase: FactsPhase::Schema,
                    },
                    &mut ProofScratch::default(),
                )?,
                ProofOutcome::Proved(_)
            ) {
                return Ok(());
            }
        }
        Err(FactsError::unproved(
            &self.budget,
            "E0277: associated projection has no exact declared trait membership",
        ))
    }

    pub(super) fn normalize_universal_callable_schema(
        &self,
        schema: &GenericSchema,
    ) -> Result<Option<GenericSchema>> {
        if !schema.env.is_generic() || !self.signature_contains_projection(&schema.signature)? {
            return Ok(None);
        }
        fn normalize(
            world: &ValidatedTraitWorld,
            ty: &Ty,
            env: &GenericEnv,
            depth: usize,
        ) -> Result<Ty> {
            charged(&world.budget, world.budget.check_depth(depth))?;
            charged(&world.budget, world.budget.charge_work(1))?;
            match ty {
                Ty::AssociatedType(projection) => {
                    if world.raw_associated_ty_is_closed(ty, depth)? {
                        return Ok(world
                            .normalize_closed_projected_type(
                                ty,
                                ProofContext {
                                    predicates: &world.empty,
                                    phase: FactsPhase::Schema,
                                },
                                true,
                            )?
                            .0);
                    }
                    world.validate_universal_projection(projection, env)?;
                    Ok(ty.clone())
                }
                Ty::Closure(closure) => {
                    Ok(Ty::Closure(Arc::new(closure.try_map(|child| {
                        normalize(world, child, env, depth + 1)
                    })?)))
                }
                Ty::Dyn(object) => {
                    let mut args = Vec::new();
                    for argument in object.args.iter() {
                        push(
                            &world.budget,
                            &mut args,
                            normalize(world, argument, env, depth + 1)?,
                        )?;
                    }
                    let mut associated_types = Vec::new();
                    for (item, value) in object.associated_types.iter() {
                        push(
                            &world.budget,
                            &mut associated_types,
                            (*item, normalize(world, value, env, depth + 1)?),
                        )?;
                    }
                    Ok(Ty::Dyn(Arc::new(ir::DynTy {
                        principal: object.principal,
                        args: args.into(),
                        associated_types: associated_types.into(),
                    })))
                }
                Ty::Ref { kind, referent } => Ok(Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(normalize(world, referent, env, depth + 1)?),
                }),
                Ty::RawPtr { kind, pointee } => Ok(Ty::RawPtr {
                    kind: *kind,
                    pointee: Arc::new(normalize(world, pointee, env, depth + 1)?),
                }),
                Ty::Array { element, len } => Ok(Ty::Array {
                    element: Arc::new(normalize(world, element, env, depth + 1)?),
                    len: *len,
                }),
                Ty::Slice { element } => Ok(Ty::Slice {
                    element: Arc::new(normalize(world, element, env, depth + 1)?),
                }),
                Ty::Tuple(fields) => {
                    let mut output = Vec::new();
                    for field in fields.iter() {
                        push(
                            &world.budget,
                            &mut output,
                            normalize(world, field, env, depth + 1)?,
                        )?;
                    }
                    Ok(Ty::Tuple(output.into()))
                }
                Ty::Struct(_) | Ty::Enum(_) => {
                    world.normalize_nominal_actuals(ty, env, depth, |argument| {
                        normalize(world, argument, env, depth + 1)
                    })
                }
                _ => Ok(ty.clone()),
            }
        }
        let mut result = clone_schema(schema, &self.budget)?;
        for parameter in &mut result.signature.params {
            *parameter = normalize(self, parameter, &schema.env, 0)?;
        }
        result.signature.result = normalize(self, &schema.signature.result, &schema.env, 0)?;
        Ok(Some(result))
    }

    pub(super) fn validate_universal_projection_type(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        self.validate_ty_metadata_in(ty, env, depth)?;
        match ty {
            Ty::Closure(closure) => {
                for child in closure.types() {
                    self.validate_universal_projection_type(child, env, depth + 1)?;
                }
            }
            Ty::Dyn(object) => {
                for child in object.types() {
                    self.validate_universal_projection_type(child, env, depth + 1)?;
                }
            }
            Ty::AssociatedType(projection) => {
                self.validate_universal_projection(projection, env)?
            }
            Ty::Ref { referent, .. }
            | Ty::RawPtr {
                pointee: referent, ..
            }
            | Ty::Array {
                element: referent, ..
            }
            | Ty::Slice { element: referent } => {
                self.validate_universal_projection_type(referent, env, depth + 1)?
            }
            Ty::Struct(value) => {
                for argument in value.def().type_args.iter() {
                    self.validate_universal_projection_type(argument, env, depth + 1)?;
                }
            }
            Ty::Enum(value) => {
                for argument in value.def().type_args.iter() {
                    self.validate_universal_projection_type(argument, env, depth + 1)?;
                }
            }
            Ty::Tuple(fields) => {
                for field in fields.iter() {
                    self.validate_universal_projection_type(field, env, depth + 1)?;
                }
            }
            _ => {}
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
        ir::AssociatedTypeProjection,
    ) {
        let program = frontend::parse("trait Parent<A>{} trait Call<A>:Parent<A>{} trait Has<A,B>{type Out;} fn caller<I:Has<i32,bool>,F:Call<(<I as Has<i32,bool>>::Out,)>>(f:F,x:<I as Has<i32,bool>>::Out)->i32{0} fn main(){}").unwrap();
        let ctx = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = ctx.trait_world().unwrap();
        let caller = ctx.function_decl(0).unwrap();
        let facts = world.callable_facts(caller).unwrap();
        let Ty::AssociatedType(projection) = &world
            .callable_header(caller)
            .unwrap()
            .schema
            .signature
            .params[1]
        else {
            panic!("missing exact projected input")
        };
        let projection = projection.as_ref().clone();
        (world, facts, projection)
    }

    #[test]
    fn projected_argument_membership_closure_is_nonrecursive_and_exact() {
        let (world, facts, projection) = fixture();
        world
            .validate_universal_projection(&projection, facts.env())
            .unwrap();
        let mut reordered = projection.clone();
        reordered.trait_ref.args = Arc::from([
            projection.trait_ref.args[1].clone(),
            projection.trait_ref.args[0].clone(),
        ]);
        assert!(
            world
                .validate_universal_projection(&reordered, facts.env())
                .is_err()
        );
        let mut foreign_item = projection.clone();
        foreign_item.item.index = foreign_item.item.index.wrapping_add(1);
        assert!(
            world
                .validate_universal_projection(&foreign_item, facts.env())
                .is_err()
        );
        let (foreign, foreign_facts, _) = fixture();
        assert!(
            foreign
                .validate_universal_projection(&projection, foreign_facts.env())
                .is_err()
        );
        assert!(
            world
                .validate_universal_projection(&projection, foreign_facts.env())
                .is_err()
        );
    }
}
