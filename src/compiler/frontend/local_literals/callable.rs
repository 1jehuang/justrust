//! Parent-body callable constraints are hints, never checked closure authority.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct CallableTerm {
    pub key: PathKey,
    pub params: Vec<Term>,
    pub result: Term,
}

impl<'a: 's, 'b: 's, 's> Solver<'a, 'b, 's> {
    pub(super) fn stored_callable(
        &mut self,
        path: &AstPath,
        depth: usize,
    ) -> Result<Option<Term>, String> {
        let checker = self.checker;
        let plan = checker.signatures.plan()?;
        let Some(syntax) = plan.stored_closure_syntax(path)? else {
            return Ok(None);
        };
        let key = checker.path_key(path, Namespace::Value)?;
        if let Some((_, term)) = self
            .closure_terms
            .iter()
            .find(|(candidate, _)| *candidate == key)
        {
            return Ok(Some(*term));
        }
        let params = syntax
            .params
            .iter()
            .map(|(_, ty)| self.source_ty(ty))
            .collect::<Result<Vec<_>, _>>()?;
        let result = self.source_ty(&syntax.result)?;
        let term = self.node(Domain::Callable(CallableTerm {
            key: key.clone(),
            params: params.clone(),
            result,
        }))?;
        if let Some(previous) = checker.closure_signature_hints.get(&key) {
            if previous.params.len() != params.len() {
                return Err("closure signature hint arity changed".into());
            }
            for (term, ty) in params.iter().zip(&previous.params) {
                let expected = self.fixed(ty.clone())?;
                self.unify(*term, expected)?;
            }
            let expected = self.fixed(previous.result.clone())?;
            self.unify(result, expected)?;
        }
        self.closure_terms.push((key, term));
        let previous_context = self.local_context;
        self.local_context = Some(syntax.lexical_scope);
        self.return_terms.push(result);
        self.scopes.push(HashMap::new());
        for ((pattern, _), term) in syntax.params.iter().zip(&params) {
            self.pattern(pattern, *term)?;
        }
        let body = self.block(&syntax.body, depth + 1)?;
        if syntax.body.tail.is_some() {
            self.coerce(body, result)?;
        } else {
            self.unit_defaults.push(result);
        }
        self.scopes.pop();
        self.return_terms.pop();
        self.local_context = previous_context;
        Ok(Some(term))
    }

    pub(super) fn closure_hints(&self) -> Result<HashMap<PathKey, ir::Signature>, String> {
        #[expect(
            clippy::mutable_key_type,
            reason = "PathKey equality and hashing use immutable path identity, never the retained allocation budget"
        )]
        let mut output = HashMap::new();
        for (key, term) in &self.closure_terms {
            let Domain::Callable(callable) = self.domain(*term) else {
                continue;
            };
            let mut params = Vec::new();
            for (index, param) in callable.params.into_iter().enumerate() {
                params.push(self.resolve(param, 0)?.ok_or_else(|| {
                    let owner = self.checker.declaration.and_then(|declaration| {
                        let plan = self.checker.signatures.plan().ok()?;
                        let item = plan.declaration(declaration).ok()?.path.item;
                        let DeclItemName::Source(name) = item else { return None; };
                        plan.program.names.spelling(name).ok().map(str::to_owned)
                    });
                    self.user_error(&format!(
                        "E0282: type annotations needed for closure parameter {index} in {} at {key:?}",
                        owner.as_deref().unwrap_or("source body"),
                    ))
                })?);
            }
            let result = self.resolve(callable.result, 0)?.ok_or_else(|| {
                self.user_error("E0282: type annotations needed for closure result")
            })?;
            output.insert(
                key.clone(),
                ir::Signature {
                    params,
                    result,
                    ref_contract: Arc::new(ir::RefContract::default()),
                },
            );
        }
        Ok(output)
    }

    pub(super) fn callable_method_application(
        &mut self,
        receiver: Term,
        name: AstIdent,
        args: &[Term],
    ) -> Result<Option<Term>, String> {
        let receiver = self.peel_refs(receiver);
        if !matches!(
            self.domain(receiver),
            Domain::Callable(_)
                | Domain::Opaque(_, _)
                | Domain::Fixed(ir::Ty::Closure(_) | ir::Ty::FnItem(_))
        ) {
            return Ok(None);
        }
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        let names = plan.program.names.as_ref();
        let mut canonical = false;
        for (index, declaration) in plan.registry.trait_ids.iter().enumerate() {
            if world
                .closure_trait_kind(*declaration)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?
                != Some(ir::ClosureKind::Fn)
            {
                continue;
            }
            let item = *plan.registry.trait_items[index]
                .first()
                .ok_or("canonical callable trait lacks invocation member")?;
            let DeclItemName::Source(member) = plan.declaration(item)?.path.item else {
                return Err("callable member has no source identity".into());
            };
            canonical = crate::compiler::semantic_name::occurrence_index(names, member)
                .map_err(native_adapter::name_diagnostic)?
                == crate::compiler::semantic_name::occurrence_index(names, name)
                    .map_err(native_adapter::name_diagnostic)?;
            break;
        }
        if !canonical {
            return Ok(None);
        }
        if let Domain::Opaque(descriptor, _) = self.domain(receiver)
            && self.opaque_excludes_callable_bounds(&descriptor)?
        {
            return Err(self.user_error(
                "E0618: expected a callable value, found opaque type without a callable bound",
            ));
        }
        if args.len() != 1 {
            return Err(self.user_error("E0057: wrong number of callable tuple arguments"));
        }
        match self.domain(args[0]) {
            Domain::Tuple(fields) => self.callable_application(receiver, &fields),
            Domain::Fixed(ir::Ty::Unit) => self.callable_application(receiver, &[]),
            _ => Ok(None),
        }
    }

    pub(super) fn callable_application(
        &mut self,
        term: Term,
        args: &[Term],
    ) -> Result<Option<Term>, String> {
        match self.domain(term) {
            Domain::Opaque(descriptor, children) => {
                self.opaque_callable_application(&descriptor, &children, args)
            }
            Domain::Callable(callable) => {
                if callable.params.len() != args.len() {
                    return Err(self.user_error("E0057: wrong number of closure arguments"));
                }
                for (actual, expected) in args.iter().zip(&callable.params) {
                    self.coerce(*actual, *expected)?;
                }
                Ok(Some(callable.result))
            }
            Domain::Fixed(ir::Ty::Closure(closure)) => {
                if closure.params().len() != args.len() {
                    return Err(self.user_error("E0057: wrong number of closure arguments"));
                }
                for (actual, expected) in args.iter().zip(closure.params()) {
                    let expected = self.fixed(expected.clone())?;
                    self.coerce(*actual, expected)?;
                }
                self.fixed(closure.result().clone()).map(Some)
            }
            Domain::Fixed(ir::Ty::FnItem(item)) => {
                let signature = self.checker.signatures.plan()?.instantiate_signature(
                    &item,
                    self.checker.facts(),
                    &self.checker.signatures.plan()?.budget,
                )?;
                if signature.params.len() != args.len() {
                    return Err(self.user_error("E0057: wrong number of function arguments"));
                }
                for (actual, expected) in args.iter().zip(&signature.params) {
                    let expected = self.fixed(expected.clone())?;
                    self.coerce(*actual, expected)?;
                }
                self.fixed(signature.result.clone()).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub(super) fn callable_bounds(
        &mut self,
        declaration: ir::DeclId,
        params: &HashMap<ir::ParamId, Term>,
    ) -> Result<(), String> {
        let plan = self.checker.signatures.plan()?;
        let facts = plan.callable_facts(declaration)?;
        self.callable_predicates(facts.predicates().predicates(), params)
    }

    pub(super) fn callable_predicates(
        &mut self,
        predicates: &[ir::Predicate],
        params: &HashMap<ir::ParamId, Term>,
    ) -> Result<(), String> {
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        for predicate in predicates {
            let trait_ref = predicate.trait_ref();
            if world
                .closure_trait_kind(trait_ref.declaration)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?
                .is_none()
            {
                if !trait_ref.args.is_empty() {
                    let receiver = self.schema(&trait_ref.self_ty, params, 0)?;
                    let mut arguments = Vec::new();
                    for argument in trait_ref.args.iter() {
                        self.checker.infer_storage(1)?;
                        arguments.push(self.schema(argument, params, 0)?);
                    }
                    self.checker.infer_storage(1)?;
                    self.trait_arguments
                        .push((trait_ref.declaration, receiver, arguments));
                }
                continue;
            }
            let term = self.schema(&trait_ref.self_ty, params, 0)?;
            let peeled = self.peel_refs(term);
            let callable = match self.domain(peeled) {
                Domain::Callable(callable) => callable,
                Domain::Opaque(_, _) | Domain::Fixed(ir::Ty::Opaque(_)) => {
                    let Some(ir::Ty::Opaque(opaque)) = self.resolve(peeled, 0)? else {
                        continue;
                    };
                    for _ in opaque.bounds() {
                        self.checker.infer_work(0)?;
                    }
                    let Some(bound) = opaque.bounds().iter().find(|bound| {
                        bound
                            .application(ir::Ty::Opaque(opaque.clone()))
                            .declaration
                            == trait_ref.declaration
                    }) else {
                        continue;
                    };
                    let application = bound.application(ir::Ty::Opaque(opaque.clone()));
                    if let (Some(expected), Some(actual)) =
                        (trait_ref.args.first(), application.args.first())
                    {
                        let expected = self.schema(expected, params, 0)?;
                        let actual = self.fixed(actual.clone())?;
                        self.unify(expected, actual)?;
                    }
                    if let ir::Predicate::ProjectionEq { projection, ty } = predicate {
                        for _ in bound.equalities() {
                            self.checker.infer_work(0)?;
                        }
                        if let Some((_, result)) = bound
                            .equalities()
                            .iter()
                            .find(|(item, _)| *item == projection.item)
                        {
                            let expected = self.schema(ty, params, 0)?;
                            let actual = self.fixed(result.clone())?;
                            self.coerce(actual, expected)?;
                        }
                    }
                    continue;
                }
                _ => continue,
            };
            if let Some(arguments) = trait_ref.args.first() {
                let arguments = self.schema(arguments, params, 0)?;
                let tuple = self.node(Domain::Tuple(callable.params.clone()))?;
                self.unify(arguments, tuple)?;
            }
            if let ir::Predicate::ProjectionEq { ty, .. } = predicate {
                let output = self.schema(ty, params, 0)?;
                self.coerce(callable.result, output)?;
            }
        }
        Ok(())
    }

    pub(super) fn inference_key(
        &self,
        input: &impl LocalInput,
        declaration: bool,
    ) -> Result<Option<LocalKey>, String> {
        let Some(context) = self.local_context else {
            return self.checker.local_key(input, declaration);
        };
        let names = self.checker.names()?;
        let Some(function) = context else {
            return input.checked_key(names);
        };
        let Some(occurrence) = input.occurrence(names)? else {
            return input.checked_key(names);
        };
        let arena = self
            .checker
            .signatures
            .plan()?
            .program
            .scopes
            .as_ref()
            .ok_or("missing closure scope arena")?;
        let binding = if declaration {
            Some(
                arena
                    .declaration_for(names, occurrence)
                    .map_err(|e| format!("closure scope: {e:?}"))?,
            )
        } else {
            arena
                .reference_for(names, occurrence)
                .map_err(|e| format!("closure scope: {e:?}"))?
        };
        let Some(binding) = binding else {
            return Ok(None);
        };
        let key = LocalKey::Anchored(binding);
        let owner = arena
            .binding_function(binding)
            .map_err(|e| format!("closure scope: {e:?}"))?;
        if owner != function
            && (declaration || !self.scopes.iter().any(|scope| scope.contains_key(&key)))
        {
            return Err("closure inference binding belongs to another body".into());
        }
        Ok(Some(key))
    }
}
