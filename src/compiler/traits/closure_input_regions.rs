//! Invert a sealed closure input-value result through its original Fn argument.
use super::*;

impl ValidatedTraitWorld {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn input_value_projection_regions(
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
        charged(&self.budget, self.budget.check_depth(depth))?;
        if !self.closed_item_callable_context(projection, context, facts, scratch)?
            || !self.checked_closure_projection(actual, facts.env())?
        {
            return Ok(None);
        }
        let Ty::Closure(closure) = &actual.trait_ref.self_ty else {
            return Ok(None);
        };
        let Some(input) = closure
            .result_receipt()
            .and_then(|receipt| receipt.input_value_origin())
        else {
            return Ok(None);
        };
        let Some(mode) = self.closure_trait_kind(actual.trait_ref.declaration)? else {
            return Ok(None);
        };
        let target = Callee::Direct(self.closure_target(closure, mode)?);
        let view = self.carrier_contract(SignatureSubject::Call(&target), facts)?;
        view.validate_for_with_scratch(facts, scratch)?;
        let mappings = charged(&self.budget, input.carriers(closure.params(), &self.budget))?;
        if view.contract().results.as_ref() != mappings.as_ref()
            || !self.eq_ty(result, closure.result(), 0)?
        {
            return Ok(None);
        }
        let Some(returned) = mappings
            .iter()
            .find(|returned| returned.location == *wanted)
        else {
            return Ok(None);
        };
        let [source] = returned.sources.as_ref() else {
            return Ok(None);
        };
        let Some((ir::CarrierStep::Field(argument), path)) = source.location.path.0.split_first()
        else {
            return Ok(None);
        };
        if source.param != 1 || *argument != input.argument || projection.trait_ref.args.len() != 1
        {
            return Ok(None);
        }
        let Ty::Tuple(arguments) = &projection.trait_ref.args[0] else {
            return Ok(None);
        };
        let Some(original) = arguments.get(*argument) else {
            return Ok(None);
        };
        retain::<ir::CarrierStep>(&self.budget, path.len())?;
        let component = ir::CarrierLocation {
            path: ir::CarrierPath(path.into()),
            channel: source.location.channel.clone(),
        };
        self.input_value_type_regions(
            original,
            &component,
            context,
            prefix,
            facts,
            depth + 1,
            scratch,
        )
    }
}
