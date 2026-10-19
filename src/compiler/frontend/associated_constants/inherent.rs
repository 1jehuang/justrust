//! Indexed original inherent owner selection, followed by independent world proof.
use super::*;

pub(super) enum ConstantReference {
    Trait {
        member: ir::DeclId,
        application: ir::TraitRef,
    },
    Inherent {
        member: ir::DeclId,
        owner: ir::Ty,
        arguments: Arc<[ir::Ty]>,
    },
}
impl ConstantReference {
    pub(super) fn value(
        &self,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<tr::SelectedAssociatedConstant, String> {
        if facts.phase() != tr::FactsPhase::Closed {
            return Err("associated constant value requires closed caller".into());
        }
        let selected = match self {
            Self::Trait {
                member,
                application,
            } => tr::SelectedAssociatedConstant::Trait(
                facts
                    .world()
                    .select_associated_constant(*member, application, facts)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?,
            ),
            Self::Inherent {
                member,
                owner,
                arguments,
            } => tr::SelectedAssociatedConstant::Inherent(
                facts
                    .world()
                    .inherent_constant_type(*member, owner, arguments, facts)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?,
            ),
        };
        selected
            .validate_for(facts)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        Ok(selected)
    }
    pub(super) fn schema(
        &self,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<tr::ConstantMemberType, String> {
        match self {
            Self::Trait {
                member,
                application,
            } => facts
                .world()
                .associated_constant_type(*member, application, facts)
                .map(tr::ConstantMemberType::Trait),
            Self::Inherent {
                member,
                owner,
                arguments,
            } => facts
                .world()
                .inherent_constant_type(*member, owner, arguments, facts)
                .map(tr::ConstantMemberType::Inherent),
        }
        .map_err(|e| e.into_legacy_diagnostic(budget))
    }
}
impl PlanCtx {
    pub(super) fn inherent_constant_reference(
        &self,
        owner: &ir::Ty,
        name: AstIdent,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Option<ConstantReference>, String> {
        self.check_budget(budget)?;
        let key = self
            .registry
            .names
            .key(name, LookupClass::Member)
            .map_err(native_adapter::name_diagnostic)?;
        let Some(candidates) = self.registry.associated_constants.inherent.get(&key) else {
            return Ok(None);
        };
        let mut selected = None;
        for member in candidates {
            budget.charge_work(1)?;
            let info = self.registry.declaration(*member)?;
            let header = self.associated_constant_header(*member, budget)?;
            let pattern = header
                .owner_ty
                .as_ref()
                .ok_or("inherent member has no retained owner pattern")?;
            let mut bindings = HashMap::new();
            let env = self.param_env(header.owner)?;
            if !match_impl_type(pattern, owner, &env, &mut bindings, budget, 0)? {
                continue;
            }
            let mut arguments = Vec::new();
            for parameter in header.env.ordered_params() {
                let value = bindings.get(&parameter).ok_or(
                    "E0283: inherent constant impl arguments are not determined by its owner",
                )?;
                sr::push(&mut arguments, value.clone(), budget)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?;
            }
            let caller = self.registry.declaration(facts.declaration())?;
            if !info
                .visibility
                .accessible_from(&self.program.modules, caller.path.module)
                .map_err(module_diagnostic)?
            {
                return Err("E0624: private associated constant".into());
            }
            // An exact structural candidate must prove its own predicates. Do not
            // silently discard a failed bound or trust the index as membership.
            facts
                .world()
                .inherent_constant_type(*member, owner, &arguments, facts)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            if selected.is_some() {
                return Err("E0034: multiple applicable inherent associated constants".into());
            }
            selected = Some(ConstantReference::Inherent {
                member: *member,
                owner: owner.clone(),
                arguments: arguments.into(),
            });
        }
        Ok(selected)
    }
}
