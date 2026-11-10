use super::*;

impl ValidatedTraitWorld {
    /// Closed shared payloads carried by a source-owned tuple substitution.
    /// The enclosing metadata replay still authenticates every nominal source.
    pub(super) fn shared_assignment_payload(&self, ty: &Ty, depth: usize) -> Result<bool> {
        self.shared_assignment_payload_in(ty, depth, &mut Vec::new())
    }

    fn shared_assignment_payload_in(
        &self,
        ty: &Ty,
        depth: usize,
        visited: &mut Vec<(TypeNode, Ty)>,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let realization = match ty {
            Ty::Struct(nominal) => nominal.realization(),
            Ty::Enum(nominal) => nominal.realization(),
            _ => None,
        };
        if let Some(read) = realization.filter(|read| read.is_component()) {
            // The component's exact definition, world, modules and equations
            // must authenticate even when its ordinary fields were seen before.
            self.closed_component_type_height(ty)?;
            return read
                .component_shared_assignment_payload(&self.budget, depth, &mut |child, depth| {
                    self.shared_assignment_payload_in(child, depth, visited)
                        .map_err(|error| {
                            crate::engine::Error::new(error.into_legacy_diagnostic(&self.budget))
                        })
                })
                .ok_or_else(|| {
                    FactsError::invalid(
                        &self.budget,
                        "missing shared assignment component capability",
                    )
                })?
                .map_err(|error| FactsError::from_message(&self.budget, &error.to_string()));
        }
        if matches!(ty, Ty::Struct(_) | Ty::Enum(_)) {
            let key = TypeWalk::key(ty).ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "shared assignment nominal has no allocation key",
                )
            })?;
            for (old, _) in visited.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if *old == key {
                    // This is a finite conjunction, not a reusable proof cache.
                    // An edge to an already queued exact immutable allocation
                    // adds no new fields. Every reached field and actual must
                    // finish before the outer query can return true.
                    return Ok(true);
                }
            }
            // The key includes declaration/application identity AND the exact
            // immutable definition allocation, whose ordered actuals cannot
            // change. Retaining the handle prevents pointer reuse in this walk.
            push(&self.budget, visited, (key, ty.clone()))?;
        }
        match ty {
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } => self.shared_assignment_payload_in(referent, depth + 1, visited),
            Ty::Ref { .. }
            | Ty::Slice { .. }
            | Ty::Param(_)
            | Ty::AssociatedType(_)
            | Ty::Closure(_)
            | Ty::Dyn(_)
            | Ty::Opaque(_) => Ok(false),
            Ty::DependentArray { element, .. } | Ty::Array { element, .. } => {
                self.shared_assignment_payload_in(element, depth + 1, visited)
            }
            Ty::Struct(value) => {
                for field in nominal_fields(&value.def().fields, &self.budget)?.iter() {
                    if !self.shared_assignment_payload_in(&field.ty, depth + 1, visited)? {
                        return Ok(false);
                    }
                }
                for argument in type_arguments(ty) {
                    if !self.shared_assignment_payload_in(argument, depth + 1, visited)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in nominal_fields(&variant.fields, &self.budget)?.iter() {
                        if !self.shared_assignment_payload_in(&field.ty, depth + 1, visited)? {
                            return Ok(false);
                        }
                    }
                }
                for argument in type_arguments(ty) {
                    if !self.shared_assignment_payload_in(argument, depth + 1, visited)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => {
                for argument in type_arguments(ty) {
                    if !self.shared_assignment_payload_in(argument, depth + 1, visited)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    #[cfg(test)]
    pub(in crate::compiler) fn shared_assignment_payload_for_tests(&self, ty: &Ty) -> Result<bool> {
        self.shared_assignment_payload(ty, 0)
    }
}
