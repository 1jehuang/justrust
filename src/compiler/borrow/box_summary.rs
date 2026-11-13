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

/// True when `ty` holds, by value, an owned `Box` whose pointee can reach a
/// reference. Loan origins moved into such a Box have no carrier channel
/// yet, so a producer that receives live origins must fail closed.
pub(super) fn holds_reference_carrying_box(
    ty: &Ty,
    ledger: Option<&ir::PlanBudget>,
    budget: &mut Budget,
) -> Result<bool, Exhausted> {
    let mut active: Vec<ir::DeclId> = Vec::new();
    holds_box(ty, ledger, budget, &mut active, 0)
}

fn holds_box(
    ty: &Ty,
    ledger: Option<&ir::PlanBudget>,
    budget: &mut Budget,
    active: &mut Vec<ir::DeclId>,
    depth: usize,
) -> Result<bool, Exhausted> {
    if depth > ir::MAX_PATTERN_PROJECTION_DEPTH {
        return Err(Exhausted);
    }
    budget.nodes(1)?;
    budget.spend(add(active.len(), 1)?)?;
    if let Some(pointee) = ty.owned_box_pointee() {
        // A carried pointee has its own cells (`CarrierStep::BoxPointee`,
        // same rule as the carrier inventory): look inside it instead.
        if let Some(ledger_budget) = ledger
            && ir::box_pointee_carried(pointee, ledger_budget).map_err(|_| Exhausted)?
        {
            return holds_box(pointee, ledger, budget, active, depth + 1);
        }
        // Erased objects keep their separate object-lifetime check.
        return box_pointee_carries_references(pointee, ledger, budget);
    }
    let nominal = match ty {
        Ty::Struct(value) => Some(value.def().source_decl),
        Ty::Enum(value) => Some(value.def().source_decl),
        _ => None,
    };
    if let Some(id) = nominal {
        if active.contains(&id) {
            return Ok(false);
        }
        checked_vec_growth(active, budget)?;
        active.push(id);
    }
    let found = holds_box_children(ty, ledger, budget, active, depth);
    if nominal.is_some() {
        active.pop();
    }
    found
}

fn holds_box_children(
    ty: &Ty,
    ledger: Option<&ir::PlanBudget>,
    budget: &mut Budget,
    active: &mut Vec<ir::DeclId>,
    depth: usize,
) -> Result<bool, Exhausted> {
    let mut child =
        |ty: &Ty| -> Result<bool, Exhausted> { holds_box(ty, ledger, budget, active, depth + 1) };
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
    /// A call producing a reference-carrying owned Box from arguments that
    /// carry live loan origins would drop those origins (the Box pointee has
    /// no carrier channel). Report it as unsupported instead of accepting.
    pub(super) fn box_ingress_unsupported(
        &mut self,
        entry: &FlowState,
        args: &[Operand],
        destination: &Place,
    ) -> Result<bool, Exhausted> {
        let ty = self.place_ty(self.f, destination)?;
        let ledger = Some(&self.ledger);
        if !holds_reference_carrying_box(&ty, ledger, self.budget)? {
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

impl CarrierBuilder<'_, '_> {
    /// A store of live loan origins into storage reached through an owned
    /// Box dereference (`*b = &s`, `**r = L::C(&s, ..)`) has no cell to
    /// receive them. Report it as unsupported instead of dropping them.
    pub(super) fn box_pointee_write_unsupported(
        &mut self,
        destination: &Place,
        value: &[(CarrierLocation, CarrierOrigins)],
    ) -> Result<bool, Exhausted> {
        if value.iter().all(|(_, origins)| origins.count() == 0) {
            return Ok(false);
        }
        let mut ty = self.f.locals[destination.local.0].ty.clone();
        for (index, step) in destination.projection.iter().enumerate() {
            self.budget.spend(index + 1)?;
            if *step == Projection::Deref
                && let Some(pointee) = ty.owned_box_pointee()
                && !ir::box_pointee_carried(pointee, &self.ledger).map_err(|_| Exhausted)?
            {
                return Ok(true);
            }
            ty = ir::projected_ty(&ty, &destination.projection[index..=index], &self.ledger)
                .map_err(|_| Exhausted)?;
        }
        Ok(false)
    }
}
