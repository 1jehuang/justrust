//! Exact source carrier lifetime authority and its nominal transport.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn source_carrier_outlives(
        &self,
        source: &ir::SourceRegionEnv,
        signature: &Signature,
        longer: &ir::CarrierRegionTerm,
        shorter: &ir::CarrierRegionTerm,
    ) -> Result<bool> {
        let mut edges = Vec::new();
        for &(a, b) in source.outlives.iter() {
            push(
                &self.budget,
                &mut edges,
                (
                    ir::CarrierRegionTerm::Named(a),
                    ir::CarrierRegionTerm::Named(b),
                ),
            )?;
        }
        for &(a, b) in signature.ref_contract.outlives.iter() {
            push(
                &self.budget,
                &mut edges,
                (
                    ir::CarrierRegionTerm::Fixed(a),
                    ir::CarrierRegionTerm::Fixed(b),
                ),
            )?;
        }
        for (index, mapped) in source.normalized.iter().enumerate() {
            if let Some(fixed) = mapped {
                let named = ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                    declaration: source.declaration,
                    index: index as u16,
                });
                push(
                    &self.budget,
                    &mut edges,
                    (named.clone(), ir::CarrierRegionTerm::Fixed(*fixed)),
                )?;
                push(
                    &self.budget,
                    &mut edges,
                    (ir::CarrierRegionTerm::Fixed(*fixed), named),
                )?;
            }
        }
        // Well-formed input types: every stored reference outlives each
        // enclosing reference that gives access to that carrier.
        for child in source.carriers.iter().filter(|row| row.parameter.is_some()) {
            for parent in source
                .carriers
                .iter()
                .filter(|row| row.parameter == child.parameter)
            {
                charged(
                    &self.budget,
                    self.budget.charge_work(
                        child.location.path.0.len() + parent.location.path.0.len() + 1,
                    ),
                )?;
                if enclosing(&parent.location, &child.location) {
                    push(
                        &self.budget,
                        &mut edges,
                        (child.region.clone(), parent.region.clone()),
                    )?;
                }
            }
        }
        let mut ready = Vec::new();
        push(&self.budget, &mut ready, longer.clone())?;
        let mut index = 0;
        while let Some(term) = ready.get(index).cloned() {
            if &term == shorter {
                return Ok(true);
            }
            index += 1;
            for (a, b) in &edges {
                charged(&self.budget, self.budget.charge_work(ready.len() + 1))?;
                if *a == term && !ready.contains(b) {
                    push(&self.budget, &mut ready, b.clone())?;
                }
            }
        }
        Ok(false)
    }
    pub(super) fn source_carrier_eligible(
        &self,
        source: &ir::SourceRegionEnv,
        signature: &Signature,
        destination: &ir::CarrierLocation,
        input: &ir::InputCarrier,
    ) -> Result<Option<bool>> {
        let output = source
            .carriers
            .iter()
            .find(|row| row.parameter.is_none() && row.location == *destination);
        let Some(output) = output else {
            return Ok(None);
        };
        let incoming = source
            .carriers
            .iter()
            .find(|row| row.parameter == Some(input.param) && row.location == input.location);
        let Some(incoming) = incoming else {
            // Missing source metadata cannot prove a named output lifetime.
            // A symbolic generic tail remains governed by its family schema.
            return Ok(Some(false));
        };
        if let (
            ir::CarrierRegionTerm::OpaqueFamily { opaque },
            ir::CarrierRegionTerm::Family { parameter },
            ir::CarrierChannel::OpaqueTail {
                opaque: destination_opaque,
                prefix: 0,
            },
            ir::CarrierChannel::ParamTail {
                parameter: input_parameter,
                prefix: 0,
            },
        ) = (
            &output.region,
            &incoming.region,
            &destination.channel,
            &input.location.channel,
        ) {
            if opaque != destination_opaque || parameter != input_parameter {
                return Err(FactsError::invalid(
                    &self.budget,
                    "opaque capture region row changed its exact channel family",
                ));
            }
            // Declaration capture eligibility is not a named-region outlives
            // assertion and cannot authorize hidden-operand transport.
            return self
                .carrier_source_eligible(signature, destination, &signature.result, input, None)
                .map(Some);
        }
        Ok(Some(self.source_carrier_outlives(
            source,
            signature,
            &incoming.region,
            &output.region,
        )?))
    }
    pub(super) fn carrier_contract_refines(
        &self,
        required: &ir::CarrierContract,
        provided: &ir::CarrierContract,
    ) -> Result<bool> {
        if required.inputs != provided.inputs
            || required.results.len() != provided.results.len()
            || required.writes.len() != provided.writes.len()
        {
            return Ok(false);
        }
        for row in provided.results.iter() {
            charged(
                &self.budget,
                self.budget.charge_work(required.results.len() + 1),
            )?;
            let Some(expected) = required
                .results
                .iter()
                .find(|other| other.location == row.location)
            else {
                return Ok(false);
            };
            for source in row.sources.iter() {
                charged(
                    &self.budget,
                    self.budget.charge_work(expected.sources.len() + 1),
                )?;
                if !expected.sources.contains(source) {
                    return Ok(false);
                }
            }
        }
        for row in provided.writes.iter() {
            charged(
                &self.budget,
                self.budget.charge_work(required.writes.len() + 1),
            )?;
            let Some(expected) = required
                .writes
                .iter()
                .find(|other| other.destination == row.destination)
            else {
                return Ok(false);
            };
            for source in row.sources.iter() {
                charged(
                    &self.budget,
                    self.budget.charge_work(expected.sources.len() + 1),
                )?;
                // Retaining the destination's original origin is not a new
                // externally observable write effect. Every other possible
                // origin must be admitted by the independently required row.
                if *source != row.destination && !expected.sources.contains(source) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub(super) fn source_write_eligible(
        &self,
        source: &ir::SourceRegionEnv,
        signature: &Signature,
        destination: &ir::InputCarrier,
        input: &ir::InputCarrier,
    ) -> Result<Option<bool>> {
        let Some(output) = source.carriers.iter().find(|row| {
            row.parameter == Some(destination.param) && row.location == destination.location
        }) else {
            return Ok(None);
        };
        let Some(incoming) = source
            .carriers
            .iter()
            .find(|row| row.parameter == Some(input.param) && row.location == input.location)
        else {
            return Ok(Some(false));
        };
        Ok(Some(self.source_carrier_outlives(
            source,
            signature,
            &incoming.region,
            &output.region,
        )?))
    }

    pub(super) fn checked_source_carriers(
        &self,
        source: &ir::SourceRegionEnv,
        raw: &Signature,
        signature: &Signature,
    ) -> Result<Arc<[ir::SourceCarrierRegion]>> {
        if source.carriers.len() > ir::MAX_LOAN_SITES {
            return Err(FactsError::resource(
                &self.budget,
                "source carrier region resource limit exceeded",
            ));
        }
        let mut rows: Vec<ir::SourceCarrierRegion> = Vec::new();
        for row in source.carriers.iter() {
            let (original, actual) = if let Some(parameter) = row.parameter {
                (raw.params.get(parameter), signature.params.get(parameter))
            } else {
                (Some(&raw.result), Some(&signature.result))
            };
            let (Some(original), Some(actual)) = (original, actual) else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source carrier parameter outside declaration",
                ));
            };
            let location = self.rebase_source_location(&row.location, original, actual)?;
            match row.region {
                ir::CarrierRegionTerm::Named(id)
                    if id.declaration == source.declaration && id.index < source.region_count =>
                {
                    ()
                }
                ir::CarrierRegionTerm::Fixed(id) if id.0 < signature.ref_contract.region_count => {
                    ()
                }
                _ => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "source carrier region outside declaration",
                    ));
                }
            }
            for old in &rows {
                charged(&self.budget, self.budget.charge_work(1))?;
                if old.parameter == row.parameter && old.location == location {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "duplicate source carrier region row",
                    ));
                }
            }
            push(
                &self.budget,
                &mut rows,
                ir::SourceCarrierRegion {
                    parameter: row.parameter,
                    location,
                    region: row.region.clone(),
                },
            )?;
        }
        Ok(rows.into())
    }
    fn rebase_source_location(
        &self,
        location: &ir::CarrierLocation,
        mut original: &Ty,
        mut actual: &Ty,
    ) -> Result<ir::CarrierLocation> {
        let mut path = Vec::new();
        for step in location.path.0.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            let mut rebased = step.clone();
            fn descend(ty: &Ty, step: ir::CarrierStep) -> Option<&Ty> {
                match (step, ty) {
                    (ir::CarrierStep::Referent, Ty::Ref { referent, .. }) => Some(referent),
                    (ir::CarrierStep::Field(index), _) => ty.field_ty(index),
                    (ir::CarrierStep::ConstantIndex(index), Ty::Array { element, len })
                        if index < *len =>
                    {
                        Some(element)
                    }
                    (ir::CarrierStep::VariantField { variant, field }, Ty::Enum(value))
                        if variant.owner == value.def().id =>
                    {
                        value
                            .def()
                            .variants
                            .get(variant.index)?
                            .fields
                            .get(field)
                            .map(|f| &f.ty)
                    }
                    _ => None,
                }
            }
            let Some(next_original) = descend(original, step.clone()) else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source carrier path has foreign nominal identity",
                ));
            };
            if let (ir::CarrierStep::VariantField { variant, field }, Ty::Enum(value)) =
                (step.clone(), actual)
            {
                rebased = ir::CarrierStep::VariantField {
                    variant: ir::VariantId {
                        owner: value.def().id,
                        index: variant.index,
                    },
                    field,
                };
            }
            let Some(next_actual) = descend(actual, rebased.clone()) else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source carrier path does not normalize structurally",
                ));
            };
            original = next_original;
            actual = next_actual;
            push(&self.budget, &mut path, rebased)?;
        }
        let result = ir::CarrierLocation {
            path: ir::CarrierPath(path.into()),
            channel: location.channel.clone(),
        };
        charged(
            &self.budget,
            ir::carrier_at(
                actual,
                &ir::CarrierLocation {
                    path: ir::CarrierPath(Arc::from([])),
                    channel: result.channel.clone(),
                },
                &self.budget,
            ),
        )?;
        Ok(result)
    }
}
fn enclosing(parent: &ir::CarrierLocation, child: &ir::CarrierLocation) -> bool {
    let ir::CarrierChannel::Fixed(parent_depth) = parent.channel else {
        return false;
    };
    if child.path == parent.path {
        return matches!(child.channel, ir::CarrierChannel::Fixed(child_depth) if child_depth.depth > parent_depth.depth);
    }
    if !child.path.0.starts_with(&parent.path.0) {
        return false;
    }
    let suffix = &child.path.0[parent.path.0.len()..];
    suffix
        .iter()
        .take_while(|step| matches!(step, ir::CarrierStep::Referent))
        .count()
        > usize::from(parent_depth.depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_carrier_refinement_rejects_added_origins_and_changed_topology() {
        let budget = PlanBudget::default();
        let program = crate::compiler::frontend::parse("fn main(){}").unwrap();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let location = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        let input = ir::InputCarrier {
            param: 0,
            location: location.clone(),
        };
        let other = ir::InputCarrier {
            param: 1,
            location: location.clone(),
        };
        let required = ir::CarrierContract {
            inputs: Arc::from([input.clone(), other.clone()]),
            results: Arc::from([ir::CarrierReturn {
                location: location.clone(),
                sources: Arc::from([input.clone()]),
            }]),
            writes: Arc::from([ir::CarrierWriteEnvelope {
                destination: input.clone(),
                sources: Arc::from([]),
            }]),
        };
        let mut provided = required.clone();
        assert!(
            world
                .carrier_contract_refines(&required, &provided)
                .unwrap()
        );
        let mut writes = provided.writes.to_vec();
        writes[0].sources = Arc::from([input.clone()]);
        provided.writes = writes.into();
        assert!(
            world
                .carrier_contract_refines(&required, &provided)
                .unwrap()
        );
        let mut returns = provided.results.to_vec();
        returns[0].sources = Arc::from([other.clone()]);
        provided.results = returns.into();
        assert!(
            !world
                .carrier_contract_refines(&required, &provided)
                .unwrap()
        );
        provided.results = required.results.clone();
        let mut writes = provided.writes.to_vec();
        writes[0].sources = Arc::from([other]);
        provided.writes = writes.into();
        assert!(
            !world
                .carrier_contract_refines(&required, &provided)
                .unwrap()
        );
        provided = required.clone();
        provided.inputs = Arc::from([]);
        assert!(
            !world
                .carrier_contract_refines(&required, &provided)
                .unwrap()
        );
    }

    #[test]
    fn distinct_nominal_carrier_binders_do_not_imply_outlives() {
        let budget = PlanBudget::default();
        let program = crate::compiler::frontend::parse("struct W<'a,'b>{a:&'a i32,b:&'b i32}impl<'a,'b> W<'a,'b>{fn first(&self)->&'a i32{self.b}}fn main(){}").unwrap();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let mut checked = false;
        for index in 0..plan.callable_count() {
            let declaration = plan.callable_decl(index).unwrap();
            let source = plan.source_region_env(declaration).unwrap();
            if source.region_count != 2 {
                continue;
            }
            let signature = world
                .schema
                .declaration_schema(declaration)
                .unwrap()
                .signature;
            let a = ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                declaration,
                index: 0,
            });
            let b = ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                declaration,
                index: 1,
            });
            assert!(
                !world
                    .source_carrier_outlives(&source, &signature, &b, &a)
                    .unwrap()
            );
            let (contract, _) = world.derive_carriers(&signature, &source).unwrap();
            for output in contract.results.iter() {
                for input in output.sources.iter() {
                    let row = source
                        .carriers
                        .iter()
                        .find(|r| r.parameter == Some(input.param) && r.location == input.location)
                        .unwrap();
                    assert_ne!(row.region, b);
                }
            }
            checked = true;
        }
        assert!(checked);
    }

    #[test]
    fn source_carrier_rows_reject_forged_regions_paths_and_parameters() {
        let budget = PlanBudget::default();
        let program =
            crate::compiler::frontend::parse("fn f<'a>(x:&'a i32)->&'a i32{x}fn main(){}").unwrap();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let signature = world
            .schema
            .declaration_schema(declaration)
            .unwrap()
            .signature;
        let source = plan.source_region_env(declaration).unwrap();
        assert!(!source.carriers.is_empty());
        assert!(
            world
                .checked_source_carriers(&source, &signature, &signature)
                .is_ok()
        );
        for kind in 0..5 {
            let mut forged = source.as_ref().clone();
            let mut rows = forged.carriers.to_vec();
            match kind {
                0 => rows[0].parameter = Some(usize::MAX),
                1 => {
                    rows[0].location.path = ir::CarrierPath(Arc::from([ir::CarrierStep::Field(9)]))
                }
                2 => {
                    rows[0].region = ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                        declaration,
                        index: u16::MAX,
                    })
                }
                3 => rows[0].region = ir::CarrierRegionTerm::Fixed(ir::RegionParamId(u16::MAX)),
                _ => rows.push(rows[0].clone()),
            }
            forged.carriers = rows.into();
            assert!(
                world
                    .checked_source_carriers(&forged, &signature, &signature)
                    .is_err(),
                "forged row kind {kind}"
            );
        }
    }
}
