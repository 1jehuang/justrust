//! Exact associated-projection families in declaration carrier transport.
use super::*;

impl CarrierContractView {
    pub fn family_actual(
        &self,
        family: &ir::CarrierFamily,
        facts: &ValidatedFacts,
    ) -> Result<Option<Ty>> {
        self.validate_for(facts)?;
        match family {
            ir::CarrierFamily::Parameter(parameter) => {
                Ok(self.parameter_actual(*parameter)?.map(|(_, ty)| ty.clone()))
            }
            ir::CarrierFamily::Projection(projection) => {
                let mut found = false;
                for input in self.original_contract.inputs.iter() {
                    if let Some(ir::CarrierFamily::Projection(candidate)) =
                        input.location.channel.family()
                    {
                        if facts.world.eq_ty(
                            &Ty::AssociatedType(candidate),
                            &Ty::AssociatedType(projection.clone()),
                            0,
                        )? {
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    for result in self.original_contract.results.iter() {
                        if let Some(ir::CarrierFamily::Projection(candidate)) =
                            result.location.channel.family()
                        {
                            if facts.world.eq_ty(
                                &Ty::AssociatedType(candidate),
                                &Ty::AssociatedType(projection.clone()),
                                0,
                            )? {
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if !found {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "projection family is absent from exact original carrier contract",
                    ));
                }
                Ok(Some(facts.world.realize_projection_family(
                    projection,
                    &self.original_env,
                    &self.arguments,
                    facts,
                )?))
            }
        }
    }
}

impl ValidatedTraitWorld {
    pub(super) fn realize_projection_family(
        &self,
        projection: &Arc<ir::AssociatedTypeProjection>,
        env: &GenericEnv,
        args: &[Ty],
        facts: &ValidatedFacts,
    ) -> Result<Ty> {
        self.check_facts(facts, false)?;
        self.validate_universal_projection(projection, env)?;
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
        let value = self.substitute_projected_schema(&schema, args, facts.env())?;
        let actual = value.signature.result;
        if self.raw_associated_ty_is_closed(&actual, 0)? {
            return Ok(self
                .normalize_closed_projected_type(&actual, ProofContext::from_facts(facts), false)?
                .0);
        }
        let Ty::AssociatedType(projection) = &actual else {
            return Err(FactsError::invalid(
                &self.budget,
                "projection family substitution changed its outer identity",
            ));
        };
        self.validate_universal_projection(projection, facts.env())?;
        Ok(actual)
    }
}
