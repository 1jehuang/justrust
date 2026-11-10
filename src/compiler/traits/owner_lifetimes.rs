//! Transport lifetime authority from exact trait and impl owners, not methods.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn validate_owner_region_application(&self, h: &ImplHeader) -> Result<()> {
        let original = self
            .schema
            .owner_region_application(h.declaration, &h.trait_ref.self_ty)?;
        match (&h.region_application, original) {
            (None, None) if self.trait_header(h.trait_ref.declaration)?.region_count == 0 => Ok(()),
            (Some(actual), Some(original)) => {
                let count = actual.self_regions.region_count;
                charged(
                    &self.budget,
                    self.budget.charge_work(
                        actual.actuals.len()
                            + actual.self_regions.carriers.len()
                            + actual.self_regions.outlives.len()
                            + usize::from(count)
                            + 1,
                    ),
                )?;
                if actual.implementation != h.declaration
                    || actual.trait_decl != h.trait_ref.declaration
                    || actual.actuals.len()
                        != usize::from(self.trait_header(actual.trait_decl)?.region_count)
                    || actual.self_regions.declaration != h.declaration
                    || actual.binder_count > count
                    || usize::from(count) > ir::MAX_REGION_PARAMS
                    || actual.self_regions.normalized.len() != usize::from(count)
                    || actual.self_signature.params.len() != 1
                    || !self.eq_ty(&actual.self_signature.params[0], &h.trait_ref.self_ty, 0)?
                    || actual.self_signature.result != Ty::Unit
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "owner lifetime application changed canonical owner or arity",
                    ));
                }
                for term in actual.actuals.iter() {
                    match term {
                        ir::CarrierRegionTerm::Static => {}
                        ir::CarrierRegionTerm::Named(id)
                            if id.declaration == h.declaration
                                && id.index < actual.binder_count => {}
                        _ => {
                            return Err(FactsError::invalid(
                                &self.budget,
                                "owner lifetime actual escapes original binder",
                            ));
                        }
                    }
                }
                // The independent source replay seals order as well as shape.
                // Two well-formed actual lists are not interchangeable.
                if actual.as_ref() != original.as_ref() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "owner lifetime application changed original ordered source",
                    ));
                }
                self.checked_source_carriers(
                    &actual.self_regions,
                    &actual.self_signature,
                    &actual.self_signature,
                )?;
                Ok(())
            }
            _ => Err(FactsError::invalid(
                &self.budget,
                "missing original owner lifetime application authority",
            )),
        }
    }

    pub(super) fn owner_applied_source_regions(
        &self,
        item: &TraitItemHeader,
        implementation: &ImplHeader,
        signature: &Signature,
    ) -> Result<Arc<ir::SourceRegionEnv>> {
        self.validate_owner_region_application(implementation)?;
        let source = self.validate_source_regions(item.declaration, signature)?;
        let Some(application) = &implementation.region_application else {
            if self.trait_header(item.trait_decl)?.region_count != 0 {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unrepresented trait owner lifetime application",
                ));
            }
            return Ok(source);
        };
        if item.trait_decl != implementation.trait_ref.declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "required lifetime application changed trait owner",
            ));
        }
        let owner = self.trait_header(item.trait_decl)?;
        let owner_count = owner.region_count;
        if owner_count > source.region_count
            || application.actuals.len() != usize::from(owner_count)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "required method lost its owner lifetime prefix",
            ));
        }
        let offset = source.region_count;
        let count = offset
            .checked_add(application.self_regions.region_count)
            .filter(|count| usize::from(*count) <= ir::MAX_REGION_PARAMS)
            .ok_or_else(|| {
                FactsError::resource(&self.budget, "applied lifetime binder limit exceeded")
            })?;
        let impl_term = |term: &ir::CarrierRegionTerm| -> Result<ir::CarrierRegionTerm> {
            match term {
                ir::CarrierRegionTerm::Named(id)
                    if id.declaration == implementation.declaration
                        && id.index < application.self_regions.region_count =>
                {
                    Ok(ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                        declaration: item.declaration,
                        index: offset + id.index,
                    }))
                }
                ir::CarrierRegionTerm::Static | ir::CarrierRegionTerm::Family { .. } => {
                    Ok(term.clone())
                }
                _ => Err(FactsError::invalid(
                    &self.budget,
                    "applied self carrier lifetime has foreign owner",
                )),
            }
        };
        let required_term = |term: &ir::CarrierRegionTerm| -> Result<ir::CarrierRegionTerm> {
            match term {
                ir::CarrierRegionTerm::Named(id)
                    if id.declaration == item.declaration && id.index < owner_count =>
                {
                    impl_term(&application.actuals[usize::from(id.index)])
                }
                _ => Ok(term.clone()),
            }
        };
        let mut normalized = Vec::new();
        retain::<Option<ir::RegionParamId>>(&self.budget, usize::from(count))?;
        normalized.extend_from_slice(&source.normalized);
        normalized.resize(usize::from(count), None);
        for i in 0..owner_count {
            normalized[usize::from(i)] = None;
            if let ir::CarrierRegionTerm::Named(mapped) =
                required_term(&ir::CarrierRegionTerm::Named(ir::SourceRegionId {
                    declaration: item.declaration,
                    index: i,
                }))?
            {
                normalized[usize::from(mapped.index)] = source.normalized[usize::from(i)];
            }
        }
        let mut edges = Vec::new();
        for &(a, b) in source.outlives.iter() {
            let a = required_term(&ir::CarrierRegionTerm::Named(a))?;
            let b = required_term(&ir::CarrierRegionTerm::Named(b))?;
            match (a, b) {
                (ir::CarrierRegionTerm::Named(a), ir::CarrierRegionTerm::Named(b)) => {
                    push(&self.budget, &mut edges, (a, b))?
                }
                (ir::CarrierRegionTerm::Static, _) => {}
                _ => {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "owner lifetime application requires a static outlives constraint",
                    ));
                }
            }
        }
        for &(a, b) in application.self_regions.outlives.iter() {
            let (ir::CarrierRegionTerm::Named(a), ir::CarrierRegionTerm::Named(b)) = (
                impl_term(&ir::CarrierRegionTerm::Named(a))?,
                impl_term(&ir::CarrierRegionTerm::Named(b))?,
            ) else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "impl lifetime edge changed source owner",
                ));
            };
            push(&self.budget, &mut edges, (a, b))?;
        }
        let mut rows = Vec::new();
        for row in source.carriers.iter() {
            push(
                &self.budget,
                &mut rows,
                ir::SourceCarrierRegion {
                    region: required_term(&row.region)?,
                    ..row.clone()
                },
            )?;
        }
        // Expand only exact raw Self-family occurrences. Canonical equal types
        // elsewhere in the signature are not evidence of an owner substitution.
        for (parameter, raw) in item
            .schema
            .signature
            .params
            .iter()
            .enumerate()
            .map(|(i, ty)| (Some(i), ty))
            .chain(std::iter::once((None, &item.schema.signature.result)))
        {
            for location in charged(&self.budget, ir::carrier_locations(raw, &self.budget))?.iter()
            {
                let ir::CarrierChannel::ParamTail {
                    parameter: generic,
                    prefix,
                } = location.channel
                else {
                    continue;
                };
                if generic != owner.self_param {
                    continue;
                }
                for self_row in application.self_regions.carriers.iter() {
                    if self_row.parameter != Some(0) {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "impl self carrier row has foreign parameter",
                        ));
                    }
                    let embedded =
                        self.embed_owner_location(location, prefix, &self_row.location)?;
                    let actual = parameter
                        .and_then(|i| signature.params.get(i))
                        .unwrap_or(&signature.result);
                    charged(
                        &self.budget,
                        ir::carrier_at(actual, &embedded, &self.budget),
                    )?;
                    let region = match &self_row.region {
                        ir::CarrierRegionTerm::Fixed(_) => {
                            let Some(i) = parameter else {
                                return Err(FactsError::unsupported(
                                    &self.budget,
                                    "unrepresented elided impl Self output lifetime",
                                ));
                            };
                            let ir::CarrierChannel::Fixed(position) = embedded.channel else {
                                return Err(FactsError::invalid(
                                    &self.budget,
                                    "impl fixed lifetime changed carrier channel",
                                ));
                            };
                            if !embedded.path.0.is_empty() {
                                return Err(FactsError::unsupported(
                                    &self.budget,
                                    "unrepresented nested elided impl Self lifetime",
                                ));
                            }
                            ir::CarrierRegionTerm::Fixed(
                                signature
                                    .ref_contract
                                    .input_regions
                                    .iter()
                                    .find(|row| {
                                        row.input.param == i && row.input.position == position
                                    })
                                    .ok_or_else(|| {
                                        FactsError::invalid(
                                            &self.budget,
                                            "impl Self lifetime has no canonical input region",
                                        )
                                    })?
                                    .region,
                            )
                        }
                        term => impl_term(term)?,
                    };
                    if let (
                        Some(i),
                        ir::CarrierChannel::Fixed(position),
                        ir::CarrierRegionTerm::Named(id),
                    ) = (parameter, &embedded.channel, &region)
                        && embedded.path.0.is_empty()
                        && let Some(fixed) = signature
                            .ref_contract
                            .input_regions
                            .iter()
                            .find(|row| row.input.param == i && row.input.position == *position)
                    {
                        let slot = &mut normalized[usize::from(id.index)];
                        if slot.is_some_and(|old| old != fixed.region) {
                            return Err(FactsError::invalid(
                                &self.budget,
                                "impl owner lifetime maps to conflicting canonical regions",
                            ));
                        }
                        *slot = Some(fixed.region);
                    }
                    if rows
                        .iter()
                        .any(|row| row.parameter == parameter && row.location == embedded)
                    {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "duplicate applied owner carrier",
                        ));
                    }
                    push(
                        &self.budget,
                        &mut rows,
                        ir::SourceCarrierRegion {
                            parameter,
                            location: embedded,
                            region,
                        },
                    )?;
                }
            }
        }
        retain::<ir::SourceRegionEnv>(&self.budget, 1)?;
        Ok(Arc::new(ir::SourceRegionEnv {
            declaration: item.declaration,
            region_count: count,
            normalized: normalized.into(),
            outlives: edges.into(),
            carriers: rows.into(),
        }))
    }

    fn embed_owner_location(
        &self,
        outer: &ir::CarrierLocation,
        prefix: u8,
        inner: &ir::CarrierLocation,
    ) -> Result<ir::CarrierLocation> {
        let mut path = Vec::new();
        charged(
            &self.budget,
            self.budget
                .charge_work(outer.path.0.len() + inner.path.0.len() + usize::from(prefix) + 1),
        )?;
        retain::<ir::CarrierStep>(
            &self.budget,
            outer.path.0.len() + inner.path.0.len() + usize::from(prefix),
        )?;
        path.extend_from_slice(&outer.path.0);
        let channel = if inner.path.0.is_empty() {
            let depth = prefix.checked_add(inner.channel.prefix()).ok_or_else(|| {
                FactsError::resource(&self.budget, "owner carrier reference depth overflow")
            })?;
            match &inner.channel {
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
                ir::CarrierChannel::ArrayTail { array, .. } => ir::CarrierChannel::ArrayTail {
                    array: array.clone(),
                    prefix: depth,
                },
                ir::CarrierChannel::OpaqueTail { opaque, .. } => ir::CarrierChannel::OpaqueTail {
                    opaque: opaque.clone(),
                    prefix: depth,
                },
            }
        } else {
            path.extend(std::iter::repeat_n(
                ir::CarrierStep::Referent,
                usize::from(prefix),
            ));
            path.extend_from_slice(&inner.path.0);
            inner.channel.clone()
        };
        Ok(ir::CarrierLocation {
            path: ir::CarrierPath(path.into()),
            channel,
        })
    }
}
