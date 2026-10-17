//! Canonical associated and trait-call selection.
use super::*;

impl PlanCtx {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lookup_associated(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        method_args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        module: &ModuleId,
        dot: bool,
        filter: Option<ReceiverKind>,
        traits_only: bool,
        budget: &ir::PlanBudget,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.check_budget(budget)?;
        self.program
            .modules
            .check(*module)
            .map_err(module_diagnostic)?;
        self.program
            .names
            .key(*name, LookupClass::Member)
            .map_err(native_adapter::name_diagnostic)?;
        let nominal = nominal_parts(owner).map(|(id, _)| id);
        let mut selected = None;
        for id in &self.registry.function_ids {
            budget.charge_work(1)?;
            let info = self.declaration(*id)?;
            let Some(associated) = &info.path.associated else {
                continue;
            };
            budget.charge_work(1)?;
            if traits_only
                || filter
                    .is_some_and(|mode| receiver_mode(mode) != receiver_mode(associated.receiver))
                || associated.trait_decl.is_some()
                || Some(associated.nominal) != nominal
                || !self.registry.source_member_equal(info.path.item, *name)?
            {
                continue;
            }
            let impl_env = self.param_env(associated.impl_decl)?;
            let _scratch = budget.reserve_bytes(32 * 512 + 128)?;
            let mut bindings = HashMap::new();
            if !match_impl_type(
                &associated.self_pattern,
                owner,
                &impl_env,
                &mut bindings,
                budget,
                0,
            )? {
                continue;
            }
            if selected.is_some() {
                return Err("ambiguous inherent associated item".into());
            }
            if dot && associated.receiver == ReceiverKind::None {
                return Err("associated function without self is not dot-callable".into());
            }
            if info.param_env.arity() != method_args.len() {
                return Err("unsupported omitted or partial method type arguments; use all explicit method arguments".into());
            }
            if !info
                .visibility
                .accessible_from(&self.program.modules, *module)
                .map_err(module_diagnostic)?
            {
                return Err("private associated item".into());
            }
            budget.retain_bytes(32 * std::mem::size_of::<ir::Ty>() + 128)?;
            let mut args = Vec::with_capacity(impl_env.arity() + method_args.len());
            for index in 0..impl_env.arity() {
                args.push(
                    bindings
                        .get(&ir::ParamId {
                            owner: impl_env.binder(),
                            index: index as u16,
                        })
                        .cloned()
                        .ok_or("unconstrained impl parameter")?,
                );
            }
            args.extend_from_slice(method_args);
            let target = ir::CallTarget {
                declaration: *id,
                type_args: args.into(),
            };
            let signature = self.instantiate_signature(&target, caller, budget)?;
            let ir::FactsCtx::Validated(facts) = caller else {
                return Err("associated call requires validated facts".into());
            };
            let proof = facts
                .service()
                .discharge_direct(&target, facts)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
            if !facts
                .world()
                .signatures_equal(proof.signature(), &signature)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
            {
                return Err("canonical associated signature mismatch".into());
            }
            selected = Some((ir::Callee::Direct(target), signature, associated.receiver));
        }
        if let Some(selected) = selected {
            return Ok(Some(selected));
        }
        if !traits_only {
            return Ok(None);
        }
        let ir::FactsCtx::Validated(facts) = caller else {
            return Err("trait selection requires validated facts".into());
        };
        let mut selected = None;
        for trait_id in &self.registry.trait_ids {
            budget.charge_work(1)?;
            let info = self.declaration(*trait_id)?;
            self.program
                .modules
                .check(*module)
                .map_err(module_diagnostic)?;
            let mut visible =
                info.path.module == *module || self.catalog.declaration(*trait_id).is_some();
            for (import_module, path) in &self.registry.trait_imports {
                budget.charge_work(1)?;
                if import_module == module && self.resolve_trait_path(path)? == *trait_id {
                    visible = true;
                }
            }
            for ir::Predicate::Implements(bound) in facts.predicates().predicates() {
                budget.charge_work(1)?;
                if bound.declaration == *trait_id
                    && metered_type_equal(&bound.self_ty, owner, budget, 0)?
                {
                    visible = true;
                }
            }
            if !visible {
                continue;
            }
            if let Some(call) = self.select_trait_filtered(
                owner,
                *trait_id,
                name,
                method_args,
                caller,
                dot,
                filter,
                budget,
            )? {
                if selected.is_some() {
                    return Err("ambiguous in-scope trait item".into());
                }
                selected = Some(call);
            }
        }
        Ok(selected)
    }
}
impl PlanCtx {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_trait(
        &self,
        owner: &ir::Ty,
        id: ir::DeclId,
        name: &AstIdent,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        dot: bool,
        budget: &ir::PlanBudget,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.select_trait_filtered(owner, id, name, args, caller, dot, None, budget)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_trait_filtered(
        &self,
        owner: &ir::Ty,
        trait_id: ir::DeclId,
        name: &AstIdent,
        method_args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        dot: bool,
        filter: Option<ReceiverKind>,
        budget: &ir::PlanBudget,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.check_budget(budget)?;
        if !method_args.is_empty() {
            return Err("unsupported: trait method type arguments".into());
        }
        let ir::FactsCtx::Validated(facts) = caller else {
            return Err("trait call requires validated facts".into());
        };
        let mut item = None;
        for (ti, id) in self.registry.trait_ids.iter().enumerate() {
            budget.charge_work(1)?;
            if *id == trait_id {
                for iid in &self.registry.trait_items[ti] {
                    let info = self.declaration(*iid)?;
                    budget.charge_work(1)?;
                    if self.registry.source_member_equal(info.path.item, *name)? {
                        item = Some(*iid);
                        break;
                    }
                }
                break;
            }
        }
        let Some(item) = item else {
            return Ok(None);
        };
        let item_receiver = self.declaration(item)?.receiver;
        if filter.is_some_and(|mode| receiver_mode(mode) != receiver_mode(item_receiver)) {
            return Ok(None);
        }
        let call = ir::TraitCall {
            trait_ref: ir::TraitRef {
                declaration: trait_id,
                self_ty: owner.clone(),
                args: Arc::from([]),
            },
            item,
            method_args: Arc::from([]),
        };
        match facts
            .service()
            .prove_goal(&ir::Predicate::Implements(call.trait_ref.clone()), facts)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
        {
            tr::ProofOutcome::NotProven => return Ok(None),
            tr::ProofOutcome::Proved(_) => {}
        }
        let signature = facts
            .service()
            .instantiate_trait_signature(&call, facts)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
        let receiver = self.declaration(item)?.receiver;
        if dot && receiver == ReceiverKind::None {
            return Err("trait associated function without self is not dot-callable".into());
        }
        Ok(Some((ir::Callee::TraitItem(call), signature, receiver)))
    }
}
