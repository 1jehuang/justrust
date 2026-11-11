//! `Owner::item(args)` where `Owner` implements one trait several times with
//! different trait arguments (`u32: From<u8>`, `u32: From<u16>`, ...). Stock
//! rustc picks the unique application whose instantiated parameter types the
//! argument types satisfy. This uses exactly known argument types or the
//! integer-only domain of an unsuffixed literal, never width fallback.
//! Ordinary selection then rediscovers it as an explicit
//! trait call and discharges every obligation; this grants no membership.
use super::*;

enum SourceApplication {
    Pending,
    Unproved,
    Proved(ir::TraitRef),
}

fn retain_trait_frontier(
    frontier: &mut Option<ir::DeclId>,
    next: ir::DeclId,
) -> Result<(), String> {
    if let Some(previous) = *frontier {
        if previous != next {
            return Err("E0034: multiple applicable associated trait items".into());
        }
    } else {
        *frontier = Some(next);
    }
    Ok(())
}

impl Checker<'_> {
    /// The unique trait application of `owner` whose `name` item accepts
    /// `args`, when ordinary selection would otherwise see several
    /// applications of one trait. `Ok(None)` leaves the ordinary path (and its
    /// ambiguity diagnostic) unchanged.
    pub(super) fn argument_selected_trait_application(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        type_args: &[Type],
        args: &[Expr],
    ) -> Result<Option<ir::TraitRef>, String> {
        if !type_args.is_empty() || args.is_empty() {
            return Ok(None);
        }
        // Trait-application disambiguation must not bypass a matching inherent
        // item, including an inherent method whose actuals remain unresolved.
        if matches!(
            self.inherent_method_actuals(owner, name, args, type_args, None, None)?,
            inherent_method_hint::InherentMethodInference::Applicable(_)
        ) {
            return Ok(None);
        }
        let evidence = args
            .iter()
            .map(|arg| self.inference_evidence(arg, 0))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(application) = self.source_argument_trait_application(owner, name, &evidence)? {
            return Ok(Some(application));
        }
        // A bare integer literal rules out non-integer formals, but never
        // chooses one integer width over another or applies integer fallback.
        self.infer_storage(args.len())?;
        let mut actual = Vec::with_capacity(args.len());
        for arg in args {
            let ty = self.scalar_hint(arg)?;
            if ty.is_none() && !matches!(arg, Expr::Integer(literal) if literal.suffix.is_none()) {
                return Ok(None);
            }
            actual.push(ty);
        }
        self.trait_application_matching_arguments(owner, name, args.len(), |formals| {
            let plan = self.signatures.plan()?;
            for (formal, actual) in formals.iter().zip(&actual) {
                match actual {
                    Some(actual) if metered_type_equal(formal, actual, &plan.budget, 0)? => {}
                    None if matches!(formal, ir::Ty::Int(_)) => {}
                    _ => return Ok(false),
                }
            }
            Ok(true)
        })
    }

    /// When `owner` has several applications of one trait whose `name` item
    /// all return the same concrete type, that result. This is only an
    /// inference hint; selection of the exact application happens later.
    pub(in crate::compiler::frontend) fn shared_trait_application_result(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        arity: usize,
    ) -> Result<Option<ir::Ty>, String> {
        let plan = self.signatures.plan()?;
        let candidates =
            plan.method_trait_candidates(owner, name, self.facts(), &self.module, &plan.budget)?;
        let candidates = candidates.as_slice();
        let Some((first, _)) = candidates.first() else {
            return Ok(None);
        };
        if candidates.len() < 2
            || candidates
                .iter()
                .any(|(application, _)| application.declaration != first.declaration)
        {
            return Ok(None);
        }
        let ir::FactsCtx::Validated(facts) = self.facts() else {
            return Ok(None);
        };
        let mut result: Option<ir::Ty> = None;
        for (application, item) in candidates {
            plan.budget.charge_work(1)?;
            if plan.param_env(*item)?.arity() != 0 {
                return Ok(None);
            }
            let call = ir::TraitCall {
                trait_ref: application.clone(),
                item: *item,
                method_args: Arc::from([]),
            };
            let signature = facts
                .service()
                .instantiate_trait_signature(&call, facts)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            if signature.params.len() != arity {
                return Ok(None);
            }
            match &result {
                None => result = Some(signature.result),
                Some(previous) => {
                    if !metered_type_equal(previous, &signature.result, &plan.budget, 0)? {
                        return Ok(None);
                    }
                }
            }
        }
        Ok(result)
    }

    /// As above, from exactly known argument types.
    pub(in crate::compiler::frontend) fn trait_application_for_argument_types(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        actual: &[ir::Ty],
    ) -> Result<Option<ir::TraitRef>, String> {
        let evidence = actual
            .iter()
            .map(|ty| self.infer_type_term(ty, None, 0))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(application) = self.source_argument_trait_application(owner, name, &evidence)? {
            return Ok(Some(application));
        }
        self.trait_application_matching_arguments(owner, name, actual.len(), |formals| {
            let plan = self.signatures.plan()?;
            for (formal, actual) in formals.iter().zip(actual) {
                if !metered_type_equal(formal, actual, &plan.budget, 0)? {
                    return Ok(false);
                }
            }
            Ok(true)
        })
    }

    /// An impl may have parameters absent from Self but present in its method
    /// inputs (`impl<K,V,const N> From<[(K,V);N]> for Map<K,V>`). Discover its
    /// exact trait application by solving those source equations together.
    /// This does not select a target: ordinary trait proof is required both
    /// here and again by the caller's checked selection.
    fn source_argument_trait_application(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        evidence: &[InferTerm],
    ) -> Result<Option<ir::TraitRef>, String> {
        let plan = self.signatures.plan()?;
        // A rigid caller parameter's authored bound is the higher-priority
        // application frontier, not a source implementation guess.
        if matches!(owner, ir::Ty::Param(_)) {
            return Ok(None);
        }
        if self.facts().validated().is_none() {
            return Ok(None);
        }
        let mut selected = None;
        let mut pending = false;
        let mut unproved = false;
        let mut trait_frontier = None;
        for (header, id) in self.source_associated_trait_candidates(name)? {
            plan.budget.charge_work(1)?;
            let trait_id = header.trait_ref.declaration;
            let env = plan.param_env(header.declaration)?;
            let mut solver = Inference::new(self, env)?;
            let formal = solver.schema(&header.trait_ref.self_ty, 0)?;
            let actual = self.infer_type_term(owner, None, 0)?;
            if let Err(error) = solver.core.constrain(&formal, &actual, 0) {
                if error.is_resource() || error.kind() == tr::FactsErrorKind::Infrastructure {
                    return Err(error.into_legacy_diagnostic(&plan.budget));
                }
                continue;
            }
            let owner_pending =
                match self.source_application_from_solver(owner, &header, &solver)? {
                    SourceApplication::Unproved => continue,
                    SourceApplication::Pending => true,
                    SourceApplication::Proved(_) => {
                        retain_trait_frontier(&mut trait_frontier, trait_id)?;
                        false
                    }
                };
            let Some(id) = id else {
                pending = true;
                continue;
            };
            let info = plan.declaration(id)?;
            if info.param_env.arity() != 0 {
                pending = true;
                continue;
            }
            let schema = plan.function_schema(id)?;
            if schema.signature.params.len() != evidence.len() {
                pending |= owner_pending;
                continue;
            }
            let mut fits = true;
            for (formal, actual) in schema.signature.params.iter().zip(evidence) {
                let formal = solver.schema(formal, 0)?;
                if let Err(error) = solver.core.constrain(&formal, actual, 0) {
                    if error.is_resource() || error.kind() == tr::FactsErrorKind::Infrastructure {
                        return Err(error.into_legacy_diagnostic(&plan.budget));
                    }
                    fits = false;
                    break;
                }
            }
            if !fits {
                continue;
            }
            if solver.finish(false)?.is_none() {
                pending = true;
                continue;
            }
            let application = match self.source_application_from_solver(owner, &header, &solver)? {
                SourceApplication::Pending => {
                    pending = true;
                    continue;
                }
                SourceApplication::Unproved => {
                    unproved = true;
                    continue;
                }
                SourceApplication::Proved(application) => application,
            };
            retain_trait_frontier(&mut trait_frontier, trait_id)?;
            if let Some(previous) = &selected {
                if previous != &application {
                    return Ok(None);
                }
            } else {
                selected = Some(application);
            }
        }
        if selected.is_none() && !pending && unproved {
            return Err("E0277: associated item requires an unsatisfied trait application".into());
        }
        Ok(if pending { None } else { selected })
    }

    fn source_application_from_solver(
        &self,
        owner: &ir::Ty,
        header: &tr::ImplHeader,
        solver: &Inference<'_, '_>,
    ) -> Result<SourceApplication, String> {
        let plan = self.signatures.plan()?;
        let ir::FactsCtx::Validated(facts) = self.facts() else {
            return Ok(SourceApplication::Pending);
        };
        let mut arguments = Vec::new();
        for argument in header.trait_ref.args.iter() {
            let formal = solver.schema(argument, 0)?;
            let Some(argument) = solver
                .core
                .resolve_bound_schema(&formal, 0, &mut |declaration, actuals| {
                    solver.materialize(declaration, actuals)
                })
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
            else {
                return Ok(SourceApplication::Pending);
            };
            self.infer_storage(1)?;
            arguments.push(argument);
        }
        let application = ir::TraitRef {
            declaration: header.trait_ref.declaration,
            self_ty: owner.clone(),
            args: arguments.into(),
        };
        match facts
            .service()
            .prove_goal(&ir::Predicate::Implements(application.clone()), facts)
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
        {
            tr::ProofOutcome::NotProven => Ok(SourceApplication::Unproved),
            tr::ProofOutcome::Proved(_) => Ok(SourceApplication::Proved(application)),
        }
    }

    pub(super) fn source_associated_trait_candidates(
        &self,
        name: &AstIdent,
    ) -> Result<Vec<(tr::ImplHeader, Option<ir::DeclId>)>, String> {
        let plan = self.signatures.plan()?;
        let world = plan.trait_world()?;
        let implicit = plan.program.implicit_prelude(self.module)?;
        let mut candidates = Vec::new();
        for trait_id in &plan.registry.trait_ids {
            plan.budget.charge_work(1)?;
            let Some(item) = plan.trait_item(*trait_id, name, &plan.budget)? else {
                continue;
            };
            let trait_info = plan.declaration(*trait_id)?;
            let mut visible = trait_info.path.module == self.module
                || plan.catalog.declaration(*trait_id).is_some();
            for (module, path) in &plan.registry.trait_imports {
                plan.budget.charge_work(1)?;
                if (*module == self.module || Some(*module) == implicit)
                    && plan.resolve_trait_path(path)? == *trait_id
                {
                    visible = true;
                }
            }
            if !visible {
                continue;
            }
            for header in world.impl_headers() {
                plan.budget.charge_work(1)?;
                if header.trait_ref.declaration != *trait_id {
                    continue;
                }
                let mut source = None;
                for (implemented, callable) in header.items.iter() {
                    plan.budget.charge_work(1)?;
                    if *implemented == item {
                        source = Some(*callable);
                    }
                }
                // An inherited default is still a competing application. Its
                // absence from the provided-method map cannot hide it.
                self.infer_storage(1)?;
                candidates.push((header.clone(), source));
            }
        }
        Ok(candidates)
    }

    fn trait_application_matching_arguments(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        arity: usize,
        mut fits: impl FnMut(&[ir::Ty]) -> Result<bool, String>,
    ) -> Result<Option<ir::TraitRef>, String> {
        if arity == 0 {
            return Ok(None);
        }
        let plan = self.signatures.plan()?;
        let candidates =
            plan.method_trait_candidates(owner, name, self.facts(), &self.module, &plan.budget)?;
        let candidates = candidates.as_slice();
        // Only the multiple-applications-of-one-trait case is changed.
        let Some((first, _)) = candidates.first() else {
            return Ok(None);
        };
        if candidates.len() < 2
            || candidates
                .iter()
                .any(|(application, _)| application.declaration != first.declaration)
        {
            return Ok(None);
        }
        let ir::FactsCtx::Validated(facts) = self.facts() else {
            return Ok(None);
        };
        let mut selected: Option<ir::TraitRef> = None;
        for (application, item) in candidates {
            plan.budget.charge_work(1)?;
            if plan.param_env(*item)?.arity() != 0 {
                return Ok(None);
            }
            if matches!(
                facts
                    .service()
                    .prove_goal(&ir::Predicate::Implements(application.clone()), facts)
                    .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?,
                tr::ProofOutcome::NotProven
            ) {
                continue;
            }
            let call = ir::TraitCall {
                trait_ref: application.clone(),
                item: *item,
                method_args: Arc::from([]),
            };
            let signature = facts
                .service()
                .instantiate_trait_signature(&call, facts)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            if signature.params.len() != arity {
                continue;
            }
            if !fits(&signature.params)? {
                continue;
            }
            if selected.replace(application.clone()).is_some() {
                return Ok(None);
            }
        }
        Ok(selected)
    }
}
