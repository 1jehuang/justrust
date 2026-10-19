//! Associated members inherited from supertraits retain the declaration and
//! ordered arguments of the trait that actually owns the member.
use super::*;

impl PlanState {
    pub(super) fn inherited_associated_owner(
        &mut self,
        registry: &DeclRegistry,
        goal: ir::TraitRef,
        item: AstIdent,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
        active: &mut Vec<ir::DeclId>,
    ) -> Result<Option<(ir::TraitRef, ir::DeclId)>, String> {
        budget.check_depth(active.len())?;
        budget.charge_work(1)?;
        if active.contains(&goal.declaration) {
            return Err("cyclic supertrait associated lookup".into());
        }
        if let Ok(id) = registry.associated_type_item(goal.declaration, item, budget) {
            return Ok(Some((goal, id)));
        }
        let info = registry.declaration(goal.declaration)?;
        if info.kind != ir::DeclKind::Trait {
            return Ok(None);
        }
        let declaration = registry.source_trait(goal.declaration)?;
        let parents = declaration.supertraits.clone();
        let mut bindings = HashMap::new();
        bindings.insert(GenericBindingKey::SelfType, goal.self_ty.clone());
        // Trait declaration environments contain implicit Self first.
        for (parameter, actual) in declaration.type_params.iter().zip(goal.args.iter()) {
            bindings.insert(registry.parameter_key(*parameter)?, actual.clone());
        }
        active.push(goal.declaration);
        let mut found = None;
        for parent in parents {
            if matches!(parent, TraitPath::Builtin(_)) {
                continue;
            }
            let parent_goal = self
                .source_trait_application(
                    registry,
                    &parent,
                    &goal.self_ty,
                    &bindings,
                    caller,
                    budget,
                )?
                .reject_equalities()?;
            if let Some(candidate) = self.inherited_associated_owner(
                registry,
                parent_goal,
                item,
                caller,
                budget,
                active,
            )? {
                if found.is_some() {
                    return Err("ambiguous associated type in supertraits".into());
                }
                found = Some(candidate);
            }
        }
        active.pop();
        Ok(found)
    }
}
