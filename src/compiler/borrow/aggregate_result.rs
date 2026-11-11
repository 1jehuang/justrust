//! Exact closed-body return certification for finite owned reference aggregates.
use super::*;

// Only the first differing path step can establish disjoint enum alternatives.
// Equal variant display names or equal later nested enum owners are insufficient.
fn exclusive_variants(left: &CarrierLocation, right: &CarrierLocation) -> bool {
    for (left, right) in left.path.0.iter().zip(right.path.0.iter()) {
        if left == right {
            continue;
        }
        return matches!(
            (left, right),
            (CarrierStep::VariantField { variant: a, .. },
             CarrierStep::VariantField { variant: b, .. })
                if a.owner == b.owner && a.index != b.index
        );
    }
    false
}

impl CarrierBuilder<'_, '_> {
    pub(super) fn closed_aggregate_result_admitted(
        &mut self,
        body: &Arc<super::super::traits::CarrierContractView>,
        outgoing: &[Option<Point>],
    ) -> Result<bool, Exhausted> {
        let (Some(facts), Some(function)) = (self.authority.clone(), self.function.clone()) else {
            return Ok(false);
        };
        if facts.phase() != super::super::traits::FactsPhase::Closed
            || self.physical.is_some()
            || self.physical_body.is_some()
            || !body.contract().writes.is_empty()
            || !self.problems.is_empty()
        {
            return Ok(false);
        }
        if let Err(message) = self.ensure_provenance(&function, &facts) {
            let error = super::super::traits::FactsError::from_message(&self.ledger, &message);
            return if error.is_resource() {
                Err(Exhausted)
            } else {
                Ok(false)
            };
        }
        match body.check_body_replay_context(&facts) {
            Ok(()) => {}
            Err(error) if error.is_resource() => return Err(Exhausted),
            Err(_) => return Ok(false),
        }
        let result = self.f.result.clone();
        if !matches!(
            result,
            Ty::Tuple(_) | Ty::Struct(_) | Ty::Enum(_) | Ty::Array { .. }
        ) {
            return Ok(false);
        }
        let Some(locations) = self.metadata_value(ir::carrier_locations(&result, &self.ledger))?
        else {
            return Ok(false);
        };
        if locations.is_empty() || locations.len() != body.contract().results.len() {
            return Ok(false);
        }
        let signature = body_signature(self.f, self.budget)?;
        let mut validated = false;
        let mut cells = Vec::new();
        self.retain(
            mul(locations.len(), size_of::<(usize, BorrowKind)>())?,
            locations.len(),
        )?;
        for (location, row) in locations.iter().zip(body.contract().results.iter()) {
            self.budget.spend(add(location.path.0.len(), 1)?)?;
            if location != &row.location
                || location.channel != CarrierChannel::Fixed(ReferencePosition { depth: 0 })
                || !location.path.0.iter().all(|step| {
                    matches!(
                        step,
                        CarrierStep::Field(_)
                            | CarrierStep::VariantField { .. }
                            | CarrierStep::ConstantIndex(_)
                    )
                })
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
            let Some(Ty::Ref { kind, referent }) =
                self.metadata_value(ir::carrier_at(&result, location, &self.ledger))?
            else {
                return Ok(false);
            };
            let Some(children) =
                self.metadata_value(ir::carrier_locations(&referent, &self.ledger))?
            else {
                return Ok(false);
            };
            if !children.is_empty() {
                return Ok(false);
            }
            self.budget
                .reserve_bytes(mul(location.path.0.len(), 2 * size_of::<Projection>())?)?;
            let key = CellKey {
                storage: CanonicalPlace {
                    root: CanonicalRoot::Owned(self.f.return_local),
                    projection: value_projection(location),
                },
                channel: CellChannel::ReferenceValue,
            };
            let Some(&cell) = self.cell_index.get(&key) else {
                return Ok(false);
            };
            checked_vec_growth(&mut cells, self.budget)?;
            cells.push((cell, kind));
        }
        let mut returned = false;
        for (block, point) in outgoing.iter().enumerate() {
            self.budget.spend(1)?;
            if !matches!(self.f.blocks[block].terminator.value, Terminator::Return) {
                continue;
            }
            let Some(point) = point else { continue };
            returned = true;
            for (index, (cell, kind)) in cells.iter().enumerate() {
                self.budget.spend(add(
                    point.families[*cell].0.0.len(),
                    point.origins[*cell].0.len(),
                )?)?;
                if point.families[*cell].0.count() != 0 {
                    return Ok(false);
                }
                for origin in point.origins[*cell].members() {
                    self.budget.spend(1)?;
                    if *kind == BorrowKind::Mut && self.kinds[origin] != BorrowKind::Mut {
                        return Ok(false);
                    }
                    for (other_index, (other, other_kind)) in cells.iter().enumerate().take(index) {
                        self.budget.spend(add(
                            locations[index].path.0.len(),
                            locations[other_index].path.0.len(),
                        )?)?;
                        if (*kind == BorrowKind::Mut || *other_kind == BorrowKind::Mut)
                            && !exclusive_variants(&locations[index], &locations[other_index])
                            && point.origins[*other].has(origin)
                        {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        // Empty alternative payloads carry no cells. The ordinary initialized
        // return, lifetime/source-contract, loan and move checks still run, and
        // only their complete successful analysis may seal these candidate rows.
        Ok(returned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_exclusivity_requires_exact_owner_and_common_prefix() {
        let source = "enum Choice<'a>{A(&'a mut[i32]),B(&'a mut[i32])}fn choose<'a,T>(x:&'a mut[i32],yes:bool)->Choice<'a>{if yes{Choice::A(x)}else{Choice::B(x)}}fn main(){}";
        let template = super::super::parameter_equality_tests::source_template(source, 0);
        let locations =
            ir::carrier_locations(&template.function().result, template.budget()).unwrap();
        assert_eq!(locations.len(), 2);
        assert!(exclusive_variants(&locations[0], &locations[1]));
        assert!(!exclusive_variants(&locations[0], &locations[0]));
        let foreign = super::super::parameter_equality_tests::source_template(source, 0);
        let foreign_locations =
            ir::carrier_locations(&foreign.function().result, foreign.budget()).unwrap();
        assert!(!exclusive_variants(&locations[0], &foreign_locations[1]));
        let prepend = |location: &CarrierLocation, field| {
            let mut path = vec![CarrierStep::Field(field)];
            path.extend_from_slice(&location.path.0);
            CarrierLocation {
                path: CarrierPath(path.into()),
                channel: location.channel.clone(),
            }
        };
        assert!(exclusive_variants(
            &prepend(&locations[0], 0),
            &prepend(&locations[1], 0)
        ));
        assert!(!exclusive_variants(
            &prepend(&locations[0], 0),
            &prepend(&locations[1], 1)
        ));
    }
}
