//! Layout-free recipes extracted from solved body terms, not source annotations.
//!
//! This module is deliberately not a realization capability. A consumer must
//! replay the owning declaration/caller facts and read each graph realization.
//! In particular, no recipe grants layout, carrier, region, or executable proof.
//! Shapes are region-erased demand descriptions, not globally reusable keys.
//! The graph adapter must include its exact world/caller authorization domain;
//! every consuming type occurrence separately replays its owned source-region
//! environment, ordered type-actual regions and carrier rows. Neither a retry
//! nor a shared layout fact permits synthesizing those rows from a shape.
use super::*;

#[cfg(test)]
#[path = "nominal_manifest_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub(in crate::compiler) enum Shape {
    /// Restricted to primitive types and exact canonical parameter identities.
    Leaf(ir::Ty),
    Nominal(ir::DeclId, Arc<[Shape]>),
    Tuple(Arc<[Shape]>),
    Ref(ir::BorrowKind, Arc<Shape>),
    RawPtr(ir::BorrowKind, Arc<Shape>),
    Array(Arc<Shape>, usize),
    Slice(Arc<Shape>),
    Projection {
        declaration: ir::DeclId,
        item: ir::DeclId,
        receiver: Arc<Shape>,
        arguments: Arc<[Shape]>,
    },
}

impl Shape {
    /// Structural identity only. The supplied meter must enforce the caller's
    /// own depth/work limits. This does not authenticate any type or projection.
    pub(in crate::compiler) fn same_with(
        &self,
        other: &Self,
        depth: usize,
        meter: &mut impl FnMut(usize) -> Result<(), String>,
    ) -> Result<bool, String> {
        meter(depth)?;
        Ok(match (self, other) {
            (Shape::Leaf(left), Shape::Leaf(right)) => {
                if !primitive_leaf(left) || !primitive_leaf(right) {
                    return Err("invalid metadata: compound type in manifest leaf".into());
                }
                left == right
            }
            (Shape::Nominal(a, left), Shape::Nominal(b, right)) => {
                a == b && same_children(left, right, depth + 1, meter)?
            }
            (Shape::Tuple(left), Shape::Tuple(right)) => {
                same_children(left, right, depth + 1, meter)?
            }
            (Shape::Ref(a, left), Shape::Ref(b, right))
            | (Shape::RawPtr(a, left), Shape::RawPtr(b, right)) => {
                a == b && left.same_with(right, depth + 1, meter)?
            }
            (Shape::Array(left, a), Shape::Array(right, b)) => {
                a == b && left.same_with(right, depth + 1, meter)?
            }
            (Shape::Slice(left), Shape::Slice(right)) => left.same_with(right, depth + 1, meter)?,
            (
                Shape::Projection {
                    declaration: a,
                    item: ai,
                    receiver: ar,
                    arguments: aa,
                },
                Shape::Projection {
                    declaration: b,
                    item: bi,
                    receiver: br,
                    arguments: ba,
                },
            ) => {
                a == b
                    && ai == bi
                    && ar.same_with(br, depth + 1, meter)?
                    && same_children(aa, ba, depth + 1, meter)?
            }
            _ => false,
        })
    }
}

fn primitive_leaf(ty: &ir::Ty) -> bool {
    matches!(
        ty,
        ir::Ty::Unit
            | ir::Ty::Never
            | ir::Ty::Bool
            | ir::Ty::Char
            | ir::Ty::Str
            | ir::Ty::Int(_)
            | ir::Ty::Float(_)
            | ir::Ty::Param(_)
    ) || matches!(ty, ir::Ty::Dyn(object)
        if object.is_closed_leaf())
}

fn same_children(
    left: &[Shape],
    right: &[Shape],
    depth: usize,
    meter: &mut impl FnMut(usize) -> Result<(), String>,
) -> Result<bool, String> {
    meter(depth)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.iter().zip(right) {
        if !left.same_with(right, depth + 1, meter)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Clone, Debug)]
pub(in crate::compiler) struct Demand {
    pub declaration: ir::DeclId,
    pub arguments: Arc<[Shape]>,
}

#[derive(Clone, Debug)]
pub(in crate::compiler) struct Manifest {
    demands: Arc<[Demand]>,
}

impl Manifest {
    pub(in crate::compiler) fn demands(&self) -> &[Demand] {
        &self.demands
    }
}

/// Borrowed inputs exist only while one graph operation prepares its body.
/// The nominal callback must use a graph-read realization capability. Returning
/// `None` means unavailable, not a failed proof and never permission to intern.
pub(in crate::compiler) enum DeclarationFields {
    Struct(Arc<[Shape]>),
    Enum(Arc<[Arc<[Shape]>]>),
}

pub(in crate::compiler) struct DeclarationRecipe {
    pub env: Arc<ir::ParamEnv>,
    pub fields: DeclarationFields,
}

impl DeclarationRecipe {
    fn fields(&self, variant: Option<usize>) -> Result<&[Shape], String> {
        match (&self.fields, variant) {
            (DeclarationFields::Struct(fields), None) => Ok(fields),
            (DeclarationFields::Enum(variants), Some(index)) => variants
                .get(index)
                .map(AsRef::as_ref)
                .ok_or_else(|| "invalid metadata: source recipe variant index".into()),
            _ => Err("invalid metadata: source recipe nominal kind".into()),
        }
    }
}

pub(in crate::compiler) struct Inputs<'a> {
    pub declaration_schema: &'a dyn Fn(ir::DeclId) -> Result<Option<DeclarationRecipe>, String>,
    pub source_type: &'a dyn Fn(&Type) -> Result<Option<Shape>, String>,
    pub nominal_type: &'a dyn Fn(&Demand) -> Result<Option<ir::Ty>, String>,
}

impl<'a> Inputs<'a> {
    pub(in crate::compiler) fn new(
        declaration_schema: &'a dyn Fn(ir::DeclId) -> Result<Option<DeclarationRecipe>, String>,
        source_type: &'a dyn Fn(&Type) -> Result<Option<Shape>, String>,
        nominal_type: &'a dyn Fn(&Demand) -> Result<Option<ir::Ty>, String>,
    ) -> Self {
        Self {
            declaration_schema,
            source_type,
            nominal_type,
        }
    }
}

pub(in crate::compiler) enum PreparedHints {
    Ready(Hints),
    Blocked(Preparation),
}

pub(in crate::compiler) enum Preparation {
    Solved(Manifest),
    Blocked {
        nominals: Arc<[Demand]>,
        declarations: Arc<[ir::DeclId]>,
        needs_source_schemas: bool,
        unresolved_constraints: bool,
    },
}

/// Node-local source seed. Keys are authenticated declaration occurrences, not
/// runtime locals. Recipes remain nonproof descriptions until graph replay.
pub(super) struct SourceSeed {
    pub(super) parameters: Vec<(LocalKey, Shape)>,
    pub(super) result: Shape,
}

/// Prepare the immutable source callable before executable parameter/result
/// obligations. The caller installs the canonical declaration and lexical scope;
/// no parameter is bound into runtime IR by this entry.
#[cfg(test)]
pub(in crate::compiler::frontend::type_check) fn prepare_callable<'a: 's, 'b: 's, 's>(
    checker: &'a Checker<'b>,
    declaration: ir::DeclId,
    receiver: Option<&Shape>,
    inputs: &'s Inputs<'s>,
) -> Result<Preparation, String> {
    let solver = solve_callable(checker, declaration, receiver, inputs)?;
    preparation_status(&solver, inputs)
}

pub(in crate::compiler::frontend::type_check) fn finalize_callable<'a: 's, 'b: 's, 's>(
    checker: &'a Checker<'b>,
    declaration: ir::DeclId,
    receiver: Option<&Shape>,
    inputs: &'s Inputs<'s>,
) -> Result<PreparedHints, String> {
    let mut solver = solve_callable(checker, declaration, receiver, inputs)?;
    match preparation_status(&solver, inputs)? {
        blocked @ Preparation::Blocked { .. } => Ok(PreparedHints::Blocked(blocked)),
        Preparation::Solved(_) => {
            let signature = solver.realized_source_signature(declaration, receiver)?;
            // Signature terms may expose a further exact nominal demand. They
            // cannot be turned into executable parameters until it is read.
            // Closure parameter projections likewise (see `finalize`).
            solver.probe_closure_parameters()?;
            match preparation_status(&solver, inputs)? {
                blocked @ Preparation::Blocked { .. } => Ok(PreparedHints::Blocked(blocked)),
                Preparation::Solved(_) => {
                    let signature = signature
                        .ok_or("unsupported: unresolved canonical callable signature recipe")?;
                    let mut hints = finalized_hints(solver)?;
                    hints.source_signature = Some(signature);
                    Ok(PreparedHints::Ready(hints))
                }
            }
        }
    }
}

impl Solver<'_, '_, '_> {
    fn realized_source_signature(
        &mut self,
        declaration: ir::DeclId,
        receiver: Option<&Shape>,
    ) -> Result<Option<SourceSignature>, String> {
        let plan = self.checker.signatures.plan()?;
        let function = plan.callable_body(&plan.program, declaration)?;
        let mut parameters = Vec::new();
        self.checker.infer_storage(
            function
                .params
                .len()
                .checked_add(usize::from(receiver.is_some()))
                .ok_or("source signature parameter allocation overflow")?,
        )?;
        let mut complete = true;
        if let Some(receiver) = receiver {
            let term = self.manifest_term(receiver, 0)?;
            match self.resolve(term, 0)? {
                Some(ty) => parameters.push(ty),
                None => complete = false,
            }
        }
        for (_, source) in &function.params {
            self.checker.infer_work(0)?;
            let term = self.source_ty(source)?;
            match self.resolve(term, 0)? {
                Some(ty) => parameters.push(ty),
                None => complete = false,
            }
        }
        let result = self.resolve(self.return_terms[0], 0)?;
        match result {
            Some(result) if complete => Ok(Some(SourceSignature { parameters, result })),
            _ => Ok(None),
        }
    }
}

fn solve_callable<'a: 's, 'b: 's, 's>(
    checker: &'a Checker<'b>,
    declaration: ir::DeclId,
    receiver: Option<&Shape>,
    inputs: &'s Inputs<'s>,
) -> Result<Solver<'a, 'b, 's>, String> {
    if checker.declaration != Some(declaration) {
        return Err("invalid metadata: preparation callable owner mismatch".into());
    }
    let plan = checker.signatures.plan()?;
    let function = plan.callable_body(&plan.program, declaration)?;
    if checker.lexical_scope != function.lexical_scope {
        return Err("invalid metadata: preparation callable scope mismatch".into());
    }
    let context = plan.callable_context(declaration)?;
    if (context.receiver != ReceiverKind::None) != receiver.is_some() {
        return Err("invalid metadata: preparation receiver recipe mismatch".into());
    }
    let result = (inputs.source_type)(&function.result)?
        .ok_or("unsupported: callable result source recipe is not available")?;
    let mut seed = SourceSeed {
        parameters: Vec::new(),
        result,
    };
    if let Some(receiver) = receiver {
        let key = checker
            .local_key(&LocalName::Receiver, true)?
            .ok_or("invalid metadata: preparation receiver declaration is absent")?;
        checker.infer_storage(1)?;
        seed.parameters.push((key, receiver.clone()));
    }
    for (name, source) in &function.params {
        checker.infer_work(0)?;
        let shape = (inputs.source_type)(source)?
            .ok_or("unsupported: callable parameter source recipe is not available")?;
        if let Some(key) = checker.local_key(name, true)? {
            checker.infer_storage(1)?;
            seed.parameters.push((key, shape));
        }
    }
    solve_body_with_seed(checker, &function.body, None, Some(inputs), Some(&seed))
}

/// Preparation is emission-free. Resuming reruns the same retained body with
/// additional immutable graph-read inputs, not a cached mutable solver fact.
#[cfg(test)]
pub(super) fn prepare<'a: 's, 'b: 's, 's>(
    checker: &'a Checker<'b>,
    block: &'s Block,
    expected: Option<&ir::Ty>,
    inputs: &'s Inputs<'s>,
) -> Result<Preparation, String> {
    let solver = solve_body(checker, block, expected, Some(inputs))?;
    preparation_status(&solver, inputs)
}

pub(super) fn finalize<'a: 's, 'b: 's, 's>(
    checker: &'a Checker<'b>,
    block: &'s Block,
    expected: Option<&ir::Ty>,
    inputs: &'s Inputs<'s>,
) -> Result<PreparedHints, String> {
    let solver = solve_body(checker, block, expected, Some(inputs))?;
    match preparation_status(&solver, inputs)? {
        blocked @ Preparation::Blocked { .. } => Ok(PreparedHints::Blocked(blocked)),
        Preparation::Solved(_) => {
            // A closure parameter may be a projection (`<T as FromStr>::Err`)
            // whose normal form is a source nominal first seen here. Reading
            // its representation records that exact graph demand; block on it
            // instead of reporting the parameter as uninferable.
            solver.probe_closure_parameters()?;
            match preparation_status(&solver, inputs)? {
                blocked @ Preparation::Blocked { .. } => Ok(PreparedHints::Blocked(blocked)),
                Preparation::Solved(_) => finalized_hints(solver).map(PreparedHints::Ready),
            }
        }
    }
}

fn preparation_status(
    solver: &Solver<'_, '_, '_>,
    inputs: &Inputs<'_>,
) -> Result<Preparation, String> {
    if solver.missing_source_schemas.get() {
        return Ok(Preparation::Blocked {
            nominals: solver.pending_nominals()?,
            declarations: solver.pending_declarations()?,
            needs_source_schemas: true,
            unresolved_constraints: true,
        });
    }
    // Results of method calls on closure-bearing receivers are typed by the
    // parent's checked selection, which owns the finalized closure type.
    // Nominals waiting only on those results are left to checked lowering.
    let mut closure_methods = !solver.methods.is_empty();
    for method in &solver.methods {
        if !solver.contains_callable(solver.peel_refs(method.receiver), 0)? {
            closure_methods = false;
            break;
        }
    }
    for term in 0..solver.variables.len() {
        solver.checker.infer_work(0)?;
        if solver.root(term) == term
            && let Domain::Nominal(_, arguments) = solver.domain(term)
            && !solver.contains_callable(term, 0)?
            && !solver.manifest_ready(&arguments, 1)?
            && !closure_methods
        {
            let nominals = solver.pending_nominals()?;
            let declarations = solver.pending_declarations()?;
            if nominals.is_empty()
                && declarations.is_empty()
                && solver.methods.is_empty()
                && solver.operators.is_empty()
                && solver.projections.is_empty()
                && solver.iterator_items.is_empty()
                && solver.associated_equalities.is_empty()
                && solver.coercions.is_empty()
                && solver.patterns.is_empty()
            {
                return Err(solver.user_error(
                    "E0282: type annotations needed: cannot infer the type of this binding",
                ));
            }
            return Ok(Preparation::Blocked {
                nominals,
                declarations,
                needs_source_schemas: false,
                unresolved_constraints: true,
            });
        }
    }
    let manifest = solver.solved_nominal_manifest(closure_methods)?;
    for demand in manifest.demands() {
        if (inputs.nominal_type)(demand)?.is_none() {
            solver.record_missing_nominal(demand.clone())?;
        }
    }
    // Iterator items of closure-bearing sources (`Map<Iter<T>, {closure}>`)
    // are likewise typed by checked lowering, which owns the closure type.
    let mut closure_items = !solver.iterator_items.is_empty();
    for pending in &solver.iterator_items {
        let source = match pending {
            super::iteration::Pending::Ordinary(source, _) => *source,
            super::iteration::Pending::ArrayMethod { receiver, .. } => *receiver,
        };
        if !solver.contains_callable(solver.peel_refs(source), 0)? {
            closure_items = false;
            break;
        }
    }
    // With closure-receiver calls deferred, constraints that read their
    // results are also typed (and diagnosed) by checked lowering.
    let unresolved_constraints = !closure_methods
        && (!solver.methods.is_empty()
            || !solver.operators.is_empty()
            || !solver.projections.is_empty()
            || (!solver.iterator_items.is_empty() && !closure_items)
            || !solver.associated_equalities.is_empty()
            || !solver.coercions.is_empty()
            || !solver.patterns.is_empty());
    let missing = solver.pending_nominals()?;
    if !missing.is_empty() || unresolved_constraints {
        Ok(Preparation::Blocked {
            nominals: missing,
            declarations: solver.pending_declarations()?,
            needs_source_schemas: false,
            unresolved_constraints,
        })
    } else {
        Ok(Preparation::Solved(manifest))
    }
}

impl<'s> Solver<'_, '_, 's> {
    /// Resolve every finalized closure parameter once, discarding the result.
    /// Resolution through a projection's representation records any source
    /// nominal it needs as a missing graph input (`record_missing_nominal`),
    /// so the following `preparation_status` blocks on that exact demand.
    /// Unknown parameters stay unknown; `closure_hints` diagnoses them.
    fn probe_closure_parameters(&self) -> Result<(), String> {
        for (_, term) in &self.closure_terms {
            self.checker.infer_work(0)?;
            let Domain::Callable(callable) = self.domain(*term) else {
                continue;
            };
            for parameter in callable.params {
                let _ = self.resolve(parameter, 0)?;
            }
            let _ = self.resolve(callable.result, 0)?;
        }
        Ok(())
    }

    fn pending_nominals(&self) -> Result<Arc<[Demand]>, String> {
        let missing = self.missing_nominals.borrow();
        self.checker.infer_storage(missing.len())?;
        Ok(missing.clone().into())
    }

    fn pending_declarations(&self) -> Result<Arc<[ir::DeclId]>, String> {
        let missing = self.missing_declarations.borrow();
        self.checker.infer_storage(missing.len())?;
        Ok(missing.clone().into())
    }

    fn declaration_recipe(
        &self,
        declaration: ir::DeclId,
    ) -> Result<Option<DeclarationRecipe>, String> {
        let inputs = self
            .preparation
            .ok_or("contract: source recipe outside preparation")?;
        let recipe = (inputs.declaration_schema)(declaration)?;
        if recipe.is_none() {
            self.missing_source_schemas.set(true);
            let mut missing = self.missing_declarations.borrow_mut();
            let mut present = false;
            for previous in missing.iter() {
                self.checker.infer_work(0)?;
                if *previous == declaration {
                    present = true;
                    break;
                }
            }
            if !present {
                self.checker.infer_storage(1)?;
                missing.push(declaration);
            }
        }
        Ok(recipe)
    }

    fn recipe_parameters(
        &mut self,
        env: &ir::ParamEnv,
        actuals: Option<&[Term]>,
    ) -> Result<HashMap<ir::ParamId, Term>, String> {
        if let Some(actuals) = actuals {
            if actuals.len() != env.arity() {
                return Err("invalid metadata: nominal recipe argument arity".into());
            }
            self.checker.infer_storage(actuals.len())?;
            Ok((0..env.arity())
                .zip(actuals.iter().copied())
                .map(|(index, term)| {
                    (
                        ir::ParamId {
                            owner: env.binder(),
                            index: index as u16,
                        },
                        term,
                    )
                })
                .collect())
        } else {
            let mut parameters = HashMap::new();
            self.checker.infer_storage(env.arity())?;
            for index in 0..env.arity() {
                self.checker.infer_work(0)?;
                parameters.insert(
                    ir::ParamId {
                        owner: env.binder(),
                        index: index as u16,
                    },
                    self.fresh()?,
                );
            }
            Ok(parameters)
        }
    }

    fn recipe_nominal(
        &mut self,
        declaration: ir::DeclId,
        env: &ir::ParamEnv,
        params: &HashMap<ir::ParamId, Term>,
    ) -> Result<Term, String> {
        self.checker.infer_storage(env.arity())?;
        let args = (0..env.arity())
            .map(|index| {
                let parameter = ir::ParamId {
                    owner: env.binder(),
                    index: index as u16,
                };
                params
                    .get(&parameter)
                    .copied()
                    .ok_or_else(|| "invalid metadata: missing recipe parameter".into())
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.node(Domain::Nominal(declaration, args))
    }

    fn recipe_field_index(
        &self,
        declaration: ir::DeclId,
        variant: Option<usize>,
        field: &FieldSelector,
    ) -> Result<usize, String> {
        match field {
            FieldSelector::Tuple(index) => Ok(*index),
            FieldSelector::Named(name) => self.checker.member_index(declaration, variant, *name),
            _ => Err("invalid metadata: non-field selector in nominal recipe".into()),
        }
    }

    pub(super) fn prepare_field(
        &mut self,
        declaration: ir::DeclId,
        args: &[Term],
        field: &FieldSelector,
    ) -> Result<Option<Term>, String> {
        let Some(recipe) = self.declaration_recipe(declaration)? else {
            return Ok(None);
        };
        let index = self.recipe_field_index(declaration, None, field)?;
        let shape = recipe
            .fields(None)?
            .get(index)
            .ok_or("invalid metadata: nominal recipe field index")?;
        let params = self.recipe_parameters(&recipe.env, Some(args))?;
        self.manifest_term_in(shape, &params, 0).map(Some)
    }

    pub(super) fn prepare_constructor(
        &mut self,
        declaration: ir::DeclId,
        variant: Option<AstIdent>,
        fields: &[(FieldSelector, &'s Expr)],
        self_ty: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<Term, String> {
        let Some(recipe) = self.declaration_recipe(declaration)? else {
            for (_, value) in fields {
                self.expr(value, depth + 1)?;
            }
            return self.fresh();
        };
        let params = self.recipe_parameters(&recipe.env, None)?;
        let variant = variant
            .map(|v| self.checker.variant_index(declaration, v))
            .transpose()?;
        let definitions = recipe.fields(variant)?;
        for (selector, value) in fields {
            self.checker.infer_work(depth)?;
            let index = self.recipe_field_index(declaration, variant, selector)?;
            let shape = definitions
                .get(index)
                .ok_or("invalid metadata: constructor recipe field index")?;
            let actual = self.expr(value, depth + 1)?;
            let formal = self.manifest_term_in(shape, &params, depth + 1)?;
            self.coerce(actual, formal)?;
        }
        let result = self.recipe_nominal(declaration, &recipe.env, &params)?;
        if let Some(self_ty) = self_ty {
            let rigid = self.fixed(self_ty.clone())?;
            self.unify(result, rigid)?;
        }
        Ok(result)
    }

    pub(super) fn prepare_pattern(
        &mut self,
        declaration: ir::DeclId,
        variant: Option<AstIdent>,
        fields: &[(FieldSelector, &'s Pattern)],
        term: Term,
    ) -> Result<(), String> {
        let Some(recipe) = self.declaration_recipe(declaration)? else {
            for (_, pattern) in fields {
                self.defer_bindings(pattern, &mut HashMap::new())?;
            }
            return Ok(());
        };
        let params = self.recipe_parameters(&recipe.env, None)?;
        let nominal = self.recipe_nominal(declaration, &recipe.env, &params)?;
        let (term, mode) = self.pattern_referent(term);
        self.unify(term, nominal)?;
        let variant = variant
            .map(|v| self.checker.variant_index(declaration, v))
            .transpose()?;
        let definitions = recipe.fields(variant)?;
        for (selector, pattern) in fields {
            self.checker.infer_work(0)?;
            let index = self.recipe_field_index(declaration, variant, selector)?;
            let shape = definitions
                .get(index)
                .ok_or("invalid metadata: pattern recipe field index")?;
            let term = self.manifest_term_in(shape, &params, 0)?;
            let binding = match mode {
                Some(kind) => self.node(Domain::Ref(kind, term))?,
                None => term,
            };
            self.pattern(pattern, binding)?;
        }
        Ok(())
    }

    pub(super) fn record_missing_nominal(&self, demand: Demand) -> Result<(), String> {
        let mut missing = self.missing_nominals.borrow_mut();
        for previous in missing.iter() {
            self.checker.infer_work(0)?;
            if previous.declaration == demand.declaration
                && self.same_manifest_children(&previous.arguments, &demand.arguments, 1)?
            {
                return Ok(());
            }
        }
        self.checker.infer_storage(1)?;
        missing.push(demand);
        Ok(())
    }

    pub(super) fn manifest_term(&mut self, shape: &Shape, depth: usize) -> Result<Term, String> {
        self.manifest_term_in(shape, &HashMap::new(), depth)
    }

    fn manifest_term_in(
        &mut self,
        shape: &Shape,
        params: &HashMap<ir::ParamId, Term>,
        depth: usize,
    ) -> Result<Term, String> {
        self.checker.infer_work(depth)?;
        if let Shape::Leaf(ir::Ty::Param(parameter)) = shape
            && let Some(term) = params.get(parameter)
        {
            return Ok(*term);
        }
        let domain = match shape {
            Shape::Leaf(ty) => match ty {
                ir::Ty::Unit
                | ir::Ty::Never
                | ir::Ty::Bool
                | ir::Ty::Char
                | ir::Ty::Str
                | ir::Ty::Int(_)
                | ir::Ty::Float(_)
                | ir::Ty::Param(_) => Domain::Fixed(ty.clone()),
                // Closed nongeneric object principal (nominal_source_shapes).
                ir::Ty::Dyn(object) if object.is_closed_leaf() => Domain::Fixed(ty.clone()),
                _ => return Err("invalid metadata: non-leaf source recipe".into()),
            },
            Shape::Nominal(declaration, arguments) => Domain::Nominal(
                *declaration,
                self.manifest_terms(arguments, params, depth + 1)?,
            ),
            Shape::Tuple(fields) => {
                Domain::Tuple(self.manifest_terms(fields, params, depth + 1)?)
            }
            Shape::Ref(kind, inner) => {
                Domain::Ref(*kind, self.manifest_term_in(inner, params, depth + 1)?)
            }
            Shape::RawPtr(kind, inner) => {
                Domain::RawPtr(*kind, self.manifest_term_in(inner, params, depth + 1)?)
            }
            Shape::Array(inner, length) => {
                Domain::Array(self.manifest_term_in(inner, params, depth + 1)?, *length)
            }
            Shape::Slice(inner) => {
                Domain::Slice(self.manifest_term_in(inner, params, depth + 1)?)
            }
            Shape::Projection {
                declaration,
                item,
                receiver,
                arguments,
            } => Domain::AssociatedProjection(associated_projection::ProjectionTerm {
                declaration: *declaration,
                item: *item,
                receiver: self.manifest_term_in(receiver, params, depth + 1)?,
                arguments: self.manifest_terms(arguments, params, depth + 1)?,
            }),
        };
        self.node(domain)
    }

    fn manifest_terms(
        &mut self,
        shapes: &[Shape],
        params: &HashMap<ir::ParamId, Term>,
        depth: usize,
    ) -> Result<Vec<Term>, String> {
        self.checker.infer_work(depth)?;
        self.checker.infer_storage(shapes.len())?;
        shapes
            .iter()
            .map(|shape| self.manifest_term_in(shape, params, depth + 1))
            .collect()
    }

    pub(super) fn manifest_ready(&self, terms: &[Term], depth: usize) -> Result<bool, String> {
        self.manifest_ready_with_defaults(terms, depth, self.projection_defaults)
    }

    pub(super) fn manifest_ready_with_defaults(
        &self,
        terms: &[Term],
        depth: usize,
        numeric_defaults: bool,
    ) -> Result<bool, String> {
        for term in terms {
            self.checker.infer_work(depth)?;
            let ready = match self.domain(*term) {
                Domain::Unknown => false,
                Domain::Callable(_) => {
                    return Err(
                        "unsupported: nominal realization requires finalized closure metadata"
                            .into(),
                    );
                }
                Domain::Integer | Domain::Float => numeric_defaults,
                Domain::Fixed(_) => true,
                Domain::Nominal(_, terms) | Domain::Tuple(terms) | Domain::Opaque(_, terms) => {
                    self.manifest_ready_with_defaults(&terms, depth + 1, numeric_defaults)?
                }
                Domain::Ref(_, inner)
                | Domain::RawPtr(_, inner)
                | Domain::Array(inner, _)
                | Domain::Slice(inner) => {
                    self.manifest_ready_with_defaults(&[inner], depth + 1, numeric_defaults)?
                }
                Domain::AssociatedProjection(projection) => {
                    self.manifest_ready_with_defaults(
                        &[projection.receiver],
                        depth + 1,
                        numeric_defaults,
                    )? && self.manifest_ready_with_defaults(
                        &projection.arguments,
                        depth + 1,
                        numeric_defaults,
                    )?
                }
            };
            if !ready {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Inspect every solved nominal root, including intermediate expressions.
    /// Never call `resolve`, `intern_nominal`, or inspect nominal field layouts.
    /// Source-region authority is retained by the owning source/caller facts,
    /// not reconstructed from these erased type shapes.
    pub(super) fn solved_nominal_manifest(&self, skip_unready: bool) -> Result<Manifest, String> {
        let mut demands: Vec<Demand> = Vec::new();
        for term in 0..self.variables.len() {
            self.checker.infer_work(0)?;
            if self.root(term) != term {
                continue;
            }
            let Domain::Nominal(declaration, arguments) = self.domain(term) else {
                continue;
            };
            // Closure-bearing applications are realized parent-locally from the
            // parent's own finalized ClosureTy, never through a graph demand.
            if self.contains_callable(term, 0)?
                || (skip_unready && !self.manifest_ready(&arguments, 1)?)
            {
                continue;
            }
            let arguments = self.manifest_children(&arguments, 1)?;
            let mut duplicate = false;
            for previous in &demands {
                self.checker.infer_work(0)?;
                if previous.declaration == declaration
                    && self.same_manifest_children(&previous.arguments, &arguments, 1)?
                {
                    duplicate = true;
                    break;
                }
            }
            if !duplicate {
                self.checker.infer_storage(1)?;
                demands.push(Demand {
                    declaration,
                    arguments,
                });
            }
        }
        Ok(Manifest {
            demands: demands.into(),
        })
    }

    pub(super) fn manifest_children(
        &self,
        terms: &[Term],
        depth: usize,
    ) -> Result<Arc<[Shape]>, String> {
        self.checker.infer_work(depth)?;
        self.checker.infer_storage(terms.len())?;
        terms
            .iter()
            .map(|term| self.manifest_shape(*term, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Arc::from)
    }

    fn manifest_shape(&self, term: Term, depth: usize) -> Result<Shape, String> {
        self.checker.infer_work(depth)?;
        self.checker.infer_storage(1)?;
        Ok(match self.domain(term) {
            Domain::Unknown => {
                return Err(
                    self.user_error("E0282: unresolved nominal actual in solved type manifest")
                );
            }
            Domain::Callable(_) => {
                return Err(
                    "unsupported: nominal realization requires finalized closure metadata".into(),
                );
            }
            Domain::Opaque(_, _) => {
                return Err(
                    "unsupported: opaque nominal actual needs an explicit manifest recipe".into(),
                );
            }
            Domain::Integer => Shape::Leaf(ir::Ty::Int(ir::IntTy::I32)),
            Domain::Float => Shape::Leaf(ir::Ty::Float(ir::FloatTy::F64)),
            Domain::Fixed(ty) => match ty {
                ir::Ty::Unit
                | ir::Ty::Never
                | ir::Ty::Bool
                | ir::Ty::Char
                | ir::Ty::Str
                | ir::Ty::Int(_)
                | ir::Ty::Float(_)
                | ir::Ty::Param(_) => Shape::Leaf(ty),
                // Closed nongeneric object principal: a rigid leaf with no
                // inference-bearing children (nominal_source_shapes).
                ir::Ty::Dyn(ref object) if object.is_closed_leaf() => Shape::Leaf(ty),
                _ => {
                    return Err(
                        "unsupported: compound fixed type needs an explicit manifest recipe".into(),
                    );
                }
            },
            Domain::Nominal(declaration, terms) => {
                Shape::Nominal(declaration, self.manifest_children(&terms, depth + 1)?)
            }
            Domain::Tuple(terms) => Shape::Tuple(self.manifest_children(&terms, depth + 1)?),
            Domain::Ref(kind, term) => {
                Shape::Ref(kind, Arc::new(self.manifest_shape(term, depth + 1)?))
            }
            Domain::RawPtr(kind, term) => {
                Shape::RawPtr(kind, Arc::new(self.manifest_shape(term, depth + 1)?))
            }
            Domain::Array(term, length) => {
                Shape::Array(Arc::new(self.manifest_shape(term, depth + 1)?), length)
            }
            Domain::Slice(term) => Shape::Slice(Arc::new(self.manifest_shape(term, depth + 1)?)),
            Domain::AssociatedProjection(projection) => Shape::Projection {
                declaration: projection.declaration,
                item: projection.item,
                receiver: Arc::new(self.manifest_shape(projection.receiver, depth + 1)?),
                arguments: self.manifest_children(&projection.arguments, depth + 1)?,
            },
        })
    }

    fn same_manifest_children(
        &self,
        left: &[Shape],
        right: &[Shape],
        depth: usize,
    ) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        if left.len() != right.len() {
            return Ok(false);
        }
        for (left, right) in left.iter().zip(right) {
            if !self.same_manifest_shape(left, right, depth + 1)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn same_manifest_shape(
        &self,
        left: &Shape,
        right: &Shape,
        depth: usize,
    ) -> Result<bool, String> {
        left.same_with(right, depth, &mut |depth| self.checker.infer_work(depth))
    }
}
