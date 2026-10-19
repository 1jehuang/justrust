//! Body-local scalar equality constraints, collected before literal fallback.
//!
//! This is scratch owned by one checking node. It does not publish semantic
//! facts or bypass ordinary checking: solved types are only expected types for
//! unannotated initializers and written iterable literals. Their complete checked
//! IR is still validated.
use super::*;

type Term = usize;
#[derive(Clone, Debug)]
enum Domain {
    Unknown,
    Integer,
    Float,
    Fixed(ir::Ty),
}
struct Variable {
    parent: Term,
    rank: u8,
    domain: Domain,
}
struct Solver<'a, 'b> {
    checker: &'a Checker<'b>,
    variables: Vec<Variable>,
    scopes: Vec<HashMap<LocalKey, Term>>,
    sites: Vec<(usize, Term)>,
    /// Block-local bindings whose initializer is an array literal or a
    /// repeat expression (scratch for builtin index inference).
    arrays: Vec<HashMap<LocalKey, bool>>,
}

pub(super) fn hints(
    checker: &Checker<'_>,
    block: &Block,
    expected: Option<&ir::Ty>,
) -> Result<HashMap<usize, ir::Ty>, String> {
    let mut solver = Solver {
        checker,
        variables: Vec::new(),
        scopes: Vec::new(),
        sites: Vec::new(),
        arrays: vec![HashMap::new()],
    };
    // Existing parameters and enclosing locals are rigid, never reinferred.
    let mut outer = HashMap::new();
    for scope in &checker.scopes {
        for (key, local) in scope {
            let term = solver.fixed(checker.locals[local.0].ty.clone())?;
            outer.insert(*key, term);
        }
    }
    solver.scopes.push(outer);
    let result = solver.block(block, 0)?;
    if let Some(expected) = expected {
        let expected = solver.fixed(expected.clone())?;
        solver.unify(result, expected)?;
    }
    let mut hints = HashMap::new();
    for (site, term) in &solver.sites {
        let root = solver.root(*term);
        let ty = match &solver.variables[root].domain {
            // No new expectation for unresolved classes: expression checking
            // may still obtain stronger peer evidence (fields/generic calls).
            // Its existing i32/f64 fallback runs only after those constraints.
            Domain::Fixed(ty @ (ir::Ty::Int(_) | ir::Ty::Float(_))) => ty.clone(),
            _ => continue,
        };
        hints.insert(*site, ty);
    }
    Ok(hints)
}

impl Solver<'_, '_> {
    fn node(&mut self, domain: Domain) -> Result<Term, String> {
        self.checker.infer_storage(1)?;
        let term = self.variables.len();
        self.variables.push(Variable {
            parent: term,
            rank: 0,
            domain,
        });
        Ok(term)
    }
    fn fixed(&mut self, ty: ir::Ty) -> Result<Term, String> {
        self.node(match ty {
            ir::Ty::Int(_) | ir::Ty::Float(_) => Domain::Fixed(ty),
            _ => Domain::Unknown,
        })
    }
    fn root(&self, mut term: Term) -> Term {
        while self.variables[term].parent != term {
            term = self.variables[term].parent;
        }
        term
    }
    fn unify(&mut self, left: Term, right: Term) -> Result<(), String> {
        self.checker.infer_work(0)?;
        let mut a = self.root(left);
        let mut b = self.root(right);
        if a == b {
            return Ok(());
        }
        let domain = match (&self.variables[a].domain, &self.variables[b].domain) {
            (Domain::Unknown, domain) | (domain, Domain::Unknown) => domain.clone(),
            (Domain::Integer, Domain::Integer) => Domain::Integer,
            (Domain::Float, Domain::Float) => Domain::Float,
            (Domain::Integer, Domain::Fixed(ty @ ir::Ty::Int(_)))
            | (Domain::Fixed(ty @ ir::Ty::Int(_)), Domain::Integer)
            | (Domain::Float, Domain::Fixed(ty @ ir::Ty::Float(_)))
            | (Domain::Fixed(ty @ ir::Ty::Float(_)), Domain::Float) => Domain::Fixed(ty.clone()),
            (Domain::Fixed(a), Domain::Fixed(b)) if a == b => Domain::Fixed(a.clone()),
            (left, right) => {
                let message =
                    format!("local scalar inference constraint {left:?} does not match {right:?}");
                // An inconsistent source equality is a user type error, not
                // corrupt compiler metadata. Keep its typed cause through the
                // canonical node's existing diagnostic transport. Legacy
                // noncanonical checking still receives the same diagnostic.
                let error = FactsError::unproved(&self.checker.signatures.plan()?.budget, &message);
                return Err(match self.checker.canonical {
                    Some(capability) => capability.render(error),
                    None => error.into_legacy_diagnostic(&self.checker.signatures.plan()?.budget),
                });
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
    fn lookup(&mut self, name: &impl LocalInput) -> Result<Term, String> {
        if let Some(key) = self.checker.local_key(name, false)? {
            for scope in self.scopes.iter().rev() {
                if let Some(term) = scope.get(&key) {
                    return Ok(*term);
                }
            }
        }
        self.node(Domain::Unknown)
    }
    fn bind(&mut self, name: &impl LocalInput, term: Term) -> Result<(), String> {
        if let Some(key) = self.checker.local_key(name, true)? {
            self.checker.infer_storage(1)?;
            self.scopes.last_mut().unwrap().insert(key, term);
        }
        Ok(())
    }
    fn pattern(&mut self, pattern: &Pattern, term: Term) -> Result<(), String> {
        match pattern {
            Pattern::Binding {
                name, by_ref: None, ..
            } => self.bind(name, term),
            // Other patterns remain the ordinary checker's responsibility.
            _ => Ok(()),
        }
    }
    fn block(&mut self, block: &Block, depth: usize) -> Result<Term, String> {
        self.checker.infer_work(depth)?;
        self.scopes.push(HashMap::new());
        self.arrays.push(HashMap::new());
        for stmt in &block.statements {
            self.checker.infer_work(depth)?;
            match stmt {
                Stmt::Let {
                    name, ty, value, ..
                } => {
                    let term = self.expr(value, depth + 1)?;
                    let array = matches!(ty, Type::Array(..))
                        || matches!(ty, Type::Infer)
                            && matches!(value, Expr::Array(_) | Expr::Repeat { .. });
                    if let Some(key) = self.checker.local_key(name, true)? {
                        self.checker.infer_storage(1)?;
                        if let Some(scope) = self.arrays.last_mut() {
                            scope.insert(key, array);
                        }
                    }
                    if !matches!(ty, Type::Infer) {
                        let annotation =
                            self.fixed(checked_local_ty(ty, self.checker.signatures)?)?;
                        self.unify(term, annotation)?;
                    } else {
                        self.checker.infer_storage(1)?;
                        self.sites.push((stmt as *const Stmt as usize, term));
                    }
                    self.bind(name, term)?;
                }
                Stmt::Declare { name, ty, .. } => {
                    let term = if matches!(ty, Type::Infer) {
                        self.node(Domain::Unknown)?
                    } else {
                        self.fixed(checked_local_ty(ty, self.checker.signatures)?)?
                    };
                    self.bind(name, term)?;
                }
                Stmt::Assign { name, value } => {
                    let target = self.lookup(name)?;
                    let value = self.expr(value, depth + 1)?;
                    self.unify(target, value)?;
                }
                Stmt::AssignOp { place, op, value } if place.fields.is_empty() => {
                    let target = self.lookup(&place.local)?;
                    let value = self.expr(value, depth + 1)?;
                    if !matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                        self.unify(target, value)?;
                    }
                }
                Stmt::Expr(expr) | Stmt::ImplicitExpr(expr) => {
                    self.expr(expr, depth + 1)?;
                }
                Stmt::Return(Some(expr)) => {
                    let value = self.expr(expr, depth + 1)?;
                    let result = self.fixed(self.checker.result.clone())?;
                    self.unify(value, result)?;
                }
                Stmt::While {
                    condition, body, ..
                } => {
                    self.expr(condition, depth + 1)?;
                    self.block(body, depth + 1)?;
                }
                Stmt::For {
                    pattern,
                    source,
                    body,
                    ..
                } => {
                    let term = match &source.iterable {
                        ForIterable::Range { start, end, .. } if source.adapters.is_empty() => {
                            let start = self.expr(start, depth + 1)?;
                            let end = self.expr(end, depth + 1)?;
                            self.unify(start, end)?;
                            start
                        }
                        ForIterable::Value(value) if source.adapters.is_empty() => match value {
                            Expr::Array(elements) => {
                                let item = self.node(Domain::Unknown)?;
                                for element in elements {
                                    let element = self.expr(element, depth + 1)?;
                                    self.unify(item, element)?;
                                }
                                item
                            }
                            Expr::Repeat { value, .. } => self.expr(value, depth + 1)?,
                            _ => self.node(Domain::Unknown)?,
                        },
                        _ => self.node(Domain::Unknown)?,
                    };
                    self.checker.infer_storage(1)?;
                    self.sites.push((stmt as *const Stmt as usize, term));
                    self.scopes.push(HashMap::new());
                    self.pattern(pattern, term)?;
                    self.block(body, depth + 1)?;
                    self.scopes.pop();
                }
                _ => {}
            }
        }
        let result = match &block.tail {
            Some(tail) => self.expr(tail, depth + 1)?,
            None => self.fixed(ir::Ty::Unit)?,
        };
        self.scopes.pop();
        self.arrays.pop();
        Ok(result)
    }
    fn array_local(&self, expr: &Expr) -> Result<bool, String> {
        let Expr::Var(path) = expr else {
            return Ok(false);
        };
        let Some(key) = self.checker.local_key(path, false)? else {
            return Ok(false);
        };
        for scope in self.arrays.iter().rev() {
            if let Some(array) = scope.get(&key) {
                return Ok(*array);
            }
        }
        Ok(false)
    }
    fn expr(&mut self, expr: &Expr, depth: usize) -> Result<Term, String> {
        self.checker.infer_work(depth)?;
        match expr {
            Expr::Integer(lit) => self.node(match lit.suffix {
                Some(ty) => Domain::Fixed(ir::Ty::Int(ty)),
                None => Domain::Integer,
            }),
            Expr::Float(lit) => self.node(match lit.suffix {
                Some(ty) => Domain::Fixed(ir::Ty::Float(ty)),
                None => Domain::Float,
            }),
            Expr::Bool(_) => self.fixed(ir::Ty::Bool),
            Expr::ConstParam(_) => self.fixed(ir::Ty::Int(ir::IntTy::Usize)),
            Expr::Char(_) => self.fixed(ir::Ty::Char),
            Expr::Str(_) => self.fixed(ir::Ty::shared_str_ty()),
            Expr::Var(path)
                if matches!(
                    (path.anchor(), path.segments()),
                    (PathAnchor::Relative, [_]) | (PathAnchor::SelfModule, [])
                ) =>
            {
                self.lookup(path)
            }
            Expr::Unary { value, .. } => self.expr(value, depth + 1),
            Expr::Binary { op, left, right } => {
                let left = self.expr(left, depth + 1)?;
                let right = self.expr(right, depth + 1)?;
                if !matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                    self.unify(left, right)?;
                }
                if matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                        | BinaryOp::And
                        | BinaryOp::Or
                ) {
                    self.fixed(ir::Ty::Bool)
                } else {
                    Ok(left)
                }
            }
            Expr::Cast { value, target } => {
                self.expr(value, depth + 1)?;
                self.fixed(checked_local_ty(target, self.checker.signatures)?)
            }
            Expr::Call { name, args } => {
                let plan = self.checker.signatures.plan()?;
                let declaration = plan
                    .registry
                    .functions
                    .get(&self.checker.path_key(name, Namespace::Value)?)
                    .copied();
                if let Some(declaration) = declaration {
                    let schema = plan.function_schema(declaration)?;
                    if !schema.env.is_generic() {
                        for (arg, param) in args.iter().zip(&schema.signature.params) {
                            let arg = self.expr(arg, depth + 1)?;
                            let param = self.fixed(param.clone())?;
                            self.unify(arg, param)?;
                        }
                        return self.fixed(schema.signature.result.clone());
                    }
                }
                self.node(Domain::Unknown)
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
            Expr::Block(block) | Expr::Unsafe(block) => self.block(block, depth + 1),
            // Runtime array/slice indices are `usize`; this flows backward to
            // unannotated integer locals used as indices (`let mut i = 0`).
            // Overloaded `Index` on user types keeps its own index type.
            Expr::DynamicIndex {
                base: base_expr,
                index,
            } => {
                // A base known to be a nominal (user `Index` impl) keeps its
                // own index type; arrays, slices and unknown bases admit only
                // `usize` for the builtin operator, which is the stock answer
                // whenever the base is array-like.
                self.expr(base_expr, depth + 1)?;
                let nominal = |ty: &ir::Ty| {
                    let ty = match ty {
                        ir::Ty::Ref { referent, .. } => referent.as_ref(),
                        other => other,
                    };
                    !matches!(ty, ir::Ty::Array { .. } | ir::Ty::Slice { .. })
                };
                let builtin = match base_expr.as_ref() {
                    Expr::Array(_) | Expr::Repeat { .. } => true,
                    Expr::Var(_) => match self.checker.scalar_hint(base_expr) {
                        Ok(Some(ty)) => !nominal(&ty),
                        _ => self.array_local(base_expr)?,
                    },
                    _ => false,
                };
                let index = self.expr(index, depth + 1)?;
                if builtin {
                    let usize = self.fixed(ir::Ty::Int(ir::IntTy::Usize))?;
                    self.unify(index, usize)?;
                }
                self.node(Domain::Unknown)
            }
            // Walk subexpressions for nested evidence (an index inside
            // `v[i].f()`), without claiming the compound result type.
            Expr::MethodCall { receiver, args, .. } => {
                self.expr(receiver, depth + 1)?;
                for arg in args {
                    self.expr(arg, depth + 1)?;
                }
                self.node(Domain::Unknown)
            }
            Expr::Field { base, .. } | Expr::Deref { value: base } => {
                self.expr(base, depth + 1)?;
                self.node(Domain::Unknown)
            }
            Expr::BorrowValue { value, .. } => {
                self.expr(value, depth + 1)?;
                self.node(Domain::Unknown)
            }
            Expr::Loop { body, .. } => {
                self.block(body, depth + 1)?;
                self.node(Domain::Unknown)
            }
            Expr::Format { arguments, .. } => {
                for arg in arguments {
                    self.expr(arg, depth + 1)?;
                }
                self.fixed(ir::Ty::Unit)
            }
            // Unknown forms contribute no speculative evidence. The ordinary
            // checker still handles/rejects them with its full semantic rules.
            _ => self.node(Domain::Unknown),
        }
    }
}
