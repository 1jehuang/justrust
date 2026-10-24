//! Discover generic trait-method constraints at the canonical receiver frontier.
//! These are inference terms only. No closure type, proof or executable target is
//! fabricated: ordinary method selection still discharges the final application.
use super::super::method_argument_selection::MethodArguments;
use super::*;

impl Solver<'_, '_, '_> {
    /// Infer arguments only from a unique unspecialized nominal implementation.
    /// These are equations, not selection authority: checked lowering must still
    /// prove the fully instantiated application and all implementation bounds.
    pub(super) fn infer_trait_arguments(&mut self) -> Result<bool, String> {
        let mut progress = false;
        for (trait_id, receiver, arguments) in std::mem::take(&mut self.trait_arguments) {
            self.checker.infer_work(0)?;
            let mut unresolved = false;
            for argument in &arguments {
                unresolved |= self.resolve(*argument, 0)?.is_none();
            }
            if !unresolved {
                continue;
            }
            let Some(header) = self.structural_impl(receiver, trait_id)? else {
                self.trait_arguments.push((trait_id, receiver, arguments));
                continue;
            };
            if header.trait_ref.args.len() != arguments.len() {
                return Err("trait inference argument arity changed".into());
            }
            let params = self.structural_impl_parameters(receiver, &header, 0)?;
            for (pattern, actual) in header.trait_ref.args.iter().zip(arguments) {
                let pattern = self.schema(pattern, &params, 0)?;
                self.unify(pattern, actual)?;
            }
            progress = true;
        }
        Ok(progress)
    }

    /// A closure-bearing nominal cannot yet be interned. A unique source impl
    /// can still contribute equations, never a proof or executable selection.
    /// Restrict this path to unspecialized nominal heads so probing cannot bind
    /// an unknown actual merely to make one implementation applicable.
    fn structural_impl(
        &self,
        receiver: Term,
        trait_id: ir::DeclId,
    ) -> Result<Option<tr::ImplHeader>, String> {
        let Domain::Nominal(declaration, actuals) = self.domain(receiver) else {
            return Ok(None);
        };
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        let mut selected = None;
        for header in world.impl_headers() {
            self.checker.infer_work(0)?;
            if header.trait_ref.declaration != trait_id {
                continue;
            }
            let Some((owner, patterns)) = nominal_parts(&header.trait_ref.self_ty) else {
                if matches!(header.trait_ref.self_ty, ir::Ty::Param(_)) {
                    return Ok(None);
                }
                continue;
            };
            if owner != declaration {
                continue;
            }
            if patterns.len() != actuals.len()
                || patterns
                    .iter()
                    .any(|ty| !matches!(ty, ir::Ty::Param(p) if header.schema_env.contains(*p)))
            {
                return Ok(None);
            }
            self.checker.infer_storage(1)?;
            if selected.replace(header.clone()).is_some() {
                return Ok(None);
            }
        }
        Ok(selected)
    }

    fn structural_impl_parameters(
        &mut self,
        receiver: Term,
        header: &tr::ImplHeader,
        depth: usize,
    ) -> Result<HashMap<ir::ParamId, Term>, String> {
        self.checker.infer_work(depth)?;
        let params = self.parameters(&header.schema_env)?;
        let pattern = self.schema(&header.trait_ref.self_ty, &params, depth + 1)?;
        self.unify(receiver, pattern)?;
        self.checker.infer_storage(1)?;
        self.structural_impl_stack
            .push((header.declaration, receiver));
        let result: Result<(), String> = (|| {
            self.callable_predicates(&header.predicates, &params)?;
            let plan = self.checker.signatures.plan()?;
            let world = plan.trait_world()?;
            for predicate in header.predicates.iter() {
                self.checker.infer_work(depth)?;
                if let ir::Predicate::ProjectionEq { projection, ty } = predicate {
                    // Callable equations were linked directly to the source closure
                    // above. Its canonical ClosureTy is deliberately not fabricated.
                    if world
                        .closure_trait_kind(projection.trait_ref.declaration)
                        .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?
                        .is_some()
                    {
                        continue;
                    }
                    let left = self.schema(
                        &ir::Ty::AssociatedType(Arc::new(projection.clone())),
                        &params,
                        depth + 1,
                    )?;
                    let right = self.schema(ty, &params, depth + 1)?;
                    self.unify(left, right)?;
                }
            }
            Ok(())
        })();
        self.structural_impl_stack.pop();
        result?;
        Ok(params)
    }

    pub(super) fn structural_projection(
        &mut self,
        receiver: Term,
        trait_id: ir::DeclId,
        item: ir::DeclId,
        arguments: &[Term],
        depth: usize,
    ) -> Result<Option<Term>, String> {
        self.checker.infer_work(depth)?;
        if !self.contains_callable(receiver, depth + 1)? {
            return Ok(None);
        }
        let Some(header) = self.structural_impl(receiver, trait_id)? else {
            return Ok(None);
        };
        self.checker.infer_work(self.structural_impl_stack.len())?;
        if self
            .structural_impl_stack
            .iter()
            .any(|(declaration, term)| {
                *declaration == header.declaration && self.root(*term) == self.root(receiver)
            })
        {
            return Ok(None);
        }
        if header.trait_ref.args.len() != arguments.len() {
            return Ok(None);
        }
        let Some(value) = header
            .associated_types
            .iter()
            .find(|value| value.associated == item)
        else {
            return Ok(None);
        };
        self.checker.infer_storage(1)?;
        self.structural_impl_stack
            .push((header.declaration, receiver));
        let result = (|| {
            let params = self.structural_impl_parameters(receiver, &header, depth + 1)?;
            for (pattern, actual) in header.trait_ref.args.iter().zip(arguments) {
                let pattern = self.schema(pattern, &params, depth + 1)?;
                self.unify(pattern, *actual)?;
            }
            self.schema(&value.rhs, &params, depth + 1).map(Some)
        })();
        self.structural_impl_stack.pop();
        result
    }

    pub(super) fn numeric_nominal_actuals(&self, receiver: Term) -> Result<bool, String> {
        let Domain::Nominal(_, actuals) = self.domain(receiver) else {
            return Ok(false);
        };
        let numeric = actuals
            .iter()
            .any(|actual| matches!(self.domain(*actual), Domain::Integer | Domain::Float));
        if !numeric {
            return Ok(false);
        }
        for actual in actuals {
            self.checker.infer_work(0)?;
            if self.resolve(actual, 0)?.is_none() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn structural_trait_method_constraints(
        &mut self,
        receiver: Term,
        name: AstIdent,
        arguments: &[Term],
        explicit: &[Term],
    ) -> Result<Option<Term>, String> {
        let plan = self.checker.signatures.plan()?;
        let implicit = plan.program.implicit_prelude(self.checker.module)?;
        let mut selected = None;
        for trait_id in &plan.registry.trait_ids {
            self.checker.infer_work(0)?;
            let Some(item) = plan.trait_item(*trait_id, &name, &plan.budget)? else {
                continue;
            };
            let info = plan.declaration(*trait_id)?;
            let mut visible = info.path.module == self.checker.module
                || plan.catalog.declaration(*trait_id).is_some();
            for (module, path) in &plan.registry.trait_imports {
                self.checker.infer_work(0)?;
                if (*module == self.checker.module || Some(*module) == implicit)
                    && plan.resolve_trait_path(path)? == *trait_id
                {
                    visible = true;
                }
            }
            if !visible {
                continue;
            }
            let header = self.structural_impl(receiver, *trait_id)?;
            if header.is_none() {
                if !self.numeric_nominal_actuals(receiver)? {
                    continue;
                }
                let Domain::Nominal(nominal, _) = self.domain(receiver) else {
                    continue;
                };
                if plan.param_env(*trait_id)?.arity() != 1 {
                    continue;
                }
                let mut exists = false;
                for candidate in plan.trait_world()?.impl_headers() {
                    self.checker.infer_work(0)?;
                    if candidate.trait_ref.declaration == *trait_id
                        && nominal_parts(&candidate.trait_ref.self_ty)
                            .is_some_and(|(owner, _)| owner == nominal)
                    {
                        exists = true;
                    }
                }
                if !exists {
                    continue;
                }
            }
            // A method schema contributes equations, not an impl selection.
            if selected.replace((header, *trait_id, item)).is_some() {
                return Ok(None);
            }
        }
        let Some((header, trait_id, item)) = selected else {
            return Ok(None);
        };
        let schema = plan.raw_schema(item)?;
        let own = plan.param_env(item)?;
        if schema.signature.params.len() != arguments.len() + 1
            || (!explicit.is_empty() && explicit.len() != own.arity())
        {
            return Ok(None);
        }
        let impl_params = match &header {
            Some(header) => self.structural_impl_parameters(receiver, header, 0)?,
            None => HashMap::new(),
        };
        let mut params = self.parameters(&schema.env)?;
        let owner = plan.param_env(trait_id)?;
        params.insert(
            ir::ParamId {
                owner: owner.binder(),
                index: 0,
            },
            receiver,
        );
        for (index, actual) in header
            .iter()
            .flat_map(|h| h.trait_ref.args.iter())
            .enumerate()
        {
            let actual = self.schema(actual, &impl_params, 0)?;
            params.insert(
                ir::ParamId {
                    owner: owner.binder(),
                    index: index as u16 + 1,
                },
                actual,
            );
        }
        for (index, actual) in explicit.iter().enumerate() {
            params.insert(
                ir::ParamId {
                    owner: own.binder(),
                    index: index as u16,
                },
                *actual,
            );
        }
        for (actual, formal) in arguments.iter().zip(schema.signature.params.iter().skip(1)) {
            let formal = self.schema(formal, &params, 0)?;
            self.coerce(*actual, formal)?;
        }
        self.callable_predicates(&schema.predicates, &params)?;
        self.schema(&schema.signature.result, &params, 0).map(Some)
    }

    pub(super) fn trait_method_constraints(
        &mut self,
        receiver: &ir::Ty,
        name: AstIdent,
        arguments: &[Term],
        explicit: &[Term],
        expected: Term,
    ) -> Result<Option<Term>, String> {
        if self.preparation.is_none()
            && !arguments.iter().any(|argument| {
                matches!(self.domain(self.peel_refs(*argument)), Domain::Callable(_))
            })
        {
            return Ok(None);
        }
        let checker = self.checker;
        let plan = checker.signatures.plan()?;
        let ir::FactsCtx::Validated(facts) = checker.facts() else {
            return Ok(None);
        };
        let mut discovered = None;
        let mut same_trait: Vec<(ir::TraitRef, ir::DeclId)> = Vec::new();
        // Reuse the ordinary receiver/adjustment and inherent-before-trait order.
        // Returning Deferred stops at the first applicable frontier, rather than
        // silently considering a lower-priority trait while its arguments wait.
        let _ = plan.select_method_with_arguments(
            receiver,
            &name,
            checker.facts(),
            (&checker.module, &plan.budget),
            |owner, mode, traits_only| {
                if !traits_only {
                    if let Some((nominal, _)) = nominal_parts(owner)
                        && self.candidate(nominal, name)?.is_some()
                    {
                        return Ok(MethodArguments::Deferred);
                    }
                    return Ok(MethodArguments::Missing);
                }
                let candidates = plan.method_trait_candidates(
                    owner,
                    &name,
                    checker.facts(),
                    &checker.module,
                    &plan.budget,
                )?;
                let mut applicable = None;
                let mut collected: Vec<_> = Vec::new();
                for (application, item) in candidates.as_slice() {
                    checker.infer_work(0)?;
                    if receiver_mode(plan.declaration(*item)?.receiver) != receiver_mode(mode) {
                        continue;
                    }
                    if matches!(
                        facts
                            .service()
                            .prove_goal(&ir::Predicate::Implements(application.clone()), facts,)
                            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?,
                        tr::ProofOutcome::NotProven
                    ) {
                        continue;
                    }
                    collected.push((application.clone(), *item));
                    if let Some((previous, _)) = applicable.replace((application.clone(), *item))
                        && previous.declaration != application.declaration
                    {
                        return Err("ambiguous in-scope trait item".into());
                    }
                }
                let Some(application) = applicable else {
                    return Ok(MethodArguments::Missing);
                };
                if collected.len() > 1 {
                    // Several applications of one trait: the trait arguments
                    // are selected below from the expected result type.
                    same_trait = collected;
                    return Ok(MethodArguments::Deferred);
                }
                discovered = Some(application);
                Ok(MethodArguments::Deferred)
            },
            |_, _| Ok(None),
        )?;
        if !same_trait.is_empty() {
            discovered = self.select_application_by_expected(&same_trait, expected)?;
            if discovered.is_none() {
                self.trait_application_pending = true;
                return Ok(None);
            }
        }
        let Some((application, item)) = discovered else {
            return Ok(None);
        };
        let schema = plan.raw_schema(item)?;
        if schema.signature.params.len() != arguments.len() + 1 {
            return Err(self.user_error("E0061: wrong number of method arguments"));
        }
        let own = plan.param_env(item)?;
        if !explicit.is_empty() && explicit.len() != own.arity() {
            return Err(self.user_error("E0107: wrong number of method generic arguments"));
        }
        let owner = plan.param_env(application.declaration)?;
        if owner.arity() != application.args.len() + 1 {
            return Err("trait constraint owner arity changed".into());
        }
        let mut params = self.parameters(&schema.env)?;
        for (index, actual) in std::iter::once(&application.self_ty)
            .chain(application.args.iter())
            .enumerate()
        {
            let term = self.fixed(actual.clone())?;
            params.insert(
                ir::ParamId {
                    owner: owner.binder(),
                    index: index as u16,
                },
                term,
            );
        }
        for (index, term) in explicit.iter().enumerate() {
            params.insert(
                ir::ParamId {
                    owner: own.binder(),
                    index: index as u16,
                },
                *term,
            );
        }
        for (argument, formal) in arguments.iter().zip(schema.signature.params.iter().skip(1)) {
            let formal = self.schema(formal, &params, 0)?;
            self.coerce(*argument, formal)?;
        }
        self.callable_predicates(&schema.predicates, &params)?;
        self.schema(&schema.signature.result, &params, 0).map(Some)
    }

    /// Chooses among several proven applications of one trait by the
    /// expected method result, as stock inference does for `x.into()` with an
    /// annotated binding. An unknown expected type defers; a known one that
    /// matches zero or several applications is E0283.
    fn select_application_by_expected(
        &mut self,
        applications: &[(ir::TraitRef, ir::DeclId)],
        expected: Term,
    ) -> Result<Option<(ir::TraitRef, ir::DeclId)>, String> {
        // The delayed method result usually flows into its context through
        // a pending coercion (`let n: i64 = x.into()`); that target is the
        // expected type. Reference results would admit coercions, so only an
        // exact unique pending target is used.
        let mut target = self.resolve(expected, 0)?;
        if target.is_none() {
            let root = self.root(expected);
            let mut found = None;
            for (actual, wanted) in self.coercions.clone() {
                self.checker.infer_work(0)?;
                if self.root(actual) == root
                    && let Some(ty) = self.resolve(wanted, 0)?
                {
                    if found.as_ref().is_some_and(|f| *f != ty) {
                        return Ok(None);
                    }
                    found = Some(ty);
                }
            }
            target = found;
        }
        let Some(expected) = target else {
            return Ok(None);
        };
        let plan = self.checker.signatures.plan()?;
        let mut chosen = None;
        for (application, item) in applications {
            let schema = plan.raw_schema(*item)?;
            let owner = plan.param_env(application.declaration)?;
            let mut params = self.parameters(&schema.env)?;
            for (index, actual) in std::iter::once(&application.self_ty)
                .chain(application.args.iter())
                .enumerate()
            {
                let term = self.fixed(actual.clone())?;
                params.insert(
                    ir::ParamId {
                        owner: owner.binder(),
                        index: index as u16,
                    },
                    term,
                );
            }
            let result = self.schema(&schema.signature.result, &params, 0)?;
            let Some(result) = self.resolve(result, 0)? else {
                return Err(super::super::trait_selection::trait_application_ambiguity());
            };
            if result == expected && chosen.replace((application.clone(), *item)).is_some() {
                return Err(super::super::trait_selection::trait_application_ambiguity());
            }
        }
        chosen
            .map(Some)
            .ok_or_else(super::super::trait_selection::trait_application_ambiguity)
    }
}
