//! Owned `Box` boundaries in carrier cell layouts.
//!
//! Cell layouts materialize reference cells only at carrier paths without a
//! `Deref`, so references stored behind an owned `Box` have no cell. A read
//! whose cover reaches such a `Box` must not be read as "no issued
//! provenance" (an invalid-program verdict): the layout is incomplete, which
//! is an explicit unsupported construct until the pointee is summarized.
use super::*;

/// True when an owned `Box` pointee can reach a shared or mutable reference
/// by value (through fields, variants, tuples, arrays, nested owned `Box`es
/// and owned sequences). Recursive nominal applications are cut at
/// re-entry: a cycle adds no new component types. All scratch is local to
/// this one query.
pub(super) fn box_pointee_carries_references(
    pointee: &Ty,
    ledger: Option<&ir::PlanBudget>,
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    let mut active: Vec<ir::NominalAppId> = Vec::new();
    reaches_reference(pointee, ledger, budget, &mut active, 0)
}

fn reaches_reference(
    ty: &Ty,
    ledger: Option<&ir::PlanBudget>,
    budget: &mut Budget,
    active: &mut Vec<ir::NominalAppId>,
    depth: usize,
) -> Result<bool, Exhausted> {
    if depth > ir::MAX_PATTERN_PROJECTION_DEPTH {
        return Err(Exhausted);
    }
    budget.nodes(1)?;
    budget.spend(add(active.len(), 1)?)?;
    let nominal = match ty {
        Ty::Struct(value) => Some(value.id()),
        Ty::Enum(value) => Some(value.id()),
        _ => None,
    };
    if let Some(id) = nominal {
        if active.contains(&id) {
            return Ok(false);
        }
        checked_vec_growth(active, budget)?;
        active.push(id);
    }
    let found = reaches_reference_children(ty, ledger, budget, active, depth);
    if nominal.is_some() {
        active.pop();
    }
    found
}

fn reaches_reference_children(
    ty: &Ty,
    ledger: Option<&ir::PlanBudget>,
    budget: &mut Budget,
    active: &mut Vec<ir::NominalAppId>,
    depth: usize,
) -> Result<bool, Exhausted> {
    let mut child = |ty: &Ty| -> Result<bool, Exhausted> {
        reaches_reference(ty, ledger, budget, active, depth + 1)
    };
    match ty {
        Ty::Ref { .. } => Ok(true),
        Ty::Tuple(fields) => {
            for field in fields.iter() {
                if child(field)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Ty::Array { element, .. } | Ty::DependentArray { element, .. } | Ty::Slice { element } => {
            child(element)
        }
        Ty::ManuallyDrop(wrapper) => child(wrapper.argument()),
        Ty::Struct(value) => {
            if let Some(pointee) = ty.owned_box_pointee()
                && child(pointee)?
            {
                return Ok(true);
            }
            if let Some(element) = ty.owned_sequence_element()
                && child(element)?
            {
                return Ok(true);
            }
            let fields = value
                .def()
                .fields
                .read_optional(ledger)
                .map_err(|_| Exhausted)?;
            for field in fields.iter() {
                if child(&field.ty)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Ty::Enum(value) => {
            for variant in value.def().variants.iter() {
                let fields = variant
                    .fields
                    .read_optional(ledger)
                    .map_err(|_| Exhausted)?;
                for field in fields.iter() {
                    if child(&field.ty)? {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        // Raw pointers, scalars, code addresses, and erased or generic
        // layouts keep their existing cover classification in `walk`.
        _ => Ok(false),
    }
}
