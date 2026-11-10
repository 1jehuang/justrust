//! Exact local expansion of graph-authenticated opaque carrier families.
//! A map is complete for one public/physical signature pair. It grants no new
//! origin edges: consumers expand only edges retained by the original view.
use super::*;

struct Row {
    parameter: Option<usize>,
    original: ir::CarrierLocation,
    physical: Arc<[ir::CarrierLocation]>,
}

pub(super) struct Mapping {
    rows: Vec<Row>,
}

fn buffer<T>(count: usize, budget: &ir::PlanBudget) -> Result<Vec<T>, String> {
    if count > ir::MAX_LOAN_SITES {
        return Err("physical carrier mapping row limit exceeded".into());
    }
    budget.charge_work(count.saturating_add(1))?;
    let bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .and_then(|bytes| bytes.checked_mul(2))
        .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
        .ok_or("physical carrier mapping allocation overflow")?;
    budget.retain_bytes(bytes)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| "physical carrier mapping allocation failed")?;
    Ok(output)
}

/// Preserve the complete original path and ordered reference prefix. Only a
/// graph-read opaque family is expanded, never a projection or display name.
pub(super) fn expand(
    location: &ir::CarrierLocation,
    representations: &opaque_instances::Representations,
    budget: &ir::PlanBudget,
) -> Result<Arc<[ir::CarrierLocation]>, String> {
    let ir::CarrierChannel::OpaqueTail { opaque, prefix } = &location.channel else {
        let mut result = buffer(1, budget)?;
        result.push(location.clone());
        return Ok(result.into());
    };
    let hidden = representations
        .normalize(&ir::Ty::Opaque(opaque.clone()), budget)
        .map_err(|error| error.to_string())?;
    let relative = ir::carrier_locations(&hidden, budget)?;
    let mut result = buffer(relative.len(), budget)?;
    for carrier in relative.iter() {
        let count = location
            .path
            .0
            .len()
            .checked_add(usize::from(*prefix))
            .and_then(|count| count.checked_add(carrier.path.0.len()))
            .ok_or("physical carrier path expansion overflow")?;
        budget.check_depth(count)?;
        let mut path = buffer(count, budget)?;
        path.extend_from_slice(&location.path.0);
        if matches!(
            carrier.channel,
            ir::CarrierChannel::ProjectionTail { .. } | ir::CarrierChannel::OpaqueTail { .. }
        ) {
            return Err(
                "unsupported: physical opaque representation retains an unresolved family".into(),
            );
        }
        let channel = if carrier.path.0.is_empty() {
            let depth = prefix
                .checked_add(carrier.channel.prefix())
                .ok_or("physical carrier reference depth overflow")?;
            match &carrier.channel {
                ir::CarrierChannel::Fixed(_) => {
                    ir::CarrierChannel::Fixed(ir::ReferencePosition { depth })
                }
                ir::CarrierChannel::ParamTail { parameter, .. } => ir::CarrierChannel::ParamTail {
                    parameter: *parameter,
                    prefix: depth,
                },
                ir::CarrierChannel::ProjectionTail { .. }
                | ir::CarrierChannel::OpaqueTail { .. } => {
                    return Err(
                        "unsupported: physical opaque representation retains an unresolved family"
                            .into(),
                    );
                }
            }
        } else {
            path.extend((0..*prefix).map(|_| ir::CarrierStep::Referent));
            path.extend_from_slice(&carrier.path.0);
            carrier.channel.clone()
        };
        result.push(ir::CarrierLocation {
            path: ir::CarrierPath(path.into()),
            channel,
        });
    }
    Ok(result.into())
}

pub(in crate::compiler) fn require_type(
    public: &ir::Ty,
    physical: &ir::Ty,
    representations: &opaque_instances::Representations,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let actual = ir::carrier_locations(physical, budget)?;
    let mut next = 0usize;
    for original in ir::carrier_locations(public, budget)?.iter() {
        for location in expand(original, representations, budget)?.iter() {
            budget.charge_work(location.path.0.len().saturating_add(1))?;
            if actual.get(next) != Some(location) {
                return Err(
                    "physical receiver changed its complete canonical carrier topology".into(),
                );
            }
            next += 1;
        }
    }
    if next != actual.len() {
        return Err("physical receiver omitted a graph-authenticated carrier".into());
    }
    Ok(())
}

impl Mapping {
    pub(super) fn new(
        public: &ir::Signature,
        physical: &ir::Signature,
        representations: &opaque_instances::Representations,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        if public.params.len() != physical.params.len() {
            return Err("physical carrier mapping changed the parameter count".into());
        }
        let mut rows = Vec::new();
        for (parameter, before, after) in public
            .params
            .iter()
            .zip(&physical.params)
            .enumerate()
            .map(|(index, (before, after))| (Some(index), before, after))
            .chain(std::iter::once((None, &public.result, &physical.result)))
        {
            let actual = ir::carrier_locations(after, budget)?;
            let mut next = 0usize;
            for original in ir::carrier_locations(before, budget)?.iter() {
                let physical = expand(original, representations, budget)?;
                for mapped in physical.iter() {
                    budget.charge_work(mapped.path.0.len().saturating_add(1))?;
                    if actual.get(next) != Some(mapped) {
                        return Err(
                            "physical opaque carrier map changed canonical path, depth or channel"
                                .into(),
                        );
                    }
                    next += 1;
                }
                if rows.len() >= ir::MAX_LOAN_SITES {
                    return Err("physical carrier mapping row limit exceeded".into());
                }
                budget.retain_bytes(std::mem::size_of::<Row>())?;
                rows.try_reserve_exact(1)
                    .map_err(|_| "physical carrier row allocation failed")?;
                rows.push(Row {
                    parameter,
                    original: original.clone(),
                    physical,
                });
            }
            if next != actual.len() {
                return Err("physical opaque carrier mapping omitted a concrete carrier".into());
            }
        }
        Ok(Self { rows })
    }

    fn locations(
        &self,
        parameter: Option<usize>,
        location: &ir::CarrierLocation,
        budget: &ir::PlanBudget,
    ) -> Result<&[ir::CarrierLocation], String> {
        for row in &self.rows {
            budget.charge_work(
                row.original
                    .path
                    .0
                    .len()
                    .saturating_add(location.path.0.len())
                    .saturating_add(1),
            )?;
            if row.parameter == parameter && row.original == *location {
                return Ok(&row.physical);
            }
        }
        Err("physical carrier edge is outside the complete original signature".into())
    }

    fn inputs(
        &self,
        original: &[ir::InputCarrier],
        budget: &ir::PlanBudget,
    ) -> Result<Arc<[ir::InputCarrier]>, String> {
        let mut count = 0usize;
        for input in original {
            count = count
                .checked_add(
                    self.locations(Some(input.param), &input.location, budget)?
                        .len(),
                )
                .ok_or("physical input edge expansion overflow")?;
        }
        let mut output = buffer(count, budget)?;
        for input in original {
            for location in self.locations(Some(input.param), &input.location, budget)? {
                output.push(ir::InputCarrier {
                    param: input.param,
                    location: location.clone(),
                });
            }
        }
        Ok(output.into())
    }

    pub(super) fn contract(
        &self,
        original: &ir::CarrierContract,
        budget: &ir::PlanBudget,
    ) -> Result<ir::CarrierContract, String> {
        let inputs = self.inputs(&original.inputs, budget)?;
        let mut result_count = 0usize;
        for row in original.results.iter() {
            result_count = result_count
                .checked_add(self.locations(None, &row.location, budget)?.len())
                .ok_or("physical result expansion overflow")?;
        }
        let mut results = buffer(result_count, budget)?;
        for row in original.results.iter() {
            let sources = self.inputs(&row.sources, budget)?;
            for location in self.locations(None, &row.location, budget)? {
                results.push(ir::CarrierReturn {
                    location: location.clone(),
                    sources: sources.clone(),
                });
            }
        }
        let mut write_count = 0usize;
        for row in original.writes.iter() {
            write_count = write_count
                .checked_add(
                    self.locations(
                        Some(row.destination.param),
                        &row.destination.location,
                        budget,
                    )?
                    .len(),
                )
                .ok_or("physical write expansion overflow")?;
        }
        let mut writes = buffer(write_count, budget)?;
        for row in original.writes.iter() {
            let sources = self.inputs(&row.sources, budget)?;
            for location in self.locations(
                Some(row.destination.param),
                &row.destination.location,
                budget,
            )? {
                writes.push(ir::CarrierWriteEnvelope {
                    destination: ir::InputCarrier {
                        param: row.destination.param,
                        location: location.clone(),
                    },
                    sources: sources.clone(),
                });
            }
        }
        Ok(ir::CarrierContract {
            inputs,
            results: results.into(),
            writes: writes.into(),
        })
    }

    pub(super) fn components(
        &self,
        original: &[ir::CarrierComponentMapping],
        representations: &opaque_instances::Representations,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<[ir::CarrierComponentMapping]>, String> {
        let mut count = 0usize;
        for row in original {
            count = count
                .checked_add(expand(&row.substituted, representations, budget)?.len())
                .ok_or("physical component mapping expansion overflow")?;
        }
        let mut output = buffer(count, budget)?;
        for row in original {
            for substituted in expand(&row.substituted, representations, budget)?.iter() {
                let original_location = if substituted == &row.substituted {
                    row.original.clone()
                } else {
                    let prefix = substituted.channel.prefix();
                    let channel = match &row.original_family {
                        ir::CarrierFamily::Parameter(parameter) => ir::CarrierChannel::ParamTail {
                            parameter: *parameter,
                            prefix,
                        },
                        ir::CarrierFamily::Opaque(opaque) => ir::CarrierChannel::OpaqueTail {
                            opaque: opaque.clone(),
                            prefix,
                        },
                        ir::CarrierFamily::Projection(projection) => {
                            ir::CarrierChannel::ProjectionTail {
                                projection: projection.clone(),
                                prefix,
                            }
                        }
                    };
                    ir::CarrierLocation {
                        path: substituted.path.clone(),
                        channel,
                    }
                };
                output.push(ir::CarrierComponentMapping {
                    original_family: row.original_family.clone(),
                    original: original_location,
                    substituted: substituted.clone(),
                });
            }
        }
        Ok(output.into())
    }

    /// Instantiate declaration-owned region coordinates using the same exact
    /// component and nominal identities as the ordinary carrier view. No source
    /// row is discarded just because its symbolic coordinate is absent from the
    /// instantiated signature.
    fn source_locations(
        &self,
        row: &ir::SourceCarrierRegion,
        view: &traits::CarrierContractView,
        facts: &traits::ValidatedFacts,
        representations: &opaque_instances::Representations,
        budget: &ir::PlanBudget,
    ) -> Result<Vec<ir::CarrierLocation>, String> {
        if let Some(family) = row.location.channel.family() {
            let mut output = buffer(0, budget)?;
            let mut found = false;
            for component in view.component_mappings() {
                budget.charge_work(1)?;
                if component.original_family != family {
                    continue;
                }
                found = true;
                let location = compose(
                    &row.location,
                    &component.substituted,
                    view.nominal_mappings(),
                    budget,
                )?;
                for mapped in self.locations(row.parameter, &location, budget)? {
                    if output.len() >= ir::MAX_LOAN_SITES {
                        return Err("physical source region row limit exceeded".into());
                    }
                    budget.retain_bytes(std::mem::size_of::<ir::CarrierLocation>())?;
                    output
                        .try_reserve_exact(1)
                        .map_err(|_| "physical source region allocation failed")?;
                    output.push(mapped.clone());
                }
            }
            if !found {
                // A missing component list is not evidence of emptiness. Ask
                // the validated original view for this exact family actual.
                let actual = view
                    .family_actual(&family, facts)
                    .map_err(|error| error.into_legacy_diagnostic(budget))?
                    .ok_or("physical source region has no authenticated family actual")?;
                let actual = representations
                    .normalize(&actual, budget)
                    .map_err(|error| error.to_string())?;
                if !ir::carrier_locations(&actual, budget)?.is_empty() {
                    return Err("physical source region omitted a nonempty family component".into());
                }
            }
            return Ok(output);
        }
        let mut location = row.location.clone();
        remap_nominals(&mut location, view.nominal_mappings(), budget)?;
        let locations = self.locations(row.parameter, &location, budget)?;
        let mut output = buffer(locations.len(), budget)?;
        output.extend_from_slice(locations);
        Ok(output)
    }

    pub(super) fn source_regions(
        &self,
        view: &traits::CarrierContractView,
        facts: &traits::ValidatedFacts,
        representations: &opaque_instances::Representations,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<ir::SourceRegionEnv>, String> {
        let original = view.source_region_env();
        let mut carriers = buffer(0, budget)?;
        for row in original.carriers.iter() {
            for location in self.source_locations(row, view, facts, representations, budget)? {
                if carriers.len() >= ir::MAX_LOAN_SITES {
                    return Err("physical source region row limit exceeded".into());
                }
                budget.retain_bytes(std::mem::size_of::<ir::SourceCarrierRegion>())?;
                carriers
                    .try_reserve_exact(1)
                    .map_err(|_| "physical source region allocation failed")?;
                carriers.push(ir::SourceCarrierRegion {
                    parameter: row.parameter,
                    location,
                    region: row.region.clone(),
                });
            }
        }
        budget.retain_bytes(std::mem::size_of::<ir::SourceRegionEnv>())?;
        Ok(Arc::new(ir::SourceRegionEnv {
            carriers: carriers.into(),
            ..original.as_ref().clone()
        }))
    }
}

fn remap_nominals(
    location: &mut ir::CarrierLocation,
    nominals: &[ir::NominalCarrierMapping],
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let mut path = buffer(location.path.0.len(), budget)?;
    for step in location.path.0.iter() {
        let mut step = step.clone();
        if let ir::CarrierStep::VariantField { variant, .. } = &mut step {
            for mapping in nominals {
                budget.charge_work(1)?;
                if variant.owner == mapping.original {
                    variant.owner = mapping.instantiated;
                    break;
                }
            }
        }
        path.push(step);
    }
    location.path = ir::CarrierPath(path.into());
    Ok(())
}

fn compose(
    original: &ir::CarrierLocation,
    relative: &ir::CarrierLocation,
    nominals: &[ir::NominalCarrierMapping],
    budget: &ir::PlanBudget,
) -> Result<ir::CarrierLocation, String> {
    let prefix = original.channel.prefix();
    let count = original
        .path
        .0
        .len()
        .checked_add(usize::from(prefix))
        .and_then(|count| count.checked_add(relative.path.0.len()))
        .ok_or("physical source region path overflow")?;
    budget.check_depth(count)?;
    let mut result = original.clone();
    remap_nominals(&mut result, nominals, budget)?;
    let mut path = buffer(count, budget)?;
    path.extend_from_slice(&result.path.0);
    result.channel = if relative.path.0.is_empty() {
        let depth = prefix
            .checked_add(relative.channel.prefix())
            .ok_or("physical source region depth overflow")?;
        match &relative.channel {
            ir::CarrierChannel::Fixed(_) => {
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth })
            }
            ir::CarrierChannel::ParamTail { parameter, .. } => ir::CarrierChannel::ParamTail {
                parameter: *parameter,
                prefix: depth,
            },
            ir::CarrierChannel::ProjectionTail { projection, .. } => {
                ir::CarrierChannel::ProjectionTail {
                    projection: projection.clone(),
                    prefix: depth,
                }
            }
            ir::CarrierChannel::OpaqueTail { opaque, .. } => ir::CarrierChannel::OpaqueTail {
                opaque: opaque.clone(),
                prefix: depth,
            },
        }
    } else {
        path.extend((0..prefix).map(|_| ir::CarrierStep::Referent));
        path.extend_from_slice(&relative.path.0);
        relative.channel.clone()
    };
    result.path = ir::CarrierPath(path.into());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(field: Option<usize>, depth: u8) -> ir::CarrierLocation {
        ir::CarrierLocation {
            path: ir::CarrierPath(field.into_iter().map(ir::CarrierStep::Field).collect()),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth }),
        }
    }

    #[test]
    fn complete_expansion_preserves_every_input_result_and_write_edge() {
        let budget = ir::PlanBudget::default();
        let original = location(None, 0);
        let physical: Arc<[ir::CarrierLocation]> =
            Arc::from([location(Some(0), 0), location(Some(1), 1)]);
        let mapping = Mapping {
            rows: [Some(0), None]
                .into_iter()
                .map(|parameter| Row {
                    parameter,
                    original: original.clone(),
                    physical: physical.clone(),
                })
                .collect(),
        };
        let input = ir::InputCarrier {
            param: 0,
            location: original.clone(),
        };
        let contract = ir::CarrierContract {
            inputs: Arc::from([input.clone()]),
            results: Arc::from([ir::CarrierReturn {
                location: original,
                sources: Arc::from([input.clone()]),
            }]),
            writes: Arc::from([ir::CarrierWriteEnvelope {
                destination: input.clone(),
                sources: Arc::from([input]),
            }]),
        };
        let actual = mapping.contract(&contract, &budget).unwrap();
        assert_eq!(actual.inputs.len(), 2);
        assert_eq!(actual.results.len(), 2);
        assert_eq!(actual.writes.len(), 2);
        for (index, expected) in physical.iter().enumerate() {
            assert_eq!(&actual.inputs[index].location, expected);
            assert_eq!(&actual.results[index].location, expected);
            assert_eq!(actual.results[index].sources, actual.inputs);
            assert_eq!(&actual.writes[index].destination.location, expected);
            assert_eq!(actual.writes[index].sources, actual.inputs);
        }
        for change in 0..3 {
            let mut forged = contract.clone();
            match change {
                0 => {
                    let mut rows = forged.results.to_vec();
                    rows[0].location = location(Some(99), 0);
                    forged.results = rows.into();
                }
                1 => {
                    let mut rows = forged.writes.to_vec();
                    rows[0].destination.param = 99;
                    forged.writes = rows.into();
                }
                _ => {
                    let mut rows = forged.results.to_vec();
                    rows[0].sources = Arc::from([ir::InputCarrier {
                        param: 0,
                        location: location(None, 1),
                    }]);
                    forged.results = rows.into();
                }
            }
            assert!(mapping.contract(&forged, &budget).is_err());
        }
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(mapping.contract(&contract, &denied).is_err());
    }
}
