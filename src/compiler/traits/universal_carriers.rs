//! Exact associated-projection families in declaration carrier transport.
use super::*;

impl CarrierContractView {
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
        self.validate_for_with_scratch(facts, scratch)?;
        match family {
            ir::CarrierFamily::Parameter(parameter) => {
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
                Ok(Some(facts.world.realize_opaque_family_with_scratch(
                    opaque,
                    &self.original_env,
                    &self.arguments,
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
                Ok(Some(facts.world.realize_projection_family_with_scratch(
                    projection,
                    &self.original_env,
                    &self.arguments,
                    facts,
                    scratch,
                )?))
            }
        }
    }
}

impl ValidatedTraitWorld {
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
            return Ok(self
                .normalize_closed_projected_type_with(
                    &actual,
                    ProofContext::from_facts(facts),
                    false,
                    scratch.for_world(self.id),
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
    fn normalize_carrier_equality_type_with_scratch(
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
            ..ProofContext::from_facts(facts)
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
            ..ProofContext::from_facts(facts)
        };
        context
            .transport
            .validate_with_replay(self, facts, &mut active.transport_validation)?;
        self.substitute_projected_schema_with(schema, args, context, active)
    }
}
