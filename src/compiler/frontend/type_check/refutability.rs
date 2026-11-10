//! E0005 for an enum constructor pattern in an irrefutable position (`let`,
//! function and closure parameters). The pattern is refutable exactly when
//! some other variant can hold a value. That is only claimed when it is
//! certain: a sibling whose inhabitedness is unknown keeps the explicit
//! unsupported diagnostic instead of a wrong rejection category.
use super::*;

/// Conservative: `true` only when a value of `ty` certainly exists.
fn definitely_inhabited(
    ty: &ir::Ty,
    seen: &mut Vec<ir::NominalAppId>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    Ok(match ty {
        ir::Ty::Never
        | ir::Ty::Param(_)
        | ir::Ty::AssociatedType(_)
        | ir::Ty::Opaque(_)
        | ir::Ty::DependentArray { .. } => false,
        ir::Ty::Tuple(fields) => {
            for field in fields.iter() {
                if !definitely_inhabited(field, seen, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        ir::Ty::Array { element, len } => {
            *len == 0 || definitely_inhabited(element, seen, budget, depth + 1)?
        }
        ir::Ty::Struct(nominal) => {
            let id = nominal.def().id;
            if seen.contains(&id) {
                return Ok(false);
            }
            seen.push(id);
            let mut inhabited = true;
            for field in nominal.def().fields.read(budget)?.iter() {
                if !definitely_inhabited(&field.ty, seen, budget, depth + 1)? {
                    inhabited = false;
                    break;
                }
            }
            seen.pop();
            inhabited
        }
        ir::Ty::Enum(nominal) => {
            let id = nominal.def().id;
            if seen.contains(&id) {
                return Ok(false);
            }
            seen.push(id);
            let mut inhabited = false;
            for variant in nominal.def().variants.iter() {
                if variant_definitely_inhabited(variant, seen, budget, depth + 1)? {
                    inhabited = true;
                    break;
                }
            }
            seen.pop();
            inhabited
        }
        _ => true,
    })
}

fn variant_definitely_inhabited(
    variant: &ir::VariantDef,
    seen: &mut Vec<ir::NominalAppId>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    for field in variant.fields.read(budget)?.iter() {
        if !definitely_inhabited(&field.ty, seen, budget, depth + 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}

impl Checker<'_> {
    /// The diagnostic for matching `variant` of a multi-variant enum where
    /// the pattern must be irrefutable.
    pub(super) fn refutable_variant_error(
        &self,
        enumeration: &ir::EnumTy,
        variant: usize,
    ) -> Result<String, String> {
        let budget = &self.signatures.plan()?.budget;
        let mut seen = vec![enumeration.def().id];
        for (index, sibling) in enumeration.def().variants.iter().enumerate() {
            if index != variant && variant_definitely_inhabited(sibling, &mut seen, budget, 0)? {
                return Ok(format!(
                    "E0005: refutable pattern in an irrefutable position: variant {} is not covered",
                    sibling.name
                ));
            }
        }
        Ok(Self::refutable_enum_let_unsupported())
    }
}
