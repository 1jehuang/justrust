//! Scope preparation is not destructor evidence. Abstract owners retain their
//! ordinary statement-temporary storage events for a later physical instance.
use super::*;

pub(super) fn required(ty: &Ty, facts: &ValidatedFacts) -> Result<bool, String> {
    facts
        .validate_ty_metadata(ty)
        .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?;
    walk(ty, facts.budget(), &mut Vec::new(), 0)
}

fn walk(
    ty: &Ty,
    budget: &ir::PlanBudget,
    active: &mut Vec<Ty>,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match ty {
        Ty::Opaque(_) | Ty::Param(_) => return Ok(true),
        // These wrappers never transfer automatic ownership of their payload.
        Ty::Ref { .. }
        | Ty::RawPtr { .. }
        | Ty::ManuallyDrop(_)
        | Ty::PhantomData(_)
        | Ty::Cell(_) => return Ok(false),
        _ => {}
    }
    for prior in active.iter() {
        if ir::identical_type(prior, ty, budget)? {
            return Ok(false);
        }
    }
    admit_cfg_push(active, Some(budget))?;
    active.push(ty.clone());
    let result = (|| {
        if let Some((_, pointee)) = ty.owned_pointer_pointee() {
            return walk(pointee, budget, active, depth + 1);
        }
        match ty {
            Ty::Array { element, .. }
            | Ty::DependentArray { element, .. }
            | Ty::Slice { element } => walk(element, budget, active, depth + 1),
            Ty::Tuple(fields) => parts(fields, budget, active, depth),
            Ty::Closure(closure) => parts(closure.captures(), budget, active, depth),
            Ty::Struct(value) => {
                for field in value.def().fields.read(budget)?.iter() {
                    if walk(&field.ty, budget, active, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    budget.charge_work(1)?;
                    for field in variant.fields.read(budget)?.iter() {
                        if walk(&field.ty, budget, active, depth + 1)? {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    })();
    active.pop();
    result
}
fn parts(
    fields: &[Ty],
    budget: &ir::PlanBudget,
    active: &mut Vec<Ty>,
    depth: usize,
) -> Result<bool, String> {
    for field in fields {
        if walk(field, budget, active, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}
