//! Exact source carrier lifetime authority and its nominal transport.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn retains_shared_slice_family(
        &self,
        signature: &Signature,
        destination: &ir::InputCarrier,
        input: &ir::InputCarrier,
    ) -> Result<bool> {
        if destination.param != input.param {
            return Ok(false);
        }
        let ir::CarrierChannel::Fixed(position) = destination.location.channel else {
            return Ok(false);
        };
        let ir::CarrierChannel::ParamTail {
            parameter,
            prefix: 0,
        } = input.location.channel
        else {
            return Ok(false);
        };
        let Some(suffix) = input
            .location
            .path
            .0
            .strip_prefix(destination.location.path.0.as_ref())
        else {
            return Ok(false);
        };
        charged(&self.budget, self.budget.charge_work(suffix.len()))?;
        if suffix.len() != usize::from(position.depth) + 2
            || suffix.last() != Some(&ir::CarrierStep::SliceElement)
            || !suffix[..suffix.len() - 1]
                .iter()
                .all(|step| *step == ir::CarrierStep::Referent)
        {
            return Ok(false);
        }
        let ty = signature.params.get(input.param).ok_or_else(|| {
            FactsError::invalid(
                &self.budget,
                "slice family source parameter outside signature",
            )
        })?;
        let descriptor = charged(
            &self.budget,
            ir::carrier_at(ty, &destination.location, &self.budget),
        )?;
        let element = charged(
            &self.budget,
            ir::carrier_at(ty, &input.location, &self.budget),
        )?;
        Ok(
            descriptor.shared_slice_element() == Some(&Ty::Param(parameter))
                && element == Ty::Param(parameter),
        )
    }

    pub(super) fn source_carrier_outlives(
        &self,
        source: &ir::SourceRegionEnv,
        signature: &Signature,
        longer: &ir::CarrierRegionTerm,
        shorter: &ir::CarrierRegionTerm,
    ) -> Result<bool> {
        if matches!(longer, ir::CarrierRegionTerm::Static) {
            return Ok(true);
        }
        if matches!(shorter, ir::CarrierRegionTerm::Static) {
            return Ok(false);
        }
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
        // Returning a borrowed slice retains its stored element origins as
        // well as its descriptor origin. An element family is eligible only
        // through an enclosing shared slice that the original contract already
        // authorizes as a source for this exact output. This is not an extra
        // lifetime relation between unrelated parameters.
        if let ir::CarrierChannel::ParamTail {
            parameter,
            prefix: 0,
        } = input.location.channel
            && input.location.path.0.last() == Some(&ir::CarrierStep::SliceElement)
            && matches!(destination.channel, ir::CarrierChannel::Fixed(_))
            && charged(
                &self.budget,
                ir::carrier_at(&signature.result, destination, &self.budget),
            )?
            .shared_slice_element()
                == Some(&Ty::Param(parameter))
        {
            let input_ty = signature.params.get(input.param).ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "slice family source parameter outside signature",
                )
            })?;
            for parent in
                charged(&self.budget, ir::carrier_locations(input_ty, &self.budget))?.iter()
            {
                let parent = ir::InputCarrier {
                    param: input.param,
                    location: parent.clone(),
                };
                if !self.retains_shared_slice_family(signature, &parent, input)? {
                    continue;
                }
                let eligible =
                    match self.source_carrier_eligible(source, signature, destination, &parent)? {
                        Some(eligible) => eligible,
                        None => self.carrier_source_eligible(
                            signature,
                            destination,
                            &signature.result,
                            &parent,
                            None,
                        )?,
                    };
                if eligible {
                    return Ok(Some(true));
                }
            }
        }
        let output = source
            .carriers
            .iter()
            .find(|row| row.parameter.is_none() && row.location == *destination);
        let Some(output) = output else {
            return Ok(None);
        };
        if let ir::CarrierRegionTerm::Family { parameter } = output.region {
            if !matches!(destination.channel,
                ir::CarrierChannel::ParamTail { parameter: actual, prefix: 0 }
                    if actual == parameter)
                || destination.path.0.last() != Some(&ir::CarrierStep::SliceElement)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "slice family output changed its exact source channel",
                ));
            }
            // A declared type-family result is not a named lifetime claim.
            // Use the original canonical family relation, including after
            // trait Self normalization exposes a slice parameter family.
            return self
                .carrier_source_eligible(signature, destination, &signature.result, input, None)
                .map(Some);
        }
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
        // Replacing a descriptor may retain its own stored element origins.
        // Exact source-parameter and carrier-path identity prevent introducing
        // an origin from another slice or an unrelated field.
        if self.retains_shared_slice_family(signature, destination, input)? {
            return Ok(Some(true));
        }
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
        // A substituted reference may only expand a family already retained
        // by the exact source signature. Missing source rows are not permission
        // to reconstruct an inner lifetime from the concrete type later.
        for (parameter, ty) in raw
            .params
            .iter()
            .enumerate()
            .map(|(i, ty)| (Some(i), ty))
            .chain(std::iter::once((None, &raw.result)))
        {
            for location in charged(&self.budget, ir::carrier_locations(ty, &self.budget))?.iter() {
                let ir::CarrierChannel::ParamTail {
                    parameter: element,
                    prefix: 0,
                } = location.channel
                else {
                    continue;
                };
                if location.path.0.last() != Some(&ir::CarrierStep::SliceElement) {
                    continue;
                }
                charged(&self.budget, self.budget.charge_work(source.carriers.len()))?;
                if !source.carriers.iter().any(|row| {
                    row.parameter == parameter
                        && row.location == *location
                        && row.region == (ir::CarrierRegionTerm::Family { parameter: element })
                }) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "missing declared slice element family region",
                    ));
                }
            }
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
                ir::CarrierRegionTerm::Static => {}
                ir::CarrierRegionTerm::Named(id)
                    if id.declaration == source.declaration && id.index < source.region_count => {}
                ir::CarrierRegionTerm::Fixed(id) if id.0 < signature.ref_contract.region_count => {}
                ir::CarrierRegionTerm::Family { parameter }
                    if location.path.0.last() == Some(&ir::CarrierStep::SliceElement)
                        && matches!(location.channel,
                            ir::CarrierChannel::ParamTail { parameter: actual, prefix: 0 }
                                if actual == parameter)
                        && charged(
                            &self.budget,
                            ir::carrier_at(original, &row.location, &self.budget),
                        )? == Ty::Param(parameter) => {}
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
        original: &Ty,
        actual: &Ty,
    ) -> Result<ir::CarrierLocation> {
        let mut original = original.clone();
        let mut actual = actual.clone();
        let mut path = Vec::new();
        for step in location.path.0.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            let mut rebased = step.clone();
            fn descend(ty: &Ty, step: ir::CarrierStep, budget: &PlanBudget) -> Result<Option<Ty>> {
                Ok(match (step, ty) {
                    (ir::CarrierStep::Referent, Ty::Ref { referent, .. }) => {
                        Some(referent.as_ref().clone())
                    }
                    (ir::CarrierStep::Field(index), _) => {
                        charged(budget, ty.field_ty(index, budget))?
                    }
                    (ir::CarrierStep::ConstantIndex(index), Ty::Array { element, len })
                        if index < *len =>
                    {
                        Some(element.as_ref().clone())
                    }
                    (ir::CarrierStep::SliceElement, Ty::Slice { element }) => {
                        Some(element.as_ref().clone())
                    }
                    (ir::CarrierStep::VariantField { variant, field }, Ty::Enum(value))
                        if variant.owner == value.def().id =>
                    {
                        match value.def().variants.get(variant.index) {
                            Some(row) => nominal_fields(&row.fields, budget)?
                                .get(field)
                                .map(|f| f.ty.clone()),
                            None => None,
                        }
                    }
                    _ => None,
                })
            }
            let Some(next_original) = descend(&original, step.clone(), &self.budget)? else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source carrier path has foreign nominal identity",
                ));
            };
            if let (ir::CarrierStep::VariantField { variant, field }, Ty::Enum(value)) =
                (step.clone(), &actual)
            {
                rebased = ir::CarrierStep::VariantField {
                    variant: ir::VariantId {
                        owner: value.def().id,
                        index: variant.index,
                    },
                    field,
                };
            }
            let Some(next_actual) = descend(&actual, rebased.clone(), &self.budget)? else {
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
                &actual,
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
pub(super) fn enclosing(parent: &ir::CarrierLocation, child: &ir::CarrierLocation) -> bool {
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
    fn static_result_carrier_never_borrows_a_nonstatic_input() {
        let budget = PlanBudget::default();
        let program = crate::compiler::frontend::parse(
            "enum Result<T, E> { Ok(T), Err(E) } fn value(s: &str) -> Result<i32, &'static str> { Result::Err(s) } fn main() {}",
        )
        .unwrap();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let signature = world
            .normalized_declaration_schema(declaration)
            .unwrap()
            .signature;
        let source = world
            .validate_source_regions(declaration, &signature)
            .unwrap();
        let output = source
            .carriers
            .iter()
            .find(|row| row.parameter.is_none())
            .unwrap();
        assert_eq!(output.region, ir::CarrierRegionTerm::Static);
        let input = ir::InputCarrier {
            param: 0,
            location: ir::CarrierLocation {
                path: ir::CarrierPath(Arc::from([])),
                channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
            },
        };
        assert_eq!(
            world
                .source_carrier_eligible(&source, &signature, &output.location, &input)
                .unwrap(),
            Some(false)
        );
        assert!(
            world
                .source_carrier_outlives(
                    &source,
                    &signature,
                    &ir::CarrierRegionTerm::Static,
                    &ir::CarrierRegionTerm::Fixed(ir::RegionParamId(0))
                )
                .unwrap()
        );
    }

    #[test]
    fn generic_slice_family_rows_reject_missing_foreign_and_offset_metadata() {
        let budget = PlanBudget::default();
        let program =
            crate::compiler::frontend::parse("fn read<T:Copy,U>(s:&[T],u:U)->T{s[0]}fn main(){}")
                .unwrap();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let source = plan.source_region_env(declaration).unwrap();
        let signature = world
            .schema
            .declaration_schema(declaration)
            .unwrap()
            .signature;
        world
            .checked_source_carriers(&source, &signature, &signature)
            .unwrap();
        let index = source
            .carriers
            .iter()
            .position(|row| matches!(row.region, ir::CarrierRegionTerm::Family { .. }))
            .unwrap();
        let ir::CarrierRegionTerm::Family { parameter } = source.carriers[index].region else {
            unreachable!()
        };
        let destination = ir::InputCarrier {
            param: 0,
            location: ir::CarrierLocation {
                path: ir::CarrierPath(Arc::from([])),
                channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
            },
        };
        let input = ir::InputCarrier {
            param: 0,
            location: source.carriers[index].location.clone(),
        };
        assert!(
            world
                .retains_shared_slice_family(&signature, &destination, &input)
                .unwrap()
        );
        let mut borrowed_slice = signature.clone();
        borrowed_slice.result = signature.params[0].clone();
        let mut outside = input.clone();
        outside.param = usize::MAX;
        assert!(
            world
                .source_carrier_eligible(&source, &borrowed_slice, &destination.location, &outside,)
                .is_err()
        );
        for mode in 0..4 {
            let mut forged = input.clone();
            match mode {
                0 => forged.param = 1,
                1 => forged.location.path = ir::CarrierPath(Arc::from([])),
                2 => {
                    forged.location.channel = ir::CarrierChannel::ParamTail {
                        parameter,
                        prefix: 1,
                    }
                }
                _ => {
                    forged.location.channel = ir::CarrierChannel::ParamTail {
                        parameter: ir::ParamId {
                            index: parameter.index + 1,
                            ..parameter
                        },
                        prefix: 0,
                    }
                }
            }
            assert!(
                !matches!(
                    world.retains_shared_slice_family(&signature, &destination, &forged),
                    Ok(true)
                ),
                "family offset mode {mode}"
            );
        }
        for mode in 0..6 {
            let mut forged = source.as_ref().clone();
            let mut rows = forged.carriers.to_vec();
            match mode {
                0 => {
                    rows.remove(index);
                }
                1 => {
                    rows[index].parameter = Some(1);
                }
                2 => {
                    rows[index].location.path = ir::CarrierPath(Arc::from([]));
                }
                3 => {
                    rows[index].location.channel = ir::CarrierChannel::ParamTail {
                        parameter,
                        prefix: 1,
                    };
                }
                4 => {
                    rows[index].region = ir::CarrierRegionTerm::Family {
                        parameter: ir::ParamId {
                            index: parameter.index + 1,
                            ..parameter
                        },
                    };
                }
                _ => {
                    rows.push(rows[index].clone());
                }
            }
            forged.carriers = rows.into();
            assert!(
                world
                    .checked_source_carriers(&forged, &signature, &signature)
                    .is_err(),
                "mode {mode}"
            );
        }
    }

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
