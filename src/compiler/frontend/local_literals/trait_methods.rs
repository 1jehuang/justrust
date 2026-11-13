//! Discover generic trait-method constraints at the canonical receiver frontier.
//! These are inference terms only. No closure type, proof or executable target is
//! fabricated: ordinary method selection still discharges the final application.
use super::super::method_argument_selection::MethodArguments;
use super::*;
#[path = "nominal_impl_match.rs"]
mod nominal_match;

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
                // A projection argument (`Extend<Self::Item>`) whose inputs are
                // now known normalizes to its exact assignment, so impl heads
                // are matched against it rather than an opaque frontier.
                if self.reduce_associated_projection(*argument)? {
                    progress = true;
                }
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
                if let Some(header) =
                    self.structural_impl_with_arguments(receiver, trait_id, &arguments)?
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
            let Some(header) =
                self.structural_impl_with_arguments(receiver, trait_id, &arguments)?
            else {
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
    /// unique structurally compatible nominal or blanket implementation.
    /// These equations do not select an implementation: checked lowering must
    /// prove it later. Competing heads keep every unknown actual pending.
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
                    if self.blanket_bound_excludes_nominal(header, parameter, declaration)? {
                        continue;
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
            if patterns.len() != actuals.len() {
                return Ok(None);
            }
            let mut compatible = true;
            for (actual, pattern) in actuals.iter().zip(patterns) {
                compatible &=
                    self.impl_pattern_compatible(*actual, pattern, &header.schema_env, 0)?;
            }
            if !compatible {
                continue;
            }
            self.checker.infer_storage(1)?;
            if selected.replace(header.clone()).is_some() {
                return Ok(None);
            }
        }
        Ok(selected)
    }

    /// A blanket head `impl<I: Bound> Trait for I` cannot apply to a nominal
    /// that no implementation of `Bound` can ever cover: no `Bound` impl has
    /// that nominal or a bare parameter as its Self type. Such a head is no
    /// competitor, so `impl<I: Iterator> IntoIterator for I` does not hide
    /// `impl<T> IntoIterator for Vec<T>` while `T` is still open. This only
    /// removes a candidate for inference equations. Catalog (built-in and
    /// auto) bounds are never used as exclusion evidence, and checked lowering
    /// still proves the selected implementation.
    fn blanket_bound_excludes_nominal(
        &self,
        header: &tr::ImplHeader,
        parameter: ir::ParamId,
        nominal: ir::DeclId,
    ) -> Result<bool, String> {
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        for predicate in header.predicates.iter() {
            self.checker.infer_work(0)?;
            let ir::Predicate::Implements(bound) = predicate else {
                continue;
            };
            if bound.self_ty != ir::Ty::Param(parameter)
                || plan.catalog.declaration(bound.declaration).is_some()
            {
                continue;
            }
            let mut possible = false;
            for candidate in world.impl_headers() {
                self.checker.infer_work(0)?;
                if candidate.trait_ref.declaration != bound.declaration {
                    continue;
                }
                possible = match &candidate.trait_ref.self_ty {
                    ir::Ty::Param(_) | ir::Ty::AssociatedType(_) | ir::Ty::Opaque(_) => true,
                    self_ty => nominal_parts(self_ty).is_some_and(|(owner, _)| owner == nominal),
                };
                if possible {
                    break;
                }
            }
            if !possible {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn structural_impl_with_arguments(
        &self,
        receiver: Term,
        trait_id: ir::DeclId,
        arguments: &[Term],
    ) -> Result<Option<tr::ImplHeader>, String> {
        let plan = self.checker.signatures.plan()?;
        let mut selected = None;
        for header in plan.trait_world()?.impl_headers() {
            self.checker.infer_work(0)?;
            if header.trait_ref.declaration != trait_id
                || !self.impl_head_compatible(receiver, arguments, header)?
            {
                continue;
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
        // An open nominal can expose its source-owned associated assignment
        // without first materializing all actuals. This links equations such
        // as IntoIter<Result<T, ?E>>::Item = Result<T, ?E> so an independently
        // known consumer may constrain ?E. The unique compatible source impl
        // below is still only an equation, never a membership proof.
        // An unsuffixed literal actual (`IterMut<{integer}>`) is still open:
        // resolving it would apply the i32 default before later uses decide.
        let open_nominal = matches!(self.domain(receiver), Domain::Nominal(..))
            && (self.resolve(receiver, depth + 1)?.is_none()
                || !self.manifest_ready_with_defaults(&[receiver], depth + 1, false)?);
        if !open_nominal && !self.contains_callable(receiver, depth + 1)? {
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
        self.structural_trait_call_constraints(receiver, name, arguments, explicit, peeled, false)
    }

    /// Associated syntax supplies an explicit receiver. Use its exact source
    /// formal, not a guess based on the first argument's type or name.
    pub(super) fn associated_trait_receiver_constraints(
        &mut self,
        receiver: Term,
        name: AstIdent,
        arguments: &[Term],
        explicit: &[Term],
    ) -> Result<Option<Term>, String> {
        self.structural_trait_call_constraints(receiver, name, arguments, explicit, false, true)
    }

    fn structural_trait_call_constraints(
        &mut self,
        receiver: Term,
        name: AstIdent,
        arguments: &[Term],
        explicit: &[Term],
        peeled: bool,
        associated: bool,
    ) -> Result<Option<Term>, String> {
        let plan = self.checker.signatures.plan()?;
        let implicit = plan.program.implicit_prelude(self.checker.module)?;
        let fixed_owner = if associated {
            self.resolve(receiver, 0)?
        } else {
            None
        };
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
            if !visible
                || (!associated && matches!(plan.declaration(item)?.receiver, ReceiverKind::None))
            {
                continue;
            }
            let header = self.structural_impl(receiver, *trait_id)?;
            if let (Some(owner), ir::FactsCtx::Validated(facts)) =
                (&fixed_owner, self.checker.facts())
            {
                let (applications, open) =
                    plan.inferred_trait_applications(owner, *trait_id, facts, &plan.budget)?;
                // An open application is not absence evidence. Only a closed
                // frontier whose every exact goal is unproved can be omitted.
                if !open {
                    let mut proved = false;
                    for application in applications.as_slice() {
                        self.checker.infer_work(0)?;
                        if matches!(
                            facts
                                .service()
                                .prove_goal(&ir::Predicate::Implements(application.clone()), facts,)
                                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?,
                            tr::ProofOutcome::Proved(_)
                        ) {
                            proved = true;
                        }
                    }
                    if !proved {
                        continue;
                    }
                }
            }
            if header.is_none() {
                if !associated && !self.numeric_nominal_actuals(receiver)? {
                    continue;
                }
                let Domain::Nominal(nominal, _) = self.domain(receiver) else {
                    continue;
                };
                if !associated && plan.param_env(*trait_id)?.arity() != 1 {
                    continue;
                }
                let mut exists = false;
                for candidate in plan.trait_world()?.impl_headers() {
                    self.checker.infer_work(0)?;
                    // For associated syntax the receiver is an explicit source
                    // parameter. All impls of one trait share that schema, so
                    // its equations do not require choosing any specialized
                    // head. Final checked selection decides applicability.
                    if candidate.trait_ref.declaration == *trait_id
                        && (nominal_parts(&candidate.trait_ref.self_ty)
                            .is_some_and(|(owner, _)| owner == nominal)
                            || (associated
                                && matches!(candidate.trait_ref.self_ty,
                                    ir::Ty::Param(p) if candidate.schema_env.contains(p))))
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
        let skip = usize::from(!associated);
        if schema.signature.params.len() != arguments.len() + skip
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
        for (actual, formal) in arguments
            .iter()
            .zip(schema.signature.params.iter().skip(skip))
        {
            let formal = self.schema(formal, &params, 0)?;
            self.coerce(*actual, formal)?;
        }
        // The item schema alone does not relate Self to the trait's source
        // argument. Retain that application so source impl-head equations can
        // infer both together after the call and its expected result are known.
        if associated && header.is_none() {
            let mut arguments = Vec::new();
            for index in 1..owner.arity() {
                let parameter = ir::ParamId {
                    owner: owner.binder(),
                    index: index as u16,
                };
                arguments.push(
                    *params
                        .get(&parameter)
                        .ok_or("missing trait argument term")?,
                );
            }
            if !arguments.is_empty() {
                self.checker.infer_storage(1)?;
                self.trait_arguments.push((trait_id, receiver, arguments));
            }
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
        // Method bodies, generic and opaque-result callables use the ordinary
        // solver rather than nominal preparation. Their trait-method results
        // (`s.parse()` -> `Result<F, F::Err>`) need the same canonical schema
        // constraints, or closure inputs downstream (`.map_err(|e| ..)`)
        // stay uninferable. Selection below still proves every application.
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
        let mut open_trait = false;
        let mut open_template = None;
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
                let mut open = Vec::new();
                let (candidates, unproved) = plan.method_trait_candidates_open(
                    owner,
                    &name,
                    checker.facts(),
                    &checker.module,
                    &plan.budget,
                    expected_type.as_ref().map(|result| (result, mode)),
                    Some(&mut open),
                )?;
                unproved_expected |= unproved;
                if !open.is_empty() {
                    // A trait argument the receiver leaves free (`U` in
                    // `impl<T, U> Map<T, U> for [T; N]`) becomes a fresh
                    // inference term below. Only one such trait, with no
                    // exact candidate beside it, contributes equations;
                    // the checked call still selects and proves it.
                    open_trait = true;
                    // Same-named methods of unrelated traits (`Iterator::map`)
                    // are candidates only when their application is proved.
                    let mut exact = false;
                    for (application, item) in candidates.as_slice() {
                        checker.infer_work(0)?;
                        if receiver_mode(plan.declaration(*item)?.receiver) != receiver_mode(mode) {
                            continue;
                        }
                        exact |= !matches!(
                            facts
                                .service()
                                .prove_goal_with(
                                    &ir::Predicate::Implements(application.clone()),
                                    facts,
                                    &mut checker.trait_scratch.borrow_mut()
                                )
                                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?,
                            tr::ProofOutcome::NotProven
                        );
                    }
                    // An open trait whose impl head does not match this
                    // receiver in this mode contributes nothing; ordinary
                    // candidates proceed as without it.
                    let mut templates = Vec::new();
                    for (trait_id, item) in open {
                        checker.infer_work(0)?;
                        if receiver_mode(plan.declaration(item)?.receiver) == receiver_mode(mode)
                            && let Some(template) =
                                plan.open_trait_application(owner, trait_id, facts, &plan.budget)?
                        {
                            templates.push((template, item));
                        }
                    }
                    if !templates.is_empty() {
                        if templates.len() == 1 && !exact {
                            open_template = templates.pop();
                        }
                        return Ok(MethodArguments::Deferred);
                    }
                    open_trait = false;
                }
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
        let mut free = Vec::new();
        if open_trait {
            let Some(((template, open_free), item)) = open_template else {
                return Ok(None);
            };
            free = open_free;
            discovered = Some((template, item));
        }
        if !same_trait.is_empty() {
            discovered = self.select_application_by_arguments(&same_trait, arguments)?;
        }
        if discovered.is_none() && !same_trait.is_empty() {
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
            let term = match actual {
                ir::Ty::Param(parameter) if index > 0 && free.contains(parameter) => {
                    self.fresh()?
                }
                _ => self.fixed(actual.clone())?,
            };
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

    /// Several applications of one trait (`Take<i32>`, `Take<bool>`): the
    /// unique one whose bound formals equal the arguments' already fixed
    /// types. Literal-only or otherwise undecided arguments choose nothing.
    fn select_application_by_arguments(
        &mut self,
        applications: &[(ir::TraitRef, ir::DeclId)],
        arguments: &[Term],
    ) -> Result<Option<(ir::TraitRef, ir::DeclId)>, String> {
        let plan = self.checker.signatures.plan()?;
        let mut chosen = None;
        for (application, item) in applications {
            self.checker.infer_work(0)?;
            let schema = plan.raw_schema(*item)?;
            if schema.signature.params.len() != arguments.len() + 1 {
                return Ok(None);
            }
            let owner = plan.param_env(application.declaration)?;
            let mut bindings = HashMap::new();
            for (index, actual) in std::iter::once(&application.self_ty)
                .chain(application.args.iter())
                .enumerate()
            {
                bindings.insert(
                    ir::ParamId {
                        owner: owner.binder(),
                        index: index as u16,
                    },
                    actual.clone(),
                );
            }
            let mut fits = true;
            let own = plan.param_env(*item)?;
            let mut bound_own = false;
            for (argument, formal) in arguments.iter().zip(schema.signature.params.iter().skip(1)) {
                let formal =
                    super::super::trait_method_hint::bind_owner(formal, &bindings, &plan.budget)?;
                // Unsuffixed integer literals inside the argument read as
                // this application's own integer argument (`Extend<u8>`), as
                // stock unifies `{integer}` with the candidate's item type.
                // Other applications see the i32 default.
                let literal = application.args.iter().find_map(|arg| match arg {
                    ir::Ty::Int(int) => Some(*int),
                    _ => None,
                });
                self.integer_default.set(literal.unwrap_or(ir::IntTy::I32));
                let resolved = match self.domain(*argument) {
                    Domain::Fixed(actual) => Ok(Some(actual)),
                    _ => self.resolve(*argument, 0),
                };
                self.integer_default.set(ir::IntTy::I32);
                let Some(actual) = resolved? else {
                    return Ok(None);
                };
                if super::super::inferred_owner::mentions_param(&actual) {
                    return Ok(None);
                }
                if let Some(fit) = super::super::trait_method_hint::bind_own_formal(
                    &formal,
                    own.binder(),
                    &actual,
                    &mut bindings,
                ) {
                    self.checker.infer_storage(1)?;
                    fits &= fit;
                    bound_own = true;
                    continue;
                }
                if super::super::inferred_owner::mentions_param(&formal) {
                    return Ok(None);
                }
                fits &= actual == formal;
            }
            if fits && bound_own {
                let ir::FactsCtx::Validated(facts) = self.checker.facts() else {
                    return Ok(None);
                };
                fits = super::super::trait_method_hint::own_bounds_hold(
                    &schema.predicates,
                    &bindings,
                    facts,
                    &self.checker.trait_scratch,
                    &plan.budget,
                )?;
            }
            if fits && chosen.replace((application.clone(), *item)).is_some() {
                return Ok(None);
            }
        }
        // Every application was decided by closed argument types and none
        // fits: no inference can make the call hold (`Vec<i32>::extend` with
        // a `Vec<u32>`), as stock's unsatisfied `Extend<_>` obligation.
        if chosen.is_none() && !applications.is_empty() {
            return Err(self.user_error(
                "E0277: no applicable trait application accepts the method arguments",
            ));
        }
        Ok(chosen)
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
