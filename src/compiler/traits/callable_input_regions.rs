//! Original argument type inversion shared by independent callable producers.
use super::*;

impl ValidatedTraitWorld {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn input_value_type_regions(
        &self,
        ty: &Ty,
        wanted: &ir::CarrierLocation,
        context: &ItemRegionContext<'_>,
        prefix: &ir::CarrierPath,
        facts: &ValidatedFacts,
        depth: usize,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Vec<ir::CarrierLocation>>> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match ty {
            Ty::Param(parameter) => self.item_parameter_regions(
                *parameter,
                wanted,
                context,
                prefix,
                facts,
                depth + 1,
                scratch,
            ),
            Ty::AssociatedType(projection) => self.item_projection_regions(
                projection,
                wanted,
                context,
                prefix,
                facts,
                depth + 1,
                scratch,
            ),
            _ => {
                let inverse = self.item_region_locations(ty, context, wanted, facts, scratch)?;
                let mut result = Vec::new();
                for (parameter, component) in inverse {
                    let Some(locations) = self.item_parameter_regions(
                        parameter,
                        &component,
                        context,
                        prefix,
                        facts,
                        depth + 1,
                        scratch,
                    )?
                    else {
                        return Ok(None);
                    };
                    for location in locations {
                        if !result.contains(&location) {
                            push(&self.budget, &mut result, location)?;
                        }
                    }
                }
                Ok((!result.is_empty()).then_some(result))
            }
        }
    }
}
