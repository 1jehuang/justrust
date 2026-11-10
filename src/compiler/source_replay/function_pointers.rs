//! Authored aggregate coercion equations, never replacement source evidence.
use super::*;

pub(super) fn input_equation(
    parameter: &ir::Ty,
    input: &ir::Ty,
    facts: &tr::ValidatedFacts,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<Option<ir::Ty>, FactsError> {
    budget
        .check_depth(depth)
        .map_err(|e| FactsError::resource(budget, &e))?;
    work(budget, 1)?;
    match (parameter, input) {
        (ir::Ty::FnPointer(_), ir::Ty::FnItem(item)) => {
            Ok(Some(facts.function_pointer_item_type(item)?))
        }
        (ir::Ty::Tuple(parameters), ir::Ty::Tuple(inputs)) if parameters.len() == inputs.len() => {
            let mut fields = Vec::new();
            let mut changed = false;
            for (parameter, input) in parameters.iter().zip(inputs.iter()) {
                let equation = input_equation(parameter, input, facts, budget, depth + 1)?;
                changed |= equation.is_some();
                push(
                    &mut fields,
                    equation.unwrap_or_else(|| input.clone()),
                    budget,
                )?;
            }
            Ok(changed.then(|| ir::Ty::Tuple(fields.into())))
        }
        (
            ir::Ty::Array { element, .. } | ir::Ty::DependentArray { element, .. },
            ir::Ty::Array {
                element: actual,
                len,
            },
        ) => {
            let Some(element) = input_equation(element, actual, facts, budget, depth + 1)? else {
                return Ok(None);
            };
            work(budget, 1)?;
            budget
                .retain_bytes(std::mem::size_of::<ir::Ty>())
                .map_err(|e| FactsError::resource(budget, &e))?;
            Ok(Some(ir::Ty::Array {
                element: Arc::new(element),
                len: *len,
            }))
        }
        // References and nominal actuals are invariant equations, not authored
        // aggregate expression coercion sites. They retain original evidence.
        _ => Ok(None),
    }
}
