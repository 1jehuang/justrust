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
                // An unsuffixed literal is still open: defaulting must not
                // preempt an impl that fixes the argument (`u32: Borrow<?Q>`).
                unresolved |= self.resolve(*argument, 0)?.is_none()
                    || matches!(self.domain(*argument), Domain::Integer | Domain::Float);
            }
            // An open receiver (`Vec<_>: FromIterator<u32>`) can also be
            // completed from a unique structural impl's argument equations.
            let open_receiver = matches!(self.domain(receiver), Domain::Nominal(..))
                && self.resolve(receiver, 0)?.is_none();
            if !unresolved && !open_receiver {
                continue;
            }
            if !unresolved && !self.contains_callable(receiver, 0)? {
                // Only structural impls apply for an open receiver.
                if let Some(header) = self.structural_impl(receiver, trait_id)?
                    && header.trait_ref.args.len() == arguments.len()
                {
                    let params = self.structural_impl_parameters(receiver, &header, 0)?;
                    for (pattern, actual) in header.trait_ref.args.iter().zip(arguments) {
                        let pattern = self.schema(pattern, &params, 0)?;
                        self.unify(pattern, actual)?;
                    }
                    progress = true;
                } else {
                    self.trait_arguments.push((trait_id, receiver, arguments));
                }
                continue;
            }
            let Some(header) = self.structural_impl(receiver, trait_id)? else {
                if self.unique_concrete_impl_arguments(receiver, trait_id, &arguments)? {
                    progress = true;
                    continue;
                }
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

    /// rustc infers trait arguments when exactly one impl can apply to a known
    /// receiver (`u32: Borrow<?Q>` only via `impl<T> Borrow<T> for T`, so
    /// `?Q = u32` before integer-literal defaulting). Candidates are impls
    /// whose self type is exactly the receiver or a bare impl parameter; any
    /// other shape, or more than one candidate, leaves the arguments open.
    /// These are equations only: final selection still proves the application.
    fn unique_concrete_impl_arguments(
        &mut self,
        receiver: Term,
        trait_id: ir::DeclId,
        arguments: &[Term],
    ) -> Result<bool, String> {
        if matches!(self.domain(receiver), Domain::Integer | Domain::Float) {
            return Ok(false);
        }
        let Some(actual) = self.resolve(receiver, 0)? else {
            return Ok(false);
        };
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        let mut selected: Option<tr::ImplHeader> = None;
        for header in world.impl_headers() {
            self.checker.infer_work(0)?;
            if header.trait_ref.declaration != trait_id {
                continue;
            }
            let self_ty = &header.trait_ref.self_ty;
            let applies = match self_ty {
                ir::Ty::Param(p) if header.schema_env.contains(*p) => true,
                _ if *self_ty == actual => true,
                _ => {
                    // A generic or differently-shaped head may still apply;
                    // refuse to guess rather than miss a candidate.
                    if !matches!(
                        self_ty,
                        ir::Ty::Int(_)
                            | ir::Ty::Float(_)
                            | ir::Ty::Bool
                            | ir::Ty::Unit
                            | ir::Ty::Char
                            | ir::Ty::Str
                    ) && std::mem::discriminant(self_ty) == std::mem::discriminant(&actual)
                    {
                        return Ok(false);
                    }
                    false
                }
            };
            if !applies {
                continue;
            }
            if header.trait_ref.args.len() != arguments.len()
                || selected.replace(header.clone()).is_some()
            {
                return Ok(false);
            }
        }
        let Some(header) = selected else {
            return Ok(false);
        };
        let params = self.parameters(&header.schema_env)?;
        let pattern = self.schema(&header.trait_ref.self_ty, &params, 1)?;
        self.unify(receiver, pattern)?;
        for (pattern, actual) in header.trait_ref.args.iter().zip(arguments) {
            let pattern = self.schema(pattern, &params, 1)?;
            self.unify(pattern, *actual)?;
        }
        Ok(true)
    }

    /// An unmaterialized nominal can still contribute equations through a
    /// unique unspecialized nominal or blanket implementation. These equations
    /// do not select an implementation: checked lowering must prove it later.
    /// Competing or specialized heads leave inference pending rather than bind
    /// an unknown actual merely to make one implementation applicable.
    pub(super) fn structural_impl(
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
                if let ir::Ty::Param(parameter) = header.trait_ref.self_ty {
                    if !header.schema_env.contains(parameter) {
                        return Ok(None);
                    }
                    self.checker.infer_storage(1)?;
                    if selected.replace(header.clone()).is_some() {
                        return Ok(None);
                    }
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

    pub(super) fn structural_impl_parameters(
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

    /// Whether `trait_id` has an impl whose self type is a reference to the
    /// receiver's nominal (`impl IntoIterator for &Vec<T>`).
    fn reference_impl_for(&self, receiver: Term, trait_id: ir::DeclId) -> Result<bool, String> {
        let Domain::Nominal(nominal, _) = self.domain(receiver) else {
            return Ok(false);
        };
        let plan = self.checker.signatures.plan()?;
        for candidate in plan.trait_world()?.impl_headers() {
            self.checker.infer_work(0)?;
            if candidate.trait_ref.declaration == trait_id
                && let ir::Ty::Ref { referent, .. } = &candidate.trait_ref.self_ty
                && nominal_parts(referent).is_some_and(|(owner, _)| owner == nominal)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether a by-value trait method named `name` on the receiver's nominal
    /// is shadowed by the same trait implemented for a reference to it.
    pub(super) fn reference_impl_shadows(
        &self,
        receiver: Term,
        name: AstIdent,
    ) -> Result<bool, String> {
        let plan = self.checker.signatures.plan()?;
        for trait_id in &plan.registry.trait_ids {
            self.checker.infer_work(0)?;
            let Some(item) = plan.trait_item(*trait_id, &name, &plan.budget)? else {
                continue;
            };
            if matches!(plan.declaration(item)?.receiver, ReceiverKind::Value { .. })
                && self.reference_impl_for(receiver, *trait_id)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn structural_trait_method_constraints(
        &mut self,
        receiver: Term,
        name: AstIdent,
        arguments: &[Term],
        explicit: &[Term],
        peeled: bool,
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
        // A by-value item on the referent of a borrowed receiver is not the
        // first probe step: the reference itself may implement the trait
        // (`IntoIterator for &Vec<T>`). Leave it to exact selection.
        if peeled
            && matches!(plan.declaration(item)?.receiver, ReceiverKind::Value { .. })
            && self.reference_impl_for(receiver, trait_id)?
        {
            return Ok(None);
        }
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
        let expected_type = self.expected_method_result(expected)?;
        let mut discovered = None;
        let mut unproved_expected = false;
        let mut deferred_inherent = false;
        let mut same_trait: Vec<(ir::TraitRef, ir::DeclId)> = Vec::new();
        // Reuse the ordinary receiver/adjustment and inherent-before-trait order.
        // Returning Deferred stops at the first applicable frontier, rather than
        // silently considering a lower-priority trait while its arguments wait.
        let _ = plan.select_method_with_arguments_scratch(
            receiver,
            &name,
            checker.facts(),
            (&checker.module, &plan.budget),
            &checker.trait_scratch,
            |owner, mode, traits_only| {
                if !traits_only {
                    if let Some((nominal, _)) = nominal_parts(owner)
                        && self.candidate(nominal, name)?.is_some()
                    {
                        deferred_inherent = true;
                        return Ok(MethodArguments::Deferred);
                    }
                    return Ok(MethodArguments::Missing);
                }
                let (candidates, unproved) = plan.method_trait_candidates_expected(
                    owner,
                    &name,
                    checker.facts(),
                    &checker.module,
                    &plan.budget,
                    expected_type.as_ref().map(|result| (result, mode)),
                )?;
                unproved_expected |= unproved;
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
                            .prove_goal_with(
                                &ir::Predicate::Implements(application.clone()),
                                facts,
                                &mut checker.trait_scratch.borrow_mut()
                            )
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
            if unproved_expected && !deferred_inherent {
                return Err(self.user_error(
                    "E0277: expected method result requires an unsatisfied trait application",
                ));
            }
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

    fn expected_method_result(&mut self, expected: Term) -> Result<Option<ir::Ty>, String> {
        let mut target = self.resolve(expected, 0)?;
        if target.is_none() {
            let root = self.root(expected);
            let mut found = None;
            for (actual, wanted) in self.coercions.clone() {
                self.checker.infer_work(0)?;
                if self.root(actual) == root
                    && let Some(ty) = self.resolve(wanted, 0)?
                {
                    if found.as_ref().is_some_and(|previous| *previous != ty) {
                        return Ok(None);
                    }
                    found = Some(ty);
                }
            }
            target = found;
        }
        Ok(target)
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
        let Some(expected) = self.expected_method_result(expected)? else {
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
        if chosen.is_none()
            && let Some((application, item)) = applications.first()
            && let ir::FactsCtx::Validated(facts) = self.checker.facts()
        {
            plan.reject_unproved_expected_trait_result(
                application,
                *item,
                &expected,
                facts,
                &plan.budget,
                &mut self.checker.trait_scratch.borrow_mut(),
            )?;
        }
        chosen
            .map(Some)
            .ok_or_else(super::super::trait_selection::trait_application_ambiguity)
    }
}
