//! Exact linear transport of owned generic carrier families into aggregates.
use super::*;

fn owned_family_location(location: &CarrierLocation) -> bool {
    location.channel.family().is_some()
        && location.channel.prefix() == 0
        && location.path.0.iter().all(|step| {
            matches!(
                step,
                CarrierStep::Field(_)
                    | CarrierStep::VariantField { .. }
                    | CarrierStep::ConstantIndex(_)
            )
        })
}

impl CarrierBuilder<'_, '_> {
    /// Moving a whole generic value may expose several mutable captures after
    /// specialization. Certify each component against the same original family
    /// occurrence, rather than treating the components as new independent loans.
    /// This admits neither reborrows nor replacement writes, splitting, or duplicated inputs.
    pub(super) fn linear_owned_family_result_admitted(
        &mut self,
        result: &Ty,
        view: &super::super::traits::CarrierContractView,
        authority: Option<&super::super::traits::ValidatedFacts>,
    ) -> Result<bool, Exhausted> {
        let facts = authority.or(self.authority.as_deref());
        if !facts.is_some_and(|facts| facts.phase() == super::super::traits::FactsPhase::Closed)
            || !view.contract().writes.is_empty()
        {
            return Ok(false);
        }
        // A declaration may retain a deferred family write envelope even when
        // specialization finds no writable referent cells. Only exact identity
        // rows may disappear here. This grants no concrete write permission.
        if !view.original_contract().writes.is_empty()
            && !exact_identity_write_rows(view.original_contract(), self.budget)?
        {
            return Ok(false);
        }
        let mut admitted = false;
        for row in view.contract().results.iter() {
            self.budget.spend(view.contract().inputs.len() + 1)?;
            if !location_is_owned(&row.location) {
                continue;
            }
            let Some(ty) =
                self.metadata_value(ir::carrier_at(result, &row.location, &self.ledger))?
            else {
                return Ok(false);
            };
            if self.copy_permission_in(&ty, authority)? {
                continue;
            }
            let [source] = row.sources.as_ref() else {
                return Ok(false);
            };
            if !view.contract().inputs.contains(source) {
                return Ok(false);
            }
            // An affine input component must occur in precisely this result,
            // including shared-result rows that could otherwise alias it.
            let mut uses = 0usize;
            for other in view.contract().results.iter() {
                self.budget.spend(other.sources.len() + 1)?;
                uses = add(
                    uses,
                    other
                        .sources
                        .iter()
                        .filter(|input| *input == source)
                        .count(),
                )?;
            }
            if uses != 1 {
                return Ok(false);
            }
            let mut exact = false;
            for original in view.original_contract().results.iter() {
                self.budget
                    .spend(original.sources.len() + view.original_contract().inputs.len() + 1)?;
                let [original_source] = original.sources.as_ref() else {
                    continue;
                };
                if !owned_family_location(&original.location)
                    || !owned_family_location(&original_source.location)
                    || original.location.channel != original_source.location.channel
                    || !view.original_contract().inputs.contains(original_source)
                {
                    continue;
                }
                let family = original.location.channel.family().ok_or(Exhausted)?;
                for mapping in view.component_mappings() {
                    self.budget.spend(1)?;
                    if mapping.original_family != family {
                        continue;
                    }
                    let output = self.compose_component(
                        &original.location,
                        &mapping.substituted,
                        view.nominal_mappings(),
                    )?;
                    let input = self.compose_component(
                        &original_source.location,
                        &mapping.substituted,
                        view.nominal_mappings(),
                    )?;
                    if output == row.location
                        && source.param == original_source.param
                        && source.location == input
                    {
                        if exact {
                            return Ok(false);
                        }
                        exact = true;
                    }
                }
            }
            if !exact {
                return Ok(false);
            }
            admitted = true;
        }
        Ok(admitted)
    }
}
