//! Transport exact function item results through original callable arguments.
use super::*;

impl ValidatedTraitWorld {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fn_item_projection_regions(
        &self,
        projection: &ir::AssociatedTypeProjection,
        actual: &ir::AssociatedTypeProjection,
        result: &Ty,
        wanted: &ir::CarrierLocation,
        context: &ItemRegionContext<'_>,
        prefix: &ir::CarrierPath,
        facts: &ValidatedFacts,
        depth: usize,
        scratch: &mut TraitScratch,
    ) -> Result<Option<Vec<ir::CarrierLocation>>> {
        if !self.closed_item_callable_context(projection, context, facts, scratch)? {
            return Ok(None);
        }
        let Ty::FnItem(_) = &actual.trait_ref.self_ty else {
            return Ok(None);
        };
        if self
            .closure_trait_kind(actual.trait_ref.declaration)?
            .is_none()
        {
            return Ok(None);
        }
        let items = &self.trait_header(actual.trait_ref.declaration)?.items;
        let [item] = items.as_ref() else {
            return Ok(None);
        };
        let call = TraitCall {
            item: *item,
            trait_ref: actual.trait_ref.clone(),
            method_args: Arc::from([]),
        };
        let signature = self.instantiate_trait_signature(&call, facts)?;
        if !self.eq_ty(&signature.result, result, 0)? || projection.trait_ref.args.len() != 1 {
            return Ok(None);
        }
        let mappings = self.fn_item_tuple_contract(&actual.trait_ref, &signature, facts)?;
        let Some(returned) = mappings
            .results
            .iter()
            .find(|returned| returned.location == *wanted)
        else {
            return Ok(None);
        };
        let Ty::Tuple(arguments) = &projection.trait_ref.args[0] else {
            return Ok(None);
        };
        let mut regions = Vec::new();
        for source in returned.sources.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            let Some((ir::CarrierStep::Field(argument), path)) =
                source.location.path.0.split_first()
            else {
                return Ok(None);
            };
            let Some(original) = arguments.get(*argument) else {
                return Ok(None);
            };
            if source.param != 1 {
                return Ok(None);
            }
            retain::<ir::CarrierStep>(&self.budget, path.len())?;
            let component = ir::CarrierLocation {
                path: ir::CarrierPath(path.into()),
                channel: source.location.channel.clone(),
            };
            let Some(locations) = self.input_value_type_regions(
                original,
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
                if !regions.contains(&location) {
                    push(&self.budget, &mut regions, location)?;
                }
            }
        }
        Ok((!regions.is_empty()).then_some(regions))
    }
}
