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

#[path = "local_literals/control_flow.rs"]
mod control_flow;
#[path = "local_literals/opaque.rs"]
mod opaque;

#[path = "local_literals/operators.rs"]
mod operators;

type Term = usize;
#[derive(Clone, Debug)]
enum Domain {
    Unknown,
    Integer,
    Float,
    Fixed(ir::Ty),
    Opaque(Arc<ir::OpaqueTy>, Vec<Term>),
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
    /// Overloaded `Deref` steps already taken from the written receiver.
    depth: usize,
    /// An earlier autoderef step found the method with unmet bounds
    /// (`Vec<N>::clone` for `N: !Clone`); reported at quiescence if no
    /// later step selects anything.
    unmet: Option<String>,
}
struct Projection {
    base: Term,
    field: FieldSelector,
    result: Term,
}
/// Admission-only scratch owned by one inference solve. It never stores a
/// normalization answer, and the borrowed original facts cannot be rebound.
struct ProjectionMetadata<'f> {
    scratch: std::cell::RefCell<tr::TraitScratch>,
    original: &'f tr::ValidatedFacts,
    // Field order drops all scratch buffers and child owners before this lease.
    _storage: ir::PlanReservation,
}
impl<'f> ProjectionMetadata<'f> {
    fn new(original: &'f tr::ValidatedFacts) -> Result<Self, String> {
        // Include the containing Option discriminant/padding conservatively.
        let storage = original
            .budget()
            .reserve_bytes(std::mem::size_of::<Option<Self>>())?;
        Ok(Self {
            scratch: std::cell::RefCell::new(tr::TraitScratch::default()),
            original,
            _storage: storage,
        })
    }
    fn normalize(
        &self,
        facts: &tr::ValidatedFacts,
        ty: &ir::Ty,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        facts
            .normalize_substituted_projection_type_scratch(ty, budget, self.original, &self.scratch)
            .map_err(|error| error.into_legacy_diagnostic(budget))
    }
}

struct Solver<'a: 's, 'b: 's, 's> {
    checker: &'a Checker<'b>,
    projection_metadata: Option<ProjectionMetadata<'b>>,
    variables: Vec<Variable>,
    scopes: Vec<HashMap<LocalKey, Term>>,
    sites: Vec<(usize, Term)>,
    receiver_sites: Vec<(usize, Term)>,
    methods: Vec<Method>,
    projections: Vec<Projection>,
    delayed_results: Vec<Term>,
    /// A dot call saw several applications of one trait with no expected
    /// result type yet. At quiescence this is stock's E0283 boundary.
    trait_application_pending: bool,
    coercions: Vec<(Term, Term)>,
    /// Reference coercions into a still-unknown target. Stock lets a later
    /// expectation (e.g. `let v: Vec<&dyn P> = vec![&a]`) fix the target first;
    /// at quiescence the remaining ones fall back to equality.
    lazy_coercions: Vec<(Term, Term)>,
    /// `&x -> &str` / `&x -> &[T]` where `x`'s type is not known yet: stock
    /// may deref-coerce (`&String -> &str`), so the referents are only equated
    /// once `x` is known, or at quiescence.
    pending_referents: Vec<(Term, Term)>,
    iterator_items: Vec<iteration::Pending>,
    /// `(carrier, payload)` for `e?` whose carrier was not yet nominal.
    tries: Vec<(Term, Term)>,
    patterns: Vec<(&'s Pattern, Term, HashMap<LocalKey, Term>)>,
    pattern_bindings: Option<HashMap<LocalKey, Term>>,
    replaying_pattern: bool,
    closure_terms: Vec<(PathKey, Term)>,
    local_context: Option<Option<super::super::super::semantic_scope::ScopeId>>,
    return_terms: Vec<Term>,
    control_targets: Vec<control_flow::ControlTarget>,
    unit_defaults: Vec<Term>,
    associated_equalities: Vec<(Term, Term)>,
    trait_arguments: Vec<(ir::DeclId, Term, Vec<Term>)>,
    structural_impl_stack: Vec<(ir::DeclId, Term)>,
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
    pub receivers: HashMap<usize, ir::Ty>,
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
    let mut receivers = HashMap::new();
    for (site, term) in &solver.receiver_sites {
        if let Some(ty) = solver.resolve(*term, 0)? {
            receivers.insert(*site, ty);
        }
    }
    Ok(Hints {
        receivers,
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
    let projection_metadata = checker
        .facts()
        .validated()
        .map(ProjectionMetadata::new)
        .transpose()?;
    let mut solver = Solver {
        checker,
        projection_metadata,
        variables: Vec::new(),
        scopes: Vec::new(),
        sites: Vec::new(),
        receiver_sites: Vec::new(),
        methods: Vec::new(),
        projections: Vec::new(),
        delayed_results: Vec::new(),
        trait_application_pending: false,
        coercions: Vec::new(),
        lazy_coercions: Vec::new(),
        pending_referents: Vec::new(),
        iterator_items: Vec::new(),
        tries: Vec::new(),
        patterns: Vec::new(),
        pattern_bindings: None,
        replaying_pattern: false,
        closure_terms: Vec::new(),
        local_context: None,
        return_terms: Vec::new(),
        control_targets: Vec::new(),
        unit_defaults: Vec::new(),
        associated_equalities: Vec::new(),
        trait_arguments: Vec::new(),
        structural_impl_stack: Vec::new(),
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
        if let ir::Ty::Param(param) = ty
            && let Some(term) = params.get(param)
        {
            return Ok(*term);
        }
        if let ir::Ty::Opaque(opaque) = ty {
            return self.opaque_schema(opaque, params, depth + 1);
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
        if self.preparation.is_none()
            && let Ok(ty) = checked_local_ty(ty, self.checker.signatures)
        {
            return self.fixed(ty);
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
            Domain::Nominal(_, args) | Domain::Tuple(args) | Domain::Opaque(_, args) => {
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
            (Domain::Opaque(a, x), Domain::Opaque(b, y))
                if x.len() == y.len() && self.opaque_identity_matches(a, b, depth + 1)? =>
            {
                for (x, y) in x.iter().zip(y) {
                    self.unify_at(*x, *y, depth + 1)?;
                }
                Domain::Opaque(a.clone(), x.clone())
            }
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
                    "E0308: mismatched types: expected {x:?}, found {y:?}"
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
    pub(super) fn contains_callable(&self, term: Term, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        Ok(match self.domain(term) {
            Domain::Callable(_) => true,
            Domain::Nominal(_, args) | Domain::Tuple(args) | Domain::Opaque(_, args) => {
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
            // A finalized closure (or a parent-local application holding one)
            // is realized by the parent, never through a graph demand.
            Domain::Fixed(ty) => super::type_check::nominal_inputs::parent_local::contains_closure(
                std::slice::from_ref(&ty),
                &self.checker.signatures.plan()?.budget,
                depth + 1,
            )?,
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
            Domain::Opaque(descriptor, children) => {
                let Some(ty) = self.resolve_opaque(&descriptor, &children, depth + 1)? else {
                    return Ok(None);
                };
                ty
            }
            Domain::AssociatedProjection(projection) => {
                let Some(ty) = self.resolve_associated_projection(&projection, depth + 1)? else {
                    return Ok(None);
                };
                let Some(ty) = self.resolve_projection_representation(&ty, depth + 1)? else {
                    return Ok(None);
                };
                ty
            }
            Domain::Nominal(decl, args) => {
                if let Some(inputs) = self.preparation {
                    if self.contains_callable(term, depth + 1)? {
                        return Ok(None);
                    }
                    if !self.manifest_ready(&args, depth + 1)? {
                        // Numeric literals alone are not a missing graph input.
                        // Let the normal quiescent defaulting round run before
                        // selecting a canonical nominal specialization. Unknown
                        // actuals still suspend, as do missing source/nominal facts.
                        if !self.manifest_ready_with_defaults(&args, depth + 1, true)? {
                            self.preparation_needs_actuals.set(true);
                        }
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
        written_owner: Option<Term>,
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
        if let Some(owner_term) = written_owner.or(receiver) {
            let associated = info
                .path
                .associated
                .as_ref()
                .ok_or("method without owner")?;
            let owner = self.schema(&associated.self_pattern, &params, 0)?;
            let owner_term = self.peel_refs(owner_term);
            self.unify(owner_term, owner)?;
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
    fn bind_declared_method_result(&mut self, result: Term, delayed: Term) -> Result<(), String> {
        // Once a declaration has supplied a bare generic result parameter,
        // that slot must accept the call site's expected type. It is no longer
        // an unknown producer which might later reveal an array/deref coercion.
        // Other, still-unresolved producers retain their coercion barriers.
        if matches!(self.domain(result), Domain::Unknown) {
            self.delayed_results.retain(|term| *term != delayed);
        }
        self.unify(result, delayed)
    }

    fn unknown_closure_parameter(&self, term: Term) -> Result<bool, String> {
        if !matches!(self.domain(term), Domain::Unknown) {
            return Ok(false);
        }
        for (_, callable) in &self.closure_terms {
            self.checker.infer_work(0)?;
            if let Domain::Callable(callable) = self.domain(*callable) {
                for parameter in callable.params {
                    self.checker.infer_work(0)?;
                    if self.root(parameter) == self.root(term) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn deferred_coercion(&self, actual: Term) -> Result<bool, String> {
        Ok(matches!(self.domain(actual), Domain::Unknown)
            && (self
                .delayed_results
                .iter()
                .any(|result| self.root(*result) == self.root(actual))
                || self.unknown_closure_parameter(actual)?))
    }

    fn coerce(&mut self, actual: Term, expected: Term) -> Result<(), String> {
        // A producer may reveal an array or a Deref carrier. A closure
        // parameter must likewise receive its callable context before its
        // body can constrain a coercion source (e.g. `&&str` to `&str`).
        if self.deferred_coercion(actual)? {
            self.coercions.push((actual, expected));
            return Ok(());
        }
        if (matches!(self.domain(actual), Domain::Ref(_, inner)
                if !matches!(self.domain(inner), Domain::Unknown))
            || matches!(self.domain(actual), Domain::Nominal(_, ref xs)
                if xs.len() == 1 && !matches!(self.domain(xs[0]), Domain::Unknown | Domain::Fixed(ir::Ty::Dyn(_)))))
            && matches!(self.domain(expected), Domain::Unknown)
        {
            self.lazy_coercions.push((actual, expected));
            return Ok(());
        }
        match (self.domain(actual), self.domain(expected)) {
            (Domain::Ref(_, a), Domain::Ref(_, b)) => {
                // A reference coercion may run an authoritative Deref chain.
                // Only equal outer constructors imply equality of type arguments.
                // The ordinary checker proves differing-referent coercions later.
                // Built-in shared-reference deref coercions preserve the
                // original carrier. Do not equate its extra reference layer
                // with a concrete expected referent during inference.
                if matches!(self.domain(a), Domain::Ref(ir::BorrowKind::Shared, _))
                    && !matches!(self.domain(b), Domain::Ref(_, _) | Domain::Unknown)
                {
                    return Ok(());
                }
                if let Domain::Nominal(actual, _) = self.domain(a) {
                    match self.domain(b) {
                        Domain::Nominal(expected, _) if actual == expected => {}
                        Domain::Unknown => {}
                        _ => return Ok(()),
                    }
                }
                // `&T -> &dyn P` unsizing is proven by the checker.
                if matches!(self.domain(b), Domain::Fixed(ir::Ty::Dyn(_))) {
                    return Ok(());
                }
                if matches!(self.domain(a), Domain::Unknown)
                    && matches!(
                        self.domain(b),
                        Domain::Fixed(ir::Ty::Str) | Domain::Slice(_)
                    )
                {
                    self.pending_referents.push((actual, expected));
                    return Ok(());
                }
                self.coerce(a, b)
            }
            // `Box<T> -> Box<dyn P>` unsizing is proven by the checker; the
            // source pointee stays independent of the object type.
            (Domain::Nominal(a, ref xs), Domain::Nominal(b, ref ys))
                if a == b
                    && xs.len() == 1
                    && ys.len() == 1
                    && matches!(self.domain(ys[0]), Domain::Fixed(ir::Ty::Dyn(_)))
                    && !matches!(
                        self.domain(xs[0]),
                        Domain::Fixed(ir::Ty::Dyn(_)) | Domain::Unknown
                    ) =>
            {
                Ok(())
            }
            (Domain::Array(a, n), Domain::Array(b, m)) if n == m => self.coerce(a, b),
            (Domain::Array(a, _), Domain::Slice(b)) => self.unify(a, b),
            _ => self.unify(actual, expected),
        }
    }
    fn join_branches(&mut self, left: Term, right: Term) -> Result<Term, String> {
        match (self.domain(left), self.domain(right)) {
            (Domain::Fixed(ir::Ty::Never), _) => Ok(right),
            (_, Domain::Fixed(ir::Ty::Never)) => Ok(left),
            (Domain::Ref(ir::BorrowKind::Shared, inner), Domain::Ref(ir::BorrowKind::Mut, _))
            | (Domain::Ref(ir::BorrowKind::Mut, _), Domain::Ref(ir::BorrowKind::Shared, inner)) => {
                // The branch result is a coercion target, not an equality of
                // carrier types. Keep each input's declared capability intact.
                // Ordinary checking must still emit and validate the genuine
                // shared reborrow of the mutable branch.
                let result = self.node(Domain::Ref(ir::BorrowKind::Shared, inner))?;
                self.coerce(left, result)?;
                self.coerce(right, result)?;
                Ok(result)
            }
            _ => {
                self.unify(left, right)?;
                Ok(left)
            }
        }
    }
    fn peel_refs(&self, mut term: Term) -> Term {
        while let Domain::Ref(_, inner) = self.domain(term) {
            term = inner;
        }
        term
    }
    fn candidate_for_receiver(
        &self,
        receiver: Term,
        name: AstIdent,
    ) -> Result<Option<ir::DeclId>, String> {
        let Domain::Nominal(nominal, arguments) = self.domain(receiver) else {
            return Ok(None);
        };
        let unfiltered = self.candidate(nominal, name)?;
        if unfiltered.is_some() || self.contains_callable(receiver, 0)? {
            return Ok(unfiltered);
        }
        // Disambiguate only with known argument types. An unresolved literal or
        // generic receiver must not choose a convenient inherent impl.
        let actuals = if self.manifest_ready(&arguments, 0)? {
            self.checker.infer_storage(arguments.len())?;
            arguments
                .iter()
                .map(|argument| self.resolve(*argument, 0))
                .collect::<Result<Option<Vec<_>>, _>>()?
        } else {
            None
        };
        self.candidate_with_actuals(nominal, name, actuals.as_deref())
    }
    fn candidate(&self, nominal: ir::DeclId, name: AstIdent) -> Result<Option<ir::DeclId>, String> {
        self.candidate_with_actuals(nominal, name, None)
    }
    fn candidate_with_actuals(
        &self,
        nominal: ir::DeclId,
        name: AstIdent,
        actuals: Option<&[ir::Ty]>,
    ) -> Result<Option<ir::DeclId>, String> {
        let plan = self.checker.signatures.plan()?;
        let mut found = None;
        for id in &plan.registry.function_ids {
            self.checker.infer_work(0)?;
            let info = plan.declaration(*id)?;
            if let Some(owner) = &info.path.associated
                && owner.trait_decl.is_none()
                && owner.nominal == nominal
                && plan.registry.source_member_equal(info.path.item, name)?
            {
                if let Some(actuals) = actuals {
                    let (_, patterns) = nominal_parts(&owner.self_pattern)
                        .ok_or("inherent candidate lost its nominal self pattern")?;
                    if patterns.len() != actuals.len() {
                        return Err("inherent candidate changed nominal argument arity".into());
                    }
                    let environment = plan.param_env(owner.impl_decl)?;
                    let mut bindings = HashMap::new();
                    let mut matches = true;
                    for (pattern, actual) in patterns.iter().zip(actuals) {
                        matches &= match_impl_type(
                            pattern,
                            actual,
                            &environment,
                            &mut bindings,
                            &plan.budget,
                            0,
                        )?;
                    }
                    if !matches {
                        continue;
                    }
                }
                if found.replace(*id).is_some() {
                    return Ok(None);
                }
            }
        }
        Ok(found)
    }
    // An unresolved method argument can itself be the only reason a later
    // receiver has no nominal actuals yet (`zip` stores `U::IntoIter`). This
    // local cycle is not a missing producer. Never include delayed producer
    // results, closure parameters, numeric defaults or absent graph reads.
    fn lazy_input_frontier(
        &self,
        term: Term,
        targets: &[Term],
        depth: usize,
    ) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        Ok(match self.domain(term) {
            Domain::Unknown => {
                for target in targets {
                    self.checker.infer_work(depth)?;
                    if self.root(*target) == self.root(term) {
                        return Ok(true);
                    }
                }
                false
            }
            // An unsuffixed literal only ever becomes a scalar: it can never
            // reveal a Deref or array carrier for a lazy coercion.
            Domain::Integer | Domain::Float => true,
            Domain::Callable(_) => false,
            Domain::Nominal(_, children)
            | Domain::Tuple(children)
            | Domain::Opaque(_, children) => {
                for child in children {
                    if !self.lazy_input_frontier(child, targets, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            Domain::Ref(_, child)
            | Domain::RawPtr(_, child)
            | Domain::Array(child, _)
            | Domain::Slice(child) => self.lazy_input_frontier(child, targets, depth + 1)?,
            Domain::AssociatedProjection(projection) => {
                if !self.lazy_input_frontier(projection.receiver, targets, depth + 1)? {
                    return Ok(false);
                }
                for argument in projection.arguments {
                    if !self.lazy_input_frontier(argument, targets, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            Domain::Fixed(_) => true,
        })
    }

    fn local_lazy_inputs_ready(&self, methods: &[Method]) -> Result<bool, String> {
        if self.lazy_coercions.is_empty()
            || self.trait_application_pending
            || self.missing_source_schemas.get()
            || !self.missing_nominals.borrow().is_empty()
            || !self.missing_declarations.borrow().is_empty()
            || self
                .operators
                .iter()
                .any(|operator| operator.lazy_frontier_inputs().is_none())
            || !self.associated_equalities.is_empty()
            || !self.patterns.is_empty()
            || !self.pending_referents.is_empty()
            || !self.tries.is_empty()
        {
            return Ok(false);
        }
        self.checker.infer_storage(self.lazy_coercions.len())?;
        let mut targets = Vec::new();
        for (actual, expected) in &self.lazy_coercions {
            self.checker.infer_work(0)?;
            if !matches!(self.domain(*expected), Domain::Unknown)
                || self.deferred_coercion(*expected)?
                || !self.lazy_input_frontier(*actual, &[], 0)?
                || !self.manifest_ready_with_defaults(&[*actual], 0, true)?
                || (self.manifest_ready(&[*actual], 0)? && self.resolve(*actual, 0)?.is_none())
            {
                return Ok(false);
            }
            targets.push(*expected);
        }
        targets.extend(self.operators.iter().filter_map(|o| o.index_result()));
        // A receiver that is itself the unresolved result of another pending
        // method in this body (`Some(&a).cloned().unwrap()`) waits on the
        // same local cycle, never on a missing graph producer.
        targets.extend(methods.iter().map(|method| method.result));
        for operator in &self.operators {
            let Some((left, right)) = operator.lazy_frontier_inputs() else {
                return Ok(false);
            };
            for input in std::iter::once(left).chain(right) {
                if !matches!(self.domain(input), Domain::Integer | Domain::Float)
                    && !self.lazy_input_frontier(input, &targets, 0)?
                {
                    return Ok(false);
                }
            }
        }
        for method in methods {
            for (index, input) in std::iter::once(&method.receiver)
                .chain(&method.actuals)
                .chain(&method.args)
                .enumerate()
            {
                // An unsuffixed literal argument cannot reveal a Deref or
                // array carrier for any lazy coercion: it only ever becomes a
                // scalar. Receivers and explicit generic actuals stay strict.
                if index > method.actuals.len()
                    && matches!(
                        self.domain(self.peel_refs(*input)),
                        Domain::Integer | Domain::Float
                    )
                {
                    continue;
                }
                if !self.lazy_input_frontier(*input, &targets, 0)? {
                    return Ok(false);
                }
            }
        }
        // A loop source waiting on a lazily coerced value (`for x in b.iter()`
        // with `b = Box::new(v)`) is a local cycle like a method receiver.
        for pending in &self.iterator_items {
            let source = match pending {
                iteration::Pending::Ordinary(source, _) => *source,
                iteration::Pending::ArrayMethod { receiver, .. } => *receiver,
            };
            if !self.lazy_input_frontier(source, &targets, 0)? {
                return Ok(false);
            }
        }
        Ok(!self.missing_source_schemas.get()
            && self.missing_nominals.borrow().is_empty()
            && self.missing_declarations.borrow().is_empty())
    }

    fn solve(&mut self) -> Result<(), String> {
        let mut methods = std::mem::take(&mut self.methods);
        let mut projections = std::mem::take(&mut self.projections);
        // Selection failures on fully resolved receivers. Reported only at
        // genuine quiescence, never while a graph input is still missing.
        let mut selection_errors: Vec<String> = Vec::new();
        loop {
            self.preparation_needs_actuals.set(false);
            let mut progress = false;
            let mut pending = Vec::new();
            selection_errors.clear();
            self.trait_application_pending = false;
            for mut method in methods {
                if let Some(result) =
                    self.callable_method_application(method.receiver, method.name, &method.args)?
                {
                    self.unify(result, method.result)?;
                    progress = true;
                    continue;
                }
                // Primitive result/argument constraints come from the same exact
                // signatures as checked lowering, never from the expected result.
                let leading_signature = match self.domain(method.receiver) {
                    Domain::Fixed(ty) => leading_zeros_signature(
                        &ty,
                        method.name,
                        method.actuals.len(),
                        method.args.len(),
                        self.checker.names()?,
                    )?,
                    _ => None,
                };
                let receiver = self.peel_refs(method.receiver);
                // Slice/str have an inherent length operation independent of
                // their element's nominal layout. Keeping this pending until
                // nominal method selection creates a false missing-input cycle.
                // These are result constraints only: checked lowering still
                // validates the exact receiver and its canonical SliceLen site.
                // Arrays deliberately keep ordinary method lookup first, since
                // an in-scope array trait method can shadow slice unsizing.
                if matches!(
                    self.domain(receiver),
                    Domain::Slice(_) | Domain::Fixed(ir::Ty::Str)
                ) && self
                    .checker
                    .names()?
                    .spelling(method.name)
                    .map_err(native_adapter::name_diagnostic)?
                    == "len"
                {
                    // The inherent `len` is selected before any trait method,
                    // so malformed calls are errors, not trait lookups.
                    if !method.actuals.is_empty() {
                        return Err(
                            self.user_error("E0107: method `len` takes 0 generic arguments")
                        );
                    }
                    if !method.args.is_empty() {
                        return Err(self.user_error("E0061: method `len` takes 0 arguments"));
                    }
                }
                if matches!(
                    self.domain(receiver),
                    Domain::Slice(_) | Domain::Fixed(ir::Ty::Str)
                ) && slice_len_method_candidate(
                    method.name,
                    method.actuals.len(),
                    method.args.len(),
                    self.checker.names()?,
                )? {
                    let result = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                    self.unify(result, method.result)?;
                    progress = true;
                    continue;
                }
                let primitive_signature = if leading_signature.is_some() {
                    leading_signature
                } else if method.actuals.is_empty()
                    && let Domain::Fixed(ty) = self.domain(receiver)
                    && let Some(scalar) = ir::ScalarTy::of(&ty)
                {
                    let name = self
                        .checker
                        .names()?
                        .spelling(method.name)
                        .map_err(native_adapter::name_diagnostic)?;
                    ir::Intrinsic::resolve(scalar, name).and_then(ir::Intrinsic::signature)
                } else {
                    None
                };
                if let Some(signature) = primitive_signature {
                    if method.args.len() + 1 != signature.params.len() {
                        return Err(format!(
                            "E0061: primitive method takes {} argument(s) but {} were supplied",
                            signature.params.len() - 1,
                            method.args.len()
                        ));
                    }
                    for (arg, param) in method.args.iter().zip(&signature.params[1..]) {
                        let param = self.fixed(param.clone())?;
                        self.coerce(*arg, param)?;
                    }
                    let result = self.fixed(signature.result)?;
                    self.unify(result, method.result)?;
                    progress = true;
                    continue;
                }
                if let Some(candidate) = self.candidate_for_receiver(receiver, method.name)? {
                    let result = self.callable(
                        candidate,
                        &method.args,
                        Some(receiver),
                        None,
                        &method.actuals,
                    )?;
                    self.bind_declared_method_result(result, method.result)?;
                    progress = true;
                    continue;
                }
                if (self.contains_callable(receiver, 0)?
                    || matches!(self.domain(receiver), Domain::Nominal(..)))
                    && let Some(result) = self.structural_trait_method_constraints(
                        receiver,
                        method.name,
                        &method.args,
                        &method.actuals,
                    )?
                {
                    self.bind_declared_method_result(result, method.result)?;
                    progress = true;
                    continue;
                }
                // A concrete receiver can also select trait or primitive methods.
                if let Some(mut ty) = self.resolve(receiver, 0)? {
                    if matches!(ty, ir::Ty::Array { .. }) {
                        let Some(original) = self.resolve(method.receiver, 0)? else {
                            pending.push(method);
                            continue;
                        };
                        ty = original;
                    }
                    if let Some(result) = self.trait_method_constraints(
                        &ty,
                        method.name,
                        &method.args,
                        &method.actuals,
                        method.result,
                    )? {
                        self.bind_declared_method_result(result, method.result)?;
                        progress = true;
                        continue;
                    }
                    let plan = self.checker.signatures.plan()?;
                    if array_len_receiver::receiver_is_array(&ty)
                        && slice_len_method_candidate(
                            method.name,
                            method.actuals.len(),
                            method.args.len(),
                            self.checker.names()?,
                        )?
                        && plan
                            .select_method_with_arguments_scratch(
                                &ty,
                                &method.name,
                                self.checker.facts(),
                                (&self.checker.module, &plan.budget),
                                &self.checker.trait_scratch,
                                |_, _, _| {
                                    Ok(method_argument_selection::MethodArguments::Ready(
                                        std::borrow::Cow::Borrowed(&[]),
                                    ))
                                },
                                |_, _| Ok(None),
                            )?
                            .is_none()
                    {
                        // Only complete absence of an applicable ordinary
                        // method permits array-to-slice builtin fallback.
                        // Selection errors and missing generic arguments are
                        // not absence and must never authorize this path.
                        let result = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                        self.unify(result, method.result)?;
                        progress = true;
                        continue;
                    }
                    if method.actuals.is_empty() {
                        // Stock probing starts at the receiver as written:
                        // `s.shout()` with `s: &str` selects `impl Tr for &str`
                        // before peeling to `str`. Use the written type when it
                        // is known; selection itself autoderefs from there.
                        let written = match self.resolve(method.receiver, 0)? {
                            Some(written @ ir::Ty::Ref { .. }) if method.depth == 0 => written,
                            _ => ty.clone(),
                        };
                        match plan.select_method(
                            &written,
                            &method.name,
                            &[],
                            self.checker.facts(),
                            &self.checker.module,
                            &plan.budget,
                        ) {
                            Ok(selected) => {
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
                            Err(error) => {
                                // Stock's final probe step unsizes `[T; N]` to
                                // `[T]`. Checked lowering admits only the shared
                                // `&[T]` carrier and reselects the same method.
                                if let Some(carrier) = type_check::array_unsize_carrier(&ty)
                                    && let Ok(selected) = plan.select_method(
                                        &carrier,
                                        &method.name,
                                        &[],
                                        self.checker.facts(),
                                        &self.checker.module,
                                        &plan.budget,
                                    )
                                {
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
                                // Once an exact scalar's ordinary method lookup
                                // has failed, a known primitive name is not an
                                // unresolved nominal dependency. Preserve the
                                // resolver's diagnostic rather than parking it.
                                if let Domain::Fixed(fixed) = self.domain(receiver)
                                    && ir::ScalarTy::of(&fixed).is_some()
                                {
                                    let name = self
                                        .checker
                                        .names()?
                                        .spelling(method.name)
                                        .map_err(native_adapter::name_diagnostic)?;
                                    if ir::IntrinsicOp::candidate_name(name)
                                        || leading_zeros_method_candidate(
                                            method.name,
                                            0,
                                            0,
                                            self.checker.names()?,
                                        )?
                                    {
                                        return Err(error);
                                    }
                                }
                                if plan.method_exists_with_unmet_bounds(
                                    &ty,
                                    &method.name,
                                    &plan.budget,
                                )? {
                                    let name = self
                                        .checker
                                        .names()?
                                        .spelling(method.name)
                                        .map_err(native_adapter::name_diagnostic)?;
                                    let error = format!(
                                        "E0599: the method `{name}` exists but its trait bounds were not satisfied"
                                    );
                                    if method.unmet.is_none() {
                                        method.unmet = Some(error.clone());
                                    }
                                    selection_errors.push(error);
                                }
                            }
                        }
                    }
                    // Every reference/autoref step of this nominal receiver
                    // failed: continue the autoderef chain through its exact
                    // `Deref::Target`, as stock probing does. Checked lowering
                    // reselects the same chain and records each deref call.
                    if method.depth < deref_coercion::MAX_DEREF_STEPS
                        && matches!(ty, ir::Ty::Struct(_) | ir::Ty::Enum(_))
                        && let Some(target) = self.checker.deref_target_hint(&ty)?
                    {
                        let receiver = self.fixed(target)?;
                        pending.push(Method {
                            receiver,
                            depth: method.depth + 1,
                            ..method
                        });
                        progress = true;
                        continue;
                    }
                    // No declaration of this name exists anywhere, so no
                    // autoderef step of this final receiver can select it.
                    if !final_receiver_has_inference(&ty)
                        && method_name_has_no_declaration(plan, &method.name, &plan.budget)?
                    {
                        let name = self
                            .checker
                            .names()?
                            .spelling(method.name)
                            .map_err(native_adapter::name_diagnostic)?;
                        let shown = match self.resolve(method.receiver, 0)? {
                            Some(original) => original,
                            None => ty.clone(),
                        };
                        selection_errors.push(format!(
                            "E0599: no method named `{name}` found for `{}` in the current scope",
                            diagnostic_ty(&shown, plan)
                        ));
                    }
                }
                if let Some(error) = &method.unmet {
                    selection_errors.push(error.clone());
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
            for (carrier, payload) in std::mem::take(&mut self.tries) {
                if let Some(inner) = self.try_payload(carrier)? {
                    self.unify(inner, payload)?;
                    progress = true;
                } else {
                    self.tries.push((carrier, payload));
                }
            }
            progress |= self.solve_associated_equalities()?;
            progress |= self.solve_operators()?;
            let coercions = std::mem::take(&mut self.coercions);
            for (actual, expected) in coercions {
                if self.deferred_coercion(actual)? {
                    self.coercions.push((actual, expected));
                } else {
                    self.coerce(actual, expected)?;
                    progress = true;
                }
            }
            progress |= self.infer_trait_arguments()?;
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
            for (actual, expected) in std::mem::take(&mut self.lazy_coercions) {
                if matches!(self.domain(expected), Domain::Unknown) {
                    self.lazy_coercions.push((actual, expected));
                } else {
                    self.coerce(actual, expected)?;
                    progress = true;
                }
            }
            for (actual, expected) in std::mem::take(&mut self.pending_referents) {
                let Domain::Ref(_, inner) = self.domain(actual) else {
                    self.coerce(actual, expected)?;
                    progress = true;
                    continue;
                };
                if matches!(self.domain(inner), Domain::Unknown) {
                    self.pending_referents.push((actual, expected));
                } else {
                    self.coerce(actual, expected)?;
                    progress = true;
                }
            }
            if !progress
                && self.preparation.is_some()
                && projections.iter().all(|projection| {
                    // Field reads of a still-pending local method result
                    // (`x.cloned().unwrap().n`) wait on the same local cycle.
                    matches!(self.domain(projection.base), Domain::Unknown)
                        && methods
                            .iter()
                            .any(|m| self.root(m.result) == self.root(projection.base))
                })
                && self.local_lazy_inputs_ready(&methods)?
            {
                for (actual, expected) in std::mem::take(&mut self.lazy_coercions) {
                    self.unify(actual, expected)?;
                }
                continue;
            }
            // Missing graph inputs are not inference quiescence. A pending
            // method producer may still reveal a nominal owner, so neither
            // referent nor lazy-coercion fallback may overwrite that owner.
            if !progress
                && self.preparation.is_some()
                && (self.preparation_needs_actuals.get()
                    || self.missing_source_schemas.get()
                    || !self.missing_nominals.borrow().is_empty())
            {
                break;
            }
            if !progress && !self.pending_referents.is_empty() {
                for (actual, expected) in std::mem::take(&mut self.pending_referents) {
                    if let (Domain::Ref(_, a), Domain::Ref(_, b)) =
                        (self.domain(actual), self.domain(expected))
                    {
                        self.unify(a, b)?;
                    } else {
                        self.unify(actual, expected)?;
                    }
                }
                continue;
            }
            if !progress && !self.lazy_coercions.is_empty() {
                for (actual, expected) in std::mem::take(&mut self.lazy_coercions) {
                    self.unify(actual, expected)?;
                }
                continue;
            }
            if !progress {
                if !self.projection_defaults {
                    self.projection_defaults = true;
                    continue;
                }
                // Only genuine quiescence permits standalone closure body
                // constraints to infer a still-unknown parameter. Missing
                // producers above retain their coercion barriers unchanged.
                let mut inferred_parameter = false;
                for (actual, expected) in std::mem::take(&mut self.coercions) {
                    if self.unknown_closure_parameter(actual)?
                        && !self
                            .delayed_results
                            .iter()
                            .any(|result| self.root(*result) == self.root(actual))
                    {
                        self.unify(actual, expected)?;
                        inferred_parameter = true;
                    } else {
                        self.coercions.push((actual, expected));
                    }
                }
                if inferred_parameter {
                    continue;
                }
                if let Some(error) = selection_errors.into_iter().find(|e| is_bound_failure(e)) {
                    return Err(error);
                }
                if self.trait_application_pending {
                    return Err(trait_selection::trait_application_ambiguity());
                }
                break;
            }
        }
        self.methods = methods;
        self.projections = projections;
        Ok(())
    }
    fn project(&mut self, base: Term, field: &FieldSelector) -> Result<Option<Term>, String> {
        // Autoderef belongs to field/index lookup. An explicit Deref must
        // consume exactly its reference or pointer layer, not the referent.
        let base = if matches!(field, FieldSelector::Deref) {
            base
        } else {
            self.peel_refs(base)
        };
        match (self.domain(base), field) {
            (Domain::Ref(_, element) | Domain::RawPtr(_, element), FieldSelector::Deref) => {
                Ok(Some(element))
            }
            (Domain::Tuple(fields), FieldSelector::Tuple(i)) => Ok(fields.get(*i).copied()),
            (
                Domain::Array(element, _) | Domain::Slice(element),
                FieldSelector::ConstantIndex(_),
            ) => Ok(Some(element)),
            (Domain::Nominal(decl, args), FieldSelector::Tuple(_) | FieldSelector::Named(_)) => {
                if !self.checker.has_member(decl, field)? {
                    // Field autoderef through the exact `Deref::Target`.
                    let Some(owner) = self.resolve(base, 0)? else {
                        return Ok(None);
                    };
                    let Some(target) = self.checker.deref_target_hint(&owner)? else {
                        return Err("unknown nominal field".into());
                    };
                    let target = self.fixed(target)?;
                    return self.project(target, field);
                }
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
                let definitions = schema.def().fields.read(&plan.budget)?;
                let Some(field) = definitions.get(index) else {
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
            term = if matches!(field, FieldSelector::ConstantIndex(_)) {
                let index = self.node(Domain::Integer)?;
                self.operator_index(term, index, false)?
            } else if matches!(field, FieldSelector::Deref)
                && !matches!(self.domain(term), Domain::Ref(..) | Domain::RawPtr(..))
            {
                // Builtin once the carrier is known, otherwise overloaded Deref.
                self.operator_deref(term)?
            } else {
                self.field(term, field.clone())?
            };
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
        let definitions = definitions.read(&plan.budget)?;
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
            Pattern::At { name, pattern, .. } => {
                // The whole-place name and its subpattern bindings share the
                // same scope rules as a plain binding.
                if let Some(key) = self.inference_key(name, true)? {
                    let term = match bindings.get(&key) {
                        Some(term) => *term,
                        None => self.fresh()?,
                    };
                    bindings.insert(key, term);
                    self.bind(name, term)?;
                }
                self.defer_bindings(pattern, bindings)?;
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
            Pattern::At {
                name,
                by_ref,
                pattern,
                ..
            } => {
                // `name @ pat` binds the whole matched value with the same
                // type and mode as a plain binding, then tests `pat` against
                // the same term. Admission (Copy/shared only) stays in the
                // checker; inference only supplies the type.
                let bound = match by_ref {
                    Some(kind) => self.node(Domain::Ref(*kind, term))?,
                    None => term,
                };
                self.bind(name, bound)?;
                self.pattern(pattern, term)
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
            Pattern::Array(_) | Pattern::ArrayRest(_) => {
                // `[a, b]` / `[a, .., z]` bind elements of an array or slice
                // referent. An unknown exact-length referent is an array;
                // rest patterns leave the container shape to the checker.
                let (term, mode) = self.pattern_referent(term);
                let element = match self.domain(term) {
                    Domain::Array(element, _) | Domain::Slice(element) => element,
                    _ => {
                        let element = self.fresh()?;
                        if let Pattern::Array(fields) = pattern
                            && matches!(self.domain(term), Domain::Unknown)
                        {
                            let array = self.node(Domain::Array(element, fields.len()))?;
                            self.unify(term, array)?;
                        }
                        element
                    }
                };
                let fields: Vec<&'s Pattern> = match pattern {
                    Pattern::Array(fields) => fields.iter().collect(),
                    Pattern::ArrayRest(rest) => rest.prefix.iter().chain(&rest.suffix).collect(),
                    _ => unreachable!(),
                };
                for field in fields {
                    let binding = match mode {
                        Some(kind) => self.node(Domain::Ref(kind, element))?,
                        None => element,
                    };
                    self.pattern(field, binding)?;
                }
                Ok(())
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
        let definitions = definitions.read(&plan.budget)?;
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
                    // Like a plain let, publish the solved initializer type so
                    // the checker types literal arms/branches of the value from
                    // the same unification (`let _ = match .. { .. => y, _ => 0 }`).
                    self.sites.push((stmt as *const Stmt as usize, term));
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
                    let index = self.expr(index, depth + 1)?;
                    let target = self.operator_index(base, index, true)?;
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
                }
                Stmt::AssignNestedIndex { target, op, value } => {
                    // The target is a place: index operands type as `usize`
                    // and the written element constrains the value.
                    let target = self.expr(target, depth + 1)?;
                    let value = self.expr(value, depth + 1)?;
                    match op {
                        Some(op) => self.operator_assignment(*op, target, value)?,
                        None => self.coerce(value, target)?,
                    }
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
                    label,
                    condition,
                    body,
                } => {
                    self.expr(condition, depth + 1)?;
                    let unit = self.fixed(ir::Ty::Unit)?;
                    self.enter_control(*label, unit, false)?;
                    self.block(body, depth + 1)?;
                    self.leave_control()?;
                }
                Stmt::WhileLet {
                    label,
                    pattern,
                    scrutinee,
                    body,
                    ..
                } => {
                    let term = self.expr(scrutinee, depth + 1)?;
                    let unit = self.fixed(ir::Ty::Unit)?;
                    self.enter_control(*label, unit, false)?;
                    self.scopes.push(HashMap::new());
                    self.pattern(pattern, term)?;
                    self.block(body, depth + 1)?;
                    self.scopes.pop();
                    self.leave_control()?;
                }
                Stmt::For {
                    label,
                    pattern,
                    source,
                    body,
                    ..
                } => {
                    let item = self.iteration_item(source, depth)?;
                    self.sites.push((stmt as *const Stmt as usize, item));
                    let unit = self.fixed(ir::Ty::Unit)?;
                    self.enter_control(*label, unit, false)?;
                    self.scopes.push(HashMap::new());
                    self.pattern(pattern, item)?;
                    self.block(body, depth + 1)?;
                    self.scopes.pop();
                    self.leave_control()?;
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
                    return Err(
                        "native syscall requires exactly seven operands: number and six usize arguments"
                            .into(),
                    );
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
                        return self.callable(*decl, &args, None, None, &actuals);
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
            Expr::AssociatedFn(value) => {
                let ty = self.checker.associated_fn_item(value)?;
                self.fixed(ty)
            }
            Expr::Var(path) => {
                if let Some(term) = self.stored_callable(path, depth + 1)? {
                    return Ok(term);
                }
                if !is_local_shaped_path(path)
                    && let Some(ty) = self.checker.fn_item_value(path)?
                {
                    return self.fixed(ty);
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
                // Cast chains (`x as u8 as u8 ...`) are linear: walk them
                // iteratively so long chains do not grow the native stack by
                // one large solver frame per cast.
                let mut targets = vec![target];
                let mut operand: &'s Expr = value;
                let mut operand_depth = depth + 1;
                while let Expr::Cast { value, target } = operand {
                    self.checker.infer_work(operand_depth)?;
                    targets.push(target);
                    operand = value;
                    operand_depth += 1;
                }
                self.expr(operand, operand_depth)?;
                let mut result = None;
                for target in targets.into_iter().rev() {
                    result = Some(self.source_ty(target)?);
                }
                Ok(result.expect("cast chain has at least one target"))
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
                    Some(id) => self.callable(id, &args, None, None, &[]),
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
                if let Some(candidate) = self.candidate_for_receiver(owner, *name)? {
                    return self.callable(candidate, &args, None, Some(owner), &actuals);
                }
                // A static trait method may be reached through a concrete type
                // (T::from_str). Reuse ordinary selection only after its owner
                // and all written type arguments are known. No unresolved slot
                // may choose an implementation or supply projection authority.
                if let Some(owner) = self.resolve(owner, 0)? {
                    let resolved = actuals
                        .iter()
                        .map(|actual| self.resolve(*actual, 0))
                        .collect::<Result<Option<Vec<_>>, _>>()?;
                    if let Some(actuals) = resolved {
                        let plan = self.checker.signatures.plan()?;
                        let mut selected = plan
                            .select_associated_with_scratch(
                                &owner,
                                name,
                                &actuals,
                                self.checker.facts(),
                                &self.checker.module,
                                false,
                                &plan.budget,
                                &self.checker.trait_scratch,
                            )
                            .ok();
                        // Several applications of one trait (`u64: From<u8>`,
                        // `u64: From<u16>`): choose only from exactly known
                        // argument types, then select as an explicit trait call.
                        if selected.is_none() && actuals.is_empty() {
                            let argument_types = args
                                .iter()
                                .map(|argument| self.resolve(*argument, 0))
                                .collect::<Result<Option<Vec<_>>, _>>()?;
                            if let Some(argument_types) = argument_types
                                && let Some(application) =
                                    self.checker.trait_application_for_argument_types(
                                        &owner,
                                        name,
                                        &argument_types,
                                    )?
                            {
                                selected = plan
                                    .select_trait_with_scratch(
                                        &application,
                                        name,
                                        &[],
                                        self.checker.facts(),
                                        false,
                                        &plan.budget,
                                        &self.checker.trait_scratch,
                                    )
                                    .ok()
                                    .flatten();
                            }
                        }
                        if let Some((_, signature, _)) = selected
                            && signature.params.len() == args.len()
                        {
                            for (argument, parameter) in args.iter().zip(&signature.params) {
                                let parameter = self.fixed(parameter.clone())?;
                                self.coerce(*argument, parameter)?;
                            }
                            return self.fixed(signature.result);
                        }
                        // Several applications whose results agree (every
                        // `u32: From<_>` returns u32): the result is known
                        // before the argument. Checked selection still picks
                        // and discharges the exact application later.
                        if actuals.is_empty()
                            && let Some(result) = self.checker.shared_trait_application_result(
                                &owner,
                                name,
                                args.len(),
                            )?
                        {
                            return self.fixed(result);
                        }
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
                let receiver_site = receiver.as_ref() as *const Expr as usize;
                let receiver = self.expr(receiver, depth + 1)?;
                // A nested generic call may acquire its result type only from
                // this method's result context. Retain its site now, then emit
                // a hint only if the complete receiver resolves after solving.
                // Checked method selection still authenticates that type.
                if matches!(self.domain(receiver), Domain::Unknown | Domain::Nominal(..)) {
                    self.checker.infer_storage(1)?;
                    self.receiver_sites.push((receiver_site, receiver));
                }
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
                if let Some(candidate) = self.candidate_for_receiver(owner, *name)? {
                    return self.callable(candidate, &args, Some(owner), None, &actuals);
                }
                let result = self.fresh()?;
                self.delayed_results.push(result);
                self.methods.push(Method {
                    receiver,
                    name: *name,
                    actuals,
                    args,
                    result,
                    depth: 0,
                    unmet: None,
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
                self.join_branches(left, right)
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
                    return self.join_branches(left, right);
                }
                Ok(left)
            }
            Expr::Match { scrutinee, arms } => {
                let term = self.expr(scrutinee, depth + 1)?;
                let mut result = self.fresh()?;
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    self.pattern(&arm.pattern, term)?;
                    if let Some(g) = &arm.guard {
                        self.expr(g, depth + 1)?;
                    }
                    let body = self.expr(&arm.body, depth + 1)?;
                    result = self.join_branches(result, body)?;
                    self.scopes.pop();
                }
                Ok(result)
            }
            Expr::Block(block) | Expr::Unsafe(block) => self.block(block, depth + 1),
            Expr::Array(elements) => {
                let item = self.fresh()?;
                for e in elements {
                    let t = self.expr(e, depth + 1)?;
                    // Elements are coercion sites (e.g. `[&a, &b]` to `[&dyn P; 2]`).
                    if matches!(self.domain(t), Domain::Ref(..))
                        && matches!(self.domain(item), Domain::Ref(..))
                    {
                        self.coerce(t, item)?;
                    } else {
                        self.unify(item, t)?;
                    }
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
                self.operator_index(base, index, false)
            }
            Expr::Index { base, .. } => {
                let base = self.expr(base, depth + 1)?;
                let index = self.node(Domain::Integer)?;
                self.operator_index(base, index, false)
            }
            // `s[a..b]` is the unsized `[T]` of an array, slice or prelude
            // Vec base; bounds are `usize`. Unknown bases stay unresolved.
            Expr::RangeIndex {
                base, start, end, ..
            } => {
                let base = self.expr(base, depth + 1)?;
                for bound in [start, end].into_iter().flatten() {
                    let bound = self.expr(bound, depth + 1)?;
                    let usize = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                    self.unify(bound, usize)?;
                }
                let base = self.peel_refs(base);
                match self.domain(base) {
                    Domain::Array(element, _) | Domain::Slice(element) => {
                        self.node(Domain::Slice(element))
                    }
                    Domain::Nominal(decl, args) if self.is_prelude_vec(decl)? => {
                        match args.first() {
                            Some(&element) => self.node(Domain::Slice(element)),
                            None => self.fresh(),
                        }
                    }
                    _ => self.fresh(),
                }
            }
            Expr::Field { base, field } => {
                let base = self.expr(base, depth + 1)?;
                self.field(base, field.clone())
            }
            Expr::Deref { value } => {
                let base = self.expr(value, depth + 1)?;
                match self.domain(base) {
                    Domain::Ref(_, t) | Domain::RawPtr(_, t) => Ok(t),
                    _ => self.operator_deref(base),
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
            Expr::Loop { label, body } => {
                let result = self.fresh()?;
                self.enter_control(*label, result, false)?;
                self.block(body, depth + 1)?;
                let (has_break, result) = self.leave_control_result()?;
                if has_break {
                    Ok(result)
                } else {
                    self.fixed(ir::Ty::Never)
                }
            }
            Expr::LabeledBlock { label, body } => {
                let result = self.fresh()?;
                self.enter_control(Some(*label), result, true)?;
                let tail = self.block(body, depth + 1)?;
                let (has_break, result) = self.leave_control_result()?;
                let result = match (self.domain(result), self.domain(tail)) {
                    (
                        Domain::Ref(ir::BorrowKind::Shared, _),
                        Domain::Ref(ir::BorrowKind::Mut, _),
                    )
                    | (
                        Domain::Ref(ir::BorrowKind::Mut, _),
                        Domain::Ref(ir::BorrowKind::Shared, _),
                    ) => self.join_branches(result, tail)?,
                    _ => {
                        self.coerce(tail, result)?;
                        result
                    }
                };
                if !has_break && matches!(self.domain(tail), Domain::Fixed(ir::Ty::Never)) {
                    Ok(tail)
                } else {
                    Ok(result)
                }
            }
            Expr::Break { label, value } => {
                let value = match value {
                    Some(value) => self.expr(value, depth + 1)?,
                    None => self.fixed(ir::Ty::Unit)?,
                };
                self.infer_break(*label, value)
            }
            Expr::Continue { label } => self.infer_continue(*label),
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
            Expr::Try(operand) => {
                // `e?` on the prelude Option/Result yields the success
                // payload: tie the result term to the carrier's `T` so later
                // uses (`x + x`) see the operand's exact type.
                let carrier = self.expr(operand, depth + 1)?;
                if let Some(payload) = self.try_payload(carrier)? {
                    return Ok(payload);
                }
                // Not yet a known carrier: a deferred equality solved in the
                // same fixed point as pending method results.
                let payload = self.fresh()?;
                self.tries.push((carrier, payload));
                Ok(payload)
            }
            // `<T as Trait>::item(args)` with an explicit closed owner and no
            // turbofish: the selected impl signature's closed result is exact
            // evidence. Anything open stays unconstrained for checked lowering,
            // which reselects and proves the same call.
            Expr::TraitCall {
                owner,
                trait_path,
                name,
                type_args,
                args,
            } if !matches!(owner, Type::Infer) && type_args.is_empty() => {
                for argument in args {
                    self.expr(argument, depth + 1)?;
                }
                let checker = self.checker;
                let plan = checker.signatures.plan()?;
                let Ok(owner) = super::type_check::checked_local_ty(owner, checker.signatures)
                else {
                    return self.fresh();
                };
                let Ok(trait_ref) = plan.resolve_trait_use(
                    &owner,
                    trait_path,
                    checker.signatures,
                    checker.facts(),
                    &plan.budget,
                ) else {
                    return self.fresh();
                };
                match plan.select_trait_with_scratch(
                    &trait_ref,
                    name,
                    &[],
                    checker.facts(),
                    false,
                    &plan.budget,
                    &checker.trait_scratch,
                ) {
                    Ok(Some((_, signature, _)))
                        if signature.result.is_closed_shallow()
                            && !generic_nominal(&signature.result) =>
                    {
                        self.fixed(signature.result)
                    }
                    _ => self.fresh(),
                }
            }
            _ => self.fresh(),
        }
    }
    fn is_prelude_vec(&self, decl: ir::DeclId) -> Result<bool, String> {
        let plan = self.checker.signatures.plan()?;
        let name = &plan.registry.declaration(decl)?.name;
        let std = crate::compiler::source::source_prelude::PRELUDE_MODULE;
        Ok(*name == format!("crate::{std}::vec::Vec"))
    }
    /// Success payload term of a prelude Option/Result carrier term.
    fn try_payload(&self, carrier: Term) -> Result<Option<Term>, String> {
        let Domain::Nominal(decl, args) = self.domain(carrier) else {
            return Ok(None);
        };
        let Some(&payload) = args.first() else {
            return Ok(None);
        };
        let plan = self.checker.signatures.plan()?;
        let name = &plan.registry.declaration(decl)?.name;
        let std = crate::compiler::source::source_prelude::PRELUDE_MODULE;
        Ok((*name == format!("crate::{std}::option::Option")
            || *name == format!("crate::{std}::result::Result"))
        .then_some(payload))
    }
}

/// A receiver is final only when no part of it can still be refined.
fn final_receiver_has_inference(ty: &ir::Ty) -> bool {
    match ty {
        ir::Ty::Opaque(_) | ir::Ty::Closure(_) | ir::Ty::AssociatedType(_) => true,
        ir::Ty::Ref { referent, .. } => final_receiver_has_inference(referent),
        ir::Ty::RawPtr { pointee, .. } => final_receiver_has_inference(pointee),
        ir::Ty::Array { element, .. } | ir::Ty::Slice { element } => {
            final_receiver_has_inference(element)
        }
        ir::Ty::Tuple(items) => items.iter().any(final_receiver_has_inference),
        ir::Ty::Struct(t) => t.def().type_args.iter().any(final_receiver_has_inference),
        ir::Ty::Enum(t) => t.def().type_args.iter().any(final_receiver_has_inference),
        _ => false,
    }
}

/// Diagnostic-only absence proof over the sealed registry: no inherent,
/// trait, trait-default or prelude callable and no builtin primitive
/// operation is spelled `name`. Never selects a callee.
fn method_name_has_no_declaration(
    plan: &PlanCtx,
    name: &AstIdent,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    let spelling = plan
        .program
        .names
        .spelling(*name)
        .map_err(native_adapter::name_diagnostic)?;
    if ir::IntrinsicOp::candidate_name(spelling)
        || matches!(spelling, "len" | "iter" | "leading_zeros")
    {
        return Ok(false);
    }
    for id in plan
        .registry
        .function_ids
        .iter()
        .chain(plan.registry.trait_items.iter().flatten())
    {
        budget.charge_work(1)?;
        let info = plan.declaration(*id)?;
        if plan.registry.source_member_equal(info.path.item, *name)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn diagnostic_ty(ty: &ir::Ty, plan: &PlanCtx) -> String {
    use ir::{FloatTy as F, IntTy as I};
    let list = |items: &[ir::Ty]| {
        items
            .iter()
            .map(|t| diagnostic_ty(t, plan))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let nominal = |name: &str, args: &[ir::Ty]| {
        let short = name.rsplit("::").next().unwrap_or(name);
        if args.is_empty() {
            short.to_owned()
        } else {
            format!("{short}<{}>", list(args))
        }
    };
    match ty {
        ir::Ty::Int(i) => match i {
            I::I8 => "i8",
            I::I16 => "i16",
            I::I32 => "i32",
            I::I64 => "i64",
            I::Isize => "isize",
            I::I128 => "i128",
            I::U8 => "u8",
            I::U16 => "u16",
            I::U32 => "u32",
            I::U64 => "u64",
            I::Usize => "usize",
            I::U128 => "u128",
        }
        .into(),
        ir::Ty::Float(F::F32) => "f32".into(),
        ir::Ty::Float(F::F64) => "f64".into(),
        ir::Ty::Char => "char".into(),
        ir::Ty::Str => "str".into(),
        ir::Ty::Bool => "bool".into(),
        ir::Ty::Unit => "()".into(),
        ir::Ty::Never => "!".into(),
        ir::Ty::Ref { kind, referent } => format!(
            "&{}{}",
            if *kind == ir::BorrowKind::Mut {
                "mut "
            } else {
                ""
            },
            diagnostic_ty(referent, plan)
        ),
        ir::Ty::RawPtr { kind, pointee } => format!(
            "*{} {}",
            if *kind == ir::BorrowKind::Mut {
                "mut"
            } else {
                "const"
            },
            diagnostic_ty(pointee, plan)
        ),
        ir::Ty::Tuple(items) => format!("({})", list(items)),
        ir::Ty::Array { element, len } => format!("[{}; {len}]", diagnostic_ty(element, plan)),
        ir::Ty::Slice { element } => format!("[{}]", diagnostic_ty(element, plan)),
        ir::Ty::Struct(t) => nominal(&t.def().name, &t.def().type_args),
        ir::Ty::Enum(t) => nominal(&t.def().name, &t.def().type_args),
        ir::Ty::Param(p) => {
            let index = p.index as usize;
            plan.declaration(p.owner.declaration)
                .ok()
                .and_then(|info| info.type_params.get(index).copied())
                .and_then(|ident| plan.program.names.spelling(ident).ok().map(str::to_owned))
                .unwrap_or_else(|| "Self".into())
        }
        ir::Ty::Dyn(d) => match plan.declaration(d.principal) {
            Ok(info) => format!("dyn {}", nominal(&info.name, &d.args)),
            Err(_) => "dyn _".into(),
        },
        ir::Ty::Opaque(_) | ir::Ty::Closure(_) | ir::Ty::FnItem(_) | ir::Ty::AssociatedType(_) => {
            "_".into()
        }
    }
}

/// An exact method-selection failure (unmet trait bound or proven absent
/// method), which stock reports as E0599/E0277 rather than an inference
/// boundary.
fn is_bound_failure(error: &str) -> bool {
    error.starts_with("E0599") || error.starts_with("E0277")
}

/// A generic nominal application in a selected signature result is a schema
/// application, not the caller's realized identity; it must come from graph
/// realization, never be fixed directly as local evidence.
fn generic_nominal(ty: &ir::Ty) -> bool {
    match ty {
        ir::Ty::Struct(n) => !n.def().type_args.is_empty(),
        ir::Ty::Enum(n) => !n.def().type_args.is_empty(),
        ir::Ty::Tuple(parts) => parts.iter().any(generic_nominal),
        ir::Ty::Ref { referent: t, .. }
        | ir::Ty::RawPtr { pointee: t, .. }
        | ir::Ty::Array { element: t, .. }
        | ir::Ty::Slice { element: t } => generic_nominal(t),
        _ => false,
    }
}

#[cfg(test)]
mod projection_metadata_tests {
    use super::*;

    fn facts() -> Arc<tr::ValidatedFacts> {
        let program = super::super::parse("fn probe<T>(x:T){}fn main(){}").unwrap();
        let plan =
            super::super::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        plan.trait_world()
            .unwrap()
            .callable_facts(plan.function_decl(0).unwrap())
            .unwrap()
    }

    #[test]
    fn solver_projection_metadata_header_exact_short_drop_and_poison() {
        for short in [false, true] {
            let facts = facts();
            let budget = facts.budget();
            let paid = std::mem::size_of::<Option<ProjectionMetadata<'_>>>();
            let before = budget.usage_snapshot().bytes;
            let filler_bytes = budget.limits().bytes - before - paid + usize::from(short);
            let filler = budget.reserve_bytes(filler_bytes).unwrap();
            let before_owner = budget.usage_snapshot().bytes;
            let result = ProjectionMetadata::new(&facts);
            assert_eq!(result.is_err(), short);
            if let Ok(owner) = result {
                assert_eq!(budget.usage_snapshot().bytes, before_owner + paid);
                drop(owner);
                assert_eq!(budget.usage_snapshot().bytes, before_owner);
            } else {
                assert_eq!(
                    budget.first_resource_failure().unwrap().kind,
                    ir::ResourceBudgetKind::Bytes
                );
                assert_eq!(budget.usage_snapshot().bytes, before_owner);
            }
            let failure = budget.first_resource_failure();
            drop(filler);
            assert_eq!(budget.usage_snapshot().bytes, before);
            assert_eq!(budget.first_resource_failure(), failure);
        }
        let facts = facts();
        let before = facts.budget().usage_snapshot().bytes;
        let owner = ProjectionMetadata::new(&facts).unwrap();
        let ty = ir::Ty::Tuple(Arc::from([ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)]));
        facts
            .validate_ty_metadata_scratch(&ty, facts.budget(), &mut owner.scratch.borrow_mut())
            .unwrap();
        assert!(facts.budget().usage_snapshot().bytes > before);
        drop(owner);
        assert_eq!(facts.budget().usage_snapshot().bytes, before);

        let owner = ProjectionMetadata::new(&facts).unwrap();
        let parent = facts.budget().reserve_bytes(64).unwrap();
        let live = facts.budget().usage_snapshot().bytes;
        facts.budget().poison_ledger_for_tests();
        assert!(
            owner
                .normalize(&facts, &ir::Ty::Unit, facts.budget())
                .is_err()
        );
        assert!(ProjectionMetadata::new(&facts).is_err());
        drop(owner);
        drop(parent);
        assert_eq!(facts.budget().usage_snapshot().bytes, live);
    }
}
