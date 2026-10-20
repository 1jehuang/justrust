//! Per-declaration nominal field wellformedness. This is not callable authority.
use super::*;

#[derive(Clone, Debug)]
pub struct NominalDeclarationSchema {
    pub declaration: DeclId,
    pub env: Arc<GenericEnv>,
    pub predicates: Arc<[Predicate]>,
    pub projections: Arc<[ir::AssociatedTypeProjection]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> crate::compiler::frontend::PlanCtx {
        let source = crate::compiler::frontend::parse("trait Base<A,B>{type Item;}trait Sub<A,B>:Base<A,B>{}struct Direct<T:Base<i32,bool>>{value:<T as Base<i32,bool>>::Item}struct Inherited<T:Sub<i32,bool>>{value:<T as Base<i32,bool>>::Item}fn main(){}").unwrap();
        crate::compiler::frontend::PlanCtx::from_program(&source, &ir::PlanBudget::default())
            .unwrap()
    }

    #[test]
    fn nominal_membership_replay_binds_source_owner_world_rows_and_paths() {
        let plan = plan();
        let world = plan.trait_world().unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let inherited = plan.nominal_declaration(1).unwrap();
        let proof = world.nominal_declaration_wellformed(declaration).unwrap();
        world
            .replay_nominal_declaration(declaration, &proof)
            .unwrap();
        assert!(world.replay_nominal_declaration(inherited, &proof).is_err());
        let foreign = self::plan();
        assert!(
            foreign
                .trait_world()
                .unwrap()
                .replay_nominal_declaration(declaration, &proof)
                .is_err()
        );
        let mut omitted = proof.clone();
        omitted.schema.projections = Arc::from([]);
        assert!(
            world
                .replay_nominal_declaration(declaration, &omitted)
                .is_err()
        );
        let mut reordered = proof.clone();
        let mut projections = reordered.schema.projections.to_vec();
        let mut arguments = projections[0].trait_ref.args.to_vec();
        arguments.swap(0, 1);
        projections[0].trait_ref.args = arguments.into();
        reordered.schema.projections = projections.into();
        assert!(
            world
                .replay_nominal_declaration(declaration, &reordered)
                .is_err()
        );
        let inherited_proof = world.nominal_declaration_wellformed(inherited).unwrap();
        world
            .replay_nominal_declaration(inherited, &inherited_proof)
            .unwrap();
        let mut wrong_path = inherited_proof.clone();
        let mut memberships = wrong_path.membership.to_vec();
        let Membership::Declared { bound, .. } = memberships[0] else {
            panic!("expected declared membership")
        };
        memberships[0] = Membership::Declared {
            bound,
            path: Arc::from([usize::MAX]),
        };
        wrong_path.membership = memberships.into();
        assert!(
            world
                .replay_nominal_declaration(inherited, &wrong_path)
                .is_err()
        );
        let mut wrong_bound = inherited_proof;
        let mut memberships = wrong_bound.membership.to_vec();
        let Membership::Declared { path, .. } = &memberships[0] else {
            panic!("expected declared membership")
        };
        memberships[0] = Membership::Declared {
            bound: usize::MAX,
            path: path.clone(),
        };
        wrong_bound.membership = memberships.into();
        assert!(
            world
                .replay_nominal_declaration(inherited, &wrong_bound)
                .is_err()
        );
    }
}

#[derive(Clone, Debug)]
enum Membership {
    Declared { bound: usize, path: Arc<[usize]> },
    Closed(Arc<[NormalizedAssociatedType]>),
}

/// Immutable output of the declaration-local graph operation. No world,
/// mutable planning budget, interner, or callable capability is retained.
#[derive(Clone, Debug)]
pub struct NominalDeclarationProof {
    world: WorldId,
    schema: NominalDeclarationSchema,
    membership: Arc<[Membership]>,
}

impl NominalDeclarationProof {
    pub(in crate::compiler) fn schema(&self) -> &NominalDeclarationSchema {
        &self.schema
    }
}

impl ValidatedTraitWorld {
    pub fn nominal_declaration_wellformed(
        &self,
        declaration: DeclId,
    ) -> Result<NominalDeclarationProof> {
        let schema = self.schema.nominal_declaration_schema(declaration)?;
        let info = self.info(declaration)?;
        if schema.declaration != declaration
            || !matches!(info.kind, DeclKind::Struct | DeclKind::Enum)
            || !self.same_env(&info.env, &schema.env)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "nominal declaration source binder changed",
            ));
        }
        for predicate in schema.predicates.iter() {
            self.validate_predicate_metadata(predicate, &schema.env)?;
        }
        let mut membership = Vec::new();
        for projection in schema.projections.iter() {
            self.validate_ty_metadata_in(
                &Ty::AssociatedType(Arc::new(projection.clone())),
                &schema.env,
                0,
            )?;
            let expected = Predicate::Implements(projection.trait_ref.clone());
            if self
                .raw_associated_ty_is_closed(&Ty::AssociatedType(Arc::new(projection.clone())), 0)?
            {
                let context = ProofContext {
                    predicates: &self.empty,
                    phase: FactsPhase::Schema,
                };
                self.require_in(&expected, context, &mut Vec::new())?;
                let (_, dependencies) = self.normalize_closed_projected_type(
                    &Ty::AssociatedType(Arc::new(projection.clone())),
                    context,
                    true,
                )?;
                for dependency in &dependencies {
                    self.validate_projected_dependency(dependency, context)?;
                }
                push(
                    &self.budget,
                    &mut membership,
                    Membership::Closed(dependencies.into()),
                )?;
                continue;
            }
            let mut pending = Vec::new();
            for (bound, predicate) in schema.predicates.iter().enumerate() {
                push(
                    &self.budget,
                    &mut pending,
                    (predicate.clone(), bound, Vec::<usize>::new()),
                )?;
            }
            let mut found = None;
            let mut cursor = 0;
            while cursor < pending.len() {
                charged(&self.budget, self.budget.charge_work(1))?;
                let (predicate, bound, previous_path) = &pending[cursor];
                let predicate = predicate.clone();
                let bound = *bound;
                let mut path = Vec::new();
                for edge in previous_path {
                    push(&self.budget, &mut path, *edge)?;
                }
                if self.eq_predicate(&predicate, &expected)? {
                    found = Some(Membership::Declared {
                        bound,
                        path: path.into(),
                    });
                    break;
                }
                if let Predicate::Implements(goal) = &predicate {
                    self.validate_predicate_metadata(&predicate, &schema.env)?;
                    for (edge, inherited) in self
                        .substitute_supertrait_predicates(goal, &schema.env, false)?
                        .into_iter()
                        .enumerate()
                    {
                        self.validate_predicate_metadata(&inherited, &schema.env)?;
                        let mut seen = false;
                        for (previous, _, _) in &pending {
                            if self.eq_predicate(previous, &inherited)? {
                                seen = true;
                                break;
                            }
                        }
                        if !seen {
                            let mut next = Vec::new();
                            for previous_edge in &path {
                                push(&self.budget, &mut next, *previous_edge)?;
                            }
                            push(&self.budget, &mut next, edge)?;
                            push(&self.budget, &mut pending, (inherited, bound, next))?;
                        }
                    }
                }
                cursor += 1;
            }
            let found = found.ok_or_else(|| {
                FactsError::unproved(
                    &self.budget,
                    "E0277: nominal field projection has no exact declared trait membership",
                )
            })?;
            push(&self.budget, &mut membership, found)?;
        }
        retain::<NominalDeclarationProof>(&self.budget, 1)?;
        Ok(NominalDeclarationProof {
            world: self.id,
            schema,
            membership: membership.into(),
        })
    }

    pub fn replay_nominal_declaration(
        &self,
        declaration: DeclId,
        proof: &NominalDeclarationProof,
    ) -> Result<()> {
        if proof.world != self.id || proof.schema.declaration != declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "nominal declaration proof belongs to another world or declaration",
            ));
        }
        let expected = self.nominal_declaration_wellformed(declaration)?;
        if !self.same_env(&proof.schema.env, &expected.schema.env)?
            || proof.schema.predicates.len() != expected.schema.predicates.len()
            || proof.schema.projections.len() != expected.schema.projections.len()
            || proof.membership.len() != expected.membership.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "nominal declaration proof omitted or changed source rows",
            ));
        }
        for (actual, expected) in proof
            .schema
            .predicates
            .iter()
            .zip(expected.schema.predicates.iter())
        {
            if !self.eq_predicate(actual, expected)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal declaration predicate changed",
                ));
            }
        }
        for (actual, expected) in proof
            .schema
            .projections
            .iter()
            .zip(expected.schema.projections.iter())
        {
            if !self.eq_ty(
                &Ty::AssociatedType(Arc::new(actual.clone())),
                &Ty::AssociatedType(Arc::new(expected.clone())),
                0,
            )? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal declaration projection identity changed",
                ));
            }
        }
        for (actual, expected) in proof.membership.iter().zip(expected.membership.iter()) {
            match (actual, expected) {
                (
                    Membership::Declared { bound: a, path: ap },
                    Membership::Declared { bound: b, path: bp },
                ) if a == b && ap == bp => {}
                (Membership::Closed(a), Membership::Closed(b)) if a.len() == b.len() => {
                    let context = ProofContext {
                        predicates: &self.empty,
                        phase: FactsPhase::Schema,
                    };
                    for (actual, expected) in a.iter().zip(b.iter()) {
                        self.validate_projected_dependency(actual, context)?;
                        if !self.same_projected_dependency(actual, expected)? {
                            return Err(FactsError::invalid(
                                &self.budget,
                                "nominal concrete membership dependency changed",
                            ));
                        }
                    }
                }
                _ => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "nominal declaration membership path changed",
                    ));
                }
            }
        }
        Ok(())
    }
}
