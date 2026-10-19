//! Canonical dynamic source metadata. Written names disappear at this boundary.
use super::*;

impl PlanState {
    pub(super) fn source_dynamic_type(
        &mut self,
        registry: &DeclRegistry,
        path: &TraitPath,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        let principal = registry.resolve_trait_path(path)?;
        registry.check_dyn_compatible(principal, budget, 0)?;
        let source = registry.source_trait(principal)?;
        let written = match path {
            TraitPath::Source(path) => path.segments.last().map_or(0, |part| part.args.len()),
            _ => 0,
        };
        if source.self_defaults.iter().any(|index| *index >= written) {
            return Err("E0393: a trait object must explicitly specify generic arguments defaulting to Self".into());
        }
        let application =
            self.source_trait_application(registry, path, &ir::Ty::Unit, bindings, caller, budget)?;
        let mut equalities = Vec::new();
        for binding in application.equalities {
            budget.charge_work(1)?;
            if equalities
                .iter()
                .any(|(item, _)| *item == binding.projection.item)
            {
                return Err("E0719: associated type equality specified more than once".into());
            }
            equalities.push((binding.projection.item, binding.ty));
        }
        equalities.sort_by_key(|(item, _)| item.index);
        fn collect(
            registry: &DeclRegistry,
            principal: ir::DeclId,
            seen: &mut Vec<ir::DeclId>,
            required: &mut Vec<ir::DeclId>,
            budget: &ir::PlanBudget,
            depth: usize,
        ) -> Result<(), String> {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            if seen.contains(&principal) {
                return Ok(());
            }
            seen.push(principal);
            let index = registry
                .trait_ids
                .iter()
                .position(|id| *id == principal)
                .ok_or("dynamic principal has no source declaration")?;
            for item in &registry.trait_associated_types[index] {
                budget.charge_work(1)?;
                required.push(*item);
            }
            for parent in &registry.source_trait(principal)?.supertraits {
                collect(
                    registry,
                    registry.resolve_trait_path(parent)?,
                    seen,
                    required,
                    budget,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        let mut required = Vec::new();
        collect(
            registry,
            principal,
            &mut Vec::new(),
            &mut required,
            budget,
            0,
        )?;
        for item in required {
            if !equalities.iter().any(|(bound, _)| *bound == item) {
                return Err(format!(
                    "E0191: the value of associated type {} must be specified",
                    registry.declaration(item)?.name
                ));
            }
        }
        Ok(ir::Ty::Dyn(Arc::new(ir::DynTy {
            principal,
            args: application.trait_ref.args,
            associated_types: equalities.into(),
        })))
    }
}
