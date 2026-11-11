//! A selected sequence element updates its existing owner summary weakly.
//! This does not refine an element loan, identify an index, or overwrite an
//! entire summary. Permission comes from original authored carrier coordinates
//! and their exact body-view transport, not from equal substituted types.
//! The reference must itself be the source-issued IndexMut result. An indexed
//! ancestor in another reference's lifetime envelope cannot prove selection.
use super::*;

pub(super) struct IndexedSummaryWrite {
    cell: usize,
}

impl IndexedSummaryWrite {
    pub(super) fn cell(&self) -> usize {
        self.cell
    }
}

impl CarrierBuilder<'_, '_> {
    fn indexed_reference_origin(
        &mut self,
        state: &FlowState,
        destination: &Place,
    ) -> Result<bool, Exhausted> {
        let Some((Projection::Deref, prefix)) = destination.projection.split_last() else {
            return Ok(false);
        };
        let carrier = Place {
            local: destination.local,
            projection: prefix.to_vec(),
        };
        let (slots, _, failure) = self.lens(state, &carrier)?;
        let [slot] = slots.as_slice() else {
            return Ok(false);
        };
        if failure.is_some() || slot.precision != TargetPrecision::Exact {
            return Ok(false);
        }
        let Some(cell) = self.cell(&CellKey {
            storage: slot.place.clone(),
            channel: CellChannel::ReferenceValue,
        })?
        else {
            return Ok(false);
        };
        let origins = &state.cells[cell];
        if origins.references.0.count() != 1 || origins.families.0.count() != 0 {
            return Ok(false);
        }
        let origin = origins.references.0.members().next().ok_or(Exhausted)?;
        let (Some(function), Some(facts)) = (self.function.clone(), self.authority.clone()) else {
            return Ok(false);
        };
        let provenance = self.ensure_provenance(&function, &facts);
        if self.metadata_value(provenance)?.is_none() {
            return Ok(false);
        }
        self.budget
            .spend(self.call_rows.iter().map(Vec::len).sum::<usize>() + 1)?;
        let Some(block) = self.call_rows.iter().position(|rows| {
            rows.iter().any(|(location, row)| {
                row.0 == origin && *location == fixed_location(ReferencePosition { depth: 0 })
            })
        }) else {
            // A reborrow/other call is not the indexed result itself. Seeing
            // an indexed ancestor in its lifetime authority is insufficient.
            return Ok(false);
        };
        let Some(_view) = self.calls[block].clone() else {
            return Ok(false);
        };
        let Terminator::Call {
            target,
            signature,
            args,
            ..
        } = &self.f.blocks[block].terminator.value
        else {
            return Ok(false);
        };
        let Some(Ty::Ref {
            kind: BorrowKind::Mut,
            referent: owner,
        }) = signature.params.first()
        else {
            return Ok(false);
        };
        let Ty::Struct(owner_struct) = owner.as_ref() else {
            return Ok(false);
        };
        if args.len() != 2
            || signature.params.len() != 2
            || signature.params[1] != Ty::Int(ir::IntTy::Usize)
            || !matches!(&signature.result, Ty::Ref { kind: BorrowKind::Mut, referent }
                if owner.owned_sequence_element() == Some(referent.as_ref())
                    && referent.as_ref() == &self.place_ty(self.f, destination)?)
        {
            return Ok(false);
        }
        let role = facts
            .native_sequence_element_for_call_with_scratch(
                target,
                signature,
                &mut self.carrier_scratch,
            )
            .map_err(|error| error.into_legacy_diagnostic(&self.ledger));
        let Some(Some((declaration, role))) = self.metadata_value(role)? else {
            return Ok(false);
        };
        Ok(role.role_for(declaration, owner_struct)
            == Some(super::super::frontend::NativeElementRole::MutIndex))
    }

    pub(super) fn indexed_summary_write(
        &mut self,
        state: &FlowState,
        destination: &Place,
        component: &CarrierLocation,
        channel: &CellChannel,
        targets: &[Target],
    ) -> Result<Option<IndexedSummaryWrite>, Exhausted> {
        let [target] = targets else {
            return Ok(None);
        };
        if target.precision != TargetPrecision::Subtree
            || target.place.projection.contains(&Projection::Deref)
            || value_projection(component).contains(&Projection::Deref)
        {
            return Ok(None);
        }
        let (owner, referent_location) = match &target.place.root {
            CanonicalRoot::ParameterReferent(owner, location) => (owner, location),
            CanonicalRoot::Owned(owner) => {
                return self.owned_indexed_summary_write(
                    state,
                    destination,
                    component,
                    channel,
                    target,
                    *owner,
                );
            }
        };
        let Some(body) = self.body.clone() else {
            return Ok(None);
        };
        let Some(original_destination) = self.original_locals.get(destination.local.0) else {
            // CFG-only scratch ordinals are not original source coordinates.
            return Ok(None);
        };
        let Some(original_value) =
            probe_projected_shape(original_destination, &destination.projection, self.budget)?
        else {
            return Ok(None);
        };
        let Some(original_owner) = self.original_locals.get(owner.0) else {
            return Ok(None);
        };
        let referent_path = value_projection(referent_location);
        let Some(Ty::Ref {
            kind: BorrowKind::Mut,
            referent: original_owner,
        }) = probe_projected_shape(original_owner, &referent_path, self.budget)?
        else {
            return Ok(None);
        };
        let Some(original_owner) =
            probe_projected_shape(&original_owner, &target.place.projection, self.budget)?
        else {
            return Ok(None);
        };
        if original_owner.owned_sequence_element() != Some(&original_value) {
            return Ok(None);
        }
        let Some(parameter) = self.f.params.iter().position(|local| local == owner) else {
            return Ok(None);
        };
        self.budget.spend(self.f.params.len() + 1)?;

        // Only this exact body view may expose the closed parameter shape.
        // Shared read covers intentionally omit mutable-reference actuals.
        // This shape comparison issues neither cells nor lifetime authority:
        // the original authored row and exact writable component transport
        // below must still identify the existing summary cell.
        let Some(expanded_owner) = body.signature_for_replay().params.get(parameter) else {
            return Ok(None);
        };
        let (Some(closed_owner), Some(facts)) =
            (self.f.locals.get(owner.0), self.authority.clone())
        else {
            return Ok(None);
        };
        // Canonical source and realized nominal definitions have distinct
        // physical identities. Compare their declaration and ordered actuals
        // through the same validated world, never by display names or layout.
        let same_owner = facts
            .world()
            .types_equal(
                std::slice::from_ref(&closed_owner.ty),
                std::slice::from_ref(expanded_owner),
            )
            .map_err(|error| error.into_legacy_diagnostic(&self.ledger));
        if self.metadata_value(same_owner)? != Some(true) {
            return Ok(None);
        }
        let Some(Ty::Ref {
            kind: BorrowKind::Mut,
            referent: expanded_owner,
        }) = probe_projected_shape(expanded_owner, &referent_path, self.budget)?
        else {
            return Ok(None);
        };
        let Some(expanded_owner) =
            probe_projected_shape(&expanded_owner, &target.place.projection, self.budget)?
        else {
            return Ok(None);
        };
        let expanded_value = self.place_ty(self.f, destination)?;
        if expanded_owner.owned_sequence_element() != Some(&expanded_value)
            || !self.indexed_reference_origin(state, destination)?
        {
            return Ok(None);
        }

        self.retain(
            mul(
                target.place.projection.len() + component.path.0.len() * 2 + 2,
                size_of::<Projection>(),
            )?,
            target.place.projection.len() + component.path.0.len() + 1,
        )?;
        let mut storage = target.place.clone();
        storage
            .projection
            .push(Projection::DynamicIndex(LocalId(usize::MAX)));
        storage.projection.extend(value_projection(component));
        let Some(cell) = self.cell(&CellKey {
            storage,
            channel: channel.clone(),
        })?
        else {
            // Never manufacture a summary cell or a new concrete channel.
            return Ok(None);
        };
        let Some(input) = self.cells[cell].input.clone() else {
            return Ok(None);
        };
        if input.param != parameter {
            return Ok(None);
        }

        self.retain(
            mul(
                referent_path.len() + target.place.projection.len() + 2,
                size_of::<Projection>(),
            )?,
            referent_path.len() + target.place.projection.len() + 2,
        )?;
        let mut owner_path = referent_path;
        owner_path.push(Projection::Deref);
        owner_path.extend_from_slice(&target.place.projection);
        owner_path.push(Projection::DynamicIndex(LocalId(usize::MAX)));
        let originals =
            ir::carrier_locations(&original_value, &self.ledger).map_err(|_| Exhausted)?;
        for original_component in originals.iter() {
            let relative = value_projection(original_component);
            if relative.contains(&Projection::Deref) {
                continue;
            }
            let original_semantic =
                ir::carrier_at(&original_value, original_component, &self.ledger)
                    .map_err(|_| Exhausted)?;
            let Some(original_channel) = key_channel(&original_semantic) else {
                continue;
            };
            for row in body.original_contract().inputs.iter() {
                self.budget
                    .spend(owner_path.len() + relative.len() + row.location.path.0.len() * 2 + 1)?;
                if row.param != parameter {
                    continue;
                }
                let authored_path = value_projection(&row.location);
                if authored_path.len() != owner_path.len() + relative.len()
                    || !authored_path.starts_with(&owner_path)
                    || authored_path[owner_path.len()..] != relative
                {
                    continue;
                }
                let Some(original_parameter) = self.original_locals.get(owner.0) else {
                    return Ok(None);
                };
                let authored_semantic =
                    ir::carrier_at(original_parameter, &row.location, &self.ledger)
                        .map_err(|_| Exhausted)?;
                if key_channel(&authored_semantic).as_ref() != Some(&original_channel) {
                    continue;
                }
                // This performs the same exact original-family component and
                // nominal mapping as ordinary body/call writes, and retains
                // only destinations in the canonical writable envelope.
                let transported = self.transport_effect_input(row, &body, true)?;
                if transported.iter().any(|(mapped, _)| *mapped == input) {
                    return Ok(Some(IndexedSummaryWrite { cell }));
                }
            }
        }
        Ok(None)
    }

    /// A locally owned sequence (`let mut d: Vec<T>`) has one issued element
    /// summary cell per carrier component. A store through its source-issued
    /// `IndexMut::index_mut` result joins that existing cell weakly, exactly
    /// as for a parameter referent, but needs no writable input envelope: the
    /// owner is body storage, not caller-visible input. Nothing is issued.
    fn owned_indexed_summary_write(
        &mut self,
        state: &FlowState,
        destination: &Place,
        component: &CarrierLocation,
        channel: &CellChannel,
        target: &Target,
        owner: LocalId,
    ) -> Result<Option<IndexedSummaryWrite>, Exhausted> {
        if self.f.params.contains(&owner) {
            // A by-value parameter's element cells carry input rows; keep
            // those on the authored-envelope route.
            return Ok(None);
        }
        let (Some(original_destination), Some(original_owner)) = (
            self.original_locals.get(destination.local.0),
            self.original_locals.get(owner.0),
        ) else {
            // CFG-only scratch ordinals are not original source coordinates.
            return Ok(None);
        };
        let (original_destination, original_owner) =
            (original_destination.clone(), original_owner.clone());
        let Some(original_value) =
            probe_projected_shape(&original_destination, &destination.projection, self.budget)?
        else {
            return Ok(None);
        };
        let Some(original_owner) =
            probe_projected_shape(&original_owner, &target.place.projection, self.budget)?
        else {
            return Ok(None);
        };
        if original_owner.owned_sequence_element() != Some(&original_value) {
            return Ok(None);
        }
        let Some(closed_owner) = self.f.locals.get(owner.0).map(|local| local.ty.clone()) else {
            return Ok(None);
        };
        let Some(closed_owner) =
            probe_projected_shape(&closed_owner, &target.place.projection, self.budget)?
        else {
            return Ok(None);
        };
        let closed_value = self.place_ty(self.f, destination)?;
        if closed_owner.owned_sequence_element() != Some(&closed_value)
            || !self.indexed_reference_origin(state, destination)?
        {
            return Ok(None);
        }
        self.retain(
            mul(
                target.place.projection.len() + component.path.0.len() * 2 + 2,
                size_of::<Projection>(),
            )?,
            target.place.projection.len() + component.path.0.len() + 1,
        )?;
        let mut storage = target.place.clone();
        storage
            .projection
            .push(Projection::DynamicIndex(LocalId(usize::MAX)));
        storage.projection.extend(value_projection(component));
        let Some(cell) = self.cell(&CellKey {
            storage,
            channel: channel.clone(),
        })?
        else {
            // Never manufacture a summary cell or a new concrete channel.
            return Ok(None);
        };
        if self.cells[cell].input.is_some() {
            return Ok(None);
        }
        Ok(Some(IndexedSummaryWrite { cell }))
    }

    /// One moved original element family may expand to several affine fields.
    /// Its fields remain paired component-for-component, never a Cartesian
    /// product or several independent moves of the whole original value.
    pub(super) fn indexed_family_write_admitted(
        &mut self,
        params: &[Ty],
        view: &super::super::traits::CarrierContractView,
    ) -> Result<bool, Exhausted> {
        let original = view.original_contract();
        let [write] = original.writes.as_ref() else {
            return Ok(false);
        };
        if !original.results.is_empty() || !view.contract().results.is_empty() {
            return Ok(false);
        }
        let CarrierChannel::ParamTail {
            parameter,
            prefix: 0,
        } = write.destination.location.channel
        else {
            return Ok(false);
        };
        if write.destination.location.path.0.as_ref()
            != [CarrierStep::Referent, CarrierStep::SliceElement]
            || write.sources.len() != 2
            || !write.sources.contains(&write.destination)
        {
            return Ok(false);
        }
        let Some(source) = write
            .sources
            .iter()
            .find(|source| **source != write.destination)
        else {
            return Ok(false);
        };
        if source.param == write.destination.param
            || !source.location.path.0.is_empty()
            || source.location.channel != write.destination.location.channel
        {
            return Ok(false);
        }
        let Some((_, actual)) = view.parameter_actual(parameter).map_err(|_| Exhausted)? else {
            return Ok(false);
        };
        let Some(Ty::Ref {
            kind: BorrowKind::Mut,
            referent: owner,
        }) = params.get(write.destination.param)
        else {
            return Ok(false);
        };
        if owner.owned_sequence_element() != Some(actual)
            || params.get(source.param) != Some(actual)
        {
            return Ok(false);
        }
        let components = ir::carrier_locations(actual, &self.ledger).map_err(|_| Exhausted)?;
        let mut destinations = Vec::new();
        for component in components.iter() {
            if value_projection(component).contains(&Projection::Deref) {
                continue;
            }
            let mut matching = view.component_mappings().iter().filter(|mapping| {
                mapping.original_family == ir::CarrierFamily::Parameter(parameter)
                    && mapping.substituted == *component
            });
            let Some(mapping) = matching.next() else {
                return Ok(false);
            };
            if matching.next().is_some() {
                return Ok(false);
            }
            let destination = InputCarrier {
                param: write.destination.param,
                location: self.compose_component(
                    &write.destination.location,
                    component,
                    view.nominal_mappings(),
                )?,
            };
            let moved = InputCarrier {
                param: source.param,
                location: self.compose_component(
                    &source.location,
                    component,
                    view.nominal_mappings(),
                )?,
            };
            // This helper authenticates relative coordinates only. No Copy
            // permission is requested or inferred for the mapped component.
            if !self.copy_family_component_matches(
                parameter,
                &write.destination.location,
                &destination.location,
                mapping,
                view,
            )? || !self.copy_family_component_matches(
                parameter,
                &source.location,
                &moved.location,
                mapping,
                view,
            )? {
                return Ok(false);
            }
            self.budget
                .spend(view.contract().writes.len() + destinations.len() + 1)?;
            let mut rows = view
                .contract()
                .writes
                .iter()
                .filter(|row| row.destination == destination);
            let Some(row) = rows.next() else {
                return Ok(false);
            };
            if rows.next().is_some()
                || row.sources.len() != 2
                || !row.sources.contains(&destination)
                || !row.sources.contains(&moved)
                || destinations.contains(&destination)
            {
                return Ok(false);
            }
            checked_vec_growth(&mut destinations, self.budget)?;
            destinations.push(destination);
        }
        Ok(destinations.len() > 1 && destinations.len() == view.contract().writes.len())
    }
}
