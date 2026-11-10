//! Contextual callable coercions. Equality still preserves function item identity.
use super::*;

impl<'a: 's, 'b: 's, 's> Solver<'a, 'b, 's> {
    pub(super) fn coerce_function_pointer(
        &mut self,
        actual: Term,
        inputs: &[Term],
        output: Term,
    ) -> Result<bool, String> {
        let (params, result) = match self.domain(actual) {
            Domain::Callable(callable) => {
                if matches!(callable.key, callable::CallableKey::Closure(_)) {
                    return Err("unsupported: closure to function pointer coercion requires a canonical native body adapter".into());
                }
                (callable.params, callable.result)
            }
            Domain::Fixed(ir::Ty::FnItem(item)) => {
                let plan = self.checker.signatures.plan()?;
                let signature =
                    plan.source_inference_signature(&item, self.checker.facts(), &plan.budget)?;
                let params = signature
                    .params
                    .iter()
                    .map(|ty| self.fixed(ty.clone()))
                    .collect::<Result<Vec<_>, _>>()?;
                (params, self.fixed(signature.result)?)
            }
            Domain::Fixed(ir::Ty::Closure(_)) => {
                return Err("unsupported: closure to function pointer coercion requires a canonical native body adapter".into());
            }
            _ => return Ok(false),
        };
        if params.len() != inputs.len() {
            return Err(self.user_error("E0308: function pointer signature arity mismatch"));
        }
        for (actual, expected) in params.into_iter().zip(inputs) {
            self.unify(actual, *expected)?;
        }
        self.unify(result, output)?;
        Ok(true)
    }
}
