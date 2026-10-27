//! Canonical associated and trait-call selection.

use super::*;

fn selection_token_error(error: super::super::token_session::TokenError) -> String {
    super::super::source::SourceError::from_token(error).to_string()
}
pub(super) fn trait_application_ambiguity() -> String {
    super::super::source::SourceError {
        kind: super::super::source::SourceErrorKind::Unsupported,
        primary: None,
        message: "multiple trait applications require explicit UFCS; trait argument inference is unsupported".into(),
        _diagnostic_lease: None,
        location: None,
    }.to_string()
}
fn match_trait_self<'actual>(
    pattern: &ir::Ty,
    actual: &'actual ir::Ty,
    env: &ir::GenericEnv,
    bindings: &mut super::super::token_session::BudgetVec<(ir::ParamId, &'actual ir::Ty)>,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    if let ir::Ty::Param(parameter) = pattern {
        budget.charge_work(env.scopes().len())?;
        if env.contains(*parameter) {
            for (bound, value) in bindings.as_slice() {
                budget.charge_work(1)?;
                if bound == parameter {
                    return metered_type_equal(value, actual, budget, depth + 1);
                }
            }
            bindings
                .push((*parameter, actual))
                .map_err(selection_token_error)?;
            return Ok(true);
        }
    }
    match (pattern, actual) {
        (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b.iter()) {
                if !match_trait_self(a, b, env, bindings, budget, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (ir::Ty::Array { element: a, len: n }, ir::Ty::Array { element: b, len: m }) if n == m => {
            match_trait_self(a, b, env, bindings, budget, depth + 1)
        }
        (
            ir::Ty::RawPtr {
                kind: a,
                pointee: x,
            },
            ir::Ty::RawPtr {
                kind: b,
                pointee: y,
            },
        ) if a == b => match_trait_self(x, y, env, bindings, budget, depth + 1),
        (ir::Ty::Slice { element: x }, ir::Ty::Slice { element: y }) => {
            match_trait_self(x, y, env, bindings, budget, depth + 1)
        }
        (
            ir::Ty::Ref {
                kind: a,
                referent: x,
            },
            ir::Ty::Ref {
                kind: b,
                referent: y,
            },
        ) if a == b => match_trait_self(x, y, env, bindings, budget, depth + 1),
        _ => {
            if let (Some((a, aa)), Some((b, bb))) = (nominal_parts(pattern), nominal_parts(actual))
            {
                if a != b || aa.len() != bb.len() {
                    return Ok(false);
                }
                for (a, b) in aa.iter().zip(bb) {
                    if !match_trait_self(a, b, env, bindings, budget, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            } else {
                metered_type_equal(pattern, actual, budget, depth + 1)
            }
        }
    }
}
impl PlanCtx {
    pub(super) fn trait_item(
        &self,
        trait_id: ir::DeclId,
        name: &AstIdent,
        budget: &ir::PlanBudget,
    ) -> Result<Option<ir::DeclId>, String> {
        let key = self
            .program
            .names
            .key(*name, LookupClass::Member)
            .map_err(native_adapter::name_diagnostic)?;
        for (index, id) in self.registry.trait_ids.iter().enumerate() {
            budget.charge_work(1)?;
            if *id != trait_id {
                continue;
            }
            return self.trait_item_at(index, name, key, budget);
        }
        Ok(None)
    }

    fn trait_item_at(
        &self,
        index: usize,
        name: &AstIdent,
        key: NameKey,
        budget: &ir::PlanBudget,
    ) -> Result<Option<ir::DeclId>, String> {
        for item in &self.registry.trait_items[index] {
            budget.charge_work(1)?;
            let matches = match self.declaration(*item)?.path.item {
                DeclItemName::Source(source) => {
                    self.program
                        .names
                        .key(source, LookupClass::Member)
                        .map_err(native_adapter::name_diagnostic)?
                        == key
                }
                other => self.registry.source_member_equal(other, *name)?,
            };
            if matches {
                return Ok(Some(*item));
            }
        }
        Ok(None)
    }
    fn selection_arguments(
        &self,
        count: usize,
        budget: &ir::PlanBudget,
    ) -> Result<Vec<ir::Ty>, String> {
        if count > budget.limits().params {
            return Err("generic parameter limit exceeded".into());
        }
        let bytes = count
            .checked_mul(std::mem::size_of::<ir::Ty>())
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(32))
            .ok_or("trait argument allocation overflow")?;
        budget.retain_bytes(bytes)?;
        let mut values = Vec::new();
        values.try_reserve_exact(count).map_err(|_| {
            selection_token_error(super::super::token_session::TokenError::Budget(
                self.program.names.budget().poison_allocation(),
            ))
        })?;
        if values.capacity() > count {
            budget.retain_bytes(
                (values.capacity() - count)
                    .checked_mul(std::mem::size_of::<ir::Ty>())
                    .ok_or("trait argument capacity overflow")?,
            )?;
        }
        Ok(values)
    }
    fn same_trait_application(
        &self,
        a: &ir::TraitRef,
        b: &ir::TraitRef,
        budget: &ir::PlanBudget,
    ) -> Result<bool, String> {
        budget.charge_work(1)?;
        let mut equal = a.declaration == b.declaration && a.args.len() == b.args.len();
        equal &= metered_type_equal(&a.self_ty, &b.self_ty, budget, 0)?;
        for (a, b) in a.args.iter().zip(b.args.iter()) {
            equal &= metered_type_equal(a, b, budget, 0)?;
        }
        Ok(equal)
    }
    fn push_trait_application(
        &self,
        applications: &mut super::super::token_session::BudgetVec<ir::TraitRef>,
        application: ir::TraitRef,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        for existing in applications.as_slice() {
            if self.same_trait_application(existing, &application, budget)? {
                return Ok(());
            }
        }
        applications
            .push(application)
            .map_err(selection_token_error)
    }
    fn trait_applications(
        &self,
        owner: &ir::Ty,
        trait_id: ir::DeclId,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<super::super::token_session::BudgetVec<ir::TraitRef>, String> {
        self.trait_applications_inner(owner, trait_id, facts, budget, None)
    }

    pub(super) fn inferred_trait_applications(
        &self,
        owner: &ir::Ty,
        trait_id: ir::DeclId,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<(super::super::token_session::BudgetVec<ir::TraitRef>, bool), String> {
        let mut deferred = false;
        let applications =
            self.trait_applications_inner(owner, trait_id, facts, budget, Some(&mut deferred))?;
        Ok((applications, deferred))
    }

    fn trait_applications_inner(
        &self,
        owner: &ir::Ty,
        trait_id: ir::DeclId,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
        mut deferred: Option<&mut bool>,
    ) -> Result<super::super::token_session::BudgetVec<ir::TraitRef>, String> {
        use super::super::{expansion_budget::Storage, token_session::BudgetVec};
        let mut applications = BudgetVec::new(self.program.names.budget(), Storage::Temporary)
            .map_err(selection_token_error)?;
        budget.charge_work(1)?;
        // A reference to an opaque value does not expose its hidden callable
        // implementation. Reject impossible callable blanket candidates before
        // enumerating their unconstrained Args, without changing receiver order.
        if facts.predicates().predicates().is_empty()
            && self
                .trait_world()?
                .closure_trait_kind(trait_id)
                .map_err(|error| error.into_legacy_diagnostic(budget))?
                .is_some()
        {
            let opaque = match owner {
                ir::Ty::Opaque(opaque) => Some(opaque),
                ir::Ty::Ref { referent, .. } => match referent.as_ref() {
                    ir::Ty::Opaque(opaque) => Some(opaque),
                    _ => None,
                },
                _ => None,
            };
            if let Some(opaque) = opaque {
                ir::FactsCtx::Validated(facts).validate_ty_with_budget(owner, budget)?;
                let mut excluded = true;
                for bound in opaque.bounds() {
                    budget.charge_work(1)?;
                    let declaration = self.declaration(bound.declaration())?;
                    if self
                        .trait_world()?
                        .closure_trait_kind(bound.declaration())
                        .map_err(|error| error.into_legacy_diagnostic(budget))?
                        .is_some()
                        || declaration.kind != ir::DeclKind::Trait
                        || declaration.source_index == usize::MAX
                        || !self
                            .registry
                            .source_trait(bound.declaration())?
                            .supertraits
                            .is_empty()
                    {
                        excluded = false;
                        break;
                    }
                }
                if excluded {
                    return Ok(applications);
                }
            }
        }
        if let ir::Ty::Dyn(object) = owner {
            for application in facts
                .dynamic_applications(object)
                .map_err(|error| error.into_legacy_diagnostic(budget))?
            {
                if application.declaration == trait_id {
                    self.push_trait_application(&mut applications, application, budget)?;
                }
            }
            return Ok(applications);
        }
        if self.param_env(trait_id)?.arity() == 1 {
            self.push_trait_application(
                &mut applications,
                ir::TraitRef {
                    declaration: trait_id,
                    self_ty: owner.clone(),
                    args: Arc::from([]),
                },
                budget,
            )?;
        }
        for predicate in facts.predicates().predicates() {
            budget.charge_work(1)?;
            // Equality constrains an exact application, but never establishes
            // membership or introduces an application for method discovery.
            let ir::Predicate::Implements(bound) = predicate else {
                continue;
            };
            if bound.declaration == trait_id
                && metered_type_equal(&bound.self_ty, owner, budget, 0)?
            {
                self.push_trait_application(&mut applications, bound.clone(), budget)?;
            }
        }
        // With no ordinary trait arguments the exact application is already
        // known. Membership, including equality-constrained impl outputs, is
        // proved by the canonical world rather than guessed from impl heads.
        if self.param_env(trait_id)?.arity() == 1 {
            return Ok(applications);
        }
        // Like rustc, a where-clause candidate on a type parameter shadows
        // impl candidates (`K: Borrow<Q>` wins over blanket `Borrow<K>`).
        if matches!(owner, ir::Ty::Param(_)) && !applications.as_slice().is_empty() {
            return Ok(applications);
        }
        'implementations: for implementation in facts.world().impl_headers() {
            budget.charge_work(1)?;
            if implementation.trait_ref.declaration != trait_id {
                continue;
            }
            let mut bindings = BudgetVec::new(self.program.names.budget(), Storage::Temporary)
                .map_err(selection_token_error)?;
            if !match_trait_self(
                &implementation.trait_ref.self_ty,
                owner,
                &implementation.schema_env,
                &mut bindings,
                budget,
                0,
            )? {
                continue;
            }
            let mut substitution =
                self.selection_arguments(implementation.schema_env.total_arity(), budget)?;
            for parameter in implementation.schema_env.ordered_params() {
                let mut value = None;
                for (bound, actual) in bindings.as_slice() {
                    budget.charge_work(1)?;
                    if *bound == parameter {
                        value = Some((*actual).clone());
                        break;
                    }
                }
                let Some(value) = value else {
                    // A parameter not fixed by the self type (`U` in
                    // `impl<T, U: From<T>> Into<U> for T`) is enumerated from
                    // the impls satisfying its own where-clause. Each match is
                    // an exact application; final selection proves it again.
                    if let Some(expanded) = self.expand_unbound_parameter(
                        implementation,
                        parameter,
                        &bindings,
                        facts,
                        budget,
                    )? {
                        for application in expanded {
                            self.push_trait_application(&mut applications, application, budget)?;
                        }
                        continue 'implementations;
                    }
                    if let Some(deferred) = deferred.as_deref_mut() {
                        *deferred = true;
                        continue 'implementations;
                    }
                    return Err(trait_application_ambiguity());
                };
                substitution.push(value);
            }
            let mut args = self.selection_arguments(implementation.trait_ref.args.len(), budget)?;
            let mut cx = self.clone();
            for argument in implementation.trait_ref.args.iter() {
                budget.charge_work(1)?;
                args.push(ir::substitute_ty(
                    argument,
                    &implementation.schema_env,
                    &substitution,
                    ir::FactsCtx::Validated(facts),
                    &mut cx,
                    budget,
                )?);
            }
            self.push_trait_application(
                &mut applications,
                ir::TraitRef {
                    declaration: trait_id,
                    self_ty: owner.clone(),
                    args: args.into(),
                },
                budget,
            )?;
        }
        Ok(applications)
    }

    /// Candidate applications for a blanket impl whose trait argument is an
    /// impl parameter `U` constrained only by a where-clause `U: Bound<Args>`
    /// with `Args` fixed by the self match. Each concrete (parameter-free)
    /// `impl Bound<Args> for X` yields the application with `U = X`. Returns
    /// `None` when the shape does not apply, leaving the ordinary boundary.
    fn expand_unbound_parameter(
        &self,
        implementation: &tr::ImplHeader,
        parameter: ir::ParamId,
        bindings: &super::super::token_session::BudgetVec<(ir::ParamId, &ir::Ty)>,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Option<Vec<ir::TraitRef>>, String> {
        // Only the exact `Trait<U> for T` shape: U is the sole trait argument
        // and every other impl parameter is fixed by the self type.
        if implementation.trait_ref.args.len() != 1
            || !matches!(&implementation.trait_ref.args[0], ir::Ty::Param(p) if *p == parameter)
            || implementation.schema_env.total_arity() != bindings.as_slice().len() + 1
        {
            return Ok(None);
        }
        let bound_value = |ty: &ir::Ty| -> Option<ir::Ty> {
            match ty {
                ir::Ty::Param(p) => bindings
                    .as_slice()
                    .iter()
                    .find(|(b, _)| b == p)
                    .map(|(_, v)| (*v).clone()),
                ty if !mentions_param(ty) => Some(ty.clone()),
                _ => None,
            }
        };
        let mut found: Option<Vec<ir::TraitRef>> = None;
        for predicate in implementation.predicates.iter() {
            budget.charge_work(1)?;
            let ir::Predicate::Implements(bound) = predicate else {
                continue;
            };
            if !matches!(&bound.self_ty, ir::Ty::Param(p) if *p == parameter)
                || bound.declaration == self.catalog.id(tr::CatalogItem::Sized)
            {
                continue;
            }
            let Some(args) = bound
                .args
                .iter()
                .map(bound_value)
                .collect::<Option<Vec<_>>>()
            else {
                return Ok(None);
            };
            if found.is_some() {
                // Several where-clauses on U: not this simple shape.
                return Ok(None);
            }
            let mut matches = Vec::new();
            let mut complete = true;
            for candidate in facts.world().impl_headers() {
                budget.charge_work(1)?;
                if candidate.trait_ref.declaration != bound.declaration
                    || candidate.trait_ref.args.len() != args.len()
                {
                    continue;
                }
                if mentions_param(&candidate.trait_ref.self_ty) {
                    complete = false;
                    continue;
                }
                let mut equal = true;
                let mut candidate_bindings = super::super::token_session::BudgetVec::new(
                    self.program.names.budget(),
                    super::super::expansion_budget::Storage::Temporary,
                )
                .map_err(selection_token_error)?;
                for (a, b) in candidate.trait_ref.args.iter().zip(&args) {
                    // A concrete self type makes a structural argument match
                    // an exact application even for generic impl arguments.
                    if !match_trait_self(
                        a,
                        b,
                        &candidate.schema_env,
                        &mut candidate_bindings,
                        budget,
                        0,
                    )? {
                        equal = false;
                        break;
                    }
                }
                if equal {
                    matches.push(ir::TraitRef {
                        declaration: implementation.trait_ref.declaration,
                        self_ty: bound_value(&implementation.trait_ref.self_ty)
                            .ok_or("unbound blanket impl self type")?,
                        args: Arc::from([candidate.trait_ref.self_ty.clone()]),
                    });
                }
            }
            if !complete && matches.is_empty() {
                return Ok(None);
            }
            found = Some(matches);
        }
        // A complete empty enumeration proves no application exists here.
        Ok(found)
    }
}

impl PlanCtx {
    #[allow(clippy::too_many_arguments)]
    /// Diagnostic-only: a trait implementation item named `name` whose impl
    /// self type matches `owner` structurally, ignoring its where-clauses.
    /// Used after ordinary selection failed, to report the unmet bound
    /// (E0599) rather than an inference boundary. Never selects a callee.
    pub(super) fn method_exists_with_unmet_bounds(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        budget: &ir::PlanBudget,
    ) -> Result<bool, String> {
        self.check_budget(budget)?;
        let Some((nominal, _)) = nominal_parts(owner) else {
            return Ok(false);
        };
        for id in &self.registry.function_ids {
            budget.charge_work(1)?;
            let info = self.declaration(*id)?;
            let Some(associated) = &info.path.associated else {
                continue;
            };
            if associated.trait_decl.is_none()
                || associated.nominal != nominal
                || associated.receiver == ReceiverKind::None
                || !self.registry.source_member_equal(info.path.item, *name)?
            {
                continue;
            }
            let impl_env = self.param_env(associated.impl_decl)?;
            // Only a generic impl can carry an unmet bound. A concrete impl
            // that matched yet failed is an ambiguity or arity diagnostic.
            if impl_env.arity() == 0 {
                continue;
            }
            let _scratch = budget.reserve_bytes(32 * 512 + 128)?;
            if match_impl_type(
                &associated.self_pattern,
                owner,
                &impl_env,
                &mut HashMap::new(),
                budget,
                0,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_associated_with_scratch(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        module: &ModuleId,
        dot: bool,
        budget: &ir::PlanBudget,
        scratch: &std::cell::RefCell<tr::TraitScratch>,
    ) -> Result<(ir::Callee, ir::Signature, ReceiverKind), String> {
        for traits_only in [false, true] {
            let selected = self.lookup_associated_with_scratch(
                owner,
                name,
                args,
                caller,
                module,
                dot,
                None,
                traits_only,
                budget,
                &mut scratch.borrow_mut(),
            )?;
            if let Some(selected) = selected {
                return Ok(selected);
            }
        }
        Err(format!(
            "no applicable associated item {}",
            self.program
                .names
                .spelling(*name)
                .map_err(native_adapter::name_diagnostic)?
        ))
    }

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
        self.lookup_associated_with_scratch(
            owner,
            name,
            method_args,
            caller,
            module,
            dot,
            filter,
            traits_only,
            budget,
            &mut tr::TraitScratch::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn lookup_associated_with_scratch(
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
        scratch: &mut tr::TraitScratch,
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
        let mut selected: Option<(ir::Callee, ir::Signature, ReceiverKind)> = None;
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
                .discharge_direct_with(&target, facts, scratch)
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
        let mut selected: Option<(ir::Callee, ir::Signature, ReceiverKind)> = None;
        let mut selected_trait = None;
        for (application, _) in self
            .method_trait_candidates(owner, name, caller, module, budget)?
            .as_slice()
        {
            if let Some(call) = self.select_trait_filtered_with_scratch(
                application,
                name,
                method_args,
                caller,
                dot,
                filter,
                budget,
                scratch,
            )? {
                // Selection may return a concrete implementation target instead
                // of a TraitItem. Retain the queried trait declaration rather
                // than inferring it from the physical call representation.
                if selected_trait == Some(application.declaration) {
                    return Err(trait_application_ambiguity());
                }
                if selected.is_some() {
                    return Err("ambiguous in-scope trait item".into());
                }
                selected_trait = Some(application.declaration);
                selected = Some(call);
            }
        }
        Ok(selected)
    }

    /// Reuse the ordinary visibility and exact application discovery for
    /// nonexecuting method-argument inference. Selection still discharges the
    /// final ordered arguments and predicates independently.
    pub(super) fn method_trait_candidates(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        caller: ir::FactsCtx<'_>,
        module: &ModuleId,
        budget: &ir::PlanBudget,
    ) -> Result<super::super::token_session::BudgetVec<(ir::TraitRef, ir::DeclId)>, String> {
        use super::super::{expansion_budget::Storage, token_session::BudgetVec};
        self.check_budget(budget)?;
        self.program
            .modules
            .check(*module)
            .map_err(module_diagnostic)?;
        let member_key = self
            .program
            .names
            .key(*name, LookupClass::Member)
            .map_err(native_adapter::name_diagnostic)?;
        let ir::FactsCtx::Validated(facts) = caller else {
            return Err("trait selection requires validated facts".into());
        };
        let mut candidates = BudgetVec::new(self.program.names.budget(), Storage::Temporary)
            .map_err(selection_token_error)?;
        // Resolve each in-scope trait import once, not once per declared trait
        // (traits x imports path resolutions, each charging token work).
        let implicit = self.program.implicit_prelude(*module)?;
        let mut imported = Vec::new();
        for (import_module, path) in &self.registry.trait_imports {
            budget.charge_work(1)?;
            if import_module == module || Some(*import_module) == implicit {
                imported.push(self.resolve_trait_path(path)?);
            }
        }
        for (trait_index, trait_id) in self.registry.trait_ids.iter().enumerate() {
            budget.charge_work(1)?;
            let info = self.declaration(*trait_id)?;
            let mut visible =
                info.path.module == *module || self.catalog.declaration(*trait_id).is_some();
            budget.charge_work(imported.len())?;
            if imported.contains(trait_id) {
                visible = true;
            }
            for predicate in facts.predicates().predicates() {
                budget.charge_work(1)?;
                let ir::Predicate::Implements(bound) = predicate else {
                    continue;
                };
                if bound.declaration == *trait_id
                    && metered_type_equal(&bound.self_ty, owner, budget, 0)?
                {
                    visible = true;
                }
            }
            if !visible {
                continue;
            }
            let Some(item) = self.trait_item_at(trait_index, name, member_key, budget)? else {
                continue;
            };
            for application in self
                .trait_applications(owner, *trait_id, facts, budget)?
                .as_slice()
            {
                candidates
                    .push((application.clone(), item))
                    .map_err(selection_token_error)?;
            }
        }
        Ok(candidates)
    }
}
impl PlanCtx {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_trait(
        &self,
        trait_ref: &ir::TraitRef,
        name: &AstIdent,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        dot: bool,
        budget: &ir::PlanBudget,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.select_trait_filtered(trait_ref, name, args, caller, dot, None, budget)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_trait_with_scratch(
        &self,
        trait_ref: &ir::TraitRef,
        name: &AstIdent,
        args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        dot: bool,
        budget: &ir::PlanBudget,
        scratch: &std::cell::RefCell<tr::TraitScratch>,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.select_trait_filtered_with_scratch(
            trait_ref,
            name,
            args,
            caller,
            dot,
            None,
            budget,
            &mut scratch.borrow_mut(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_trait_filtered(
        &self,
        trait_ref: &ir::TraitRef,
        name: &AstIdent,
        method_args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        dot: bool,
        filter: Option<ReceiverKind>,
        budget: &ir::PlanBudget,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.select_trait_filtered_with_scratch(
            trait_ref,
            name,
            method_args,
            caller,
            dot,
            filter,
            budget,
            &mut tr::TraitScratch::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn select_trait_filtered_with_scratch(
        &self,
        trait_ref: &ir::TraitRef,
        name: &AstIdent,
        method_args: &[ir::Ty],
        caller: ir::FactsCtx<'_>,
        dot: bool,
        filter: Option<ReceiverKind>,
        budget: &ir::PlanBudget,
        scratch: &mut tr::TraitScratch,
    ) -> Result<Option<(ir::Callee, ir::Signature, ReceiverKind)>, String> {
        self.check_budget(budget)?;
        for argument in method_args {
            budget.charge_work(1)?;
            caller.validate_ty_with_budget_scratch(argument, budget, scratch)?;
        }
        let ir::FactsCtx::Validated(facts) = caller else {
            return Err("trait call requires validated facts".into());
        };
        caller.validate_ty_with_budget_scratch(&trait_ref.self_ty, budget, scratch)?;
        for argument in trait_ref.args.iter() {
            budget.charge_work(1)?;
            caller.validate_ty_with_budget_scratch(argument, budget, scratch)?;
        }
        // Check every supplied type before reporting a capability boundary.
        for argument in trait_ref.args.iter() {
            ir::reject_references(argument, budget)?;
        }
        if self.param_env(trait_ref.declaration)?.arity() != trait_ref.args.len().saturating_add(1)
        {
            return Err("trait argument arity mismatch".into());
        }
        let Some(item) = self.trait_item(trait_ref.declaration, name, budget)? else {
            return Ok(None);
        };
        let item_receiver = self.declaration(item)?.receiver;
        if filter.is_some_and(|mode| receiver_mode(mode) != receiver_mode(item_receiver)) {
            return Ok(None);
        }
        let call = ir::TraitCall {
            trait_ref: trait_ref.clone(),
            item,
            method_args: Arc::from(method_args),
        };
        match facts
            .service()
            .prove_goal_with(
                &ir::Predicate::Implements(call.trait_ref.clone()),
                facts,
                scratch,
            )
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
        {
            tr::ProofOutcome::NotProven => return Ok(None),
            tr::ProofOutcome::Proved(_) => {}
        }
        let signature = facts
            .service()
            .instantiate_trait_signature_with(&call, facts, scratch)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
        let receiver = self.declaration(item)?.receiver;
        if dot && receiver == ReceiverKind::None {
            return Err("trait associated function without self is not dot-callable".into());
        }
        if matches!(call.trait_ref.self_ty, ir::Ty::Dyn(_)) {
            // Methods of a trait object dispatch through its vtable. By-value
            // `self` cannot move out of an unsized object (E0161).
            if matches!(receiver, ReceiverKind::Value { .. }) {
                return Err(
                    "E0161: cannot move a value of type `dyn Trait`: by-value `self` method on a trait object"
                        .into(),
                );
            }
            return Ok(Some((
                ir::Callee::Virtual(ir::VirtualCall { call }),
                signature,
                receiver,
            )));
        }
        Ok(Some((ir::Callee::TraitItem(call), signature, receiver)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_trait_actuals_and_unique_dot_dispatch_remain_distinct() {
        for source in [
            "struct Token;trait Convert<T>{fn convert(self)->T;}impl Convert<i32> for Token{fn convert(self)->i32{42}}impl Convert<bool> for Token{fn convert(self)->bool{true}}fn value()->i32{let x=<Token as Convert<i32>>::convert(Token);let flag=<Token as Convert<bool>>::convert(Token);if flag{x}else{0}}fn main(){}",
            "struct Token;trait Convert<T>{fn convert(self)->T;}impl Convert<i32> for Token{fn convert(self)->i32{42}}fn value()->i32{Token.convert()}fn main(){}",
            "struct Wrap<T>(T);trait Convert<T>{fn convert(self)->T;}impl<T> Convert<T> for Wrap<T>{fn convert(self)->T{let Wrap(x)=self;x}}fn value()->i32{Wrap(42).convert()}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn trait_actuals_flow_through_rigid_bounds_supertraits_and_defaults() {
        for source in [
            "trait Convert<T>{fn convert(self)->T;}fn run<T:Convert<i32>>(value:T)->i32{value.convert()}fn main(){}",
            "trait Value<T>:Sized{fn raw(self)->T;fn value(self)->T{self.raw()}}struct Token;impl Value<i32> for Token{fn raw(self)->i32{42}}fn run()->i32{Token.value()}fn main(){}",
            "trait Base<T>{fn base(self)->T;}trait Child<T>:Base<T>{}fn run<T:Child<i32>>(value:T)->i32{value.base()}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn multiple_complete_result_applications_follow_expected_type() {
        // Result-directed selection is supported. Keep the distinct input-only
        // inference boundary below, rather than rejecting stock-valid results.
        super::super::parser_tests::stock_native_run(
            "struct Token;trait Convert<T>{fn convert(self)->T;}impl Convert<i32> for Token{fn convert(self)->i32{42}}impl Convert<bool> for Token{fn convert(self)->bool{true}}fn value()->i32{Token.convert()}fn flag()->bool{Token.convert()}fn main(){println!(\"{} {}\",value(),flag());}",
        );
    }

    #[test]
    fn multiple_complete_input_applications_require_explicit_ufcs() {
        for source in [
            "struct Token;trait Take<T>{fn take(self,value:T)->i32;}impl Take<i32> for Token{fn take(self,value:i32)->i32{value}}impl Take<bool> for Token{fn take(self,value:bool)->i32{if value{42}else{0}}}fn value()->i32{Token.take(42)}fn main(){}",
        ] {
            let program = parse(source).unwrap();
            let error = check(&program).unwrap_err();
            assert!(
                error.contains("unsupported: multiple trait applications require explicit UFCS"),
                "{source}: {error}"
            );
            let stock = super::super::parser_tests::stock_compile(
                source,
                std::path::Path::new("unused"),
                true,
            );
            assert!(
                stock.status.success(),
                "{}",
                String::from_utf8_lossy(&stock.stderr)
            );
            for workers in [1, 8] {
                for check in [true, false] {
                    let error = crate::compiler::compile(source.into(), workers, check)
                        .err()
                        .expect("inference boundary must remain explicit");
                    assert!(
                        error
                            .to_string()
                            .contains("multiple trait applications require explicit UFCS"),
                        "{error}"
                    );
                }
            }
        }
        super::super::parser_tests::stock_native_run(
            "struct Token;trait Convert<T>{fn convert(self)->T;}impl Convert<i32> for Token{fn convert(self)->i32{42}}impl Convert<bool> for Token{fn convert(self)->bool{true}}fn main(){println!(\"{}\",<Token as Convert<i32>>::convert(Token));}",
        );
    }

    #[test]
    fn equality_selection_retains_full_trait_application_without_result_inference() {
        let source = "trait Source<A>{type Out;fn get(self)->Self::Out;}fn both<T:Source<i32,Out=i64>+Source<bool,Out=bool>>(a:T,b:T)->(i64,bool){(<T as Source<i32>>::get(a),<T as Source<bool>>::get(b))}fn main(){}";
        let program = parse(source).unwrap();
        check(&program).unwrap_or_else(|error| panic!("{source}: {error}"));

        let source = "trait Source<A>{type Out;fn get(self)->Self::Out;}fn ambiguous<T:Source<i32,Out=i64>+Source<bool,Out=bool>>(a:T)->i64{a.get()}fn main(){}";
        let program = parse(source).unwrap();
        let error = check(&program).unwrap_err();
        assert!(
            error.contains("multiple trait applications require explicit UFCS"),
            "{error}"
        );
    }

    #[test]
    fn missing_actual_obligation_and_unused_bad_generic_body_reject() {
        for source in [
            "struct Token;trait Convert<T>{fn convert(self)->T;}impl Convert<i32> for Token{fn convert(self)->i32{42}}fn value()->bool{<Token as Convert<bool>>::convert(Token)}fn main(){}",
            "trait Bad<T>:Sized{fn bad(self,value:T)->i32{value}}fn main(){}",
        ] {
            let result = parse(source).and_then(|program| check(&program));
            assert!(result.is_err(), "accepted {source}");
        }
    }
}

/// Whether a type mentions any generic parameter (conservatively: anything
/// not a closed primitive or closed composite counts as open).
fn mentions_param(ty: &ir::Ty) -> bool {
    match ty {
        ir::Ty::Param(_) | ir::Ty::AssociatedType(_) => true,
        ir::Ty::Ref { referent, .. } => mentions_param(referent),
        ir::Ty::RawPtr { pointee, .. } => mentions_param(pointee),
        ir::Ty::Slice { element } | ir::Ty::Array { element, .. } => mentions_param(element),
        ir::Ty::Tuple(parts) => parts.iter().any(mentions_param),
        ir::Ty::Int(_)
        | ir::Ty::Float(_)
        | ir::Ty::Bool
        | ir::Ty::Char
        | ir::Ty::Str
        | ir::Ty::Unit
        | ir::Ty::Never => false,
        ir::Ty::Struct(s) => s.def().type_args.iter().any(mentions_param),
        ir::Ty::Enum(e) => e.def().type_args.iter().any(mentions_param),
        _ => true,
    }
}
