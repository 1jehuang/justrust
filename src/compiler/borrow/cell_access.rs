use super::*;

impl Plan {
    pub(super) fn cell_storage_problem(
        &self,
        f: &cfg::Function,
        target: &Target,
        access: Access,
        point: AccessPoint,
        budget: &mut Budget,
    ) -> Result<Option<Problem>, Exhausted> {
        let invalid = |message: &str| Some(Problem::Invalid(message.into()));
        let Some(authority) = self.authority.as_ref() else {
            return Ok(invalid(
                "Cell interior access lacks validated storage authority",
            ));
        };
        let operation = point
            .statement
            .and_then(|statement| f.blocks.get(point.block.0)?.statements.get(statement))
            .map(|located| &located.value);
        let reference = match (operation, access) {
            (
                Some(Statement::Assign {
                    value: Rvalue::CellGet { reference, .. },
                    ..
                }),
                Access::CellRead,
            )
            | (
                Some(Statement::Assign {
                    value: Rvalue::CellSet { reference, .. },
                    ..
                }),
                Access::CellWrite,
            ) => reference,
            _ => {
                return Ok(invalid(
                    "Cell interior access does not match its original operation",
                ));
            }
        };
        let Some(local) = f.locals.get(reference.local.0) else {
            return Ok(invalid("Cell operation reference local is missing"));
        };
        let reference_ty = match authority.projected_ty(&local.ty, &reference.projection) {
            Ok(ty) => ty,
            Err(error) if error.is_resource() => return Err(Exhausted),
            Err(error) => return Ok(Some(Problem::Invalid(error.to_string()))),
        };
        let Ty::Ref {
            kind: BorrowKind::Shared,
            referent,
        } = reference_ty
        else {
            return Ok(invalid(
                "Cell operation requires an exact shared Cell reference",
            ));
        };
        if !matches!(referent.as_ref(), Ty::Cell(cell) if cell.is_concrete()) {
            return Ok(invalid(
                "Cell operation reference lacks canonical primitive Cell storage",
            ));
        }
        if let Err(error) = authority.validate_ty(&referent) {
            return if error.is_resource() {
                Err(Exhausted)
            } else {
                Ok(Some(Problem::Invalid(error.to_string())))
            };
        }
        let base = match &target.place.root {
            CanonicalRoot::Owned(local) => f.locals.get(local.0).map(|local| local.ty.clone()),
            CanonicalRoot::ParameterReferent(local, position) => {
                let Some(local) = f.locals.get(local.0) else {
                    return Ok(invalid("Cell storage root is missing"));
                };
                match ir::carrier_at(&local.ty, position, authority.budget()) {
                    Ok(Ty::Ref { referent, .. }) => Some(referent.as_ref().clone()),
                    _ => None,
                }
            }
        };
        let Some(base) = base else {
            return Ok(invalid(
                "Cell storage root lacks an exact reference carrier",
            ));
        };
        let actual = match authority.projected_ty(&base, &target.place.projection) {
            Ok(actual) => actual,
            Err(error) if error.is_resource() => return Err(Exhausted),
            Err(error) => return Ok(Some(Problem::Invalid(error.to_string()))),
        };
        if target.precision == TargetPrecision::Exact {
            if !matches!(&actual, Ty::Cell(cell) if cell.is_concrete()) {
                return Ok(invalid(
                    "Cell interior access requires canonical primitive Cell storage",
                ));
            }
        } else {
            // A source-owned Deref result can prove only a conservative footprint
            // in its owner's issued cover. It is not a physical field address.
            // Keep that footprint and every loan authority, and require that the
            // exact original Cell reference resolves to this very target here.
            let count = reference.projection.len().checked_add(1).ok_or(Exhausted)?;
            budget.reserve_bytes(
                count
                    .checked_mul(size_of::<Projection>())
                    .ok_or(Exhausted)?,
            )?;
            let mut projection = Vec::new();
            projection.try_reserve_exact(count).map_err(|_| Exhausted)?;
            if projection.capacity() > count {
                budget.reserve_bytes(
                    (projection.capacity() - count)
                        .checked_mul(size_of::<Projection>())
                        .ok_or(Exhausted)?,
                )?;
            }
            projection.extend_from_slice(&reference.projection);
            projection.push(Projection::Deref);
            let storage = Place {
                local: reference.local,
                projection,
            };
            let issued =
                match self.resolve(&storage, point.block, point.statement, point.phase, budget)? {
                    Ok(issued) => issued,
                    Err(problem) => return Ok(Some(problem)),
                };
            let mut matches = false;
            for issued in &issued {
                let root_work = |target: &Target| match &target.place.root {
                    CanonicalRoot::Owned(_) => 0,
                    CanonicalRoot::ParameterReferent(_, location) => location.path.0.len(),
                };
                let work = issued
                    .place
                    .projection
                    .len()
                    .checked_add(issued.authority.len())
                    .and_then(|work| work.checked_add(target.place.projection.len()))
                    .and_then(|work| work.checked_add(target.authority.len()))
                    .and_then(|work| work.checked_add(root_work(issued)))
                    .and_then(|work| work.checked_add(root_work(target)))
                    .and_then(|work| work.checked_add(1))
                    .ok_or(Exhausted)?;
                budget.spend(work)?;
                if issued == target {
                    matches = true;
                    break;
                }
            }
            if !matches {
                return Ok(invalid(
                    "Cell footprint differs from the original issued reference target",
                ));
            }
        }
        if let Err(error) = authority.validate_ty(&actual) {
            return if error.is_resource() {
                Err(Exhausted)
            } else {
                Ok(Some(Problem::Invalid(error.to_string())))
            };
        }
        Ok(None)
    }
}
