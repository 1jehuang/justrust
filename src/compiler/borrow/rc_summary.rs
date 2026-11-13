//! Owned `Rc` boundaries in carrier cell layouts.
//!
//! An `Rc<T>` pointee lives behind a raw pointer and has no carrier channel,
//! so loan origins moved into an `Rc` whose pointee can reach a reference
//! would be dropped. Until the pointee is summarized, such a producer is an
//! explicit unsupported construct rather than an accepted program.
use super::box_summary::box_pointee_carries_references;
use super::*;

fn holds_reference_carrying_rc(
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
    if let Some((ir::OwnedPointerKind::Rc, pointee)) = ty.owned_pointer_pointee() {
        return box_pointee_carries_references(pointee, ledger, budget);
    }
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
    let found = rc_children(ty, ledger, budget, active, depth);
    if nominal.is_some() {
        active.pop();
    }
    found
}

fn rc_children(
    ty: &Ty,
    ledger: Option<&ir::PlanBudget>,
    budget: &mut Budget,
    active: &mut Vec<ir::NominalAppId>,
    depth: usize,
) -> Result<bool, Exhausted> {
    let mut child = |ty: &Ty| -> Result<bool, Exhausted> {
        holds_reference_carrying_rc(ty, ledger, budget, active, depth + 1)
    };
    match ty {
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
            if let Some(pointee) = ty.owned_box_pointee() {
                return child(pointee);
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
        _ => Ok(false),
    }
}

impl CarrierBuilder<'_, '_> {
    /// A call producing a value that holds a reference-carrying `Rc` from
    /// arguments with live loan origins would drop those origins.
    pub(super) fn rc_ingress_unsupported(
        &mut self,
        entry: &FlowState,
        args: &[Operand],
        destination: &Place,
    ) -> Result<bool, Exhausted> {
        let ty = self.place_ty(self.f, destination)?;
        let mut active = Vec::new();
        if !holds_reference_carrying_rc(&ty, Some(&self.ledger), self.budget, &mut active, 0)? {
            return Ok(false);
        }
        for operand in args {
            for (_, origins) in self.operand_value(entry, operand)? {
                if origins.count() != 0 {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
