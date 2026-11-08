//! Body-only mutable reborrows with unchanged, source-owned shared carrier cells.
use super::*;

pub(super) fn fixed_shared_aggregate_location(location: &CarrierLocation) -> bool {
    location.channel == CarrierChannel::Fixed(ReferencePosition { depth: 0 })
        && matches!(location.path.0.first(), Some(CarrierStep::Referent))
        && !location.path.0[1..].is_empty()
        && location.path.0[1..]
            .iter()
            .all(|step| matches!(step, CarrierStep::Field(_) | CarrierStep::ConstantIndex(_)))
}

pub(super) fn computed_identity_writes(
    declared: &ir::CarrierContract,
    computed: &[ir::CarrierWriteEnvelope],
    must_overwrite: &[InputCarrier],
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    if !must_overwrite.is_empty()
        || !exact_identity_write_rows(declared, budget)?
        || computed.len() != declared.writes.len()
    {
        return Ok(false);
    }
    for (actual, expected) in computed.iter().zip(declared.writes.iter()) {
        budget.spend(add(actual.sources.len(), 1)?)?;
        if actual.destination != expected.destination
            || actual.sources.as_ref() != std::slice::from_ref(&expected.destination)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

impl CarrierBuilder<'_, '_> {
    /// Unlike the general call gate, this consumes the exact original body,
    /// its sealed source view and every reachable return's actual storage.
    pub(super) fn closed_shared_aggregate_reborrow_admitted(
        &mut self,
        body: &Arc<super::super::traits::CarrierContractView>,
        outgoing: &[Option<Point>],
        points: &[Option<Point>],
        offsets: &[usize],
        writes: &[ir::CarrierWriteEnvelope],
        must_overwrite: &[InputCarrier],
    ) -> Result<bool, Exhausted> {
        let (Some(facts), Some(function)) = (self.authority.clone(), self.function.clone()) else {
            return Ok(false);
        };
        if facts.phase() != super::super::traits::FactsPhase::Closed
            || self.physical.is_some()
            || self.physical_body.is_some()
            || !self.problems.is_empty()
        {
            return Ok(false);
        }
        if let Err(message) = self.ensure_provenance(&function, &facts) {
            let error = super::super::traits::FactsError::from_message(&self.ledger, &message);
            if error.is_resource() {
                return Err(Exhausted);
            }
            return Ok(false);
        }
        match body.check_body_replay_context(&facts) {
            Ok(()) => {}
            Err(error) if error.is_resource() => return Err(Exhausted),
            Err(_) => return Ok(false),
        }
        let Ty::Ref {
            kind: BorrowKind::Mut,
            referent,
        } = self.f.result.clone()
        else {
            return Ok(false);
        };
        if !matches!(
            referent.as_ref(),
            Ty::Struct(_) | Ty::Tuple(_) | Ty::Array { .. }
        ) {
            return Ok(false);
        }
        let result = self.f.result.clone();
        let Some(locations) = self.metadata_value(ir::carrier_locations(&result, &self.ledger))?
        else {
            return Ok(false);
        };
        if locations.len() < 2 || body.contract().results.len() != locations.len() {
            return Ok(false);
        }
        let signature = body_signature(self.f, self.budget)?;
        let mut validated = false;
        let mut roots = 0;
        for (location, row) in locations.iter().zip(body.contract().results.iter()) {
            self.budget.spend(add(location.path.0.len(), 1)?)?;
            if location != &row.location
                || result_flow_problem(
                    Some(body),
                    Some(&facts),
                    ir::SignatureSubject::Body,
                    &signature,
                    location,
                    &mut validated,
                )?
                .is_some()
            {
                return Ok(false);
            }
            if location.path.0.is_empty()
                && location.channel == CarrierChannel::Fixed(ReferencePosition { depth: 0 })
            {
                roots += 1;
                continue;
            }
            if !fixed_shared_aggregate_location(location) {
                return Ok(false);
            }
            let Some(ty) = self.metadata_value(ir::carrier_at(&result, location, &self.ledger))?
            else {
                return Ok(false);
            };
            if !matches!(
                ty,
                Ty::Ref {
                    kind: BorrowKind::Shared,
                    ..
                }
            ) {
                return Ok(false);
            }
        }
        if roots != 1
            || !exact_identity_write_rows(body.original_contract(), self.budget)?
            || !computed_identity_writes(body.contract(), writes, must_overwrite, self.budget)?
        {
            return Ok(false);
        }
        // Every authored write must transport once, and only once, to an
        // unchanged shared slot. Instantiation cannot invent an extra write.
        let mut destinations = Vec::new();
        for write in body.original_contract().writes.iter() {
            for (destination, _) in self.transport_effect_input(&write.destination, body, true)? {
                self.budget
                    .spend(add(destinations.len(), body.contract().writes.len())?)?;
                if destinations.contains(&destination)
                    || !body
                        .contract()
                        .writes
                        .iter()
                        .any(|row| row.destination == destination)
                {
                    return Ok(false);
                }
                let Some(local) = self.f.params.get(destination.param) else {
                    return Ok(false);
                };
                let parameter = self.f.locals[local.0].ty.clone();
                let Some(ty) = self.metadata_value(ir::carrier_at(
                    &parameter,
                    &destination.location,
                    &self.ledger,
                ))?
                else {
                    return Ok(false);
                };
                if !matches!(
                    ty,
                    Ty::Ref {
                        kind: BorrowKind::Shared,
                        ..
                    }
                ) {
                    return Ok(false);
                }
                self.retain(size_of::<InputCarrier>(), 1)?;
                checked_vec_growth(&mut destinations, self.budget)?;
                destinations.push(destination);
            }
        }
        if destinations.len() != body.contract().writes.len() {
            return Ok(false);
        }
        let root_key = CellKey {
            storage: CanonicalPlace {
                root: CanonicalRoot::Owned(self.f.return_local),
                projection: Vec::new(),
            },
            channel: CellChannel::ReferenceValue,
        };
        let Some(&root_cell) = self.cell_index.get(&root_key) else {
            return Ok(false);
        };
        let mut returned = false;
        for (block, point) in outgoing.iter().enumerate() {
            self.budget.spend(1)?;
            if !matches!(self.f.blocks[block].terminator.value, Terminator::Return) {
                continue;
            }
            let Some(point) = point else { continue };
            let Some(before_index) = offsets.get(block + 1).and_then(|end| end.checked_sub(1))
            else {
                return Ok(false);
            };
            let Some(before) = points.get(before_index).and_then(Option::as_ref) else {
                return Ok(false);
            };
            self.budget.spend(add(
                point.origins[root_cell].0.len(),
                point.families[root_cell].0.0.len(),
            )?)?;
            // `live` indexes cells before the terminator, not reference origins
            // after it. A return has no CFG successors and hence no live exits.
            if !before.live.has(root_cell)
                || before.origins[root_cell] != point.origins[root_cell]
                || before.families[root_cell] != point.families[root_cell]
            {
                return Ok(false);
            }
            returned = true;
            if point.origins[root_cell].count() == 0 || point.families[root_cell].0.count() != 0 {
                return Ok(false);
            }
            for origin in point.origins[root_cell].members() {
                self.budget.spend(1)?;
                if self.kinds[origin] != BorrowKind::Mut || self.targets[origin].is_empty() {
                    return Ok(false);
                }
                for target_index in 0..self.targets[origin].len() {
                    let original = &self.targets[origin][target_index];
                    let bytes = add(
                        size_of::<Target>(),
                        add(
                            mul(original.place.projection.len(), size_of::<Projection>())?,
                            mul(original.authority.len(), size_of::<usize>())?,
                        )?,
                    )?;
                    self.retain(bytes, 1)?;
                    let target = self.targets[origin][target_index].clone();
                    let CanonicalRoot::ParameterReferent(local, input_location) =
                        &target.place.root
                    else {
                        return Ok(false);
                    };
                    if target.precision != TargetPrecision::Exact
                        || !input_location.path.0.is_empty()
                        || input_location.channel
                            != CarrierChannel::Fixed(ReferencePosition { depth: 0 })
                        || !target.place.projection.iter().all(|p| {
                            matches!(p, Projection::Field(_) | Projection::ConstantIndex(_))
                        })
                    {
                        return Ok(false);
                    }
                    let Some(param) = self.f.params.iter().position(|p| p == local) else {
                        return Ok(false);
                    };
                    let Ty::Ref {
                        kind: BorrowKind::Mut,
                        referent: input,
                    } = &self.f.locals[local.0].ty
                    else {
                        return Ok(false);
                    };
                    let Some(target_ty) = self.metadata_value(ir::projected_ty_optional(
                        input,
                        &target.place.projection,
                        Some(&self.ledger),
                    ))?
                    else {
                        return Ok(false);
                    };
                    match facts.world().types_equal(
                        std::slice::from_ref(&target_ty),
                        std::slice::from_ref(referent.as_ref()),
                    ) {
                        Ok(true) => {}
                        Err(error) if error.is_resource() => return Err(Exhausted),
                        _ => return Ok(false),
                    }
                    for location in locations
                        .iter()
                        .filter(|location| !location.path.0.is_empty())
                    {
                        let count = add(
                            add(1, target.place.projection.len())?,
                            location.path.0.len() - 1,
                        )?;
                        self.retain(mul(count, size_of::<CarrierStep>())?, 1)?;
                        let mut path = Vec::new();
                        path.try_reserve_exact(count).map_err(|_| Exhausted)?;
                        path.push(CarrierStep::Referent);
                        for step in &target.place.projection {
                            path.push(match step {
                                Projection::Field(index) => CarrierStep::Field(*index),
                                Projection::ConstantIndex(index) => {
                                    CarrierStep::ConstantIndex(*index)
                                }
                                _ => return Ok(false),
                            });
                        }
                        path.extend_from_slice(&location.path.0[1..]);
                        checked_arc_transfer::<CarrierStep>(path.len(), self.budget)?;
                        let expected = InputCarrier {
                            param,
                            location: CarrierLocation {
                                path: ir::CarrierPath(path.into()),
                                channel: location.channel.clone(),
                            },
                        };
                        let mut transported = 0;
                        for source in body.original_contract().inputs.iter() {
                            for (actual, _) in self.transport_effect_input(source, body, false)? {
                                transported += usize::from(actual == expected);
                            }
                        }
                        if transported != 1 || !destinations.contains(&expected) {
                            return Ok(false);
                        }
                        let Some(row) = body
                            .contract()
                            .results
                            .iter()
                            .find(|row| row.location == *location)
                        else {
                            return Ok(false);
                        };
                        if !row.sources.contains(&expected) {
                            return Ok(false);
                        }
                        let projection_len =
                            add(target.place.projection.len(), location.path.0.len() - 1)?;
                        self.retain(mul(projection_len, size_of::<Projection>())?, 1)?;
                        let mut projection = Vec::new();
                        projection
                            .try_reserve_exact(projection_len)
                            .map_err(|_| Exhausted)?;
                        projection.extend_from_slice(&target.place.projection);
                        for step in &location.path.0[1..] {
                            projection.push(match step {
                                CarrierStep::Field(index) => Projection::Field(*index),
                                CarrierStep::ConstantIndex(index) => {
                                    Projection::ConstantIndex(*index)
                                }
                                _ => return Ok(false),
                            });
                        }
                        let key = CellKey {
                            storage: CanonicalPlace {
                                root: target.place.root.clone(),
                                projection,
                            },
                            channel: CellChannel::ReferenceValue,
                        };
                        let Some(&cell) = self.cell_index.get(&key) else {
                            return Ok(false);
                        };
                        let Some((_, initial)) = self
                            .reference_inputs
                            .iter()
                            .find(|(input, _)| *input == expected)
                        else {
                            return Ok(false);
                        };
                        if point.families[cell].0.count() != 0
                            || point.origins[cell].count() != 1
                            || !point.origins[cell].has(initial.0)
                        {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        Ok(returned)
    }
}
