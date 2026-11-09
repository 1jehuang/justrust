//! Per-declaration nominal field wellformedness. This is not callable authority.
use super::*;

#[derive(Clone, Debug)]
pub struct NominalDeclarationSchema {
    pub representation_alignment: Option<u128>,
    pub declaration: DeclId,
    pub env: Arc<GenericEnv>,
    pub predicates: Arc<[Predicate]>,
    pub projections: Arc<[ir::AssociatedTypeProjection]>,
    pub defaults: Arc<[NominalDefaultSchema]>,
}

#[derive(Clone, Debug)]
pub struct NominalDefaultSchema {
    pub parameter: ir::ParamId,
    pub syntax: [crate::compiler::token_session::SpanId; 2],
    pub ty: Ty,
    pub obligations: Arc<[Predicate]>,
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
    fn nominal_declared_membership(
        &self,
        schema: &NominalDeclarationSchema,
        expected: &Predicate,
    ) -> Result<Option<Membership>> {
        let mut pending = Vec::new();
        for (bound, predicate) in schema.predicates.iter().enumerate() {
            push(
                &self.budget,
                &mut pending,
                (predicate.clone(), bound, Vec::<usize>::new()),
            )?;
        }
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
            if self.eq_predicate(&predicate, expected)? {
                return Ok(Some(Membership::Declared {
                    bound,
                    path: path.into(),
                }));
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
        Ok(None)
    }

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
        if let Some(alignment) = schema.representation_alignment {
            if !alignment.is_power_of_two() || alignment > (1u128 << 29) {
                return Err(FactsError::unproved(
                    &self.budget,
                    "E0589: invalid repr(align) attribute: alignment must be a power of two no larger than 2^29",
                ));
            }
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: repr(align) layout is not implemented",
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
                    transport: ParentLocalTransport::default(),
                    original: None,
                    source_metadata: None,
                    predicates: &self.empty,
                    executable_storage: true,
                    phase: FactsPhase::Schema,
                };
                self.require_in(&expected, context, &mut ProofScratch::default())?;
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
            let found = self
                .nominal_declared_membership(&schema, &expected)?
                .ok_or_else(|| {
                    FactsError::unproved(
                        &self.budget,
                        "E0277: nominal field projection has no exact declared trait membership",
                    )
                })?;
            push(&self.budget, &mut membership, found)?;
        }
        for default in schema.defaults.iter() {
            self.validate_ty_metadata_in(&default.ty, &schema.env, 0)?;
            for goal in default.obligations.iter() {
                self.validate_predicate_metadata(goal, &schema.env)?;
                let evidence = if let Predicate::Implements(trait_ref) = goal
                    && self.goal_is_closed(trait_ref, &schema.env)?
                {
                    // The canonical empty context, never an invented nominal
                    // caller environment. Replay reconstructs this same proof.
                    self.require_in(
                        goal,
                        ProofContext {
                            transport: ParentLocalTransport::default(),
                            original: None,
                            source_metadata: None,
                            predicates: &self.empty,
                            executable_storage: true,
                            phase: FactsPhase::Schema,
                        },
                        &mut ProofScratch::default(),
                    )?;
                    Membership::Closed(Arc::from([]))
                } else if let Some(declared) = self.nominal_declared_membership(&schema, goal)? {
                    declared
                } else {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: conditional generic nominal default obligations require a broader source proof service",
                    ));
                };
                push(&self.budget, &mut membership, evidence)?;
            }
        }
        retain::<NominalDeclarationProof>(&self.budget, 1)?;
        Ok(NominalDeclarationProof {
            world: self.id,
            schema,
            membership: membership.into(),
        })
    }

    pub(super) fn replay_nominal_declaration_with(
        &self,
        replays: &mut NominalReplays,
        declaration: DeclId,
        proof: &NominalDeclarationProof,
    ) -> Result<()> {
        // Repeat live issuer and module admission even for a completed proof.
        self.info(declaration)?;
        for (retained_declaration, retained) in &replays.nominal_declaration_replays {
            charged(&self.budget, self.budget.charge_work(1))?;
            if *retained_declaration == declaration
                && proof.world == self.id
                && retained.world == self.id
                && proof.schema.declaration == declaration
                && retained.schema.representation_alignment == proof.schema.representation_alignment
                && Arc::ptr_eq(&retained.schema.env, &proof.schema.env)
                && Arc::ptr_eq(&retained.schema.predicates, &proof.schema.predicates)
                && Arc::ptr_eq(&retained.schema.projections, &proof.schema.projections)
                && Arc::ptr_eq(&retained.schema.defaults, &proof.schema.defaults)
                && Arc::ptr_eq(&retained.membership, &proof.membership)
            {
                return Ok(());
            }
        }
        self.replay_nominal_declaration(declaration, proof)?;
        if replays.nominal_declaration_replays.len() < MAX_SCRATCH_OBLIGATIONS {
            push_scratch_row(
                &self.budget,
                &mut replays.nominal_declaration_replays,
                &mut replays.nominal_declaration_replay_storage,
                (declaration, proof.clone()),
            )?;
        }
        Ok(())
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
        if proof.schema.representation_alignment != expected.schema.representation_alignment
            || !self.same_env(&proof.schema.env, &expected.schema.env)?
            || proof.schema.predicates.len() != expected.schema.predicates.len()
            || proof.schema.projections.len() != expected.schema.projections.len()
            || proof.schema.defaults.len() != expected.schema.defaults.len()
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
        for (actual, expected) in proof
            .schema
            .defaults
            .iter()
            .zip(expected.schema.defaults.iter())
        {
            self.validate_ty_metadata_in(&actual.ty, &proof.schema.env, 0)?;
            if actual.parameter != expected.parameter
                || actual.syntax != expected.syntax
                || !self.eq_ty(&actual.ty, &expected.ty, 0)?
                || actual.obligations.len() != expected.obligations.len()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal default source occurrence, parameter slot, type or order changed",
                ));
            }
            for (actual, expected) in actual.obligations.iter().zip(expected.obligations.iter()) {
                self.validate_predicate_metadata(actual, &proof.schema.env)?;
                if !self.eq_predicate(actual, expected)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "nominal default obligation changed",
                    ));
                }
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
                        transport: ParentLocalTransport::default(),
                        original: None,
                        source_metadata: None,
                        predicates: &self.empty,
                        executable_storage: true,
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

impl crate::engine::Fact for NominalDeclarationProof {}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> crate::compiler::frontend::PlanCtx {
        let source = crate::compiler::frontend::parse("trait Base<A,B>{type Item;}trait Sub<A,B>:Base<A,B>{}struct Direct<T:Base<i32,bool>>{value:<T as Base<i32,bool>>::Item}struct Inherited<T:Sub<i32,bool>>{value:<T as Base<i32,bool>>::Item}fn main(){}").unwrap();
        crate::compiler::frontend::PlanCtx::from_program(&source, &ir::PlanBudget::default())
            .unwrap()
    }

    #[test]
    fn nominal_representation_alignment_proof_replay_rejects_injected_and_changed_fields() {
        let source = crate::compiler::frontend::parse("struct Plain{value:u8}fn main(){}").unwrap();
        let plan =
            crate::compiler::frontend::PlanCtx::from_program(&source, &ir::PlanBudget::default())
                .unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let issued = world.nominal_declaration_wellformed(declaration).unwrap();
        assert_eq!(issued.schema.representation_alignment, None);
        let mut replays = NominalReplays::default();
        for _ in 0..2 {
            world
                .replay_nominal_declaration_with(&mut replays, declaration, &issued)
                .unwrap();
            assert_eq!(replays.nominal_declaration_replays.len(), 1);
        }
        world
            .replay_nominal_declaration(declaration, &issued)
            .unwrap();

        let mut injected = issued.clone();
        injected.schema.representation_alignment = Some(3);
        assert!(
            world
                .replay_nominal_declaration_with(&mut replays, declaration, &injected)
                .is_err()
        );
        let error = world
            .replay_nominal_declaration(declaration, &injected)
            .unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
        assert!(error.to_string().contains("omitted or changed source rows"));

        let mut changed = injected;
        changed.schema.representation_alignment = Some(8);
        assert!(
            world
                .replay_nominal_declaration_with(&mut replays, declaration, &changed)
                .is_err()
        );
        let error = world
            .replay_nominal_declaration(declaration, &changed)
            .unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
        assert!(error.to_string().contains("omitted or changed source rows"));

        // Restoring the genuinely issued plain schema is valid. There is no
        // required alignment to omit from an authentic plain declaration.
        changed.schema.representation_alignment = None;
        world
            .replay_nominal_declaration_with(&mut replays, declaration, &changed)
            .unwrap();
        assert_eq!(replays.nominal_declaration_replays.len(), 1);
        world
            .replay_nominal_declaration(declaration, &changed)
            .unwrap();
        world
            .replay_nominal_declaration(declaration, &issued)
            .unwrap();
    }

    #[test]
    fn nominal_representation_alignment_proof_replay_rejects_omitted_source_alignment() {
        let source = crate::compiler::frontend::parse(
            "struct Plain{value:u8}#[repr(align(3))]struct Invalid{value:u8}#[repr(align(8))]struct Unsupported{value:u8}fn main(){}",
        ).unwrap();
        let plan =
            crate::compiler::frontend::PlanCtx::from_program(&source, &ir::PlanBudget::default())
                .unwrap();
        let world = plan.trait_world().unwrap();
        let plain = plan.nominal_declaration(0).unwrap();
        let issued = world.nominal_declaration_wellformed(plain).unwrap();
        world.replay_nominal_declaration(plain, &issued).unwrap();

        for (index, alignment, kind, diagnostic) in [
            (1, 3, FactsErrorKind::Unproved, "E0589"),
            (
                2,
                8,
                FactsErrorKind::Unsupported,
                "unsupported: repr(align) layout",
            ),
        ] {
            let declaration = plan.nominal_declaration(index).unwrap();
            let authentic = world
                .schema
                .nominal_declaration_schema(declaration)
                .unwrap();
            assert_eq!(authentic.declaration, declaration);
            assert_eq!(authentic.representation_alignment, Some(alignment));
            let error = world
                .nominal_declaration_wellformed(declaration)
                .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert!(error.to_string().contains(diagnostic));

            // Aligned proofs cannot genuinely issue in this bounded milestone.
            // Exercise the accessible same-world replay seam using an otherwise
            // issued plain proof with the authentic target schema. Only its
            // representation row is then omitted or changed. Re-derivation of
            // the original source, not the forged value, must reject admission.
            for replacement in [None, Some(4), Some(16)] {
                let mut forged = issued.clone();
                forged.schema = authentic.clone();
                forged.schema.representation_alignment = replacement;
                let error = world
                    .replay_nominal_declaration(declaration, &forged)
                    .unwrap_err();
                assert_eq!(error.kind(), kind, "replacement={replacement:?}");
                assert!(error.to_string().contains(diagnostic), "{error}");
            }
        }
        world.replay_nominal_declaration(plain, &issued).unwrap();
    }

    #[test]
    fn nominal_default_replay_binds_original_slots_syntax_types_and_obligations() {
        let source = crate::compiler::frontend::parse(
            "struct First<T=i32,U=bool>{a:T,b:U}struct Second<T=bool>{value:T}struct Conditional<T:Copy,U:Copy=T>{a:T,b:U}fn main(){}"
        ).unwrap();
        let plan =
            crate::compiler::frontend::PlanCtx::from_program(&source, &ir::PlanBudget::default())
                .unwrap();
        let world = plan.trait_world().unwrap();
        let first = plan.nominal_declaration(0).unwrap();
        let second = plan.nominal_declaration(1).unwrap();
        let conditional = plan.nominal_declaration(2).unwrap();
        let proof = world.nominal_declaration_wellformed(first).unwrap();
        world.replay_nominal_declaration(first, &proof).unwrap();
        let other = world.nominal_declaration_wellformed(second).unwrap();
        let conditional_proof = world.nominal_declaration_wellformed(conditional).unwrap();
        world
            .replay_nominal_declaration(conditional, &conditional_proof)
            .unwrap();
        assert!(
            conditional_proof
                .membership
                .iter()
                .any(|entry| matches!(entry, Membership::Declared { .. }))
        );
        let mut omitted = proof.clone();
        omitted.schema.defaults = Arc::from([]);
        assert!(world.replay_nominal_declaration(first, &omitted).is_err());
        let mut swapped = proof.clone();
        Arc::make_mut(&mut swapped.schema.defaults).swap(0, 1);
        assert!(world.replay_nominal_declaration(first, &swapped).is_err());
        let mut wrong_slot = proof.clone();
        Arc::make_mut(&mut wrong_slot.schema.defaults)[0]
            .parameter
            .index = 1;
        assert!(
            world
                .replay_nominal_declaration(first, &wrong_slot)
                .is_err()
        );
        let mut foreign_syntax = proof.clone();
        Arc::make_mut(&mut foreign_syntax.schema.defaults)[0].syntax =
            other.schema.defaults[0].syntax;
        assert!(
            world
                .replay_nominal_declaration(first, &foreign_syntax)
                .is_err()
        );
        let mut fallback_type = proof.clone();
        Arc::make_mut(&mut fallback_type.schema.defaults)[0].ty =
            other.schema.defaults[0].ty.clone();
        assert!(
            world
                .replay_nominal_declaration(first, &fallback_type)
                .is_err()
        );
        let mut erased_goals = proof.clone();
        Arc::make_mut(&mut erased_goals.schema.defaults)[0].obligations = Arc::from([]);
        assert!(
            world
                .replay_nominal_declaration(first, &erased_goals)
                .is_err()
        );
        let mut foreign_goals = proof.clone();
        Arc::make_mut(&mut foreign_goals.schema.defaults)[0].obligations =
            other.schema.defaults[0].obligations.clone();
        assert!(
            world
                .replay_nominal_declaration(first, &foreign_goals)
                .is_err()
        );
        let mut erased_membership = proof.clone();
        erased_membership.membership = Arc::from([]);
        assert!(
            world
                .replay_nominal_declaration(first, &erased_membership)
                .is_err()
        );
        let mut forged_conditional = conditional_proof.clone();
        let index = forged_conditional
            .membership
            .iter()
            .position(|entry| matches!(entry, Membership::Declared { .. }))
            .unwrap();
        Arc::make_mut(&mut forged_conditional.membership)[index] = Membership::Declared {
            bound: usize::MAX,
            path: Arc::from([]),
        };
        assert!(
            world
                .replay_nominal_declaration(conditional, &forged_conditional)
                .is_err()
        );
        assert!(world.replay_nominal_declaration(second, &proof).is_err());
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
