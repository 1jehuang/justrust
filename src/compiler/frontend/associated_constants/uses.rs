//! Exact original-site authorization for closed associated constant demands.
//! A logical impl witness alone never authorizes a source expression.
use super::inherent::ConstantReference;
use super::*;

#[derive(Clone, Debug)]
pub(in crate::compiler) struct AuthorizedAssociatedConstant {
    caller: ir::CallTarget,
    path: Arc<[u32]>,
    selected: tr::SelectedAssociatedConstant,
}
impl AuthorizedAssociatedConstant {
    pub(in crate::compiler) fn caller(&self) -> &ir::CallTarget {
        &self.caller
    }
    pub(in crate::compiler) fn path(&self) -> &[u32] {
        &self.path
    }
    pub(in crate::compiler) fn selected(&self) -> &tr::SelectedAssociatedConstant {
        &self.selected
    }
    pub(in crate::compiler) fn validate(
        &self,
        plan: &PlanCtx,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let replay = plan.authorize_associated_constant(&self.caller, &self.path, budget)?;
        if replay.selected.declaration() != self.selected.declaration()
            || replay.selected.member() != self.selected.member()
            || replay.selected.arguments() != self.selected.arguments()
            || replay.selected.ty() != self.selected.ty()
        {
            return Err("associated constant source selection changed".into());
        }
        let facts = plan.instance_facts(&self.caller)?;
        self.selected
            .validate_for(&facts)
            .map_err(|e| e.into_legacy_diagnostic(budget))
    }
}
impl PlanCtx {
    pub(in crate::compiler) fn authorize_associated_constant(
        &self,
        target: &ir::CallTarget,
        path: &[u32],
        budget: &ir::PlanBudget,
    ) -> Result<AuthorizedAssociatedConstant, String> {
        self.check_budget(budget)?;
        budget.check_depth(path.len())?;
        if let Some(boundary) = path.iter().position(|coordinate| *coordinate == u32::MAX) {
            let mut authorization =
                self.authorize_associated_constant(target, &path[..boundary], budget)?;
            let facts = self.instance_facts(target)?;
            let mut ancestors = Vec::new();
            sr::push(&mut ancestors, authorization.selected.clone(), budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            let mut remaining = &path[boundary + 1..];
            if remaining.is_empty() {
                return Err("associated constant dependency chain is empty".into());
            }
            while let Some((length, tail)) = remaining.split_first() {
                let length = *length as usize;
                let step = tail
                    .get(..length)
                    .ok_or("associated constant dependency node frame is truncated")?;
                if step.contains(&u32::MAX) {
                    return Err(
                        "associated constant initializer coordinate contains a chain marker".into(),
                    );
                }
                let selected =
                    self.select_initializer_member(&authorization.selected, step, &facts, budget)?;
                for ancestor in &ancestors {
                    budget.charge_work(1)?;
                    if ancestor.declaration() == selected.declaration()
                        && facts
                            .world()
                            .types_equal(ancestor.arguments(), selected.arguments())
                            .map_err(|e| e.into_legacy_diagnostic(budget))?
                    {
                        return Err(
                            "E0391: cycle detected when evaluating associated constant initializer"
                                .into(),
                        );
                    }
                }
                sr::push(&mut ancestors, selected.clone(), budget)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?;
                authorization.selected = selected;
                remaining = &tail[length..];
            }
            budget.retain_bytes(std::mem::size_of_val(path))?;
            authorization.path = Arc::from(path);
            return Ok(authorization);
        }
        let facts = self.instance_facts(target)?;
        let function = self.callable_body(&self.program, target.declaration)?;
        let inventory = SourceInventory::build(
            &self.program,
            function,
            sr::OriginalBodyId::new(budget).map_err(|e| e.into_legacy_diagnostic(budget))?,
            budget,
        )
        .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let original = inventory
            .associated_constants
            .iter()
            .find(|(node, _)| node.path.as_ref() == path)
            .ok_or("associated constant key does not identify an original member use")?;
        let source = &original.1;
        let env = self.callable_env(target.declaration)?;
        if target.type_args.len() != env.total_arity() {
            return Err("associated constant caller argument arity changed".into());
        }
        let mut bindings = generic_bindings(&self.registry, &env, budget)?;
        if let Some(self_ty) = self.callable_context(target.declaration)?.self_ty {
            bindings.insert(GenericBindingKey::SelfType, self_ty);
        }
        let mut context = self.clone();
        for value in bindings.values_mut() {
            budget.charge_work(1)?;
            *value = ir::substitute_ty(
                value,
                &env,
                &target.type_args,
                ir::FactsCtx::from_validated(&facts),
                &mut context,
                budget,
            )?;
        }
        let reference = self.associated_constant_application(
            target.declaration,
            source,
            &bindings,
            &facts,
            Some(&target.type_args),
            budget,
        )?;
        let selected = reference.value(&facts, budget)?;
        budget.retain_bytes(
            std::mem::size_of::<AuthorizedAssociatedConstant>()
                + std::mem::size_of_val(original.0.path.as_ref()),
        )?;
        Ok(AuthorizedAssociatedConstant {
            caller: target.clone(),
            path: original.0.path.clone(),
            selected,
        })
    }
    pub(in crate::compiler::frontend) fn associated_constant_operand(
        &self,
        declaration: ir::DeclId,
        source: &AssociatedConstUse,
        path: Arc<[u32]>,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<ir::AssociatedConstOperand, String> {
        self.check_budget(budget)?;
        if declaration != facts.declaration() || !Arc::ptr_eq(facts.world(), &self.trait_world()?) {
            return Err("associated constant symbolic operand changed caller authority".into());
        }
        let env = self.callable_env(declaration)?;
        let mut bindings = generic_bindings(&self.registry, &env, budget)?;
        if let Some(self_ty) = self.callable_context(declaration)?.self_ty {
            bindings.insert(GenericBindingKey::SelfType, self_ty);
        }
        let reference = self.associated_constant_application(
            declaration,
            source,
            &bindings,
            facts,
            None,
            budget,
        )?;
        let schema = reference.schema(facts, budget)?;
        ir::AssociatedConstOperand::new(declaration, path, schema, facts, budget)
    }
    fn associated_constant_from_bounds(
        &self,
        declaration: ir::DeclId,
        actuals: Option<&[ir::Ty]>,
        owner: &ir::Ty,
        name: AstIdent,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<(ir::DeclId, ir::TraitRef), String> {
        let mut selected = None;
        let universal =
            if self.registry.declaration(declaration)?.kind == ir::DeclKind::AssociatedConst {
                facts
                    .world()
                    .associated_constant_facts(declaration)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?
            } else {
                self.callable_facts(declaration)?
            };
        let mut context = self.clone();
        for predicate in universal.predicates().predicates() {
            budget.charge_work(1)?;
            let ir::Predicate::Implements(original) = predicate else {
                continue;
            };
            let application = if let Some(actuals) = actuals {
                let caller = ir::FactsCtx::from_validated(facts);
                let self_ty = ir::substitute_ty(
                    &original.self_ty,
                    universal.env(),
                    actuals,
                    caller,
                    &mut context,
                    budget,
                )?;
                let mut args = Vec::new();
                for ty in original.args.iter() {
                    let ty = ir::substitute_ty(
                        ty,
                        universal.env(),
                        actuals,
                        caller,
                        &mut context,
                        budget,
                    )?;
                    sr::push(&mut args, ty, budget)
                        .map_err(|e| e.into_legacy_diagnostic(budget))?;
                }
                ir::TraitRef {
                    declaration: original.declaration,
                    self_ty,
                    args: args.into(),
                }
            } else {
                original.clone()
            };
            if !facts
                .world()
                .types_equal(
                    std::slice::from_ref(&application.self_ty),
                    std::slice::from_ref(owner),
                )
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                continue;
            }
            let info = self.registry.declaration(application.declaration)?;
            if info.kind != ir::DeclKind::Trait {
                return Err("associated constant bound selected non-trait owner".into());
            }
            let Some(declaration) = self.program.traits.get(info.source_index) else {
                continue;
            };
            for (index, candidate) in declaration.associated_constants.iter().enumerate() {
                budget.charge_work(1)?;
                if !name_scope::ident_eq(
                    self.registry.names.as_ref(),
                    candidate.name,
                    name,
                    LookupClass::Member,
                )? {
                    continue;
                }
                let value = (
                    self.registry.associated_constants.traits[info.source_index][index],
                    application.clone(),
                );
                if let Some(previous) = &selected {
                    if previous != &value {
                        return Err(
                            "E0034: multiple applicable associated constants in caller bounds"
                                .into(),
                        );
                    }
                } else {
                    selected = Some(value);
                }
            }
        }
        selected.ok_or_else(|| "unsupported: implicit associated constant lookup requires a matching retained caller trait bound".into())
    }
    pub(super) fn associated_constant_application(
        &self,
        declaration: ir::DeclId,
        source: &AssociatedConstUse,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        facts: &tr::ValidatedFacts,
        actuals: Option<&[ir::Ty]>,
        budget: &ir::PlanBudget,
    ) -> Result<ConstantReference, String> {
        let env = self.registry.declaration(declaration)?.env.clone();
        let caller = ir::FactsCtx::from_validated(facts);
        let owner = self.source_ty(&source.owner, bindings, caller, budget)?;
        let Some(path) = source.trait_path.as_ref() else {
            if let Some(reference) =
                self.inherent_constant_reference(&owner, source.name, facts, budget)?
            {
                return Ok(reference);
            }
            let (member, application) = self.associated_constant_from_bounds(
                declaration,
                actuals,
                &owner,
                source.name,
                facts,
                budget,
            )?;
            return Ok(ConstantReference::Trait {
                member,
                application,
            });
        };
        // Structural interning must not hold PlanState while validating closed
        // trait membership, since nominal validation can re-enter the plan.
        let application = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "generic plan context poisoned")?;
            let previous = state.source_context;
            state.source_context = Some(self.registry.source_type_context(declaration)?);
            let result = state.source_trait_application(
                &self.registry,
                path,
                &owner,
                bindings,
                ir::FactsCtx::Universal(&env),
                budget,
            );
            state.source_context = previous;
            result?.reject_equalities()?
        };
        let trait_info = self.registry.declaration(application.declaration)?;
        let declaration = self
            .program
            .traits
            .get(trait_info.source_index)
            .ok_or("associated constant trait has no source declaration")?;
        let mut member = None;
        for (index, candidate) in declaration.associated_constants.iter().enumerate() {
            budget.charge_work(1)?;
            if name_scope::ident_eq(
                self.registry.names.as_ref(),
                candidate.name,
                source.name,
                LookupClass::Member,
            )? {
                member =
                    Some(self.registry.associated_constants.traits[trait_info.source_index][index]);
                break;
            }
        }
        let member =
            member.ok_or("E0576: associated constant is not a member of the selected trait")?;
        Ok(ConstantReference::Trait {
            member,
            application,
        })
    }
}
