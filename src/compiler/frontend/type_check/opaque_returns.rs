//! Local collection of final hidden return representations during source checking.
//! The collection is frozen into TEMPLATE_BODY output 2 before publication.
use super::*;

fn opaque_bound_diagnostic(owner: Option<ir::DeclId>, goal: &ir::Predicate) -> String {
    let (application, requirement) = match goal {
        ir::Predicate::Implements(application) => (application, "membership".to_owned()),
        ir::Predicate::ProjectionEq { projection, .. } => (
            &projection.trait_ref,
            format!("associated Output/equality item={:?}", projection.item),
        ),
    };
    let hidden = match &application.self_ty {
        ir::Ty::Closure(closure) => format!(
            "closure owner={:?} provider={:?} site={} kind={:?}",
            closure.owner(),
            closure.provider(),
            closure.site(),
            closure.kind()
        ),
        _ => "hidden type is not a closure".into(),
    };
    format!(
        "E0277: hidden return type does not satisfy its declared opaque bounds; defining owner={owner:?}; {hidden}; required {requirement}; trait={:?}; ordered argument count={}",
        application.declaration,
        application.args.len()
    )
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn opaque_bound_diagnostic_distinguishes_membership_and_output() {
        let declaration = ir::DeclId {
            snapshot: ir::SnapshotId::allocate().unwrap(),
            index: 7,
        };
        let application = ir::TraitRef {
            declaration,
            self_ty: ir::Ty::Unit,
            args: vec![ir::Ty::Unit].into(),
        };
        let membership = opaque_bound_diagnostic(
            Some(declaration),
            &ir::Predicate::Implements(application.clone()),
        );
        let output = opaque_bound_diagnostic(
            Some(declaration),
            &ir::Predicate::ProjectionEq {
                projection: ir::AssociatedTypeProjection {
                    trait_ref: application,
                    item: declaration,
                },
                ty: ir::Ty::Unit,
            },
        );
        for diagnostic in [&membership, &output] {
            assert!(diagnostic.starts_with(
                "E0277: hidden return type does not satisfy its declared opaque bounds;"
            ));
            assert!(diagnostic.contains("defining owner=Some(DeclId"));
            assert!(diagnostic.contains("trait=DeclId"));
            assert!(diagnostic.len() < 600);
            assert!(!diagnostic.contains("self_ty:"));
        }
        assert!(membership.contains("required membership;"));
        assert!(!membership.contains("required associated"));
        assert!(output.contains("required associated Output/equality item=DeclId"));
        assert!(!output.contains("required membership;"));
    }
}

fn hidden_contains_opaque(
    ty: &ir::Ty,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    let children: Box<dyn Iterator<Item = &ir::Ty> + '_> = match ty {
        ir::Ty::Opaque(_) => return Ok(true),
        ir::Ty::Ref { referent, .. } => Box::new(std::iter::once(referent.as_ref())),
        ir::Ty::RawPtr { pointee, .. } => Box::new(std::iter::once(pointee.as_ref())),
        ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
            Box::new(std::iter::once(element.as_ref()))
        }
        ir::Ty::Tuple(fields) => Box::new(fields.iter()),
        ir::Ty::Struct(value) => Box::new(value.def().type_args.iter()),
        ir::Ty::Enum(value) => Box::new(value.def().type_args.iter()),
        ir::Ty::Closure(value) => Box::new(value.types()),
        ir::Ty::Dyn(value) => Box::new(value.types()),
        ir::Ty::AssociatedType(value) => Box::new(value.trait_ref.args.iter()),
        _ => return Ok(false),
    };
    for child in children {
        if hidden_contains_opaque(child, budget, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Checker<'_> {
    pub(super) fn finalize_opaque_returns(&self) -> Result<(), String> {
        if let ir::Ty::Opaque(opaque) = &self.result {
            if self.declaration != Some(opaque.owner()) {
                return Err("contract: defining opaque result has a foreign callable owner".into());
            }
            if !self
                .opaque_hidden
                .iter()
                .any(|record| record.opaque() == opaque)
            {
                return Err("unsupported: recursive or diverging-only opaque return has no concrete hidden type".into());
            }
        }
        let budget = &self.signatures.plan()?.budget;
        for record in &self.opaque_hidden {
            if self.declaration != Some(record.opaque().owner()) {
                return Err("contract: hidden opaque record has a foreign defining owner".into());
            }
            if hidden_contains_opaque(record.hidden(), budget, 0)? {
                return Err("unsupported: nested opaque hidden types require a cycle-safe declaration protocol".into());
            }
        }
        Ok(())
    }

    pub(super) fn coerce_opaque_return(
        &mut self,
        value: ir::ExprId,
        opaque: &Arc<ir::OpaqueTy>,
    ) -> Result<ir::ExprId, String> {
        let source = self.exprs.get(value.0).ok_or("opaque operand is absent")?;
        let expected = ir::Ty::Opaque(opaque.clone());
        if source.ty == expected || source.ty == ir::Ty::Never {
            return Ok(value);
        }
        if self.declaration != Some(opaque.owner()) {
            return Err(
                "E0308: only an opaque type's defining callable may choose its hidden type".into(),
            );
        }
        let hidden = source.ty.clone();
        let completion = source.completion;
        let facts = self
            .signatures
            .facts
            .as_deref()
            .ok_or("opaque return checking requires canonical callable facts")?;
        let world = self.signatures.plan()?.trait_world()?;
        if let Some(previous) = self
            .opaque_hidden
            .iter()
            .find(|record| record.opaque().as_ref() == opaque.as_ref())
        {
            if !world
                .types_equal(
                    std::slice::from_ref(previous.hidden()),
                    std::slice::from_ref(&hidden),
                )
                .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?
            {
                return Err(
                    "E0308: opaque return paths have different hidden concrete types".into(),
                );
            }
        } else {
            for bound in opaque.bounds() {
                let application = bound.application(hidden.clone());
                let membership = ir::Predicate::Implements(application.clone());
                self.require_opaque_return_goal(&membership)?;
                for (item, ty) in bound.equalities() {
                    self.require_opaque_return_goal(&ir::Predicate::ProjectionEq {
                        projection: ir::AssociatedTypeProjection {
                            trait_ref: application.clone(),
                            item: *item,
                        },
                        ty: ty.clone(),
                    })?;
                }
            }
            self.charge_node(std::mem::size_of::<ir::OpaqueHiddenType>())?;
            self.opaque_hidden
                .push(ir::OpaqueHiddenType::new(opaque.clone(), hidden));
        }
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        let wrapped = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty: expected,
            completion,
            kind: ir::ExprKind::OpaqueWrap {
                opaque: opaque.clone(),
                value,
            },
        });
        Ok(wrapped)
    }

    fn require_opaque_return_goal(&self, goal: &ir::Predicate) -> Result<(), String> {
        let facts = self
            .signatures
            .facts
            .as_deref()
            .ok_or("opaque return checking requires canonical callable facts")?;
        match facts
            .service()
            .prove_goal(goal, facts)
            .map_err(|error| error.into_legacy_diagnostic(facts.budget()))?
        {
            tr::ProofOutcome::Proved(_) => Ok(()),
            tr::ProofOutcome::NotProven => Err(opaque_bound_diagnostic(self.declaration, goal)),
        }
    }
}
