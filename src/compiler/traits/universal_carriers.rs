//! Exact associated-projection families in declaration carrier transport.
use super::*;

/// A lexical borrow of one fully replayed immutable view and its exact caller.
/// It carries no family answers and cannot be retained in facts or scratch.
#[derive(Clone, Copy)]
pub(in crate::compiler) struct CarrierViewReplay<'a> {
    view: &'a CarrierContractView,
    facts: &'a ValidatedFacts,
}

impl<'a> CarrierViewReplay<'a> {
    pub(in crate::compiler) fn view(&self) -> &'a CarrierContractView {
        self.view
    }
    pub(in crate::compiler) fn facts(&self) -> &'a ValidatedFacts {
        self.facts
    }

    pub(in crate::compiler) fn validate_for(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        facts.world.check_facts(facts, true)?;
        facts.world.check_context(ProofContext::from_facts(facts))?;
        facts.world.check_normalization_budget()?;
        checked_module(
            facts.budget(),
            facts
                .world
                .catalog
                .modules()
                .budget()
                .check()
                .map_err(ModuleError::from),
        )?;
        if !std::ptr::eq(self.view, view)
            || !std::ptr::eq(self.facts, facts)
            || view.physical.is_some()
            || view.world != facts.world.id
            || view.registry != facts.registry_snapshot()
            || view.environment != facts.predicates.index
            || view.phase != facts.phase()
            || !view.budget.same_authority(facts.budget())
            || view.executable_storage != view.source_schema.is_none()
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "carrier replay borrow changed exact view or live caller context",
            ));
        }
        Ok(())
    }
}

impl CarrierContractView {
    pub(in crate::compiler) fn replay_for_families<'a>(
        &'a self,
        facts: &'a ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<CarrierViewReplay<'a>> {
        self.validate_for_with_scratch(facts, scratch)?;
        Ok(CarrierViewReplay { view: self, facts })
    }

    pub fn family_actual(
        &self,
        family: &ir::CarrierFamily,
        facts: &ValidatedFacts,
    ) -> Result<Option<Ty>> {
        self.family_actual_with_scratch(family, facts, &mut TraitScratch::default())
    }

    pub fn family_actual_with_scratch(
        &self,
        family: &ir::CarrierFamily,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Ty>> {
        let replay = self.replay_for_families(facts, scratch)?;
        self.family_actual_with_replay(family, facts, &replay, scratch)
    }

    pub(in crate::compiler) fn family_actual_with_replay(
        &self,
        family: &ir::CarrierFamily,
        facts: &ValidatedFacts,
        replay: &CarrierViewReplay<'_>,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Ty>> {
        replay.validate_for(self, facts)?;
        let actuals = if self.source_schema.is_some() || self.nominal.is_some() {
            nominal_carriers::CarrierFamilyActuals::from_signature(
                &facts.world,
                &facts
                    .world
                    .normalized_declaration_schema(self.declaration)?,
                &self.arguments,
                &self.result_signature,
            )?
        } else {
            nominal_carriers::CarrierFamilyActuals::default()
        };
        match family {
            ir::CarrierFamily::Array(array) => {
                let mut found = false;
                for location in self
                    .original_contract
                    .inputs
                    .iter()
                    .map(|input| &input.location)
                    .chain(
                        self.original_contract
                            .results
                            .iter()
                            .map(|result| &result.location),
                    )
                    .chain(
                        self.original_contract
                            .writes
                            .iter()
                            .map(|write| &write.destination.location),
                    )
                {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if let Some(ir::CarrierFamily::Array(candidate)) = location.channel.family()
                        && candidate == *array
                    {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "array family is absent from exact original carrier contract",
                    ));
                }
                let logical = facts.world.realize_array_family_with_scratch(
                    array,
                    &self.original_env,
                    &self.arguments,
                    facts,
                    scratch,
                )?;
                Ok(Some(actuals.actual(
                    &facts.world,
                    family,
                    logical,
                    facts,
                    scratch,
                )?))
            }
            ir::CarrierFamily::Parameter(parameter) => {
                if self.source_schema.is_some() || self.nominal.is_some() {
                    let arguments = facts.world.carrier_storage_arguments(
                        self.declaration,
                        &self.arguments,
                        &self.result_signature,
                    )?;
                    return Ok(self
                        .original_env
                        .ordered_params()
                        .position(|p| p == *parameter)
                        .and_then(|index| arguments.get(index))
                        .cloned());
                }
                Ok(self.parameter_actual(*parameter)?.map(|(_, ty)| ty.clone()))
            }
            ir::CarrierFamily::Opaque(opaque) => {
                let mut found = false;
                for location in self
                    .original_contract
                    .inputs
                    .iter()
                    .map(|input| &input.location)
                    .chain(
                        self.original_contract
                            .results
                            .iter()
                            .map(|result| &result.location),
                    )
                {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if let Some(ir::CarrierFamily::Opaque(candidate)) = location.channel.family()
                        && facts.world.eq_ty(
                            &Ty::Opaque(candidate),
                            &Ty::Opaque(opaque.clone()),
                            0,
                        )?
                    {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque family is absent from exact original carrier contract",
                    ));
                }
                let logical = facts.world.realize_opaque_family_with_scratch(
                    opaque,
                    &self.original_env,
                    &self.arguments,
                    facts,
                    scratch,
                )?;
                Ok(Some(actuals.actual(
                    &facts.world,
                    family,
                    logical,
                    facts,
                    scratch,
                )?))
            }
            ir::CarrierFamily::Projection(projection) => {
                let mut found = false;
                for input in self.original_contract.inputs.iter() {
                    if let Some(ir::CarrierFamily::Projection(candidate)) =
                        input.location.channel.family()
                        && facts.world.eq_ty(
                            &Ty::AssociatedType(candidate),
                            &Ty::AssociatedType(projection.clone()),
                            0,
                        )?
                    {
                        found = true;
                        break;
                    }
                }
                if !found {
                    for result in self.original_contract.results.iter() {
                        if let Some(ir::CarrierFamily::Projection(candidate)) =
                            result.location.channel.family()
                            && facts.world.eq_ty(
                                &Ty::AssociatedType(candidate),
                                &Ty::AssociatedType(projection.clone()),
                                0,
                            )?
                        {
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projection family is absent from exact original carrier contract",
                    ));
                }
                let logical = facts.world.realize_projection_family_with_scratch(
                    projection,
                    &self.original_env,
                    &self.arguments,
                    facts,
                    scratch,
                )?;
                Ok(Some(actuals.actual(
                    &facts.world,
                    family,
                    logical,
                    facts,
                    scratch,
                )?))
            }
        }
    }
}

impl ValidatedTraitWorld {
    pub(super) fn realize_array_family_with_scratch(
        &self,
        array: &Arc<Ty>,
        env: &GenericEnv,
        args: &[Ty],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Ty> {
        self.check_facts(facts, false)?;
        scratch.for_facts(facts);
        if !matches!(array.as_ref(), Ty::DependentArray { .. }) {
            return Err(FactsError::invalid(
                &self.budget,
                "array family lost its source-dependent extent",
            ));
        }
        let schema = GenericSchema {
            env: Arc::new(env.clone()),
            signature: Signature {
                params: Vec::new(),
                result: array.as_ref().clone(),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let value =
            self.substitute_carrier_schema(&schema, args, facts, scratch.for_world(self.id))?;
        if !matches!(
            value.signature.result,
            Ty::Array { .. } | Ty::DependentArray { .. }
        ) {
            return Err(FactsError::invalid(
                &self.budget,
                "array family substitution changed its shape",
            ));
        }
        self.normalize_carrier_equality_type_with_scratch(&value.signature.result, facts, scratch)
    }

    pub(super) fn realize_projection_family_with_scratch(
        &self,
        projection: &Arc<ir::AssociatedTypeProjection>,
        env: &GenericEnv,
        args: &[Ty],
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Ty> {
        self.check_facts(facts, false)?;
        scratch.for_facts(facts);
        self.validate_universal_projection_with(projection, env, scratch.for_world(self.id))?;
        let raw = Ty::AssociatedType(projection.clone());
        let schema = GenericSchema {
            env: Arc::new(env.clone()),
            signature: Signature {
                params: Vec::new(),
                result: raw.clone(),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let value =
            self.substitute_carrier_schema(&schema, args, facts, scratch.for_world(self.id))?;
        let actual = value.signature.result;
        if self.raw_associated_ty_is_closed(&actual, 0)? {
            // A family realizes declaration carrier topology, not executable
            // field storage. Match the logical equality path below even when
            // the substituted projection is fully closed. Executable roots
            // still require their independent physical signature validation.
            return Ok(self
                .normalize_closed_projected_type_for_use(
                    &actual,
                    ProofContext::from_facts(facts).for_storage(false),
                    false,
                    scratch.for_world(self.id),
                    false,
                )?
                .0);
        }
        let Ty::AssociatedType(projection) = &actual else {
            return Err(FactsError::invalid(
                &self.budget,
                "projection family substitution changed its outer identity",
            ));
        };
        self.validate_universal_projection_with(
            projection,
            facts.env(),
            scratch.for_world(self.id),
        )?;
        self.normalize_carrier_equality_type_with_scratch(&actual, facts, scratch)
    }

    // Carrier channels and the instantiated signature must use the same
    // authenticated equality normal form. A rigid family stays rigid only
    // when the caller has no reduction for that exact associated item.
    pub(super) fn normalize_carrier_equality_type_with_scratch(
        &self,
        ty: &Ty,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Ty> {
        self.check_facts(facts, true)?;
        retain::<ir::RefContract>(&self.budget, 1)?;
        let inputs = facts.parent_local_signature_inputs_with_scratch(
            &Signature {
                params: Vec::new(),
                result: ty.clone(),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            scratch,
        )?;
        let context = ProofContext {
            transport: ParentLocalTransport::new(&inputs),
            ..ProofContext::from_facts(facts).for_storage(false)
        };
        context
            .transport
            .validate_with_scratch(self, facts, scratch)?;
        self.validate_ty_metadata_with_facts(
            ty,
            facts.env(),
            0,
            &mut scratch.for_world(self.id).replays,
            Some(facts),
        )?;
        self.map_universal_projection_type_for_use(
            ty,
            facts.env(),
            0,
            Some((facts, context.transport)),
            false,
            &mut |projection| {
                if let FactsSubject::UniversalCallable(declaration) = facts.subject
                    && let Some(normal) =
                        self.normalize_exact_owner_projection(declaration, projection)?
                {
                    return Ok(normal);
                }
                if self.checked_closure_projection(projection, facts.env())?
                    || self.goal_is_closed_in_with(
                        &projection.trait_ref,
                        context,
                        scratch.for_world(self.id),
                    )?
                    || associated_types::nominal_application_self(&projection.trait_ref.self_ty)
                    || (context.phase == FactsPhase::Universal
                        && associated_types::implementation_application_self(
                            &projection.trait_ref.self_ty,
                        )
                        && matches!(
                            self.prove_implementation_in(
                                &Predicate::Implements(projection.trait_ref.clone()),
                                context,
                                scratch.for_world(self.id),
                            )?,
                            ProofOutcome::Proved(_)
                        ))
                {
                    let evidence = self.normalize_closed_equality_projection_for_use(
                        projection,
                        context,
                        scratch.for_world(self.id),
                        false,
                    )?;
                    return Ok(evidence.ty().clone());
                }
                if self
                    .declared_equality_rhs(projection, &facts.predicates.predicates, facts.env())?
                    .is_some()
                {
                    let evidence = self.normalize_projection_equality(projection, facts)?;
                    return Ok(evidence.ty().clone());
                }
                self.validate_universal_projection_with(
                    projection,
                    facts.env(),
                    scratch.for_world(self.id),
                )?;
                self.require_in(
                    &Predicate::Implements(projection.trait_ref.clone()),
                    context,
                    scratch.for_world(self.id),
                )?;
                retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
                Ok(Ty::AssociatedType(Arc::new(projection.clone())))
            },
        )
    }

    /// A declaration-owned associated producer can carry another family only
    /// when its canonical bound declares that exact output. Sharing a generic
    /// owner or a field path alone never creates this edge.
    pub(super) fn associated_carrier_source(
        &self,
        destination: &ir::CarrierChannel,
        source: &ir::CarrierChannel,
        env: &GenericEnv,
    ) -> Result<bool> {
        let ir::CarrierChannel::ProjectionTail {
            projection: input,
            prefix: 0,
        } = source
        else {
            return Ok(false);
        };
        let wanted = match destination {
            ir::CarrierChannel::ProjectionTail {
                projection,
                prefix: 0,
            } => {
                retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
                Ty::AssociatedType(projection.clone())
            }
            ir::CarrierChannel::ParamTail {
                parameter,
                prefix: 0,
            } => Ty::Param(*parameter),
            _ => return Ok(false),
        };
        let rows = self.associated_bounds_for(input.item, &input.trait_ref, env)?;
        for row in rows.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            let Predicate::ProjectionEq {
                projection: produced,
                ty,
            } = row
            else {
                continue;
            };
            if !self.eq_ty(
                &produced.trait_ref.self_ty,
                &Ty::AssociatedType(input.clone()),
                0,
            )? || !self.eq_ty(ty, &wanted, 0)?
            {
                continue;
            }
            // An assignment without its associated producer membership is not
            // enough. The complete validated bound batch supplies both.
            for member in rows.iter() {
                if matches!(member, Predicate::Implements(_))
                    && self
                        .eq_predicate(member, &Predicate::Implements(produced.trait_ref.clone()))?
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub(super) fn substitute_carrier_schema(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        facts: &ValidatedFacts,
        active: &mut ProofScratch,
    ) -> Result<InstantiatedSchema> {
        self.check_facts(facts, true)?;
        let mut nominals = Vec::new();
        for argument in args {
            for nominal in facts.parent_local_receiver_inputs_with_active(argument, active)? {
                push(&self.budget, &mut nominals, nominal)?;
            }
        }
        let context = ProofContext {
            transport: ParentLocalTransport::new(&nominals),
            ..ProofContext::from_facts(facts).for_storage(false)
        };
        context
            .transport
            .validate_with_replay(self, facts, &mut active.transport_validation)?;
        self.substitute_projected_schema_with(schema, args, context, active)
    }
}
