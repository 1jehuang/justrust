//! Body-local structural equality constraints, solved before lowering.
//! Scratch never escapes the checking operation. Inference variables are union-find
//! roots and constructors contain other roots, so evidence from later uses flows
//! back through arbitrary nesting rather than defaulting a binding prematurely.
use super::*;

#[cfg(test)]
#[path = "local_literals/native_scalar_tests.rs"]
mod native_scalar_tests;
#[path = "local_literals/nominal_manifest.rs"]
pub(super) mod nominal_manifest;

#[path = "local_literals/iteration.rs"]
mod iteration;

#[path = "local_literals/callable.rs"]
mod callable;

#[path = "local_literals/trait_methods.rs"]
mod trait_methods;

#[path = "local_literals/positional_pattern.rs"]
mod positional_pattern;
#[path = "local_literals/projection_call.rs"]
mod projection_call;

#[path = "local_literals/associated_projection.rs"]
mod associated_projection;

#[path = "local_literals/operators.rs"]
mod operators;

type Term = usize;
#[derive(Clone, Debug)]
enum Domain {
    Unknown,
    Integer,
    Float,
    Fixed(ir::Ty),
    Nominal(ir::DeclId, Vec<Term>),
    Tuple(Vec<Term>),
    Ref(ir::BorrowKind, Term),
    RawPtr(ir::BorrowKind, Term),
    Array(Term, usize),
    Slice(Term),
    Callable(callable::CallableTerm),
    AssociatedProjection(associated_projection::ProjectionTerm),
}
struct Variable {
    parent: Term,
    rank: u8,
    domain: Domain,
}
struct Method {
    receiver: Term,
    name: AstIdent,
    actuals: Vec<Term>,
    args: Vec<Term>,
    result: Term,
}
struct Projection {
    base: Term,
    field: FieldSelector,
    result: Term,
}
struct Solver<'a: 's, 'b: 's, 's> {
    checker: &'a Checker<'b>,
    variables: Vec<Variable>,
    scopes: Vec<HashMap<LocalKey, Term>>,
    sites: Vec<(usize, Term)>,
    methods: Vec<Method>,
    projections: Vec<Projection>,
    delayed_results: Vec<Term>,
    coercions: Vec<(Term, Term)>,
    iterator_items: Vec<(Term, Term)>,
    patterns: Vec<(&'s Pattern, Term, HashMap<LocalKey, Term>)>,
    pattern_bindings: Option<HashMap<LocalKey, Term>>,
    replaying_pattern: bool,
    closure_terms: Vec<(PathKey, Term)>,
    local_context: Option<Option<super::super::super::semantic_scope::ScopeId>>,
    return_terms: Vec<Term>,
    unit_defaults: Vec<Term>,
    associated_equalities: Vec<(Term, Term)>,
    operators: Vec<operators::Operation>,
    projection_defaults: bool,
    opaque_return: Option<Term>,
    // Borrowed graph-read inputs and local suspension bookkeeping. Neither is
    // part of a published manifest or executable type authority.
    preparation: Option<&'s nominal_manifest::Inputs<'s>>,
    missing_nominals: std::cell::RefCell<Vec<nominal_manifest::Demand>>,
    missing_source_schemas: std::cell::Cell<bool>,
    missing_declarations: std::cell::RefCell<Vec<ir::DeclId>>,
    preparation_needs_actuals: std::cell::Cell<bool>,
}

pub(in crate::compiler) struct Hints {
    pub locals: HashMap<usize, ir::Ty>,
    pub closures: HashMap<PathKey, ir::Signature>,
    // Present only for a canonical source-callable solve with explicit inputs.
    // These values still require ordinary executable facts/carrier validation.
    pub source_signature: Option<SourceSignature>,
}

pub(in crate::compiler) struct SourceSignature {
    pub parameters: Vec<ir::Ty>,
    pub result: ir::Ty,
}

pub(super) fn hints(
    checker: &Checker<'_>,
    block: &Block,
    expected: Option<&ir::Ty>,
) -> Result<Hints, String> {
    let solver = solve_body(checker, block, expected, None)?;
    finalized_hints(solver)
}

pub(super) fn hints_with_inputs<'a: 's, 'b: 's, 's>(
    checker: &'a Checker<'b>,
    block: &'s Block,
    expected: Option<&ir::Ty>,
    inputs: &'s nominal_manifest::Inputs<'s>,
) -> Result<nominal_manifest::PreparedHints, String> {
    nominal_manifest::finalize(checker, block, expected, inputs)
}

fn finalized_hints(solver: Solver<'_, '_, '_>) -> Result<Hints, String> {
    let closures = solver.closure_hints()?;
    let mut hints = HashMap::new();
    for (site, term) in solver.sites.clone() {
        if let Some(ty) = solver.resolve(term, 0)? {
            hints.insert(site, ty);
        } else if matches!(solver.domain(term), Domain::Nominal(..))
            && !solver.contains_callable(term, 0)?
        {
            return Err(solver.user_error(
                "E0282: type annotations needed: cannot infer the type of this binding",
            ));
        }
    }
    Ok(Hints {
        locals: hints,
        closures,
        source_signature: None,
    })
}

fn solve_body<'a: 's, 'b: 's, 's>(
    checker: &'a Checker<'b>,
    block: &'s Block,
    expected: Option<&ir::Ty>,
    preparation: Option<&'s nominal_manifest::Inputs<'s>>,
) -> Result<Solver<'a, 'b, 's>, String> {
    solve_body_with_seed(checker, block, expected, preparation, None)
}

fn solve_body_with_seed<'a: 's, 'b: 's, 's>(
    checker: &'a Checker<'b>,
    block: &'s Block,
    expected: Option<&ir::Ty>,
    preparation: Option<&'s nominal_manifest::Inputs<'s>>,
    source_seed: Option<&nominal_manifest::SourceSeed>,
) -> Result<Solver<'a, 'b, 's>, String> {
    let mut solver = Solver {
        checker,
        variables: Vec::new(),
        scopes: Vec::new(),
        sites: Vec::new(),
        methods: Vec::new(),
        projections: Vec::new(),
        delayed_results: Vec::new(),
        coercions: Vec::new(),
        iterator_items: Vec::new(),
        patterns: Vec::new(),
        pattern_bindings: None,
        replaying_pattern: false,
        closure_terms: Vec::new(),
        local_context: None,
        return_terms: Vec::new(),
        unit_defaults: Vec::new(),
        associated_equalities: Vec::new(),
        operators: Vec::new(),
        projection_defaults: false,
        opaque_return: None,
        preparation,
        missing_nominals: std::cell::RefCell::new(Vec::new()),
        missing_source_schemas: std::cell::Cell::new(false),
        missing_declarations: std::cell::RefCell::new(Vec::new()),
        preparation_needs_actuals: std::cell::Cell::new(false),
    };
    let return_term = match source_seed {
        Some(seed) => solver.manifest_term(&seed.result, 0)?,
        None => solver.return_expected(&checker.result.clone())?,
    };
    solver.return_terms.push(return_term);
    let mut outer = HashMap::new();
    if let Some(seed) = source_seed {
        for (key, shape) in &seed.parameters {
            solver.checker.infer_work(0)?;
            let term = solver.manifest_term(shape, 0)?;
            if outer.insert(*key, term).is_some() {
                return Err("invalid metadata: duplicate source parameter binding".into());
            }
        }
    } else {
        for scope in &checker.scopes {
            for (key, local) in scope {
                let term = solver.fixed(checker.locals[local.0].ty.clone())?;
                outer.insert(*key, term);
            }
        }
    }
    solver.scopes.push(outer);
    let result = solver.block(block, 0)?;
    if source_seed.is_some() {
        solver.coerce(result, return_term)?;
    }
    if let Some(expected) = expected {
        let expected = solver.return_expected(expected)?;
        solver.coerce(result, expected)?;
    }
    solver.solve()?;
    for term in solver.unit_defaults.clone() {
        if matches!(solver.domain(term), Domain::Unknown) {
            let unit = solver.fixed(ir::Ty::Unit)?;
            solver.unify(term, unit)?;
        }
    }
    Ok(solver)
}

impl<'s> Solver<'_, '_, 's> {
    fn node(&mut self, domain: Domain) -> Result<Term, String> {
        let domain = match domain {
            Domain::Tuple(fields) if fields.is_empty() => Domain::Fixed(ir::Ty::Unit),
            domain => domain,
        };
        self.checker.infer_storage(1)?;
        let term = self.variables.len();
        self.variables.push(Variable {
            parent: term,
            rank: 0,
            domain,
        });
        Ok(term)
    }
    fn user_error(&self, message: &str) -> String {
        let budget = &self.checker.signatures.plan.as_ref().unwrap().budget;
        let error = FactsError::unproved(budget, message);
        match self.checker.canonical {
            Some(capability) => capability.render(error),
            None => error.into_legacy_diagnostic(budget),
        }
    }
    fn fresh(&mut self) -> Result<Term, String> {
        self.node(Domain::Unknown)
    }
    fn return_expected(&mut self, ty: &ir::Ty) -> Result<Term, String> {
        if matches!(ty, ir::Ty::Opaque(opaque) if self.checker.declaration == Some(opaque.owner()))
        {
            if let Some(term) = self.opaque_return {
                return Ok(term);
            }
            let term = self.fresh()?;
            self.opaque_return = Some(term);
            return Ok(term);
        }
        self.fixed(ty.clone())
    }
    fn fixed(&mut self, ty: ir::Ty) -> Result<Term, String> {
        self.schema(&ty, &HashMap::new(), 0)
    }
    fn schema(
        &mut self,
        ty: &ir::Ty,
        params: &HashMap<ir::ParamId, Term>,
        depth: usize,
    ) -> Result<Term, String> {
        self.checker.infer_work(depth)?;
        if let ir::Ty::Param(param) = ty {
            if let Some(term) = params.get(param) {
                return Ok(*term);
            }
        }
        if let ir::Ty::AssociatedType(projection) = ty {
            return self.associated_projection(projection, params, depth + 1);
        }
        let domain = match ty {
            ir::Ty::Struct(_) | ir::Ty::Enum(_) => {
                let (decl, args) = nominal_parts(ty).unwrap();
                let args = args
                    .iter()
                    .map(|arg| self.schema(arg, params, depth + 1))
                    .collect::<Result<_, _>>()?;
                Domain::Nominal(decl, args)
            }
            ir::Ty::Ref { kind, referent } => {
                Domain::Ref(*kind, self.schema(referent, params, depth + 1)?)
            }
            ir::Ty::RawPtr { kind, pointee } => {
                Domain::RawPtr(*kind, self.schema(pointee, params, depth + 1)?)
            }
            ir::Ty::Array { element, len } => {
                Domain::Array(self.schema(element, params, depth + 1)?, *len)
            }
            ir::Ty::Slice { element } => Domain::Slice(self.schema(element, params, depth + 1)?),
            ir::Ty::Tuple(fields) => Domain::Tuple(
                fields
                    .iter()
                    .map(|f| self.schema(f, params, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            _ => Domain::Fixed(ty.clone()),
        };
        self.node(domain)
    }
    fn source_has_inference(&self, ty: &Type, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        Ok(match ty {
            Type::Infer => true,
            Type::Apply(path) => {
                let mut found = false;
                for segment in &path.segments {
                    for argument in &segment.args {
                        found |= self.source_has_inference(argument, depth + 1)?;
                    }
                }
                found
            }
            Type::Tuple(fields) => {
                let mut found = false;
                for field in fields {
                    found |= self.source_has_inference(field, depth + 1)?;
                }
                found
            }
            Type::Ref { referent, .. } => self.source_has_inference(referent, depth + 1)?,
            Type::RawPtr { pointee, .. } => self.source_has_inference(pointee, depth + 1)?,
            Type::Array(element, _) | Type::Slice(element) => {
                self.source_has_inference(element, depth + 1)?
            }
            _ => false,
        })
    }
    fn source_ty(&mut self, ty: &Type) -> Result<Term, String> {
        if matches!(ty, Type::Infer) {
            return self.fresh();
        }
        let partial = self.preparation.is_some() && self.source_has_inference(ty, 0)?;
        if let Some(inputs) = self.preparation.filter(|_| !partial) {
            // The source-schema provider authenticates the retained occurrence,
            // aliases, projections and region rows without instantiating fields.
            // Unknown source occurrences have no generic resumable identity:
            // report the explicit boundary rather than inventing a token or
            // taking the eager checked_local_ty compatibility path.
            return match (inputs.source_type)(ty)? {
                Some(shape) => self.manifest_term(&shape, 0),
                None => Err(
                    "unsupported: source type preparation requires an authenticated source recipe"
                        .into(),
                ),
            };
        }
        // Resolved aliases and primitive paths need not name a nominal. Only
        // genuinely partial types need reconstruction with fresh variables.
        if self.preparation.is_none() {
            if let Ok(ty) = checked_local_ty(ty, self.checker.signatures) {
                return self.fixed(ty);
            }
        }
        match ty {
            Type::Infer => self.fresh(),
            Type::Apply(path) => {
                if partial
                    && path
                        .segments
                        .iter()
                        .any(|segment| !segment.associated_types.is_empty())
                {
                    return Err(
                        "unsupported: associated equalities in partial nominal source type".into(),
                    );
                }
                let path_id = path.path(self.checker.names()?)?;
                let decl = self
                    .checker
                    .nominal_decl(&path_id)
                    .map_err(|error| format!("{error} in partial local type {path_id:?}"))?;
                if partial {
                    let argument_count = path
                        .segments
                        .iter()
                        .map(|segment| segment.args.len())
                        .sum::<usize>();
                    if self.checker.signatures.plan()?.param_env(decl)?.arity() != argument_count
                        || path
                            .segments
                            .iter()
                            .take(path.segments.len().saturating_sub(1))
                            .any(|segment| {
                                !segment.args.is_empty() || !segment.region_args.is_empty()
                            })
                    {
                        return Err("unsupported: partial nominal source application owner or arity changed".into());
                    }
                }
                let args = path
                    .segments
                    .iter()
                    .flat_map(|s| &s.args)
                    .map(|ty| self.source_ty(ty))
                    .collect::<Result<_, _>>()?;
                self.node(Domain::Nominal(decl, args))
            }
            Type::Ref { kind, referent, .. } => {
                let inner = self.source_ty(referent)?;
                self.node(Domain::Ref(*kind, inner))
            }
            Type::RawPtr { kind, pointee } => {
                let inner = self.source_ty(pointee)?;
                self.node(Domain::RawPtr(*kind, inner))
            }
            Type::Array(element, len) => {
                let inner = self.source_ty(element)?;
                self.node(Domain::Array(inner, *len))
            }
            Type::Slice(element) => {
                let inner = self.source_ty(element)?;
                self.node(Domain::Slice(inner))
            }
            Type::Tuple(fields) => {
                let fields = fields
                    .iter()
                    .map(|f| self.source_ty(f))
                    .collect::<Result<_, _>>()?;
                self.node(Domain::Tuple(fields))
            }
            _ => self.fixed(checked_local_ty(ty, self.checker.signatures)?),
        }
    }
    fn root(&self, mut term: Term) -> Term {
        while self.variables[term].parent != term {
            term = self.variables[term].parent;
        }
        term
    }
    fn domain(&self, term: Term) -> Domain {
        self.variables[self.root(term)].domain.clone()
    }
    fn occurs(&self, needle: Term, term: Term, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        let term = self.root(term);
        if needle == term {
            return Ok(true);
        }
        match &self.variables[term].domain {
            Domain::Nominal(_, args) | Domain::Tuple(args) => {
                for arg in args {
                    if self.occurs(needle, *arg, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Domain::AssociatedProjection(projection) => {
                if self.occurs(needle, projection.receiver, depth + 1)? {
                    return Ok(true);
                }
                for term in &projection.arguments {
                    if self.occurs(needle, *term, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Domain::Callable(callable) => {
                for term in callable
                    .params
                    .iter()
                    .chain(std::iter::once(&callable.result))
                {
                    if self.occurs(needle, *term, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Domain::Ref(_, t) | Domain::RawPtr(_, t) | Domain::Array(t, _) | Domain::Slice(t) => {
                self.occurs(needle, *t, depth + 1)
            }
            _ => Ok(false),
        }
    }
    fn unify(&mut self, left: Term, right: Term) -> Result<(), String> {
        self.unify_at(left, right, 0)
    }
    fn unify_at(&mut self, left: Term, right: Term, depth: usize) -> Result<(), String> {
        self.checker.infer_work(depth)?;
        let mut a = self.root(left);
        let mut b = self.root(right);
        if a == b {
            return Ok(());
        }
        self.reduce_associated_projection(a)?;
        self.reduce_associated_projection(b)?;
        let x = self.domain(a);
        let y = self.domain(b);
        if !matches!(x, Domain::Unknown)
            && !matches!(y, Domain::Unknown)
            && (matches!(x, Domain::AssociatedProjection(_))
                || matches!(y, Domain::AssociatedProjection(_)))
        {
            self.associated_equalities.push((a, b));
            return Ok(());
        }
        if matches!(x, Domain::Fixed(ir::Ty::Never)) || matches!(y, Domain::Fixed(ir::Ty::Never)) {
            return Ok(());
        }
        let domain = match (&x, &y) {
            (Domain::Unknown, _) => {
                if self.occurs(a, b, depth + 1)? {
                    return Err("E0282: cyclic local type inference constraint".into());
                }
                y.clone()
            }
            (_, Domain::Unknown) => {
                if self.occurs(b, a, depth + 1)? {
                    return Err("E0282: cyclic local type inference constraint".into());
                }
                x.clone()
            }
            (Domain::Integer, Domain::Integer) => Domain::Integer,
            (Domain::Float, Domain::Float) => Domain::Float,
            (Domain::Integer, Domain::Fixed(ty @ ir::Ty::Int(_)))
            | (Domain::Fixed(ty @ ir::Ty::Int(_)), Domain::Integer)
            | (Domain::Float, Domain::Fixed(ty @ ir::Ty::Float(_)))
            | (Domain::Fixed(ty @ ir::Ty::Float(_)), Domain::Float) => Domain::Fixed(ty.clone()),
            (Domain::Fixed(x), Domain::Fixed(y)) if x == y => Domain::Fixed(x.clone()),
            (Domain::Nominal(d, x), Domain::Nominal(e, y)) if d == e && x.len() == y.len() => {
                for (x, y) in x.iter().zip(y) {
                    self.unify_at(*x, *y, depth + 1)?;
                }
                Domain::Nominal(*d, x.clone())
            }
            (Domain::Tuple(x), Domain::Tuple(y)) if x.len() == y.len() => {
                for (x, y) in x.iter().zip(y) {
                    self.unify_at(*x, *y, depth + 1)?;
                }
                Domain::Tuple(x.clone())
            }
            (Domain::Ref(k, x), Domain::Ref(l, y)) if k == l => {
                self.unify_at(*x, *y, depth + 1)?;
                Domain::Ref(*k, *x)
            }
            (Domain::RawPtr(k, x), Domain::RawPtr(l, y)) if k == l => {
                self.unify_at(*x, *y, depth + 1)?;
                Domain::RawPtr(*k, *x)
            }
            (Domain::Array(x, n), Domain::Array(y, m)) if n == m => {
                self.unify_at(*x, *y, depth + 1)?;
                Domain::Array(*x, *n)
            }
            (Domain::Callable(x), Domain::Callable(y)) if x.key == y.key => {
                for (x, y) in x.params.iter().zip(&y.params) {
                    self.unify_at(*x, *y, depth + 1)?;
                }
                self.unify_at(x.result, y.result, depth + 1)?;
                Domain::Callable(x.clone())
            }
            (Domain::Slice(x), Domain::Slice(y)) => {
                self.unify_at(*x, *y, depth + 1)?;
                Domain::Slice(*x)
            }
            _ => {
                return Err(self.user_error(&format!(
                    "E0308: local inference constraint {x:?} does not match {y:?}"
                )));
            }
        };
        if self.variables[a].rank < self.variables[b].rank {
            std::mem::swap(&mut a, &mut b);
        }
        self.variables[b].parent = a;
        if self.variables[a].rank == self.variables[b].rank {
            self.variables[a].rank += 1;
        }
        self.variables[a].domain = domain;
        Ok(())
    }
    /// Whether a term's solved structure holds a closure type. Closure types
    /// have no source-spellable resolution here; like a bare closure local,
    /// such bindings are left to ordinary checking rather than rejected.
    fn contains_callable(&self, term: Term, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        Ok(match self.domain(term) {
            Domain::Callable(_) => true,
            Domain::Nominal(_, args) | Domain::Tuple(args) => {
                for arg in args {
                    if self.contains_callable(arg, depth + 1)? {
                        return Ok(true);
                    }
                }
                false
            }
            Domain::Ref(_, inner)
            | Domain::RawPtr(_, inner)
            | Domain::Array(inner, _)
            | Domain::Slice(inner) => self.contains_callable(inner, depth + 1)?,
            _ => false,
        })
    }
    fn resolve(&self, term: Term, depth: usize) -> Result<Option<ir::Ty>, String> {
        self.checker.infer_work(depth)?;
        Ok(Some(match self.domain(term) {
            Domain::Unknown | Domain::Callable(_) => return Ok(None),
            Domain::Integer => ir::Ty::Int(ir::IntTy::I32),
            Domain::Float => ir::Ty::Float(ir::FloatTy::F64),
            Domain::Fixed(ty) => ty,
            Domain::AssociatedProjection(projection) => {
                let Some(ty) = self.resolve_associated_projection(&projection, depth + 1)? else {
                    return Ok(None);
                };
                ty
            }
            Domain::Nominal(decl, args) => {
                if let Some(inputs) = self.preparation {
                    if !self.manifest_ready(&args, depth + 1)? {
                        self.preparation_needs_actuals.set(true);
                        return Ok(None);
                    }
                    let demand = nominal_manifest::Demand {
                        declaration: decl,
                        arguments: self.manifest_children(&args, depth + 1)?,
                    };
                    return match (inputs.nominal_type)(&demand)? {
                        Some(ty) => Ok(Some(ty)),
                        None => {
                            self.record_missing_nominal(demand)?;
                            Ok(None)
                        }
                    };
                }
                let mut actuals = Vec::new();
                for arg in args {
                    let Some(ty) = self.resolve(arg, depth + 1)? else {
                        return Ok(None);
                    };
                    actuals.push(ty);
                }
                let plan = self.checker.signatures.plan()?;
                plan.intern_nominal(decl, &actuals, self.checker.facts(), &plan.budget)?
            }
            Domain::Tuple(fields) => {
                let mut types = Vec::new();
                for f in fields {
                    let Some(ty) = self.resolve(f, depth + 1)? else {
                        return Ok(None);
                    };
                    types.push(ty);
                }
                ir::Ty::Tuple(types.into())
            }
            Domain::Ref(kind, term) => {
                let Some(ty) = self.resolve(term, depth + 1)? else {
                    return Ok(None);
                };
                ir::Ty::Ref {
                    kind,
                    referent: Arc::new(ty),
                }
            }
            Domain::RawPtr(kind, term) => {
                let Some(ty) = self.resolve(term, depth + 1)? else {
                    return Ok(None);
                };
                ir::Ty::RawPtr {
                    kind,
                    pointee: Arc::new(ty),
                }
            }
            Domain::Array(term, len) => {
                let Some(ty) = self.resolve(term, depth + 1)? else {
                    return Ok(None);
                };
                ir::Ty::Array {
                    element: Arc::new(ty),
                    len,
                }
            }
            Domain::Slice(term) => {
                let Some(ty) = self.resolve(term, depth + 1)? else {
                    return Ok(None);
                };
                ir::Ty::Slice {
                    element: Arc::new(ty),
                }
            }
        }))
    }
    fn lookup(&mut self, name: &impl LocalInput) -> Result<Term, String> {
        if let Some(key) = self.inference_key(name, false)? {
            for scope in self.scopes.iter().rev() {
                if let Some(term) = scope.get(&key) {
                    return Ok(*term);
                }
            }
        }
        self.fresh()
    }
    fn bind(&mut self, name: &impl LocalInput, term: Term) -> Result<(), String> {
        if let Some(key) = self.inference_key(name, true)? {
            if let Some(previous) = self
                .pattern_bindings
                .as_ref()
                .and_then(|b| b.get(&key))
                .copied()
            {
                return self.unify(previous, term);
            }
            self.scopes.last_mut().unwrap().insert(key, term);
        }
        Ok(())
    }
    fn parameters(&mut self, env: &ir::GenericEnv) -> Result<HashMap<ir::ParamId, Term>, String> {
        env.ordered_params()
            .map(|p| Ok((p, self.fresh()?)))
            .collect()
    }
    fn callable(
        &mut self,
        decl: ir::DeclId,
        args: &[Term],
        receiver: Option<Term>,
        actuals: &[Term],
    ) -> Result<Term, String> {
        let plan = self.checker.signatures.plan()?;
        let schema = plan.function_schema(decl)?;
        let info = plan.declaration(decl)?;
        let mut params = self.parameters(&schema.env)?;
        for (i, actual) in actuals.iter().enumerate() {
            params.insert(
                ir::ParamId {
                    owner: info.param_env.binder(),
                    index: i as u16,
                },
                *actual,
            );
        }
        if let Some(receiver) = receiver {
            let associated = info
                .path
                .associated
                .as_ref()
                .ok_or("method without owner")?;
            let owner = self.schema(&associated.self_pattern, &params, 0)?;
            let receiver = self.peel_refs(receiver);
            self.unify(receiver, owner)?;
        }
        let instantiated = self.projection_call_signature(decl, &schema, &params)?;
        let signature = instantiated.as_ref().unwrap_or(&schema.signature);
        let mut formals = signature.params.iter();
        if receiver.is_some() {
            formals.next();
        }
        for (arg, formal) in args.iter().zip(formals) {
            let formal = self.schema(formal, &params, 0)?;
            self.coerce(*arg, formal)?;
        }
        self.callable_bounds(decl, &params)?;
        self.schema(&signature.result, &params, 0)
    }
    fn coerce(&mut self, actual: Term, expected: Term) -> Result<(), String> {
        // A producer may reveal an array or a Deref carrier, so assigning its
        // unknown result the expected shape would erase a genuine coercion.
        if matches!(self.domain(actual), Domain::Unknown)
            && self
                .delayed_results
                .iter()
                .any(|result| self.root(*result) == self.root(actual))
        {
            self.coercions.push((actual, expected));
            return Ok(());
        }
        match (self.domain(actual), self.domain(expected)) {
            (Domain::Ref(_, a), Domain::Ref(_, b)) => {
                // A reference coercion may run an authoritative Deref chain.
                // Only equal outer constructors imply equality of type arguments.
                // The ordinary checker proves differing-referent coercions later.
                if let Domain::Nominal(actual, _) = self.domain(a) {
                    match self.domain(b) {
                        Domain::Nominal(expected, _) if actual == expected => {}
                        Domain::Unknown => {}
                        _ => return Ok(()),
                    }
                }
                self.coerce(a, b)
            }
            (Domain::Array(a, _), Domain::Slice(b)) => self.unify(a, b),
            _ => self.unify(actual, expected),
        }
    }
    fn peel_refs(&self, mut term: Term) -> Term {
        while let Domain::Ref(_, inner) = self.domain(term) {
            term = inner;
        }
        term
    }
    fn candidate(&self, nominal: ir::DeclId, name: AstIdent) -> Result<Option<ir::DeclId>, String> {
        let plan = self.checker.signatures.plan()?;
        let mut found = None;
        for id in &plan.registry.function_ids {
            self.checker.infer_work(0)?;
            let info = plan.declaration(*id)?;
            if let Some(owner) = &info.path.associated {
                if owner.trait_decl.is_none()
                    && owner.nominal == nominal
                    && plan.registry.source_member_equal(info.path.item, name)?
                {
                    if found.replace(*id).is_some() {
                        return Ok(None);
                    }
                }
            }
        }
        Ok(found)
    }
    fn solve(&mut self) -> Result<(), String> {
        let mut methods = std::mem::take(&mut self.methods);
        let mut projections = std::mem::take(&mut self.projections);
        loop {
            self.preparation_needs_actuals.set(false);
            let mut progress = false;
            let mut pending = Vec::new();
            for method in methods {
                if let Some(result) =
                    self.callable_method_application(method.receiver, method.name, &method.args)?
                {
                    self.unify(result, method.result)?;
                    progress = true;
                    continue;
                }
                let receiver = self.peel_refs(method.receiver);
                if let Domain::Nominal(decl, _) = self.domain(receiver) {
                    if let Some(candidate) = self.candidate(decl, method.name)? {
                        let result = self.callable(
                            candidate,
                            &method.args,
                            Some(receiver),
                            &method.actuals,
                        )?;
                        self.unify(result, method.result)?;
                        progress = true;
                        continue;
                    }
                }
                // A concrete receiver can also select trait or primitive methods.
                if let Some(ty) = self.resolve(receiver, 0)? {
                    if let Some(result) = self.trait_method_constraints(
                        &ty,
                        method.name,
                        &method.args,
                        &method.actuals,
                    )? {
                        self.unify(result, method.result)?;
                        progress = true;
                        continue;
                    }
                    let plan = self.checker.signatures.plan()?;
                    if method.actuals.is_empty() {
                        if let Ok(selected) = plan.select_method(
                            &ty,
                            &method.name,
                            &[],
                            self.checker.facts(),
                            &self.checker.module,
                            &plan.budget,
                        ) {
                            for (arg, param) in method
                                .args
                                .iter()
                                .zip(selected.signature.params.iter().skip(1))
                            {
                                let param = self.fixed(param.clone())?;
                                self.coerce(*arg, param)?;
                            }
                            let result = self.fixed(selected.signature.result)?;
                            self.unify(result, method.result)?;
                            progress = true;
                            continue;
                        }
                    }
                }
                pending.push(method);
            }
            methods = pending;
            let mut pending = Vec::new();
            for projection in projections {
                if let Some(field) = self.project(projection.base, &projection.field)? {
                    self.unify(field, projection.result)?;
                    progress = true;
                } else {
                    pending.push(projection);
                }
            }
            projections = pending;
            progress |= self.solve_iterator_items()?;
            progress |= self.solve_associated_equalities()?;
            progress |= self.solve_operators()?;
            let coercions = std::mem::take(&mut self.coercions);
            for (actual, expected) in coercions {
                if matches!(self.domain(actual), Domain::Unknown) {
                    self.coercions.push((actual, expected));
                } else {
                    self.coerce(actual, expected)?;
                    progress = true;
                }
            }
            let patterns = std::mem::take(&mut self.patterns);
            let force = !progress;
            for (pattern, term, bindings) in patterns {
                if !force && matches!(self.domain(self.peel_refs(term)), Domain::Unknown) {
                    self.patterns.push((pattern, term, bindings));
                    continue;
                }
                self.pattern_bindings = Some(bindings);
                self.replaying_pattern = true;
                let result = self.pattern(pattern, term);
                self.replaying_pattern = false;
                self.pattern_bindings = None;
                result?;
                progress = true;
            }
            if !progress {
                if self.preparation.is_some()
                    && (self.preparation_needs_actuals.get()
                        || self.missing_source_schemas.get()
                        || !self.missing_nominals.borrow().is_empty())
                {
                    // A missing graph input is not inference quiescence. In
                    // particular it cannot authorize an i32/f64 fallback that
                    // would choose an early nominal realization identity.
                    break;
                }
                if !self.projection_defaults {
                    self.projection_defaults = true;
                    continue;
                }
                break;
            }
        }
        self.methods = methods;
        self.projections = projections;
        Ok(())
    }
    fn project(&mut self, base: Term, field: &FieldSelector) -> Result<Option<Term>, String> {
        let base = self.peel_refs(base);
        match (self.domain(base), field) {
            (Domain::RawPtr(_, element), FieldSelector::Deref) => Ok(Some(element)),
            (Domain::Tuple(fields), FieldSelector::Tuple(i)) => Ok(fields.get(*i).copied()),
            (
                Domain::Array(element, _) | Domain::Slice(element),
                FieldSelector::ConstantIndex(_),
            ) => Ok(Some(element)),
            (Domain::Nominal(decl, args), _) => {
                if self.preparation.is_some() {
                    return self.prepare_field(decl, &args, field);
                }
                let plan = self.checker.signatures.plan()?;
                let (env, schema) = plan.nominal_schema(decl, &plan.budget)?;
                let ir::Ty::Struct(schema) = schema else {
                    return Ok(None);
                };
                let index = match field {
                    FieldSelector::Tuple(i) => *i,
                    FieldSelector::Named(name) => self.checker.member_index(decl, None, *name)?,
                    _ => return Ok(None),
                };
                let Some(field) = schema.def().fields.get(index) else {
                    return Ok(None);
                };
                let params = args
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| {
                        (
                            ir::ParamId {
                                owner: env.binder(),
                                index: i as u16,
                            },
                            t,
                        )
                    })
                    .collect();
                Ok(Some(self.schema(&field.ty, &params, 0)?))
            }
            _ => Ok(None),
        }
    }
    fn field(&mut self, base: Term, field: FieldSelector) -> Result<Term, String> {
        if let Some(result) = self.project(base, &field)? {
            return Ok(result);
        }
        let result = self.fresh()?;
        self.delayed_results.push(result);
        self.projections.push(Projection {
            base,
            field,
            result,
        });
        Ok(result)
    }
    fn place(&mut self, place: &Place) -> Result<Term, String> {
        let mut term = self.lookup(&place.local)?;
        for field in &place.fields {
            term = self.field(term, field.clone())?;
        }
        Ok(term)
    }
    fn constructor(
        &mut self,
        name: &AstPath,
        variant: Option<AstIdent>,
        fields: &[(FieldSelector, &'s Expr)],
        depth: usize,
    ) -> Result<Term, String> {
        let plan = self.checker.signatures.plan()?;
        let self_ty = if self.checker.self_path(name)? {
            self.checker.signatures.self_ty.as_ref()
        } else {
            None
        };
        let decl = match self_ty.and_then(nominal_parts) {
            Some((decl, _)) => decl,
            None => self.checker.nominal_decl(name)?,
        };
        if self.preparation.is_some() {
            return self.prepare_constructor(decl, variant, fields, self_ty, depth);
        }
        let (env, schema) = plan.nominal_schema(decl, &plan.budget)?;
        let mut params = HashMap::new();
        for index in 0..env.arity() {
            params.insert(
                ir::ParamId {
                    owner: env.binder(),
                    index: index as u16,
                },
                self.fresh()?,
            );
        }
        let variant = variant
            .map(|v| self.checker.variant_index(decl, v))
            .transpose()?;
        let definitions = match (&schema, variant) {
            (ir::Ty::Struct(s), None) => &s.def().fields,
            (ir::Ty::Enum(e), Some(v)) => &e.def().variants[v].fields,
            _ => return self.fresh(),
        };
        for (selector, value) in fields {
            let index = match selector {
                FieldSelector::Tuple(i) => *i,
                FieldSelector::Named(n) => self.checker.member_index(decl, variant, *n)?,
                _ => continue,
            };
            if let Some(field) = definitions.get(index) {
                let actual = self.expr(value, depth + 1)?;
                let formal = self.schema(&field.ty, &params, depth + 1)?;
                self.coerce(actual, formal)?;
            }
        }
        let result = self.schema(&schema, &params, depth + 1)?;
        if let Some(self_ty) = self_ty {
            let rigid = self.fixed(self_ty.clone())?;
            self.unify(result, rigid)?;
        }
        Ok(result)
    }
    fn defer_bindings(
        &mut self,
        pattern: &Pattern,
        bindings: &mut HashMap<LocalKey, Term>,
    ) -> Result<(), String> {
        self.checker.infer_work(0)?;
        match pattern {
            Pattern::Binding { name, .. } => {
                if let Some(key) = self.inference_key(name, true)? {
                    let term = match bindings.get(&key) {
                        Some(term) => *term,
                        None => self.fresh()?,
                    };
                    bindings.insert(key, term);
                    self.bind(name, term)?;
                }
            }
            Pattern::Variant { fields, .. } => {
                for (_, child) in fields {
                    self.defer_bindings(child, bindings)?;
                }
            }
            Pattern::Struct { fields, .. } => {
                for (_, child) in fields {
                    self.defer_bindings(child, bindings)?;
                }
            }
            Pattern::Apply { pattern, .. } | Pattern::Ref { pattern, .. } => {
                self.defer_bindings(pattern, bindings)?;
            }
            Pattern::Tuple(fields) | Pattern::Array(fields) | Pattern::Alternatives(fields) => {
                for child in fields {
                    self.defer_bindings(child, bindings)?;
                }
            }
            Pattern::TupleRest(fields)
            | Pattern::ArrayRest(fields)
            | Pattern::VariantRest { fields, .. } => {
                for child in fields.prefix.iter().chain(&fields.suffix) {
                    self.defer_bindings(child, bindings)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn pattern(&mut self, pattern: &'s Pattern, term: Term) -> Result<(), String> {
        if !self.replaying_pattern
            && matches!(self.domain(self.peel_refs(term)), Domain::Unknown)
            && matches!(
                pattern,
                Pattern::Variant { .. } | Pattern::Struct { .. } | Pattern::Tuple(_)
            )
        {
            let mut bindings = HashMap::new();
            self.defer_bindings(pattern, &mut bindings)?;
            self.patterns.push((pattern, term, bindings));
            return Ok(());
        }
        match pattern {
            Pattern::Variant { name, fields, .. } => {
                let fields = fields
                    .iter()
                    .map(|(field, p)| (field.clone(), p))
                    .collect::<Vec<_>>();
                self.positional_pattern(name, &fields, term)
            }
            Pattern::Struct { name, fields, .. } => {
                let fields = fields
                    .iter()
                    .map(|(name, p)| (FieldSelector::Named(*name), p))
                    .collect::<Vec<_>>();
                self.nominal_pattern(name, None, &fields, term)
            }
            Pattern::Ref { kind, pattern } => {
                let inner = self.fresh()?;
                let reference = self.node(Domain::Ref(*kind, inner))?;
                self.unify(term, reference)?;
                self.pattern(pattern, inner)
            }
            Pattern::Binding { name, by_ref, .. } => {
                let bound = match by_ref {
                    Some(kind) => self.node(Domain::Ref(*kind, term))?,
                    None => term,
                };
                self.bind(name, bound)
            }
            Pattern::Tuple(fields) => {
                let (term, mode) = self.pattern_referent(term);
                let mut terms = Vec::new();
                for field in fields {
                    let t = self.fresh()?;
                    let binding = match mode {
                        Some(kind) => self.node(Domain::Ref(kind, t))?,
                        None => t,
                    };
                    self.pattern(field, binding)?;
                    terms.push(t);
                }
                let tuple = self.node(Domain::Tuple(terms))?;
                self.unify(term, tuple)
            }
            Pattern::Apply { pattern, .. } => self.pattern(pattern, term),
            Pattern::Alternatives(patterns) => {
                for p in patterns {
                    self.pattern(p, term)?;
                }
                Ok(())
            }
            Pattern::Char(_) => {
                let t = self.fixed(ir::Ty::Char)?;
                self.unify(self.peel_refs(term), t)
            }
            Pattern::Bool(_) => {
                let t = self.fixed(ir::Ty::Bool)?;
                self.unify(self.peel_refs(term), t)
            }
            Pattern::Int { literal, .. } => {
                let t = self.integer(literal)?;
                self.unify(self.peel_refs(term), t)
            }
            _ => Ok(()),
        }
    }
    fn pattern_referent(&self, mut term: Term) -> (Term, Option<ir::BorrowKind>) {
        let mut mode = None;
        while let Domain::Ref(kind, inner) = self.domain(term) {
            if mode != Some(ir::BorrowKind::Shared) {
                mode = Some(kind);
            }
            term = inner;
        }
        (term, mode)
    }
    fn nominal_pattern(
        &mut self,
        name: &AstPath,
        variant_name: Option<AstIdent>,
        fields: &[(FieldSelector, &'s Pattern)],
        term: Term,
    ) -> Result<(), String> {
        let plan = self.checker.signatures.plan()?;
        let decl = if self.checker.self_path(name)? {
            self.checker
                .signatures
                .self_ty
                .as_ref()
                .and_then(nominal_parts)
                .map(|p| p.0)
                .ok_or("missing nominal Self in pattern")?
        } else {
            self.checker.nominal_decl(name)?
        };
        if self.preparation.is_some() {
            return self.prepare_pattern(decl, variant_name, fields, term);
        }
        let (env, schema) = plan.nominal_schema(decl, &plan.budget)?;
        let mut params = HashMap::new();
        for index in 0..env.arity() {
            params.insert(
                ir::ParamId {
                    owner: env.binder(),
                    index: index as u16,
                },
                self.fresh()?,
            );
        }
        let nominal = self.schema(&schema, &params, 0)?;
        let (term, mode) = self.pattern_referent(term);
        self.unify(term, nominal)?;
        let variant = variant_name
            .map(|v| self.checker.variant_index(decl, v))
            .transpose()?;
        let definitions = match (&schema, variant) {
            (ir::Ty::Struct(s), None) => &s.def().fields,
            (ir::Ty::Enum(e), Some(v)) => &e.def().variants[v].fields,
            _ => return Ok(()),
        };
        for (selector, pattern) in fields {
            let index = match selector {
                FieldSelector::Tuple(i) => *i,
                FieldSelector::Named(n) => self.checker.member_index(decl, variant, *n)?,
                _ => continue,
            };
            if let Some(field) = definitions.get(index) {
                let term = self.schema(&field.ty, &params, 0)?;
                let binding = match mode {
                    Some(kind) => self.node(Domain::Ref(kind, term))?,
                    None => term,
                };
                self.pattern(pattern, binding)?;
            }
        }
        Ok(())
    }
    fn block(&mut self, block: &'s Block, depth: usize) -> Result<Term, String> {
        self.checker.infer_work(depth)?;
        self.scopes.push(HashMap::new());
        for stmt in &block.statements {
            self.checker.infer_work(depth)?;
            match stmt {
                Stmt::Let {
                    name, ty, value, ..
                } => {
                    let mut term = self.expr(value, depth + 1)?;
                    if !matches!(ty, Type::Infer) {
                        let ty = self.source_ty(ty)?;
                        self.coerce(term, ty)?;
                        term = ty;
                    }
                    self.sites.push((stmt as *const Stmt as usize, term));
                    self.bind(name, term)?;
                }
                Stmt::Declare { name, ty, .. } => {
                    let term = self.source_ty(ty)?;
                    self.sites.push((stmt as *const Stmt as usize, term));
                    self.bind(name, term)?;
                }
                Stmt::LetPattern { pattern, ty, value } => {
                    let mut term = self.expr(value, depth + 1)?;
                    if !matches!(ty, Type::Infer) {
                        let ty = self.source_ty(ty)?;
                        self.coerce(term, ty)?;
                        term = ty;
                    }
                    self.pattern(pattern, term)?;
                }
                Stmt::Assign { name, value } => {
                    let target = self.lookup(name)?;
                    let value = self.expr(value, depth + 1)?;
                    self.coerce(value, target)?;
                }
                Stmt::AssignPlace { place, value, .. } => {
                    let target = self.place(place)?;
                    let value = self.expr(value, depth + 1)?;
                    self.coerce(value, target)?;
                }
                Stmt::AssignOp {
                    place, op, value, ..
                } => {
                    let target = self.place(place)?;
                    let value = self.expr(value, depth + 1)?;
                    self.operator_assignment(*op, target, value)?;
                }
                Stmt::AssignIndex {
                    array,
                    index,
                    value,
                    ..
                }
                | Stmt::AssignIndexOp {
                    array,
                    index,
                    value,
                    ..
                } => {
                    let base = self.place(array)?;
                    let target = self.field(base, FieldSelector::ConstantIndex(0))?;
                    let value = self.expr(value, depth + 1)?;
                    if !matches!(
                        stmt,
                        Stmt::AssignIndexOp {
                            op: BinaryOp::Shl | BinaryOp::Shr,
                            ..
                        }
                    ) {
                        self.coerce(value, target)?;
                    }
                    let index = self.expr(index, depth + 1)?;
                    let usize = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                    self.unify(index, usize)?;
                }
                Stmt::Expr(expr) | Stmt::ImplicitExpr(expr) => {
                    self.expr(expr, depth + 1)?;
                }
                Stmt::Return(Some(expr)) => {
                    let value = self.expr(expr, depth + 1)?;
                    let result = *self
                        .return_terms
                        .last()
                        .ok_or("missing inference return context")?;
                    self.coerce(value, result)?;
                }
                Stmt::While {
                    condition, body, ..
                } => {
                    self.expr(condition, depth + 1)?;
                    self.block(body, depth + 1)?;
                }
                Stmt::WhileLet {
                    pattern,
                    scrutinee,
                    body,
                    ..
                } => {
                    let term = self.expr(scrutinee, depth + 1)?;
                    self.scopes.push(HashMap::new());
                    self.pattern(pattern, term)?;
                    self.block(body, depth + 1)?;
                    self.scopes.pop();
                }
                Stmt::For {
                    pattern,
                    source,
                    body,
                    ..
                } => {
                    let item = self.iteration_item(source, depth)?;
                    self.sites.push((stmt as *const Stmt as usize, item));
                    self.scopes.push(HashMap::new());
                    self.pattern(pattern, item)?;
                    self.block(body, depth + 1)?;
                    self.scopes.pop();
                }
                _ => {}
            }
        }
        let result = match &block.tail {
            Some(tail) => self.expr(tail, depth + 1)?,
            None if block.statements.is_empty() => self.fixed(ir::Ty::Unit)?,
            None => self.fresh()?,
        };
        self.scopes.pop();
        Ok(result)
    }
    fn integer(&mut self, lit: &IntegerLiteral) -> Result<Term, String> {
        self.node(match lit.suffix {
            Some(ty) => Domain::Fixed(ir::Ty::Int(ty)),
            None => Domain::Integer,
        })
    }
    fn expr(&mut self, expr: &'s Expr, depth: usize) -> Result<Term, String> {
        self.checker.infer_work(depth)?;
        match expr {
            Expr::NativeSyscall(arguments) => {
                if arguments.len() != 7 {
                    return Err("native syscall requires number and six usize arguments".into());
                }
                let usize_term = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                for argument in arguments {
                    let actual = self.expr(argument, depth + 1)?;
                    self.coerce(actual, usize_term)?;
                }
                Ok(usize_term)
            }
            Expr::Struct { name, fields } => {
                let fields = fields
                    .iter()
                    .map(|(n, v)| (FieldSelector::Named(*n), v))
                    .collect::<Vec<_>>();
                self.constructor(name, None, &fields, depth)
            }
            Expr::TupleStruct { name, fields } => {
                let fields = fields
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (FieldSelector::Tuple(i), v))
                    .collect::<Vec<_>>();
                self.constructor(name, None, &fields, depth)
            }
            Expr::EnumVariant { name, fields, .. } => {
                let (owner, variant) = self.checker.split_variant(name)?;
                let fields = fields
                    .iter()
                    .map(|(n, v)| (n.clone(), v))
                    .collect::<Vec<_>>();
                self.constructor(&owner, Some(variant), &fields, depth)
            }
            Expr::Apply { path, value } => {
                let actuals = path
                    .segments
                    .iter()
                    .flat_map(|s| &s.args)
                    .map(|a| self.source_ty(a))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Expr::Call { name, args } = value.as_ref() {
                    let args = args
                        .iter()
                        .map(|a| self.expr(a, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?;
                    let plan = self.checker.signatures.plan()?;
                    if let Some(decl) = plan
                        .registry
                        .functions
                        .get(&self.checker.path_key(name, Namespace::Value)?)
                    {
                        return self.callable(*decl, &args, None, &actuals);
                    }
                }
                let result = self.expr(value, depth + 1)?;
                if let Domain::Nominal(_, inferred) = self.domain(result) {
                    for (a, b) in actuals.iter().zip(inferred) {
                        self.unify(*a, b)?;
                    }
                }
                Ok(result)
            }
            Expr::Integer(lit) => self.integer(lit),
            Expr::Float(lit) => self.node(match lit.suffix {
                Some(ty) => Domain::Fixed(ir::Ty::Float(ty)),
                None => Domain::Float,
            }),
            Expr::Bool(_) => self.fixed(ir::Ty::Bool),
            Expr::Char(_) => self.fixed(ir::Ty::Char),
            Expr::Str(_) => self.fixed(ir::Ty::shared_str_ty()),
            Expr::ByteStr(bytes) => self.fixed(ir::Ty::byte_string_ty(bytes.len())),
            Expr::ConstItem(_) => {
                let value = self
                    .checker
                    .canonical
                    .ok_or("contract: constant constraints require sealed graph inputs")?
                    .constant_value(expr)?;
                self.fixed(const_inputs::scalar_ir_type(value.ty()))
            }
            Expr::Var(path) => {
                if let Some(term) = self.stored_callable(path, depth + 1)? {
                    return Ok(term);
                }
                if matches!(
                    (path.anchor(), path.segments()),
                    (PathAnchor::Relative, [_]) | (PathAnchor::SelfModule, [])
                ) {
                    self.lookup(path)
                } else {
                    self.fresh()
                }
            }
            Expr::Unary { op, value, .. } => {
                let value = self.expr(value, depth + 1)?;
                self.operator_unary(*op, value)
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let left = self.expr(left, depth + 1)?;
                let right = self.expr(right, depth + 1)?;
                self.operator_binary(*op, left, right)
            }
            Expr::Cast { value, target } => {
                self.expr(value, depth + 1)?;
                self.source_ty(target)
            }
            Expr::Call { name, args } => {
                let args = args
                    .iter()
                    .map(|a| self.expr(a, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                let plan = self.checker.signatures.plan()?;
                let declaration = plan
                    .registry
                    .functions
                    .get(&self.checker.path_key(name, Namespace::Value)?)
                    .copied();
                match declaration {
                    Some(id) => self.callable(id, &args, None, &[]),
                    None => {
                        let callee = self.lookup(name)?;
                        match self.callable_application(callee, &args)? {
                            Some(result) => Ok(result),
                            None => self.fresh(),
                        }
                    }
                }
            }
            Expr::AssociatedCall {
                owner,
                name,
                type_args,
                args,
            } => {
                let owner = self.source_ty(owner)?;
                let args = args
                    .iter()
                    .map(|a| self.expr(a, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                let actuals = type_args
                    .iter()
                    .map(|a| self.source_ty(a))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Domain::Nominal(decl, _) = self.domain(owner) {
                    if let Some(candidate) = self.candidate(decl, *name)? {
                        let result = self.callable(candidate, &args, None, &actuals)?;
                        let plan = self.checker.signatures.plan()?;
                        let info = plan.declaration(candidate)?;
                        let schema = plan.function_schema(candidate)?;
                        // Constructors normally return Self. Their result must agree
                        // with the written owner's explicit or inferred arguments.
                        if nominal_parts(&schema.signature.result).map(|p| p.0) == Some(decl) {
                            self.unify(owner, result)?;
                        }
                        let _ = info;
                        return Ok(result);
                    }
                }
                self.fresh()
            }
            Expr::MethodCall {
                receiver,
                name,
                type_args,
                args,
            } => {
                let receiver = self.expr(receiver, depth + 1)?;
                let args = args
                    .iter()
                    .map(|a| self.expr(a, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                let actuals = type_args
                    .iter()
                    .map(|a| self.source_ty(a))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(result) = self.callable_method_application(receiver, *name, &args)? {
                    return Ok(result);
                }
                let owner = self.peel_refs(receiver);
                if let Domain::Nominal(decl, _) = self.domain(owner) {
                    if let Some(candidate) = self.candidate(decl, *name)? {
                        return self.callable(candidate, &args, Some(owner), &actuals);
                    }
                }
                let result = self.fresh()?;
                self.delayed_results.push(result);
                self.methods.push(Method {
                    receiver,
                    name: *name,
                    actuals,
                    args,
                    result,
                });
                Ok(result)
            }
            Expr::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expr(condition, depth + 1)?;
                let left = self.block(then_block, depth + 1)?;
                let right = self.block(else_block, depth + 1)?;
                self.unify(left, right)?;
                Ok(left)
            }
            Expr::IfLet {
                pattern,
                scrutinee,
                then_block,
                else_branch,
            } => {
                let term = self.expr(scrutinee, depth + 1)?;
                self.scopes.push(HashMap::new());
                self.pattern(pattern, term)?;
                let left = self.block(then_block, depth + 1)?;
                self.scopes.pop();
                if let Some(right) = else_branch {
                    let right = self.expr(right, depth + 1)?;
                    self.unify(left, right)?;
                }
                Ok(left)
            }
            Expr::Match { scrutinee, arms } => {
                let term = self.expr(scrutinee, depth + 1)?;
                let result = self.fresh()?;
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    self.pattern(&arm.pattern, term)?;
                    if let Some(g) = &arm.guard {
                        self.expr(g, depth + 1)?;
                    }
                    let body = self.expr(&arm.body, depth + 1)?;
                    self.unify(result, body)?;
                    self.scopes.pop();
                }
                Ok(result)
            }
            Expr::Block(block) | Expr::Unsafe(block) => self.block(block, depth + 1),
            Expr::Array(elements) => {
                let item = self.fresh()?;
                for e in elements {
                    let t = self.expr(e, depth + 1)?;
                    self.unify(item, t)?;
                }
                self.node(Domain::Array(item, elements.len()))
            }
            Expr::Repeat { value, len } => {
                let item = self.expr(value, depth + 1)?;
                self.node(Domain::Array(item, *len))
            }
            Expr::Tuple(fields) => {
                let fields = fields
                    .iter()
                    .map(|f| self.expr(f, depth + 1))
                    .collect::<Result<_, _>>()?;
                self.node(Domain::Tuple(fields))
            }
            Expr::DynamicIndex { base, index, .. } => {
                let base = self.expr(base, depth + 1)?;
                let index = self.expr(index, depth + 1)?;
                if matches!(
                    self.domain(self.peel_refs(base)),
                    Domain::Array(..) | Domain::Slice(..)
                ) {
                    let usize = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                    self.unify(index, usize)?;
                }
                self.field(base, FieldSelector::ConstantIndex(0))
            }
            Expr::Index { base, index, .. } => {
                let base = self.expr(base, depth + 1)?;
                self.field(base, FieldSelector::ConstantIndex(*index))
            }
            Expr::Field { base, field } => {
                let base = self.expr(base, depth + 1)?;
                self.field(base, field.clone())
            }
            Expr::Deref { value } => {
                let base = self.expr(value, depth + 1)?;
                match self.domain(base) {
                    Domain::Ref(_, t) | Domain::RawPtr(_, t) => Ok(t),
                    _ => self.fresh(),
                }
            }
            Expr::BorrowValue { kind, value } => {
                let value = self.expr(value, depth + 1)?;
                self.node(Domain::Ref(*kind, value))
            }
            Expr::Borrow { kind, place } => {
                let value = self.place(place)?;
                self.node(Domain::Ref(*kind, value))
            }
            Expr::Loop { body, .. } => {
                self.block(body, depth + 1)?;
                self.fresh()
            }
            Expr::Format { arguments, .. } => {
                for a in arguments {
                    self.expr(a, depth + 1)?;
                }
                self.fixed(ir::Ty::Unit)
            }
            Expr::FormatInto(source) => {
                for arg in source.arguments.iter().chain(source.rest.iter().flatten()) {
                    self.expr(arg, depth + 1)?;
                }
                self.fixed(ir::Ty::Unit)
            }
            _ => self.fresh(),
        }
    }
}
