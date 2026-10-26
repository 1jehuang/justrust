//! Canonical symbolic equality RHS terms, including projection-to-projection
//! relations written by associated declaration bounds.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn reduce_declared_equality_rhs(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rows: &[Predicate],
        env: &GenericEnv,
        active: &mut Vec<ir::AssociatedTypeProjection>,
    ) -> Result<Option<Ty>> {
        charged(&self.budget, self.budget.check_depth(active.len()))?;
        for previous in active.iter() {
            if self.same_projection(previous, projection)? {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: cyclic declared projection equalities",
                ));
            }
        }
        let mut raw = None;
        for row in rows {
            charged(&self.budget, self.budget.charge_work(1))?;
            if let Predicate::ProjectionEq {
                projection: candidate,
                ty,
            } = row
                && self.same_projection(projection, candidate)?
            {
                raw = Some(ty);
                break;
            }
        }
        let Some(raw) = raw else {
            return Ok(None);
        };
        if let Ty::AssociatedType(rhs) = raw
            && self.same_projection(projection, rhs)?
        {
            return Ok(Some(raw.clone()));
        }
        push(&self.budget, active, projection.clone())?;
        let result = self.map_universal_projection_type(raw, env, 0, &mut |child| {
            Ok(self
                .reduce_declared_equality_rhs(child, rows, env, active)?
                .unwrap_or_else(|| Ty::AssociatedType(Arc::new(child.clone()))))
        });
        active.pop();
        result.map(Some)
    }

    pub(super) fn declared_rhs_proves_sized(
        &self,
        ty: &Ty,
        rows: &[Predicate],
        env: &GenericEnv,
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        self.validate_ty_metadata_in(ty, env, depth)?;
        match ty {
            Ty::Param(parameter) => self.predicates_prove_sized(*parameter, rows),
            Ty::AssociatedType(projection) => {
                if !self.schema.associated_type_is_sized(projection.item)? {
                    return Ok(false);
                }
                let membership = Predicate::Implements(projection.trait_ref.clone());
                for row in rows {
                    if self.eq_predicate(row, &membership)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Ty::Ref { .. } | Ty::RawPtr { .. } => Ok(true),
            Ty::Slice { .. } | Ty::Str | Ty::Dyn(_) => Ok(false),
            Ty::Struct(nominal) => {
                for field in nominal_fields(&nominal.def().fields, &self.budget)?.iter() {
                    if !self.declared_rhs_proves_sized(&field.ty, rows, env, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::Enum(nominal) => {
                for variant in nominal.def().variants.iter() {
                    for field in nominal_fields(&variant.fields, &self.budget)?.iter() {
                        if !self.declared_rhs_proves_sized(&field.ty, rows, env, depth + 1)? {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
            _ if !self.raw_associated_ty_is_closed(ty, depth)? => {
                for child in type_arguments(ty) {
                    if !self.declared_rhs_proves_sized(child, rows, env, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(matches!(
                self.prove_in(
                    &Predicate::Implements(TraitRef {
                        declaration: self.catalog.id(CatalogItem::Sized),
                        self_ty: ty.clone(),
                        args: Arc::from([]),
                    }),
                    ProofContext {
                        original: None,
                        predicates: &self.empty,
                        phase: FactsPhase::Closed
                    },
                    &mut ProofScratch::default()
                )?,
                ProofOutcome::Proved(_)
            )),
        }
    }
}
