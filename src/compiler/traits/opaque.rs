//! Opaque return bounds are authenticated against the defining callable schema.
//! This module never reads a defining body or its hidden representation.
use super::*;

impl ValidatedTraitWorld {
    pub(super) fn realize_opaque_family(
        &self,
        opaque: &Arc<ir::OpaqueTy>,
        env: &GenericEnv,
        args: &[Ty],
        facts: &ValidatedFacts,
    ) -> Result<Ty> {
        self.check_facts(facts, false)?;
        self.validate_opaque_metadata(opaque, env)?;
        let schema = GenericSchema {
            env: Arc::new(env.clone()),
            signature: Signature {
                params: Vec::new(),
                result: Ty::Opaque(opaque.clone()),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let actual = self
            .substitute_projected_schema(&schema, args, facts.env())?
            .signature
            .result;
        let Ty::Opaque(opaque) = &actual else {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque family substitution changed its outer declaration identity",
            ));
        };
        self.validate_opaque_metadata(opaque, facts.env())?;
        Ok(actual)
    }
    pub(super) fn prove_opaque(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
    ) -> Result<Option<ProofOutcome>> {
        let Ty::Opaque(opaque) = &predicate.trait_ref().self_ty else {
            return Ok(None);
        };
        self.validate_opaque_metadata(opaque, context.env())?;
        let mut declared = Vec::new();
        for bound in opaque.bounds() {
            let application = bound.application(predicate.trait_ref().self_ty.clone());
            self.append_predicate(&mut declared, &Predicate::Implements(application.clone()))?;
            for (item, ty) in bound.equalities() {
                self.append_predicate(
                    &mut declared,
                    &Predicate::ProjectionEq {
                        projection: ir::AssociatedTypeProjection {
                            trait_ref: application.clone(),
                            item: *item,
                        },
                        ty: ty.clone(),
                    },
                )?;
            }
        }
        // Return-position opaque types are Sized unless an explicit relaxed
        // bound is retained. That syntax is rejected until represented here.
        self.append_predicate(
            &mut declared,
            &Predicate::Implements(TraitRef {
                declaration: self.catalog.id(CatalogItem::Sized),
                self_ty: predicate.trait_ref().self_ty.clone(),
                args: Arc::from([]),
            }),
        )?;
        self.close_predicates(&mut declared, context.env())?;
        for requirement in &declared {
            if self.eq_predicate(requirement, predicate)? {
                return self
                    .evidence(predicate, context, ProofOrigin::OpaqueDeclared)
                    .map(Some);
            }
        }
        // A blanket impl may still establish a trait from the public bounds.
        // Ordinary implementation selection never receives the hidden type.
        Ok(None)
    }

    pub(super) fn validate_opaque_metadata(
        &self,
        opaque: &ir::OpaqueTy,
        env: &GenericEnv,
    ) -> Result<()> {
        let owner = self.info(opaque.owner())?;
        if !matches!(
            owner.kind,
            DeclKind::Function | DeclKind::AssociatedFunction
        ) || owner.env.total_arity() != opaque.owner_args().len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque return changed its defining callable or argument arity",
            ));
        }
        for actual in opaque.owner_args() {
            self.validate_ty_metadata_in(actual, env, 0)?;
        }
        let schema = self.schema.declaration_schema(opaque.owner())?;
        let original = self
            .opaque_occurrence(&schema.signature.result, opaque.owner(), opaque.site(), 0)?
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "opaque return occurrence is absent from its defining result schema",
                )
            })?;
        if !original.try_eq(opaque, |raw, actual| {
            self.validate_rhs_substitution(raw, actual, &owner.env, opaque.owner_args(), 0)?;
            Ok::<_, FactsError>(true)
        })? {
            return Err(FactsError::invalid(
                &self.budget,
                "opaque return changed its declaration-owned bounds or capture identities",
            ));
        }
        // The enclosing type walk validates every bound child separately.
        // Revalidating a complete bound application here would revisit its
        // opaque Self and recurse into this same declaration check.
        Ok(())
    }

    fn opaque_occurrence<'a>(
        &self,
        ty: &'a Ty,
        owner: DeclId,
        site: u32,
        depth: usize,
    ) -> Result<Option<&'a ir::OpaqueTy>> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match ty {
            Ty::Opaque(opaque) => {
                if opaque.owner() == owner && opaque.site() == site {
                    return Ok(Some(opaque));
                }
                for child in opaque.types() {
                    if let Some(found) = self.opaque_occurrence(child, owner, site, depth + 1)? {
                        return Ok(Some(found));
                    }
                }
            }
            Ty::Ref { referent, .. } => {
                return self.opaque_occurrence(referent, owner, site, depth + 1);
            }
            Ty::RawPtr { pointee, .. } => {
                return self.opaque_occurrence(pointee, owner, site, depth + 1);
            }
            Ty::Array { element, .. } | Ty::Slice { element } => {
                return self.opaque_occurrence(element, owner, site, depth + 1);
            }
            Ty::Tuple(_) | Ty::Struct(_) | Ty::Enum(_) => {
                for child in type_arguments(ty) {
                    if let Some(found) = self.opaque_occurrence(child, owner, site, depth + 1)? {
                        return Ok(Some(found));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }
}
