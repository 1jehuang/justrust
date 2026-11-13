//! Infer only a required trait member's own generic parameters. The trait
//! application and its ordered owner actuals are fixed before inference starts.
use super::*;

impl Checker<'_> {
    /// Candidate discovery is not membership. Inapplicable same-named traits
    /// may be skipped, but metadata and source-nominal admission errors remain
    /// hard failures in this exact caller and planning authority.
    pub(super) fn trait_method_application_is_proven(
        &self,
        application: &ir::TraitRef,
    ) -> Result<bool, String> {
        let plan = self.signatures.plan()?;
        let ir::FactsCtx::Validated(facts) = self.facts() else {
            return Err("trait method inference requires canonical caller facts".into());
        };
        let mut nominals = Vec::new();
        for actual in std::iter::once(&application.self_ty).chain(application.args.iter()) {
            let inputs = facts
                .parent_local_receiver_inputs(actual)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
            self.infer_storage(inputs.len())?;
            nominals
                .try_reserve(inputs.len())
                .map_err(|_| "trait inference nominal input allocation failed")?;
            nominals.extend(inputs);
        }
        Ok(matches!(
            facts
                .service()
                .prove_goal_with_nominals(
                    &ir::Predicate::Implements(application.clone()),
                    facts,
                    &nominals,
                    &mut tr::TraitScratch::default(),
                )
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?,
            tr::ProofOutcome::Proved(_)
        ))
    }

    pub(super) fn inferred_trait_method_actuals(
        &self,
        application: &ir::TraitRef,
        name: &AstIdent,
        explicit: &[Type],
        arguments: &[Expr],
        expected: Option<&ir::Ty>,
        dot: bool,
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        let plan = self.signatures.plan()?;
        let item = plan
            .trait_item(application.declaration, name, &plan.budget)?
            .ok_or("unknown required trait method")?;
        if dot && plan.declaration(item)?.receiver == ReceiverKind::None {
            return Err("associated function without self is not dot-callable".into());
        }
        let own = plan.param_env(item)?;
        if !explicit.is_empty() && explicit.len() != own.arity() {
            return Err("E0107: wrong number of trait method generic arguments".into());
        }
        if own.arity() == 0 {
            return Ok(Some(Vec::new()));
        }
        if !self.trait_method_application_is_proven(application)? {
            return Err("trait obligation not proven".into());
        }
        let owner = plan.param_env(application.declaration)?;
        if owner.arity() != application.args.len() + 1 {
            return Err("trait argument arity mismatch".into());
        }
        self.infer_storage(owner.arity())?;
        let mut bindings = HashMap::new();
        for (index, actual) in std::iter::once(&application.self_ty)
            .chain(application.args.iter())
            .enumerate()
        {
            self.facts().validate_ty_with_budget(actual, &plan.budget)?;
            bindings.insert(
                ir::ParamId {
                    owner: owner.binder(),
                    index: index as u16,
                },
                actual.clone(),
            );
        }
        // This is the original source schema, used only to build inference
        // terms. It grants no executable authority. Final select_trait checks
        // the complete application, inferred method actuals and obligations.
        let schema = plan.raw_schema(item)?;
        let offset = usize::from(dot);
        if schema.signature.params.len() != arguments.len() + offset {
            return Err("trait method argument count mismatch".into());
        }
        let mut solver = Inference::new(self, own.clone())?;
        for (slot, actual) in explicit.iter().enumerate() {
            let formal = self.infer_node(InferKind::Slot(slot))?;
            let actual = self.partial_type_term(actual, 0)?;
            solver.constrain(&formal, &actual, 0)?;
        }
        self.infer_storage(arguments.len() + usize::from(expected.is_some()))?;
        let mut constraints = Vec::new();
        if let Some(expected) = expected {
            let formal =
                self.method_owner_term(&solver.schema(&schema.signature.result, 0)?, &bindings, 0)?;
            let evidence = self.infer_type_term(expected, None, 0)?;
            constraints.push((formal, evidence));
        }
        // Literal-only producers (`vec![200]`) for a bare method parameter
        // are re-inferred under the fixed application's integer argument
        // (`Extend<u8>`), as stock unifies `{integer}` with the item type.
        let literal = LiteralDefault::set(self, application);
        for (raw_formal, argument) in schema.signature.params[offset..].iter().zip(arguments) {
            let formal = self.method_owner_term(&solver.schema(raw_formal, 0)?, &bindings, 0)?;
            let mut evidence = self.inference_evidence(argument, 0)?;
            // A literal-only generic producer (`vec![2, 3]`) for a bare
            // method parameter (`I` in `extend<I: IntoIterator<Item = A>>`)
            // takes its numeric default only when the method's own bounds
            // then hold under this fixed application.
            if matches!(evidence.as_ref(), InferKind::Unknown(_))
                && let ir::Ty::Param(parameter) = raw_formal
                && parameter.owner == own.binder()
                && let Some(defaulted) = self.defaulted_call_type(argument, 0)?
                && let ir::FactsCtx::Validated(facts) = self.facts()
            {
                let mut bound = bindings.clone();
                bound.insert(*parameter, defaulted.clone());
                if own_bounds_hold(
                    &schema.predicates,
                    &bound,
                    facts,
                    &self.trait_scratch,
                    &plan.budget,
                )? {
                    evidence = self.infer_type_term(&defaulted, None, 0)?;
                }
            }
            let (formal, evidence, _) = sr::call_input_equation(&formal, &evidence);
            constraints.push((formal.clone(), evidence.clone()));
        }
        drop(literal);
        if !solver.constrain_projected_terms(constraints, true)? {
            return Ok(None);
        }
        // `F: FnMut(Self::Item) -> B`: once the closure argument and the
        // owner's Self are known, the normalized `Output` is evidence for B.
        for predicate in schema.predicates.iter() {
            if let ir::Predicate::ProjectionEq { projection, ty } = predicate
                && !matches!(ty, ir::Ty::AssociatedType(_))
                && inferred_owner::mentions_param(ty)
            {
                let projection = bind_owner(
                    &ir::Ty::AssociatedType(Arc::new(projection.clone())),
                    &bindings,
                    &self.signatures.plan()?.budget,
                )?;
                let ty = bind_owner(ty, &bindings, &self.signatures.plan()?.budget)?;
                solver.constrain_projection_equality(&projection, &ty, Some(&bindings), true)?;
            }
        }
        solver.finish(true)
    }
}

impl Checker<'_> {
    /// Whether every argument's settled type equals its formal under the
    /// trait application `app`. `None` when any formal stays generic or any
    /// argument is not yet a closed type: the caller must not decide then.
    pub(super) fn trait_application_fits_arguments(
        &self,
        app: &ir::TraitRef,
        item: ir::DeclId,
        arguments: &[Expr],
    ) -> Result<Option<bool>, String> {
        let plan = self.signatures.plan()?;
        let owner = plan.param_env(app.declaration)?;
        if owner.arity() != app.args.len() + 1 {
            return Err("trait argument arity mismatch".into());
        }
        let schema = plan.raw_schema(item)?;
        if schema.signature.params.len() != arguments.len() + 1 {
            return Ok(None);
        }
        self.infer_storage(owner.arity())?;
        let mut bindings = HashMap::new();
        for (index, actual) in std::iter::once(&app.self_ty)
            .chain(app.args.iter())
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
        // `{integer}` evidence reads as this application's own integer
        // argument (`Extend<u8>`), as stock unifies the literal variable
        // with the candidate's item type. Other applications use i32.
        // `{integer}` evidence (`[1, 2]`, `vec![200]`) is ranked under this
        // application's own integer argument (`Extend<u8>`), as stock
        // unifies the literal variable with the candidate's item type.
        let _literal = LiteralDefault::set(self, app);
        let solver = Inference::empty(self)?;
        let own = plan.param_env(item)?;
        let mut fits = true;
        let mut bound_own = false;
        let mut defaulted = false;
        for (formal, argument) in schema.signature.params[1..].iter().zip(arguments) {
            plan.budget.charge_work(1)?;
            let formal = bind_owner(formal, &bindings, &plan.budget)?;
            let evidence = self.inference_evidence(argument, 0)?;
            // An unsuffixed literal element (`vec![2, 3]`) is ranked under
            // its numeric default. A defaulted mismatch only leaves the
            // choice undecided; final selection proves the chosen one again.
            let actual = match solver.resolve(&evidence, false, 0)? {
                Some(actual) => actual,
                None => match solver.resolve(&evidence, true, 0)? {
                    Some(actual) => {
                        defaulted = true;
                        actual
                    }
                    None => match self.defaulted_call_type(argument, 0)? {
                        Some(actual) => {
                            defaulted = true;
                            actual
                        }
                        None => return Ok(None),
                    },
                },
            };
            if inferred_owner::mentions_param(&actual) {
                return Ok(None);
            }
            if let Some(fit) = bind_own_formal(&formal, own.binder(), &actual, &mut bindings) {
                self.infer_storage(1)?;
                fits &= fit;
                bound_own = true;
                continue;
            }
            if inferred_owner::mentions_param(&formal) {
                return Ok(None);
            }
            fits &= actual == formal || self.nominal_input_types_equal(&actual, &formal)?;
        }
        if fits && bound_own {
            let ir::FactsCtx::Validated(facts) = self.facts() else {
                return Ok(None);
            };
            fits = own_bounds_hold(
                &schema.predicates,
                &bindings,
                facts,
                &self.trait_scratch,
                &plan.budget,
            )?;
        }
        // A defaulted literal only rules out applications no numeric type
        // could satisfy (`Extend<&i32>` against `{integer}` items). Against
        // a numeric trait argument (`Extend<u8>`) the default decides nothing.
        if defaulted
            && !fits
            && app
                .args
                .iter()
                .any(|arg| matches!(arg, ir::Ty::Int(_) | ir::Ty::Float(_)))
        {
            return Ok(None);
        }
        Ok(Some(fits))
    }

    /// The numeric-defaulted type of a generic call argument whose own
    /// arguments are literal-only (`vec![2, 3]` expands to a block binding
    /// `__jr_vec_seed(2, 2usize)`). Used only to rank candidate trait
    /// applications; it never becomes the argument's type.
    fn defaulted_call_type(&self, expr: &Expr, depth: usize) -> Result<Option<ir::Ty>, String> {
        self.infer_work(depth)?;
        match expr {
            Expr::Block(block) => {
                if let (
                    Some(Stmt::Let {
                        name: BindingName::Named(bound),
                        ty: Type::Infer,
                        value,
                        ..
                    }),
                    Some(Expr::Var(path)),
                ) = (block.statements.first(), block.tail.as_deref())
                    && matches!(path.anchor(), PathAnchor::Relative)
                    && path.segments().len() == 1
                    && path.checked_key(self.names()?)? == bound.checked_key(self.names()?)?
                    && !block.statements[1..]
                        .iter()
                        .any(|stmt| matches!(stmt, Stmt::Let { .. }))
                {
                    return self.defaulted_call_type(value, depth + 1);
                }
                Ok(None)
            }
            Expr::Call { name, args } => {
                let plan = self.signatures.plan()?;
                let Some(declaration) = plan
                    .registry
                    .functions
                    .get(&self.path_key(name, Namespace::Value)?)
                    .copied()
                else {
                    return Ok(None);
                };
                if plan.param_env(declaration)?.arity() == 0 {
                    return Ok(None);
                }
                let Some(type_args) = self.infer_call_args(declaration, args, None, true)? else {
                    return Ok(None);
                };
                Ok(Some(
                    plan.source_inference_signature(
                        &ir::CallTarget {
                            declaration,
                            type_args: type_args.into(),
                        },
                        self.facts(),
                        &plan.budget,
                    )?
                    .result,
                ))
            }
            _ => Ok(None),
        }
    }

    /// Complete an open trait application (`template`, whose `free` impl
    /// parameters stand bare in its trait arguments) from the call itself.
    /// A free argument is fixed only by direct evidence: an argument whose
    /// formal is exactly that trait parameter, an expected result that is
    /// exactly it, or a callable `Output` equality (`F: FnMut(T) -> U`) whose
    /// projection is closed once the method's own slots are inferred. Every
    /// piece of evidence must agree. Anything else returns `None` and keeps the
    /// ordinary boundary. The completed application is proved again by the
    /// caller and by final selection.
    pub(super) fn infer_open_trait_application(
        &self,
        template: &ir::TraitRef,
        free: &[ir::ParamId],
        item: ir::DeclId,
        explicit: &[Type],
        arguments: &[Expr],
        expected: Option<&ir::Ty>,
    ) -> Result<Option<ir::TraitRef>, String> {
        let plan = self.signatures.plan()?;
        let owner = plan.param_env(template.declaration)?;
        if owner.arity() != template.args.len() + 1 {
            return Err("trait argument arity mismatch".into());
        }
        let own = plan.param_env(item)?;
        if !explicit.is_empty() && explicit.len() != own.arity() {
            return Err("E0107: wrong number of trait method generic arguments".into());
        }
        // Trait parameter -> slot in `free`, for each open trait argument.
        self.infer_storage(owner.arity())?;
        let mut open = Vec::new();
        let mut bindings = HashMap::new();
        for (index, actual) in std::iter::once(&template.self_ty)
            .chain(template.args.iter())
            .enumerate()
        {
            let parameter = ir::ParamId {
                owner: owner.binder(),
                index: index as u16,
            };
            match actual {
                ir::Ty::Param(p) if index > 0 && free.contains(p) => {
                    open.push((parameter, *p));
                }
                _ => {
                    if inferred_owner::mentions_param(actual) {
                        return Ok(None);
                    }
                    self.facts().validate_ty_with_budget(actual, &plan.budget)?;
                    bindings.insert(parameter, actual.clone());
                }
            }
        }
        if open.len() != free.len() {
            return Ok(None);
        }
        let is_open = |ty: &ir::Ty| -> bool {
            matches!(ty, ir::Ty::Param(p) if open.iter().any(|(q, _)| q == p))
        };
        // Whether `ty` mentions any open trait parameter. Conservative: any
        // shape this cannot inspect counts as mentioning one.
        fn mentions_open(ty: &ir::Ty, open: &[(ir::ParamId, ir::ParamId)]) -> bool {
            match ty {
                ir::Ty::Param(p) => open.iter().any(|(q, _)| q == p),
                ir::Ty::Ref { referent, .. } => mentions_open(referent, open),
                ir::Ty::RawPtr { pointee, .. } => mentions_open(pointee, open),
                ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
                    mentions_open(element, open)
                }
                ir::Ty::Tuple(fields) => fields.iter().any(|t| mentions_open(t, open)),
                ir::Ty::AssociatedType(p) => {
                    mentions_open(&p.trait_ref.self_ty, open)
                        || p.trait_ref.args.iter().any(|t| mentions_open(t, open))
                }
                ir::Ty::Int(_)
                | ir::Ty::Float(_)
                | ir::Ty::Bool
                | ir::Ty::Char
                | ir::Ty::Str
                | ir::Ty::Unit
                | ir::Ty::Never => false,
                ir::Ty::Struct(t) => t.def().type_args.iter().any(|t| mentions_open(t, open)),
                ir::Ty::Enum(t) => t.def().type_args.iter().any(|t| mentions_open(t, open)),
                _ => !open.is_empty(),
            }
        }
        let schema = plan.raw_schema(item)?;
        if schema.signature.params.len() != arguments.len() + 1 {
            return Err("trait method argument count mismatch".into());
        }
        let mut solver = Inference::new(self, own.clone())?;
        for (slot, actual) in explicit.iter().enumerate() {
            let formal = self.infer_node(InferKind::Slot(slot))?;
            let actual = self.partial_type_term(actual, 0)?;
            solver.constrain(&formal, &actual, 0)?;
        }
        self.infer_storage(arguments.len())?;
        let mut constraints = Vec::new();
        let mut direct = Vec::new();
        for (formal, argument) in schema.signature.params[1..].iter().zip(arguments) {
            if is_open(formal) {
                direct.push((formal.clone(), argument));
                continue;
            }
            if mentions_open(formal, &open) {
                continue;
            }
            let formal = self.method_owner_term(&solver.schema(formal, 0)?, &bindings, 0)?;
            let evidence = self.inference_evidence(argument, 0)?;
            let (formal, evidence, _) = sr::call_input_equation(&formal, &evidence);
            constraints.push((formal.clone(), evidence.clone()));
        }
        if !solver.constrain_projected_terms(constraints, true)? {
            return Ok(None);
        }
        let mut values: Vec<Option<ir::Ty>> = vec![None; open.len()];
        // Record one piece of evidence; false on a conflict or open value.
        fn record(
            values: &mut [Option<ir::Ty>],
            open: &[(ir::ParamId, ir::ParamId)],
            parameter: &ir::Ty,
            value: ir::Ty,
        ) -> bool {
            let ir::Ty::Param(p) = parameter else {
                return false;
            };
            let Some(slot) = open.iter().position(|(q, _)| q == p) else {
                return false;
            };
            if inferred_owner::mentions_param(&value) {
                return false;
            }
            match &values[slot] {
                Some(previous) if *previous != value => false,
                _ => {
                    values[slot] = Some(value);
                    true
                }
            }
        }
        for predicate in schema.predicates.iter() {
            plan.budget.charge_work(1)?;
            let ir::Predicate::ProjectionEq { projection, ty } = predicate else {
                continue;
            };
            let projection = ir::Ty::AssociatedType(Arc::new(projection.clone()));
            if mentions_open(&projection, &open) {
                if mentions_open(ty, &open) {
                    return Ok(None);
                }
                continue;
            }
            let projection = bind_owner(&projection, &bindings, &plan.budget)?;
            if is_open(ty) {
                let Some(term) = solver.projected_schema(&projection, 0, true)? else {
                    return Ok(None);
                };
                let Some(value) = solver.resolve(&term, true, 0)? else {
                    return Ok(None);
                };
                if !record(&mut values, &open, ty, value) {
                    return Ok(None);
                }
            } else if !matches!(ty, ir::Ty::AssociatedType(_))
                && inferred_owner::mentions_param(ty)
                && !mentions_open(ty, &open)
            {
                let ty = bind_owner(ty, &bindings, &plan.budget)?;
                solver.constrain_projection_equality(&projection, &ty, Some(&bindings), true)?;
            }
        }
        for (formal, argument) in direct {
            let evidence = self.inference_evidence(argument, 0)?;
            let Some(value) = solver.resolve(&evidence, true, 0)? else {
                return Ok(None);
            };
            if !record(&mut values, &open, &formal, value) {
                return Ok(None);
            }
        }
        // The expected result is only a fallback. With call evidence present
        // the ordinary result check reports any mismatch (it may coerce).
        if let Some(expected) = expected
            && is_open(&schema.signature.result)
            && let ir::Ty::Param(p) = &schema.signature.result
            && let Some(slot) = open.iter().position(|(q, _)| q == p)
            && values[slot].is_none()
            && !record(
                &mut values,
                &open,
                &schema.signature.result,
                expected.clone(),
            )
        {
            return Ok(None);
        }
        let mut args = Vec::new();
        self.infer_storage(template.args.len())?;
        for (index, actual) in template.args.iter().enumerate() {
            let parameter = ir::ParamId {
                owner: owner.binder(),
                index: (index + 1) as u16,
            };
            match open.iter().position(|(q, _)| *q == parameter) {
                Some(slot) => match values[slot].take() {
                    Some(value) => {
                        self.facts().validate_ty_with_budget(&value, &plan.budget)?;
                        args.push(value);
                    }
                    None => return Ok(None),
                },
                None => args.push(actual.clone()),
            }
        }
        Ok(Some(ir::TraitRef {
            declaration: template.declaration,
            self_ty: template.self_ty.clone(),
            args: args.into(),
        }))
    }
}

/// Replace enclosing trait-owner parameters (Self and trait arguments) with
/// the selected application's actuals. Method-own parameters stay slots.
pub(super) fn bind_owner(
    ty: &ir::Ty,
    bindings: &HashMap<ir::ParamId, ir::Ty>,
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, String> {
    budget.charge_work(1)?;
    Ok(match ty {
        ir::Ty::Param(p) => bindings.get(p).cloned().unwrap_or_else(|| ty.clone()),
        ir::Ty::Ref { kind, referent } => {
            budget
                .retain_bytes(std::mem::size_of::<ir::Ty>() + 2 * std::mem::size_of::<usize>())?;
            ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(bind_owner(referent, bindings, budget)?),
            }
        }
        ir::Ty::PhantomData(marker) => ir::Ty::PhantomData(marker.try_map(
            budget,
            |error| error,
            |ty| bind_owner(ty, bindings, budget),
        )?),
        ir::Ty::ManuallyDrop(marker) => ir::Ty::ManuallyDrop(marker.try_map(
            budget,
            |error| error,
            |ty| bind_owner(ty, bindings, budget),
        )?),
        ir::Ty::Cell(marker) => ir::Ty::Cell(marker.try_map(
            budget,
            |error| error,
            |ty| bind_owner(ty, bindings, budget),
        )?),
        ir::Ty::FnPointer(pointer) => ir::Ty::FnPointer(pointer.try_map(
            budget,
            |e| e,
            |t| bind_owner(t, bindings, budget),
        )?),
        ir::Ty::Tuple(ts) => {
            let mut mapped = ir::FnPointerTy::storage(ts.len(), budget)?;
            for t in ts.iter() {
                mapped.push(bind_owner(t, bindings, budget)?);
            }
            ir::Ty::Tuple(ir::FnPointerTy::slice(mapped, budget)?)
        }
        ir::Ty::AssociatedType(p) => {
            let mut args = ir::FnPointerTy::storage(p.trait_ref.args.len(), budget)?;
            for t in p.trait_ref.args.iter() {
                args.push(bind_owner(t, bindings, budget)?);
            }
            budget.retain_bytes(
                std::mem::size_of::<ir::AssociatedTypeProjection>()
                    + 2 * std::mem::size_of::<usize>(),
            )?;
            let mut p = p.as_ref().clone();
            p.trait_ref.self_ty = bind_owner(&p.trait_ref.self_ty, bindings, budget)?;
            p.trait_ref.args = ir::FnPointerTy::slice(args, budget)?;
            ir::Ty::AssociatedType(Arc::new(p))
        }
        other => other.clone(),
    })
}

/// Binds a bare method-own formal (`I` in `extend<I: IntoIterator<Item = A>>(
/// iter: I)`) to its fixed argument type. Returns `Some(fits)` when `formal`
/// is such a parameter, `None` otherwise.
pub(in crate::compiler::frontend) fn bind_own_formal(
    formal: &ir::Ty,
    own: ir::BinderId,
    actual: &ir::Ty,
    bindings: &mut HashMap<ir::ParamId, ir::Ty>,
) -> Option<bool> {
    let ir::Ty::Param(parameter) = formal else {
        return None;
    };
    if parameter.owner != own {
        return None;
    }
    Some(match bindings.get(parameter) {
        Some(previous) => previous == actual,
        None => {
            bindings.insert(*parameter, actual.clone());
            true
        }
    })
}

/// Whether every method bound closed by `bindings` is provable. This decides
/// between several applications of one trait (`Vec<i32>: Extend<i32>` and
/// `Extend<&i32>`) whose formals are method-own parameters: the argument
/// fixes `I`, and `I: IntoIterator<Item = A>` holds for exactly one `A`.
/// Bounds still naming unbound parameters decide nothing. Final selection
/// proves the chosen application again.
pub(in crate::compiler::frontend) fn own_bounds_hold(
    predicates: &[ir::Predicate],
    bindings: &HashMap<ir::ParamId, ir::Ty>,
    facts: &tr::ValidatedFacts,
    scratch: &std::cell::RefCell<tr::TraitScratch>,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let bind_ref = |application: &ir::TraitRef| -> Result<ir::TraitRef, String> {
        let mut args = Vec::with_capacity(application.args.len());
        for argument in application.args.iter() {
            args.push(bind_owner(argument, bindings, budget)?);
        }
        Ok(ir::TraitRef {
            declaration: application.declaration,
            self_ty: bind_owner(&application.self_ty, bindings, budget)?,
            args: args.into(),
        })
    };
    for predicate in predicates {
        budget.charge_work(1)?;
        let goal = match predicate {
            ir::Predicate::Implements(application) => {
                ir::Predicate::Implements(bind_ref(application)?)
            }
            ir::Predicate::ProjectionEq { projection, ty } => ir::Predicate::ProjectionEq {
                projection: ir::AssociatedTypeProjection {
                    trait_ref: bind_ref(&projection.trait_ref)?,
                    item: projection.item,
                },
                ty: bind_owner(ty, bindings, budget)?,
            },
        };
        if goal.metadata_types().any(inferred_owner::mentions_param) {
            continue;
        }
        if matches!(
            facts
                .service()
                .prove_goal_with(&goal, facts, &mut scratch.borrow_mut())
                .map_err(|error| error.into_legacy_diagnostic(budget))?,
            tr::ProofOutcome::NotProven
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Restores the checker's i32 literal fallback when one candidate
/// application's ranking ends, including on early return.
struct LiteralDefault<'c, 's> {
    checker: &'c Checker<'s>,
    previous: ir::IntTy,
}

impl<'c, 's> LiteralDefault<'c, 's> {
    fn set(checker: &'c Checker<'s>, application: &ir::TraitRef) -> Self {
        let previous = checker.literal_integer_default.get();
        if let Some(int) = application.args.iter().find_map(|arg| match arg {
            ir::Ty::Int(int) => Some(*int),
            _ => None,
        }) {
            checker.literal_integer_default.set(int);
        }
        Self { checker, previous }
    }
}

impl Drop for LiteralDefault<'_, '_> {
    fn drop(&mut self) {
        self.checker.literal_integer_default.set(self.previous);
    }
}
