//! Private nominal139 closed header/projection replay draft.
//! No callable subject, fake function or retained mutable world is introduced.
use super::*;

mod dependency_fields;

#[derive(Clone, Debug)]
pub(in crate::compiler) struct ClosedNominalInput {
    world: WorldId,
    declaration: DeclId,
    source: NominalDeclarationProof,
    actuals: Arc<[Ty]>,
    obligations: Arc<[Predicate]>,
}

impl ClosedNominalInput {
    /// Allocation identity of this exact closed input (node-local memo only).
    pub(super) fn obligation_identity(&self) -> &Arc<[Predicate]> {
        &self.obligations
    }
}

impl ValidatedTraitWorld {
    /// Discharge `predicate` with a fresh proof ancestry that shares the
    /// caller's node-local nominal replay memo.
    fn require_sharing(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
        replays: &mut NominalReplays,
    ) -> Result<ProofEvidence> {
        if replays.obligation_proofs.is_none() {
            replays._obligation_proofs_storage = Some(charged(
                &self.budget,
                self.budget
                    .reserve_bytes(std::mem::size_of::<ProofScratch>()),
            )?);
            replays.obligation_proofs = Some(Box::default());
        }
        let mut scratch = replays.obligation_proofs.take().unwrap();
        scratch.memo = std::mem::take(&mut replays.obligation_memo);
        scratch.memo_index = std::mem::take(&mut replays.obligation_index);
        scratch.active.clear();
        scratch.cycles = 0;
        scratch.replays = std::mem::take(replays);
        let result = self.require_in(predicate, context, &mut scratch);
        let mut restored = std::mem::take(&mut scratch.replays);
        restored.obligation_memo = std::mem::take(&mut scratch.memo);
        restored.obligation_index = std::mem::take(&mut scratch.memo_index);
        restored.obligation_proofs = Some(scratch);
        *replays = restored;
        result
    }

    pub(in crate::compiler) fn replay_nominal_call_selection_with_scratch(
        &self,
        original: &ir::Callee,
        selected: &ir::Callee,
        env: &GenericEnv,
        arguments: &[Ty],
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        let active = scratch.for_world(self.id);
        let same_role = match (original, selected) {
            (ir::Callee::Direct(a), ir::Callee::Direct(b)) => {
                a.declaration == b.declaration && a.type_args.len() == b.type_args.len()
            }
            (ir::Callee::TraitItem(a), ir::Callee::TraitItem(b))
            | (
                ir::Callee::Virtual(ir::VirtualCall { call: a }),
                ir::Callee::Virtual(ir::VirtualCall { call: b }),
            ) => {
                a.item == b.item
                    && a.trait_ref.declaration == b.trait_ref.declaration
                    && a.trait_ref.args.len() == b.trait_ref.args.len()
                    && a.method_args.len() == b.method_args.len()
            }
            _ => false,
        };
        if !same_role
            || env.ordered_params().count() != arguments.len()
            || original.metadata_types().count() != selected.metadata_types().count()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closed call changed original selection identity or arity",
            ));
        }
        for (raw, actual) in original.metadata_types().zip(selected.metadata_types()) {
            if !self.type_contains_projection(raw, 0)? {
                self.validate_rhs_substitution(raw, actual, env, arguments, 0)?;
                continue;
            }
            // Closed call selection has already normalized associated projections.
            // Replay raw substitution first, without treating normalization as a
            // structural substitution or granting it authority over source terms.
            let schema = GenericSchema {
                env: Arc::new(env.clone()),
                signature: Signature {
                    params: Vec::new(),
                    result: raw.clone(),
                    ref_contract: Arc::new(ir::RefContract::default()),
                },
                predicates: Arc::from([]),
            };
            let substituted =
                self.substitute_projected_schema(&schema, arguments, &self.empty.env)?;
            let context = ProofContext {
                transport: ParentLocalTransport::default(),
                original: None,
                source_metadata: None,
                predicates: &self.empty,
                executable_storage: true,
                phase: FactsPhase::Closed,
            };
            // This authenticates the selected source application, not its
            // executable storage. ClosedReceipt subsequently replays the
            // current-engine typed mapped signature and every physical root.
            // Reducing source field schemas here would demand layout from an
            // identity that the canonical graph producer has not yet reminted.
            let (normalized, dependencies) = self.normalize_closed_projected_type_for_use(
                &substituted.signature.result,
                context,
                false,
                active,
                false,
            )?;
            for dependency in &dependencies {
                self.validate_projected_dependency_with_for_use(
                    dependency, context, active, false, false,
                )?;
            }
            if !self.eq_ty(&normalized, actual, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closed call selection changed substituted normalized source type",
                ));
            }
        }
        Ok(())
    }

    pub(in crate::compiler) fn nominal_realization_world(&self) -> u64 {
        self.id.0.graph_word()
    }

    pub(in crate::compiler) fn nominal_realization_parameter(
        &self,
        input: &ClosedNominalInput,
        parameter: &ir::ParamId,
    ) -> Result<Ty> {
        self.nominal_realization_parameter_with(&mut NominalReplays::default(), input, parameter)
    }

    fn nominal_realization_parameter_with(
        &self,
        replays: &mut NominalReplays,
        input: &ClosedNominalInput,
        parameter: &ir::ParamId,
    ) -> Result<Ty> {
        self.replay_closed_nominal_input_with(replays, input)?;
        self.close_nominal_bound_term(
            replays,
            &Ty::Param(*parameter),
            &input.source.schema().env,
            &input.actuals,
            0,
        )
    }

    fn replay_closed_nominal_input_with(
        &self,
        replays: &mut NominalReplays,
        input: &ClosedNominalInput,
    ) -> Result<()> {
        if input.world != self.id || input.declaration != input.source.schema().declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "foreign closed nominal input",
            ));
        }
        if replays.contains_input(input, self)? {
            return Ok(());
        }
        self.replay_nominal_declaration_with(replays, input.declaration, &input.source)?;
        if input.actuals.len() != input.source.schema().env.total_arity()
            || input.obligations.len() != input.source.schema().predicates.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "closed nominal obligation inventory changed",
            ));
        }
        for actual in input.actuals.iter() {
            self.validate_ty_metadata_with(actual, &self.empty.env, 0, replays)?;
            if !self.raw_associated_ty_is_closed(actual, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closed nominal input contains an open actual",
                ));
            }
        }
        let context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &self.empty,
            executable_storage: true,
            phase: FactsPhase::Schema,
        };
        for (source, retained) in input
            .source
            .schema()
            .predicates
            .iter()
            .zip(input.obligations.iter())
        {
            self.budget
                .charge_work(1)
                .map_err(|e| FactsError::from_message(&self.budget, &e))?;
            let expected = self.close_nominal_predicate(
                replays,
                source,
                &input.source.schema().env,
                &input.actuals,
            )?;
            if !self.eq_predicate(&expected, retained)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closed nominal obligation order or terms changed",
                ));
            }
            self.require_sharing(&expected, context, replays)?;
        }
        replays.record_input(input, self)
    }

    pub(in crate::compiler) fn close_nominal_input(
        &self,
        declaration: DeclId,
        source: &NominalDeclarationProof,
        actuals: &[Ty],
    ) -> Result<ClosedNominalInput> {
        self.replay_nominal_declaration(declaration, source)?;
        if actuals.len() != source.schema().env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "nominal actual arity changed",
            ));
        }
        for actual in actuals {
            self.validate_ty_metadata_in(actual, &self.empty.env, 0)?;
            if !self.raw_associated_ty_is_closed(actual, 0)? {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "open nominal realization actual",
                ));
            }
        }
        let context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &self.empty,
            executable_storage: true,
            phase: FactsPhase::Schema,
        };
        let mut obligations = Vec::new();
        for original in source.schema().predicates.iter() {
            let goal = self.close_nominal_predicate(
                &mut NominalReplays::default(),
                original,
                &source.schema().env,
                actuals,
            )?;
            // Declared source assumptions are not copied into this empty context.
            // Every actual implementation/equality is independently discharged.
            self.require_in(&goal, context, &mut ProofScratch::default())?;
            self.budget
                .retain_bytes(std::mem::size_of::<Predicate>())
                .map_err(|e| FactsError::from_message(&self.budget, &e))?;
            obligations.try_reserve_exact(1).map_err(|_| {
                FactsError::resource(&self.budget, "nominal obligation allocation failed")
            })?;
            obligations.push(goal);
        }
        self.budget
            .retain_bytes(std::mem::size_of_val(actuals))
            .map_err(|e| FactsError::from_message(&self.budget, &e))?;
        Ok(ClosedNominalInput {
            world: self.id,
            declaration,
            source: source.clone(),
            actuals: Arc::from(actuals),
            obligations: obligations.into(),
        })
    }

    pub(in crate::compiler) fn replay_closed_nominal_application(
        &self,
        replays: &mut NominalReplays,
        input: &ClosedNominalInput,
        declaration: DeclId,
        proof: &NominalDeclarationProof,
        source: &Ty,
        actuals: &[Ty],
    ) -> Result<()> {
        self.replay_closed_nominal_input_with(replays, input)?;
        self.replay_nominal_declaration_with(replays, declaration, proof)?;
        if input.declaration != declaration || !self.eq_types(&input.actuals, actuals, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "nominal realization input application changed",
            ));
        }
        self.validate_ty_metadata_with(source, &proof.schema().env, 0, replays)?;
        let (source_declaration, source_actuals) = match source {
            Ty::Struct(n) => (n.def().source_decl, &n.def().type_args),
            Ty::Enum(n) => (n.def().source_decl, &n.def().type_args),
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal realization source is not a source schema",
                ));
            }
        };
        if source_declaration != declaration
            || source_actuals.len() != proof.schema().env.total_arity()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "nominal realization source binder changed",
            ));
        }
        for (actual, parameter) in source_actuals
            .iter()
            .zip(proof.schema().env.ordered_params())
        {
            if !self.eq_ty(actual, &Ty::Param(parameter), 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal realization source binder order changed",
                ));
            }
        }
        Ok(())
    }

    pub(in crate::compiler) fn replay_nominal_field_term(
        &self,
        replays: &mut NominalReplays,
        input: &ClosedNominalInput,
        source: &Ty,
        realized: &Ty,
        depth: usize,
    ) -> Result<()> {
        self.budget
            .check_depth(depth)
            .map_err(|e| FactsError::from_message(&self.budget, &e))?;
        self.budget
            .charge_work(1)
            .map_err(|e| FactsError::from_message(&self.budget, &e))?;
        let valid = match (source, realized) {
            (Ty::Cell(source), Ty::Cell(realized)) => {
                source.try_eq(realized, |source, realized| {
                    self.replay_nominal_field_term(replays, input, source, realized, depth + 1)?;
                    Ok::<_, FactsError>(true)
                })?
            }
            (Ty::ManuallyDrop(source), Ty::ManuallyDrop(realized)) => {
                source.try_eq(realized, |source, realized| {
                    self.replay_nominal_field_term(replays, input, source, realized, depth + 1)?;
                    Ok::<_, FactsError>(true)
                })?
            }
            (Ty::PhantomData(source), Ty::PhantomData(realized)) => {
                source.try_eq(realized, |source, realized| {
                    self.replay_nominal_field_term(replays, input, source, realized, depth + 1)?;
                    Ok::<_, FactsError>(true)
                })?
            }
            (Ty::FnPointer(source), Ty::FnPointer(realized)) => {
                source.try_eq(realized, |source, realized| {
                    self.replay_nominal_field_term(replays, input, source, realized, depth + 1)?;
                    Ok::<_, FactsError>(true)
                })?
            }
            (Ty::Param(parameter), _) => {
                let expected =
                    self.nominal_realization_parameter_with(replays, input, parameter)?;
                self.eq_ty(&expected, realized, depth + 1)?
            }
            (Ty::AssociatedType(projection), _) => {
                let (expected, _) =
                    self.normalize_nominal_field_projection_with(replays, input, projection)?;
                self.eq_ty(&expected, realized, depth + 1)?
            }
            (Ty::Struct(a), Ty::Struct(b)) => {
                a.def().source_decl == b.def().source_decl
                    && self.replay_nominal_field_arguments(
                        replays,
                        input,
                        &a.def().type_args,
                        &b.def().type_args,
                        depth + 1,
                    )?
            }
            (Ty::Enum(a), Ty::Enum(b)) => {
                a.def().source_decl == b.def().source_decl
                    && self.replay_nominal_field_arguments(
                        replays,
                        input,
                        &a.def().type_args,
                        &b.def().type_args,
                        depth + 1,
                    )?
            }
            (
                Ty::Ref {
                    kind: a,
                    referent: x,
                },
                Ty::Ref {
                    kind: b,
                    referent: y,
                },
            )
            | (
                Ty::RawPtr {
                    kind: a,
                    pointee: x,
                },
                Ty::RawPtr {
                    kind: b,
                    pointee: y,
                },
            ) => {
                if a != b {
                    false
                } else {
                    self.replay_nominal_field_term(replays, input, x, y, depth + 1)?;
                    true
                }
            }
            (Ty::Slice { element: a }, Ty::Slice { element: b }) => {
                self.replay_nominal_field_term(replays, input, a, b, depth + 1)?;
                true
            }
            (Ty::Array { element: a, len: x }, Ty::Array { element: b, len: y }) => {
                if x != y {
                    false
                } else {
                    self.replay_nominal_field_term(replays, input, a, b, depth + 1)?;
                    true
                }
            }
            (Ty::Tuple(a), Ty::Tuple(b)) => {
                self.replay_nominal_field_arguments(replays, input, a, b, depth + 1)?
            }
            (Ty::Closure(_), _) => {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "nominal field closure replay needs early parent-solved metadata",
                ));
            }
            _ => self.eq_ty(source, realized, depth + 1)?,
        };
        if !valid {
            return Err(FactsError::invalid(
                &self.budget,
                "realized nominal field changed its canonical source term",
            ));
        }
        Ok(())
    }

    fn replay_nominal_field_arguments(
        &self,
        replays: &mut NominalReplays,
        input: &ClosedNominalInput,
        source: &[Ty],
        actual: &[Ty],
        depth: usize,
    ) -> Result<bool> {
        if source.len() != actual.len() {
            return Ok(false);
        }
        for (source, actual) in source.iter().zip(actual) {
            self.replay_nominal_field_term(replays, input, source, actual, depth + 1)?;
        }
        Ok(true)
    }

    pub(in crate::compiler) fn replay_nominal_normalizations(
        &self,
        replays: &mut NominalReplays,
        input: &ClosedNominalInput,
        source: &Ty,
        dependencies: &[NormalizedAssociatedType],
    ) -> Result<()> {
        fn walk(
            world: &ValidatedTraitWorld,
            replays: &mut NominalReplays,
            input: &ClosedNominalInput,
            source: &Ty,
            dependencies: &[NormalizedAssociatedType],
            cursor: &mut usize,
            depth: usize,
        ) -> Result<()> {
            world
                .budget
                .check_depth(depth)
                .map_err(|e| FactsError::from_message(&world.budget, &e))?;
            world
                .budget
                .charge_work(1)
                .map_err(|e| FactsError::from_message(&world.budget, &e))?;
            match source {
                Ty::AssociatedType(projection) => {
                    let (_, expected) = world
                        .normalize_nominal_field_projection_with(replays, input, projection)?;
                    for receipt in &expected {
                        let actual = dependencies.get(*cursor).ok_or_else(|| {
                            FactsError::invalid(
                                &world.budget,
                                "nominal normalization receipt omitted",
                            )
                        })?;
                        world.validate_projected_dependency(
                            actual,
                            ProofContext {
                                transport: ParentLocalTransport::default(),
                                original: None,
                                source_metadata: None,
                                predicates: &world.empty,
                                executable_storage: true,
                                phase: FactsPhase::Schema,
                            },
                        )?;
                        if !world.same_projected_dependency(actual, receipt)? {
                            return Err(FactsError::invalid(
                                &world.budget,
                                "nominal normalization receipt order or identity changed",
                            ));
                        }
                        *cursor += 1;
                    }
                }
                Ty::Struct(nominal) => {
                    for argument in nominal.def().type_args.iter() {
                        walk(
                            world,
                            replays,
                            input,
                            argument,
                            dependencies,
                            cursor,
                            depth + 1,
                        )?;
                    }
                }
                Ty::Enum(nominal) => {
                    for argument in nominal.def().type_args.iter() {
                        walk(
                            world,
                            replays,
                            input,
                            argument,
                            dependencies,
                            cursor,
                            depth + 1,
                        )?;
                    }
                }
                Ty::Cell(marker) => {
                    walk(
                        world,
                        replays,
                        input,
                        marker.argument(),
                        dependencies,
                        cursor,
                        depth + 1,
                    )?;
                }
                Ty::ManuallyDrop(marker) => {
                    walk(
                        world,
                        replays,
                        input,
                        marker.argument(),
                        dependencies,
                        cursor,
                        depth + 1,
                    )?;
                }
                Ty::PhantomData(marker) => {
                    walk(
                        world,
                        replays,
                        input,
                        marker.argument(),
                        dependencies,
                        cursor,
                        depth + 1,
                    )?;
                }
                Ty::FnPointer(pointer) => {
                    for component in pointer.types() {
                        walk(
                            world,
                            replays,
                            input,
                            component,
                            dependencies,
                            cursor,
                            depth + 1,
                        )?;
                    }
                }
                Ty::Tuple(fields) => {
                    for field in fields.iter() {
                        walk(
                            world,
                            replays,
                            input,
                            field,
                            dependencies,
                            cursor,
                            depth + 1,
                        )?;
                    }
                }
                Ty::Ref { referent, .. } => walk(
                    world,
                    replays,
                    input,
                    referent,
                    dependencies,
                    cursor,
                    depth + 1,
                )?,
                Ty::RawPtr { pointee, .. } => walk(
                    world,
                    replays,
                    input,
                    pointee,
                    dependencies,
                    cursor,
                    depth + 1,
                )?,
                Ty::Array { element, .. } | Ty::Slice { element } => {
                    walk(
                        world,
                        replays,
                        input,
                        element,
                        dependencies,
                        cursor,
                        depth + 1,
                    )?;
                }
                _ => {}
            }
            Ok(())
        }
        let mut cursor = 0;
        match source {
            Ty::Struct(nominal) => {
                for field in nominal_fields(&nominal.def().fields, &self.budget)?.iter() {
                    walk(
                        self,
                        replays,
                        input,
                        &field.ty,
                        dependencies,
                        &mut cursor,
                        0,
                    )?;
                }
            }
            Ty::Enum(nominal) => {
                for variant in nominal.def().variants.iter() {
                    for field in nominal_fields(&variant.fields, &self.budget)?.iter() {
                        walk(
                            self,
                            replays,
                            input,
                            &field.ty,
                            dependencies,
                            &mut cursor,
                            0,
                        )?;
                    }
                }
            }
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "normalization receipts require a nominal source",
                ));
            }
        }
        if cursor != dependencies.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "extra nominal normalization receipt",
            ));
        }
        Ok(())
    }

    pub(in crate::compiler) fn normalize_nominal_field_projection(
        &self,
        input: &ClosedNominalInput,
        original: &ir::AssociatedTypeProjection,
    ) -> Result<(Ty, Vec<NormalizedAssociatedType>)> {
        self.normalize_nominal_field_projection_with(
            &mut NominalReplays::default(),
            input,
            original,
        )
    }

    fn normalize_nominal_field_projection_with(
        &self,
        replays: &mut NominalReplays,
        input: &ClosedNominalInput,
        original: &ir::AssociatedTypeProjection,
    ) -> Result<(Ty, Vec<NormalizedAssociatedType>)> {
        self.replay_closed_nominal_input_with(replays, input)?;
        let mut found = false;
        for retained in input.source.schema().projections.iter() {
            self.budget
                .charge_work(1)
                .map_err(|e| FactsError::from_message(&self.budget, &e))?;
            if self.same_projection(original, retained)? {
                found = true;
                break;
            }
        }
        if !found {
            return Err(FactsError::invalid(
                &self.budget,
                "projection is absent from exact nominal source schema",
            ));
        }
        let projection = ir::AssociatedTypeProjection {
            trait_ref: self.close_nominal_trait_ref(
                replays,
                &original.trait_ref,
                &input.source.schema().env,
                &input.actuals,
            )?,
            item: original.item,
        };
        let context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &self.empty,
            executable_storage: true,
            phase: FactsPhase::Schema,
        };
        self.require_sharing(
            &Predicate::Implements(projection.trait_ref.clone()),
            context,
            replays,
        )?;
        let (normalized, dependencies) = self.normalize_closed_projected_type(
            &Ty::AssociatedType(Arc::new(projection)),
            context,
            false,
        )?;
        for dependency in &dependencies {
            self.validate_projected_dependency(
                dependency,
                ProofContext {
                    transport: ParentLocalTransport::default(),
                    original: None,
                    source_metadata: None,
                    predicates: &self.empty,
                    executable_storage: true,
                    phase: FactsPhase::Schema,
                },
            )?;
        }
        Ok((normalized, dependencies))
    }

    fn close_nominal_predicate(
        &self,
        replays: &mut NominalReplays,
        predicate: &Predicate,
        env: &GenericEnv,
        actuals: &[Ty],
    ) -> Result<Predicate> {
        Ok(match predicate {
            Predicate::Implements(goal) => {
                Predicate::Implements(self.close_nominal_trait_ref(replays, goal, env, actuals)?)
            }
            Predicate::ProjectionEq { projection, ty } => Predicate::ProjectionEq {
                projection: ir::AssociatedTypeProjection {
                    trait_ref: self.close_nominal_trait_ref(
                        replays,
                        &projection.trait_ref,
                        env,
                        actuals,
                    )?,
                    item: projection.item,
                },
                ty: self.close_nominal_bound_term(replays, ty, env, actuals, 0)?,
            },
        })
    }

    fn close_nominal_trait_ref(
        &self,
        replays: &mut NominalReplays,
        goal: &TraitRef,
        env: &GenericEnv,
        actuals: &[Ty],
    ) -> Result<TraitRef> {
        let mut args = Vec::new();
        self.budget
            .retain_bytes(std::mem::size_of_val(goal.args.as_ref()))
            .map_err(|e| FactsError::from_message(&self.budget, &e))?;
        args.try_reserve_exact(goal.args.len()).map_err(|_| {
            FactsError::resource(&self.budget, "nominal trait argument allocation failed")
        })?;
        for argument in goal.args.iter() {
            args.push(self.close_nominal_bound_term(replays, argument, env, actuals, 0)?);
        }
        Ok(TraitRef {
            declaration: goal.declaration,
            self_ty: self.close_nominal_bound_term(replays, &goal.self_ty, env, actuals, 0)?,
            args: args.into(),
        })
    }

    fn close_nominal_bound_term(
        &self,
        replays: &mut NominalReplays,
        ty: &Ty,
        env: &GenericEnv,
        actuals: &[Ty],
        depth: usize,
    ) -> Result<Ty> {
        self.budget
            .check_depth(depth)
            .map_err(|e| FactsError::from_message(&self.budget, &e))?;
        self.budget
            .charge_work(1)
            .map_err(|e| FactsError::from_message(&self.budget, &e))?;
        let result = match ty {
            Ty::Cell(marker) => Ty::Cell(marker.try_map(
                &self.budget,
                |e| FactsError::from_message(&self.budget, &e),
                |ty| self.close_nominal_bound_term(replays, ty, env, actuals, depth + 1),
            )?),
            Ty::ManuallyDrop(marker) => Ty::ManuallyDrop(marker.try_map(
                &self.budget,
                |e| FactsError::from_message(&self.budget, &e),
                |ty| self.close_nominal_bound_term(replays, ty, env, actuals, depth + 1),
            )?),
            Ty::PhantomData(marker) => Ty::PhantomData(marker.try_map(
                &self.budget,
                |e| FactsError::from_message(&self.budget, &e),
                |ty| self.close_nominal_bound_term(replays, ty, env, actuals, depth + 1),
            )?),
            Ty::FnPointer(pointer) => Ty::FnPointer(pointer.try_map(
                &self.budget,
                |e| FactsError::from_message(&self.budget, &e),
                |component| {
                    self.close_nominal_bound_term(replays, component, env, actuals, depth + 1)
                },
            )?),
            Ty::Param(parameter) => {
                let mut selected = None;
                for (ordinal, declared) in env.ordered_params().enumerate() {
                    self.budget
                        .charge_work(1)
                        .map_err(|e| FactsError::from_message(&self.budget, &e))?;
                    if &declared == parameter {
                        selected = actuals.get(ordinal);
                        break;
                    }
                }
                selected.cloned().ok_or_else(|| {
                    FactsError::invalid(
                        &self.budget,
                        "nominal bound references a foreign parameter",
                    )
                })?
            }
            Ty::Ref { kind, referent } => Ty::Ref {
                kind: *kind,
                referent: Arc::new(self.close_nominal_bound_term(
                    replays,
                    referent,
                    env,
                    actuals,
                    depth + 1,
                )?),
            },
            Ty::RawPtr { kind, pointee } => Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(self.close_nominal_bound_term(
                    replays,
                    pointee,
                    env,
                    actuals,
                    depth + 1,
                )?),
            },
            Ty::Slice { element } => Ty::Slice {
                element: Arc::new(self.close_nominal_bound_term(
                    replays,
                    element,
                    env,
                    actuals,
                    depth + 1,
                )?),
            },
            Ty::Array { element, len } => Ty::Array {
                len: *len,
                element: Arc::new(self.close_nominal_bound_term(
                    replays,
                    element,
                    env,
                    actuals,
                    depth + 1,
                )?),
            },
            Ty::Tuple(fields) => {
                self.budget
                    .retain_bytes(std::mem::size_of_val(fields.as_ref()))
                    .map_err(|e| FactsError::from_message(&self.budget, &e))?;
                let mut out = Vec::new();
                out.try_reserve_exact(fields.len()).map_err(|_| {
                    FactsError::resource(&self.budget, "nominal tuple argument allocation failed")
                })?;
                for field in fields.iter() {
                    out.push(self.close_nominal_bound_term(
                        replays,
                        field,
                        env,
                        actuals,
                        depth + 1,
                    )?);
                }
                Ty::Tuple(out.into())
            }
            Ty::Closure(_) => {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "closure nominal realization requires an early parent-solved graph frontier",
                ));
            }
            // Existing closed source nominal terms are checked below. An open
            // nominal cannot pass the empty environment and must instead be
            // supplied by an explicit child139 realization read, never reminted.
            _ => ty.clone(),
        };
        self.validate_ty_metadata_with(&result, &self.empty.env, depth, replays)?;
        Ok(result)
    }
}

// Candidate-scoped source proof replay. This returns obligations, not a
// successful recursive type or a provisional nominal read capability.
impl ValidatedTraitWorld {
    /// A completed closed component proves finite metadata without unfolding
    /// recursive pointer fields. Its term language excludes unsized, open and
    /// projected terms, so every admitted nominal is valid in each TypeUse.
    pub(super) fn closed_component_type_height(&self, ty: &Ty) -> Result<Option<usize>> {
        self.closed_component_type_height_with_replays(ty, &mut NominalReplays::default())
    }

    pub(super) fn closed_component_type_height_with_replays(
        &self,
        ty: &Ty,
        replays: &mut NominalReplays,
    ) -> Result<Option<usize>> {
        let (capability, id, declaration) = match ty {
            Ty::Struct(n) => (n.realization(), n.def().id, n.def().source_decl),
            Ty::Enum(n) => (n.realization(), n.def().id, n.def().source_decl),
            _ => return Ok(None),
        };
        let Some(capability) = capability else {
            return Ok(None);
        };
        let Some(height) = capability.component_metadata_height() else {
            return Ok(None);
        };
        capability
            .check_definition(ty)
            .map_err(|error| FactsError::from_message(&self.budget, &error.to_string()))?;
        if id.snapshot != self.registry || declaration.snapshot != self.registry {
            return Err(FactsError::invalid(
                &self.budget,
                "foreign nominal identity",
            ));
        }
        validate_nominal_modules(ty, self.catalog.modules(), &self.budget)?;
        capability
            .replay_with(ty, self, replays)
            .map_err(|error| FactsError::from_message(&self.budget, &error.to_string()))?;
        Ok(Some(height))
    }

    #[cfg(test)]
    pub(in crate::compiler) fn component_type_walk_replays_for_tests(
        &self,
        roots: &[Ty],
        caller: &ValidatedFacts,
    ) -> Result<usize> {
        let mut walk = TypeWalk::default();
        for root in roots {
            self.validate_ty_inner_with_facts(
                root,
                caller.env(),
                0,
                TypeUse::Root,
                &mut walk,
                Some(caller),
            )?;
        }
        Ok(walk
            .component_replays
            .as_deref()
            .map_or(0, |arena| arena.replay_counts().0))
    }

    pub(in crate::compiler) fn symbolic_nominal_requirements(
        &self,
        declaration: DeclId,
        proof: &NominalDeclarationProof,
        source: &Ty,
    ) -> Result<Vec<Ty>> {
        self.symbolic_nominal_requirements_with(
            declaration,
            proof,
            source,
            &mut NominalReplays::default(),
        )
    }

    pub(in crate::compiler) fn symbolic_nominal_requirements_with(
        &self,
        declaration: DeclId,
        proof: &NominalDeclarationProof,
        source: &Ty,
        replays: &mut NominalReplays,
    ) -> Result<Vec<Ty>> {
        self.replay_nominal_declaration_with(replays, declaration, proof)?;
        let index = match source {
            Ty::Struct(n) => n.def().id.index,
            Ty::Enum(n) => n.def().id.index,
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "symbolic source is not nominal",
                ));
            }
        };
        if !matches!(index, ir::NominalIndex::SourceSlot(_)) {
            return Err(FactsError::invalid(
                &self.budget,
                "symbolic source is not a source template",
            ));
        }
        // Authenticate the source allocation/shape in its own exact binder.
        // No closed recursive nominal is invented to make this validation pass.
        // Keep the canonical construction owner across sibling component
        // proofs. validate_full still replays every recursive source payload,
        // receipt and ordered actual. A node-only or metadata-only success
        // cannot stand in for this full source authentication.
        if replays.nominal_session.is_none() {
            replays.nominal_session = self.schema.nominal_validation_session()?;
        }
        let replayed = match replays.nominal_session.as_mut() {
            Some(session) => {
                session.validate_full(source, &proof.schema().env, None, &self.budget)?
            }
            None => false,
        };
        if !replayed {
            self.schema
                .validate_nominal(source, &proof.schema().env, &self.budget)?;
        }
        let (source_declaration, actuals) = match source {
            Ty::Struct(n) => (n.def().source_decl, &n.def().type_args),
            Ty::Enum(n) => (n.def().source_decl, &n.def().type_args),
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "symbolic source is not nominal",
                ));
            }
        };
        if source_declaration != declaration || actuals.len() != proof.schema().env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "symbolic source declaration or binder changed",
            ));
        }
        for (actual, parameter) in actuals.iter().zip(proof.schema().env.ordered_params()) {
            if actual != &Ty::Param(parameter) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "symbolic source binder order changed",
                ));
            }
        }
        if !proof.schema().projections.is_empty() {
            return Err(FactsError::unsupported(
                &self.budget,
                "recursive symbolic projection obligations",
            ));
        }
        let mut requirements = Vec::new();
        for predicate in proof.schema().predicates.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            let Predicate::Implements(goal) = predicate else {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "recursive symbolic equality obligation",
                ));
            };
            if goal.declaration != self.catalog.id(CatalogItem::Sized) || !goal.args.is_empty() {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "recursive symbolic non-Sized obligation",
                ));
            }
            push(&self.budget, &mut requirements, goal.self_ty.clone())?;
        }
        Ok(requirements)
    }

    pub(in crate::compiler) fn require_symbolic_external_sized(
        &self,
        ty: &Ty,
        replays: &mut NominalReplays,
    ) -> Result<()> {
        let predicate = Predicate::Implements(TraitRef {
            declaration: self.catalog.id(CatalogItem::Sized),
            self_ty: ty.clone(),
            args: Arc::from([]),
        });
        self.require_sharing(
            &predicate,
            ProofContext {
                transport: ParentLocalTransport::default(),
                original: None,
                source_metadata: None,
                predicates: &self.empty,
                executable_storage: true,
                phase: FactsPhase::Closed,
            },
            replays,
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_selection_normalizes_only_the_exact_substituted_projection() {
        for source in [
            "struct S;trait Has<A>{type Out;type Other;}impl Has<i32> for S{type Out=i32;type Other=bool;}fn caller<T:Has<i32>>(x:T){}fn main(){}",
            "enum Option<T>{None,Some(T)}struct S;trait Has<A>{type Out;type Other;}trait It{type Item;}struct Buffer<I:It>{value:Option<I::Item>}impl<X:It> Has<i32> for X{type Out=Buffer<X>;type Other=bool;}impl It for S{type Item=i32;}fn caller<T:Has<i32>>(x:T){}fn main(){}",
        ] {
            let source = crate::compiler::frontend::parse(source).unwrap();
            let plan = crate::compiler::frontend::PlanCtx::from_program(
                &source,
                &ir::PlanBudget::default(),
            )
            .unwrap();
            let world = plan.trait_world().unwrap();
            let caller = world
                .headers
                .callables
                .iter()
                .find(|h| h.schema.env.is_generic())
                .unwrap();
            let implementation = &world.headers.impls[0];
            let parameter = caller.schema.env.ordered_params().next().unwrap();
            let projection = ir::AssociatedTypeProjection {
                trait_ref: TraitRef {
                    declaration: implementation.trait_ref.declaration,
                    self_ty: Ty::Param(parameter),
                    args: Arc::from([Ty::Int(ir::IntTy::I32)]),
                },
                item: implementation.associated_types[0].associated,
            };
            let raw = Ty::AssociatedType(Arc::new(projection.clone()));
            let target = |ty| {
                ir::Callee::Direct(ir::CallTarget {
                    declaration: caller.declaration,
                    type_args: Arc::from([ty]),
                })
            };
            let actual_self = if implementation.schema_env.is_generic() {
                world.headers.impls[1].trait_ref.self_ty.clone()
            } else {
                implementation.trait_ref.self_ty.clone()
            };
            let arguments = [actual_self];
            let expected = if implementation.schema_env.is_generic() {
                let schema = GenericSchema {
                    env: implementation.schema_env.clone(),
                    signature: Signature {
                        params: Vec::new(),
                        result: implementation.associated_types[0].rhs.clone(),
                        ref_contract: Arc::new(ir::RefContract::default()),
                    },
                    predicates: Arc::from([]),
                };
                // Reconstruct closed source identity from its genuine generic
                // header, rather than declaring unresolved physical storage.
                world
                    .substitute_projected_schema(&schema, &arguments, &world.empty.env)
                    .unwrap()
                    .signature
                    .result
            } else {
                implementation.associated_types[0].rhs.clone()
            };
            let mut selection_scratch = TraitScratch::default();
            for _ in 0..3 {
                world
                    .replay_nominal_call_selection_with_scratch(
                        &target(raw.clone()),
                        &target(expected.clone()),
                        &caller.schema.env,
                        &arguments,
                        &mut selection_scratch,
                    )
                    .unwrap();
            }
            // Structural substitution itself still cannot erase a projection.
            assert!(
                world
                    .validate_rhs_substitution(&raw, &expected, &caller.schema.env, &arguments, 0,)
                    .is_err()
            );
            for forged in [Ty::Unit, Ty::Int(ir::IntTy::U64)] {
                assert!(
                    world
                        .replay_nominal_call_selection_with_scratch(
                            &target(raw.clone()),
                            &target(forged),
                            &caller.schema.env,
                            &arguments,
                            &mut selection_scratch,
                        )
                        .is_err()
                );
            }
            let mut wrong_argument = projection.clone();
            wrong_argument.trait_ref.args = Arc::from([Ty::Bool]);
            let mut wrong_item = projection;
            wrong_item.item = implementation.associated_types[1].associated;
            for forged in [wrong_argument, wrong_item] {
                assert!(
                    world
                        .replay_nominal_call_selection_with_scratch(
                            &target(Ty::AssociatedType(Arc::new(forged))),
                            &target(expected.clone()),
                            &caller.schema.env,
                            &arguments,
                            &mut selection_scratch,
                        )
                        .is_err()
                );
            }
        }
    }

    fn plan() -> crate::compiler::frontend::PlanCtx {
        let source = crate::compiler::frontend::parse("trait It{type Item;}impl It for i32{type Item=i32;}struct Peek<I:It>{value:I::Item}fn main(){}")
            .unwrap();
        crate::compiler::frontend::PlanCtx::from_program(&source, &ir::PlanBudget::default())
            .unwrap()
    }

    #[test]
    fn symbolic_full_source_replay_retains_only_canonical_construction_session() {
        let source = "struct Outer<T>{head:Inner<T>,tail:(T,T)}struct Inner<T>{value:T}fn main(){}";
        let make_plan = || {
            let program = crate::compiler::frontend::parse(source).unwrap();
            crate::compiler::frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default())
                .unwrap()
        };
        let plan = make_plan();
        let world = plan.trait_world().unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let proof = world.nominal_declaration_wellformed(declaration).unwrap();
        let template = plan.nominal_realization_template(declaration).unwrap();
        let mut replays = NominalReplays::default();
        let expected = world
            .symbolic_nominal_requirements_with(declaration, &proof, &template, &mut replays)
            .unwrap();
        let session = std::ptr::from_ref(replays.nominal_session.as_deref().unwrap());
        let before = world.budget.usage_snapshot().work;
        let actual = world
            .symbolic_nominal_requirements_with(declaration, &proof, &template, &mut replays)
            .unwrap();
        let warm = world.budget.usage_snapshot().work - before;
        assert_eq!(actual, expected);
        assert!(std::ptr::addr_eq(
            session,
            std::ptr::from_ref(replays.nominal_session.as_deref().unwrap())
        ));
        let before = world.budget.usage_snapshot().work;
        let cold = world
            .symbolic_nominal_requirements_with(
                declaration,
                &proof,
                &template,
                &mut NominalReplays::default(),
            )
            .unwrap();
        let cold_work = world.budget.usage_snapshot().work - before;
        assert_eq!(cold, expected);
        assert!(
            warm < cold_work,
            "full source reconstruction did not reuse its canonical session: {warm}/{cold_work}"
        );

        let Ty::Struct(original) = &template else {
            panic!("struct template")
        };
        let mut forged = original.def().clone();
        forged.fields = Arc::<[ir::FieldDef]>::from([]).into();
        let forged = Ty::Struct(ir::StructTy::new_parent_local(forged));
        assert!(
            world
                .symbolic_nominal_requirements_with(declaration, &proof, &forged, &mut replays,)
                .is_err()
        );
        let foreign = make_plan();
        let foreign_world = foreign.trait_world().unwrap();
        assert!(
            foreign_world
                .symbolic_nominal_requirements_with(declaration, &proof, &template, &mut replays,)
                .is_err()
        );
        assert_eq!(
            world
                .symbolic_nominal_requirements_with(declaration, &proof, &template, &mut replays,)
                .unwrap(),
            expected
        );
        assert!(world.budget.first_resource_failure().is_none());
        world.budget.poison_ledger_for_tests();
        let failure = world.budget.first_resource_failure();
        assert!(
            world
                .symbolic_nominal_requirements_with(declaration, &proof, &template, &mut replays,)
                .is_err()
        );
        assert_eq!(world.budget.first_resource_failure(), failure);
    }

    #[test]
    fn nominal_pointer_field_replay_substitutes_only_its_declared_binder() {
        let program = crate::compiler::frontend::parse(
            "struct Marker<T>{field:[fn(T,bool)->T;0]}struct Other<T>{field:T}fn main(){}",
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let source = world.nominal_declaration_wellformed(declaration).unwrap();
        let actual = Ty::Int(ir::IntTy::I32);
        let input = world
            .close_nominal_input(declaration, &source, std::slice::from_ref(&actual))
            .unwrap();
        let template = plan.nominal_realization_template(declaration).unwrap();
        let field = template.field_ty(0, &budget).unwrap().unwrap();
        let pointer = |inputs: Vec<Ty>, output| Ty::Array {
            element: Arc::new(Ty::FnPointer(
                ir::FnPointerTy::new(inputs, output, &budget).unwrap(),
            )),
            len: 0,
        };
        let realized = pointer(vec![actual.clone(), Ty::Bool], actual.clone());
        world
            .replay_nominal_field_term(&mut NominalReplays::default(), &input, &field, &realized, 0)
            .unwrap();
        let closed = world
            .close_nominal_bound_term(
                &mut NominalReplays::default(),
                &field,
                &source.schema().env,
                std::slice::from_ref(&actual),
                0,
            )
            .unwrap();
        assert!(world.eq_ty(&closed, &realized, 0).unwrap());
        for changed in [
            pointer(vec![Ty::Bool, actual.clone()], actual.clone()),
            pointer(vec![actual.clone()], actual.clone()),
            pointer(vec![actual.clone(), Ty::Bool], Ty::Bool),
        ] {
            assert!(
                world
                    .replay_nominal_field_term(
                        &mut NominalReplays::default(),
                        &input,
                        &field,
                        &changed,
                        0
                    )
                    .is_err()
            );
        }
        let foreign = world
            .nominal_declaration_wellformed(plan.nominal_declaration(1).unwrap())
            .unwrap();
        let foreign_parameter = Ty::Param(foreign.schema().env.ordered_params().next().unwrap());
        let forged = pointer(vec![foreign_parameter.clone(), Ty::Bool], foreign_parameter);
        assert!(
            world
                .replay_nominal_field_term(
                    &mut NominalReplays::default(),
                    &input,
                    &forged,
                    &realized,
                    0
                )
                .is_err()
        );
        assert!(
            world
                .close_nominal_bound_term(
                    &mut NominalReplays::default(),
                    &forged,
                    &source.schema().env,
                    std::slice::from_ref(&actual),
                    0
                )
                .is_err()
        );
        let mut wrong_input = input.clone();
        wrong_input.declaration = foreign.schema().declaration;
        assert!(
            world
                .replay_nominal_field_term(
                    &mut NominalReplays::default(),
                    &wrong_input,
                    &field,
                    &realized,
                    0
                )
                .is_err()
        );
    }

    #[test]
    fn closed_nominal_input_replays_real_bounds_and_source_projection() {
        let plan = plan();
        let declaration = plan.nominal_declaration(0).unwrap();
        let world = plan.trait_world().unwrap();
        let source = world.nominal_declaration_wellformed(declaration).unwrap();
        let input = world
            .close_nominal_input(declaration, &source, &[Ty::Int(ir::IntTy::I32)])
            .unwrap();
        let projection = &source.schema().projections[0];
        let mut shared = NominalReplays::default();
        let context = ProofContext {
            transport: ParentLocalTransport::default(),
            original: None,
            source_metadata: None,
            predicates: &world.empty,
            executable_storage: true,
            phase: FactsPhase::Schema,
        };
        let obligation = input
            .obligations
            .first()
            .expect("bounded nominal obligation");
        let cold = world
            .require_sharing(obligation, context, &mut shared)
            .unwrap();
        let arena = shared.obligation_proofs.as_ref().unwrap().as_ref() as *const ProofScratch;
        assert!(shared.obligation_proofs.as_ref().unwrap().active.is_empty());
        let warm = world
            .require_sharing(obligation, context, &mut shared)
            .unwrap();
        assert!(Arc::ptr_eq(&cold.0, &warm.0));
        assert_eq!(
            arena,
            shared.obligation_proofs.as_ref().unwrap().as_ref() as *const ProofScratch
        );
        assert!(shared.obligation_proofs.as_ref().unwrap().active.is_empty());
        let (normalized, receipts) = world
            .normalize_nominal_field_projection(&input, projection)
            .unwrap();
        assert_eq!(normalized, Ty::Int(ir::IntTy::I32));
        assert_eq!(
            world
                .close_nominal_dependency_field(
                    &input,
                    &Ty::AssociatedType(Arc::new(projection.clone()))
                )
                .unwrap(),
            normalized,
        );
        assert!(!receipts.is_empty());
        let template = plan.nominal_realization_template(declaration).unwrap();
        world
            .replay_nominal_normalizations(
                &mut NominalReplays::default(),
                &input,
                &template,
                &receipts,
            )
            .unwrap();
        assert!(
            world
                .replay_nominal_normalizations(
                    &mut NominalReplays::default(),
                    &input,
                    &template,
                    &[]
                )
                .is_err()
        );
        let mut duplicated = receipts.clone();
        duplicated.extend(receipts.iter().cloned());
        assert!(
            world
                .replay_nominal_normalizations(
                    &mut NominalReplays::default(),
                    &input,
                    &template,
                    &duplicated
                )
                .is_err()
        );
        assert!(
            world
                .close_nominal_input(declaration, &source, &[])
                .is_err()
        );
        assert!(
            world
                .close_nominal_input(declaration, &source, &[Ty::Bool])
                .is_err()
        );
        let foreign = self::plan();
        assert!(
            foreign
                .trait_world()
                .unwrap()
                .normalize_nominal_field_projection(&input, projection)
                .is_err()
        );
        let mut absent = projection.clone();
        absent.item = declaration;
        assert!(
            world
                .normalize_nominal_field_projection(&input, &absent)
                .is_err()
        );
        let foreign_world = foreign.trait_world().unwrap();
        assert!(
            foreign_world
                .close_nominal_dependency_field(
                    &input,
                    &Ty::AssociatedType(Arc::new(projection.clone()))
                )
                .is_err()
        );
        assert!(
            world
                .close_nominal_dependency_field(&input, &Ty::AssociatedType(Arc::new(absent)))
                .is_err()
        );
        assert!(
            foreign_world
                .require_sharing(
                    obligation,
                    ProofContext {
                        transport: ParentLocalTransport::default(),
                        original: None,
                        source_metadata: None,
                        predicates: &foreign_world.empty,
                        executable_storage: true,
                        phase: FactsPhase::Schema,
                    },
                    &mut shared,
                )
                .is_err()
        );
        assert!(world.budget.charge_work(usize::MAX).is_err());
        assert!(
            world
                .close_nominal_dependency_field(
                    &input,
                    &Ty::AssociatedType(Arc::new(projection.clone()))
                )
                .is_err()
        );
        assert!(
            world
                .require_sharing(obligation, context, &mut shared)
                .is_err()
        );
    }
}
