//! Fully resolved call actuals use the ordinary sealed signature instantiation.
//! No inference variable can authorize a projection equality.
use super::*;

impl<'s> Solver<'_, '_, 's> {
    pub(super) fn projection_call_signature(
        &mut self,
        declaration: ir::DeclId,
        schema: &ir::GenericSignature,
        params: &HashMap<ir::ParamId, Term>,
    ) -> Result<Option<ir::Signature>, String> {
        let plan = self.checker.signatures.plan()?;
        let mut projected = ir::contains_associated_types(&schema.signature.result, &plan.budget)?;
        for param in &schema.signature.params {
            projected |= ir::contains_associated_types(param, &plan.budget)?;
        }
        if !projected {
            return Ok(None);
        }
        self.checker.infer_storage(schema.env.total_arity())?;
        let mut actuals = Vec::new();
        for param in schema.env.ordered_params() {
            let Some(term) = params.get(&param).copied() else {
                return Ok(None);
            };
            let Some(actual) = self.resolve(term, 0)? else {
                return Ok(None);
            };
            actuals.push(actual);
        }
        let target = ir::CallTarget {
            declaration,
            type_args: actuals.into(),
        };
        plan.instantiate_signature(&target, self.checker.facts(), &plan.budget)
            .map(Some)
    }
}
