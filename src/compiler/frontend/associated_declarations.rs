//! Freeze applied associated declaration bounds from their exact retained source.
use super::*;

pub(super) fn seal(
    registry: &mut Arc<DeclRegistry>,
    state: &mut PlanState,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let mut rows = Vec::new();
    for source in registry.program.associated_bound_sources.iter() {
        registry
            .program
            .validate_associated_bound_source(source, budget)?;
        let owner = registry.trait_ids[source.trait_index];
        let associated = registry.trait_associated_types[source.trait_index][source.member_index];
        let info = registry.declaration(owner)?;
        let mut bindings = generic_bindings(registry, &info.env, budget)?;
        let mut arguments = symbolic_args(&info.param_env, budget)?;
        if arguments.is_empty() {
            return Err("associated bound owner has no Self parameter".into());
        }
        let self_ty = arguments.remove(0);
        bindings.insert(GenericBindingKey::SelfType, self_ty.clone());
        let subject = ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: ir::TraitRef {
                declaration: owner,
                self_ty,
                args: arguments.into(),
            },
            item: associated,
        }));
        state.source_context = Some(registry.source_type_context(associated)?);
        let mut predicates = Vec::new();
        for (_, path) in source.bounds.iter() {
            let application = state.source_trait_application(
                registry,
                path,
                &subject,
                &bindings,
                ir::FactsCtx::Universal(&info.env),
                budget,
            )?;
            budget.charge_work(application.equalities.len().saturating_add(1))?;
            budget.retain_bytes(
                (application.equalities.len().saturating_add(1))
                    .saturating_mul(std::mem::size_of::<ir::Predicate>()),
            )?;
            predicates
                .try_reserve_exact(application.equalities.len().saturating_add(1))
                .map_err(|_| "associated bound predicate allocation failed")?;
            predicates.push(ir::Predicate::Implements(application.trait_ref));
            for equality in application.equalities {
                predicates.push(ir::Predicate::ProjectionEq {
                    projection: equality.projection,
                    ty: equality.ty,
                });
            }
        }
        budget.retain_bytes(std::mem::size_of::<(ir::DeclId, Arc<[ir::Predicate]>)>())?;
        rows.try_reserve_exact(1)
            .map_err(|_| "associated bound row allocation failed")?;
        rows.push((associated, Arc::<[ir::Predicate]>::from(predicates)));
    }
    state.source_context = None;
    let registry =
        Arc::get_mut(registry).ok_or("registry shared before associated bounds were sealed")?;
    for (associated, predicates) in rows {
        registry
            .associated_type_bounds
            .get_mut(&associated)
            .ok_or("missing associated bound source identity")?
            .predicates = predicates;
    }
    Ok(())
}

pub(super) fn contains_projection(
    registry: &DeclRegistry,
    associated: ir::DeclId,
    projection: &SourceProjection,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let info = registry.declaration(associated)?;
    let owner = registry.declaration(info.owner.ok_or("associated source has no owner")?)?;
    let index = registry.trait_associated_types[owner.source_index]
        .iter()
        .position(|id| *id == associated)
        .ok_or("associated source identity is not its owner's member")?;
    let source = &registry.program.traits[owner.source_index].associated_types[index];
    let mut found = false;
    for bound in &source.bounds {
        if let TraitPath::Source(path) = bound {
            for segment in &path.segments {
                for ty in segment
                    .args
                    .iter()
                    .chain(segment.associated_types.iter().map(|row| &row.value))
                {
                    found |=
                        retained_type_projection(registry.names.as_ref(), ty, projection, budget)?;
                }
            }
        }
    }
    Ok(found)
}
