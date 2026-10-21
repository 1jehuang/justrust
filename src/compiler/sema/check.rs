//! Bidirectional body checker over `hir`, using the inference context and
//! trait solver. One `check_body` call is one node's computation.

use super::hir::*;
use super::infer::{InferCtxt, TypeError, VarKind};
use super::solve::{Fulfill, Goal, ParamEnv, SolveError, Solver, TraitDb};
use super::ty::*;
use std::collections::HashMap;
use std::sync::Arc;

/// Callable signature over its full generic list (parent params first).
#[derive(Clone, Debug)]
pub struct FnSig {
    pub generics: u32,
    /// Number of leading generics owned by the parent impl or trait.
    pub parent_generics: u32,
    pub inputs: Vec<Ty>,
    pub output: Ty,
    pub predicates: Vec<Predicate>,
    /// Owning impl or trait, if this is an associated function.
    pub owner: Option<Owner>,
    /// First input is a `self` receiver.
    pub has_self: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Impl(DefId),
    Trait(DefId),
}

#[derive(Clone, Debug)]
pub struct VariantData {
    pub fields: Vec<(String, Ty)>,
}

#[derive(Clone, Debug)]
pub struct AdtData {
    pub generics: u32,
    pub variants: Vec<VariantData>,
}

/// Language items the checker needs beyond the solver's.
#[derive(Clone, Debug, Default)]
pub struct CheckLang {
    /// Operator traits: (op, trait, Output item).
    pub binops: Vec<(BinOp, DefId, DefId)>,
    pub index: Option<(DefId, DefId)>,
    pub index_mut: Option<DefId>,
    pub range: Option<DefId>,
    pub range_inclusive: Option<DefId>,
    pub string: Option<DefId>,
    /// `Option` and `Result` for the `?` operator.
    pub option: Option<DefId>,
    pub result: Option<DefId>,
}

pub trait ItemDb: TraitDb {
    fn fn_sig(&self, f: DefId) -> &FnSig;
    fn adt(&self, a: DefId) -> &AdtData;
    /// Associated functions of one impl: (name, fn).
    fn impl_items(&self, i: DefId) -> &[(String, DefId)];
    /// Associated constants of one inherent impl: (name, type over the
    /// impl's parameters).
    fn impl_consts(&self, _i: DefId) -> &[(String, Ty)] {
        &[]
    }
    fn check_lang(&self) -> &CheckLang;
    fn as_trait_db(&self) -> &dyn TraitDb;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckErrorKind {
    Mismatch { expected: Ty, found: Ty },
    Unsatisfied(String),
    Ambiguous(String),
    NoMethod(String),
    NoField(String),
    NotCallable(Ty),
    AnnotationsNeeded,
    Limit,
    Unsupported(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckError {
    pub expr: Option<ExprId>,
    pub kind: CheckErrorKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodRes {
    pub def: DefId,
    pub args: Vec<Ty>,
    /// Autoderef count and autoref applied to the receiver.
    pub derefs: u32,
    pub autoref: Option<Mutbl>,
}

/// Output of one body check. Fully resolved: no inference variables.
#[derive(Clone, Debug, Default)]
pub struct Typeck {
    pub expr_tys: Vec<Ty>,
    pub local_tys: Vec<Ty>,
    pub methods: HashMap<ExprId, MethodRes>,
    /// Primitive intrinsic calls with the receiver autoderef count.
    pub intrinsics: HashMap<ExprId, (crate::compiler::ir::Intrinsic, u32)>,
    /// Builtin slice/array `.len()` calls with the receiver autoderef count.
    pub slice_lens: HashMap<ExprId, u32>,
    /// Native array/slice `.iter()` calls, typed as `&[T]`.
    pub native_iters: std::collections::HashSet<ExprId>,
    pub errors: Vec<CheckError>,
}

struct Checker<'a> {
    db: &'a dyn ItemDb,
    body: &'a HirBody,
    cx: InferCtxt,
    env: &'a ParamEnv,
    traits_in_scope: &'a [DefId],
    fulfill: Fulfill,
    expr_tys: Vec<Option<Ty>>,
    locals: Vec<Ty>,
    ret: Ty,
    loops: Vec<(u32, Ty)>,
    methods: HashMap<ExprId, MethodRes>,
    intrinsics: HashMap<ExprId, (crate::compiler::ir::Intrinsic, u32)>,
    slice_lens: HashMap<ExprId, u32>,
    native_iters: std::collections::HashSet<ExprId>,
    errors: Vec<CheckError>,
    closure_ret: Vec<Ty>,
}

pub fn check_body(
    db: &dyn ItemDb,
    env: &ParamEnv,
    traits_in_scope: &[DefId],
    body: &HirBody,
    limit: u64,
) -> Typeck {
    let mut cx = InferCtxt::new(limit);
    let locals = (0..body.locals).map(|_| cx.new_var()).collect();
    let ret = cx.fill_holes(&body.ret);
    let mut c = Checker {
        db,
        body,
        cx,
        env,
        traits_in_scope,
        fulfill: Fulfill::default(),
        expr_tys: vec![None; body.exprs.len()],
        locals,
        ret: ret.clone(),
        loops: Vec::new(),
        methods: HashMap::new(),
        intrinsics: HashMap::new(),
        slice_lens: HashMap::new(),
        native_iters: Default::default(),
        errors: Vec::new(),
        closure_ret: Vec::new(),
    };
    for (pat, ty) in &body.params {
        let ty = c.cx.fill_holes(ty);
        let ty = c.normalize(&ty);
        c.pat(pat, &ty, None);
    }
    let ret_n = c.normalize(&ret);
    c.ret = ret_n.clone();
    let t = c.expr(body.root, Some(&ret_n));
    c.coerce(body.root, &t, &ret_n);
    c.finish()
}

impl<'a> Checker<'a> {
    fn solver(&self) -> Solver<'a> {
        Solver::new(self.db.as_trait_db(), self.env)
    }

    fn err(&mut self, expr: Option<ExprId>, kind: CheckErrorKind) {
        if self.errors.len() < 64 {
            self.errors.push(CheckError { expr, kind });
        }
    }

    fn type_err(&mut self, expr: ExprId, e: TypeError, expected: &Ty, found: &Ty) {
        let kind = match e {
            TypeError::Limit => CheckErrorKind::Limit,
            _ => CheckErrorKind::Mismatch {
                expected: self.cx.resolve(expected),
                found: self.cx.resolve(found),
            },
        };
        self.err(Some(expr), kind);
    }

    fn normalize(&mut self, ty: &Ty) -> Ty {
        let mut goals = Vec::new();
        let solver = self.solver();
        let t = solver.normalize(&mut self.cx, ty, &mut goals);
        for g in goals {
            self.fulfill.register(g, 0);
        }
        t
    }

    fn register(&mut self, g: Goal, cause: u32) {
        self.fulfill.register(g, cause);
    }

    fn select(&mut self) {
        let solver = self.solver();
        let errs = self.fulfill.select_where_possible(&solver, &mut self.cx);
        for (ob, e) in errs {
            self.solve_err(ob.cause, e);
        }
    }

    fn solve_err(&mut self, cause: u32, e: SolveError) {
        let kind = match e {
            SolveError::Limit | SolveError::Type(TypeError::Limit) => CheckErrorKind::Limit,
            SolveError::Unsatisfied(g) => CheckErrorKind::Unsatisfied(self.goal_text(&g)),
            SolveError::Ambiguous(g) => CheckErrorKind::Ambiguous(self.goal_text(&g)),
            SolveError::Type(TypeError::Mismatch(a, b) | TypeError::Cyclic(a, b)) => {
                CheckErrorKind::Mismatch {
                    expected: self.cx.resolve(&a),
                    found: self.cx.resolve(&b),
                }
            }
        };
        self.err(Some(cause), kind);
    }

    fn goal_text(&self, g: &Goal) -> String {
        match g {
            Goal::Trait(t) => format!("{}: trait#{}", self.cx.resolve(&t.self_ty), t.trait_id),
            Goal::Projection(p, t) => format!(
                "<{} as trait#{}>::item#{} == {}",
                self.cx.resolve(&p.trait_ref.self_ty),
                p.trait_ref.trait_id,
                p.item,
                self.cx.resolve(t)
            ),
        }
    }

    fn record(&mut self, e: ExprId, t: Ty) -> Ty {
        self.expr_tys[e as usize] = Some(t.clone());
        t
    }

    fn unify_at(&mut self, e: ExprId, expected: &Ty, found: &Ty) {
        if let Err(err) = self.cx.try_unify(expected, found) {
            self.type_err(e, err, expected, found);
        }
    }

    /// Resolve the outermost structure, running the solver once if the type
    /// is still an unresolved variable (needed before field/method access).
    fn structurally_resolve(&mut self, ty: &Ty) -> Ty {
        let t = self.cx.shallow(ty);
        if matches!(t.as_ref(), TyKind::Var(_)) {
            self.select();
            return self.cx.shallow(ty);
        }
        t
    }

    // ---------------------------------------------------------------- coercion

    fn coerce(&mut self, e: ExprId, found: &Ty, expected: &Ty) {
        if let Err(err) = self.try_coerce(found, expected) {
            self.type_err(e, err, expected, found);
        }
    }

    fn try_coerce(&mut self, found: &Ty, expected: &Ty) -> Result<(), TypeError> {
        let f = self.cx.shallow(found);
        let x = self.cx.shallow(expected);
        if matches!(f.as_ref(), TyKind::Never) {
            return Ok(());
        }
        match (f.as_ref(), x.as_ref()) {
            (TyKind::Ref(fm, fi), TyKind::Ref(xm, xi)) => {
                if *fm == Mutbl::Not && *xm == Mutbl::Mut {
                    return Err(TypeError::Mismatch(x.clone(), f.clone()));
                }
                let (fi, xi) = (fi.clone(), xi.clone());
                if self.cx.try_unify(&fi, &xi).is_ok() {
                    return Ok(());
                }
                // Unsize [T; N] -> [T].
                if let (TyKind::Array(a, _), TyKind::Slice(b)) =
                    (self.cx.shallow(&fi).as_ref(), self.cx.shallow(&xi).as_ref())
                {
                    return self.cx.try_unify(a, b);
                }
                // Deref coercion.
                let mut cur = fi;
                for _ in 0..8 {
                    let Some(next) = self.deref_step(&cur) else {
                        break;
                    };
                    if self.cx.try_unify(&next, &xi).is_ok() {
                        return Ok(());
                    }
                    cur = next;
                }
                Err(TypeError::Mismatch(x.clone(), f.clone()))
            }
            (TyKind::Ref(fm, fi), TyKind::RawPtr(xm, xi))
                if !(*fm == Mutbl::Not && *xm == Mutbl::Mut) =>
            {
                self.cx.try_unify(fi, xi)
            }
            (TyKind::Closure(c), TyKind::FnPtr(..)) => {
                let fp = mk(TyKind::FnPtr(c.inputs.clone(), c.output.clone()));
                self.cx.try_unify(&fp, &x)
            }
            _ => self.cx.try_unify(&x, &f),
        }
    }

    /// One autoderef step: built-in refs/raw pointers, else `Deref::Target`.
    fn deref_step(&mut self, ty: &Ty) -> Option<Ty> {
        let t = self.cx.shallow(ty);
        match t.as_ref() {
            TyKind::Ref(_, inner) => Some(inner.clone()),
            TyKind::Adt(..) => {
                let l = self.db.lang().clone();
                let (deref, target) = (l.deref?, l.deref_target?);
                let tr = TraitRef {
                    trait_id: deref,
                    self_ty: t.clone(),
                    args: Vec::new(),
                };
                // Only if an impl actually applies.
                let solver = self.solver();
                let mut f = Fulfill::default();
                f.register(Goal::Trait(tr.clone()), 0);
                let v = self.cx.new_var();
                f.register(
                    Goal::Projection(
                        ProjectionTy {
                            trait_ref: tr,
                            item: target,
                        },
                        v.clone(),
                    ),
                    0,
                );
                let ok = self.cx.probe(|cx| {
                    let errs = f.select_where_possible(&solver, cx);
                    if errs.is_empty() && f.pending.is_empty() {
                        Ok(())
                    } else {
                        Err(())
                    }
                });
                ok.ok().map(|_| self.cx.resolve(&v))
            }
            _ => None,
        }
    }

    // ---------------------------------------------------------------- exprs

    fn expr(&mut self, e: ExprId, expected: Option<&Ty>) -> Ty {
        if self.cx.meter.charge(1).is_err() {
            self.err(Some(e), CheckErrorKind::Limit);
            return self.record(e, error());
        }
        let t = self.expr_inner(e, expected);
        self.record(e, t)
    }

    fn expr_coerce(&mut self, e: ExprId, expected: &Ty) -> Ty {
        let t = self.expr(e, Some(expected));
        self.coerce(e, &t, expected);
        t
    }

    fn lit(&mut self, l: &HirLit, expected: Option<&Ty>) -> Ty {
        match l {
            HirLit::Int(_, Some(i)) => int(*i),
            HirLit::Int(_, None) => {
                if let Some(x) = expected {
                    let x = self.cx.shallow(x);
                    if matches!(x.as_ref(), TyKind::Int(_)) {
                        return x;
                    }
                }
                self.cx.new_int_var()
            }
            HirLit::Float(Some(f)) => mk(TyKind::Float(*f)),
            HirLit::Float(None) => self.cx.new_float_var(),
            HirLit::Bool => bool_ty(),
            HirLit::Char => mk(TyKind::Char),
            HirLit::Str => reference(Mutbl::Not, mk(TyKind::Str)),
        }
    }

    fn expr_inner(&mut self, e: ExprId, expected: Option<&Ty>) -> Ty {
        let body = self.body;
        match &body.exprs[e as usize] {
            HirExpr::Lit(l) => self.lit(l, expected),
            HirExpr::Const(t) => {
                let t = self.cx.fill_holes(t);
                self.normalize(&t)
            }
            HirExpr::Try(x) => self.try_expr(e, *x),
            HirExpr::TraitCall {
                trait_id,
                self_ty,
                name,
                args,
            } => {
                let self_ty = self.cx.fill_holes(self_ty);
                let found = self
                    .db
                    .trait_data(*trait_id)
                    .methods
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, f)| *f);
                let Some(f) = found else {
                    self.err(Some(e), CheckErrorKind::NoMethod(name.clone()));
                    for a in args {
                        self.expr(*a, None);
                    }
                    return error();
                };
                let (ft, _) = self.fn_item(e, f, Some(&self_ty), &[]);
                let TyKind::FnPtr(inputs, output) = ft.as_ref() else {
                    return error();
                };
                if inputs.len() != args.len() {
                    self.err(
                        Some(e),
                        CheckErrorKind::Unsupported("wrong number of arguments"),
                    );
                    return output.clone();
                }
                let (inputs, output) = (inputs.clone(), output.clone());
                self.check_args(args, &inputs);
                output
            }
            HirExpr::Local(l) => self.locals[*l as usize].clone(),
            HirExpr::Fn { def, self_ty, args } => self.fn_item(e, *def, self_ty.as_ref(), args).0,
            HirExpr::AssocFn { self_ty, name } => {
                let self_ty = self.cx.fill_holes(self_ty);
                match self.resolve_assoc(e, &self_ty, name) {
                    Some(t) => t,
                    None => {
                        self.err(Some(e), CheckErrorKind::NoMethod(name.clone()));
                        error()
                    }
                }
            }
            HirExpr::AssocConst { self_ty, name } => {
                let self_ty = self.cx.fill_holes(self_ty);
                let self_ty = self.structurally_resolve(&self_ty);
                match self.resolve_assoc_const(&self_ty, name) {
                    Some(t) => t,
                    None => {
                        self.err(
                            Some(e),
                            CheckErrorKind::Unsupported("associated constant lookup"),
                        );
                        error()
                    }
                }
            }
            HirExpr::Ctor { adt: a, variant } => self.ctor(*a, *variant),
            HirExpr::Call { callee, args } => self.call(e, *callee, args, expected),
            HirExpr::MethodCall {
                receiver,
                name,
                turbofish,
                args,
            } => self.method_call(e, *receiver, name, turbofish, args),
            HirExpr::Field { base, name } => self.field(e, *base, name),
            HirExpr::TupleField { base, index } => {
                let bt = self.expr(*base, None);
                let mut cur = self.structurally_resolve(&bt);
                for _ in 0..8 {
                    if let TyKind::Tuple(v) = cur.as_ref()
                        && let Some(t) = v.get(*index as usize)
                    {
                        return t.clone();
                    }
                    if let TyKind::Adt(a, args) = cur.as_ref() {
                        let data = self.db.adt(*a);
                        if data.variants.len() == 1
                            && let Some((_, t)) = data.variants[0].fields.get(*index as usize)
                        {
                            let t = subst(t, args);
                            return self.normalize(&t);
                        }
                    }
                    match self.deref_step(&cur) {
                        Some(n) => cur = self.structurally_resolve(&n),
                        None => break,
                    }
                }
                self.err(Some(e), CheckErrorKind::NoField(index.to_string()));
                error()
            }
            HirExpr::Index { base, index } => self.index(e, *base, *index),
            HirExpr::Struct {
                adt: a,
                variant,
                fields,
                base,
            } => {
                let data = self.db.adt(*a).clone();
                let args = self.cx.fresh_args(data.generics as usize);
                let t = adt(*a, args.clone());
                if let Some(x) = expected {
                    let _ = self.cx.try_unify(x, &t);
                }
                for (fi, fe) in fields {
                    let Some((_, ft)) = data.variants[*variant as usize].fields.get(*fi as usize)
                    else {
                        self.err(Some(*fe), CheckErrorKind::NoField(fi.to_string()));
                        continue;
                    };
                    let ft = subst(ft, &args);
                    let ft = self.normalize(&ft);
                    self.expr_coerce(*fe, &ft);
                }
                if let Some(b) = base {
                    self.expr_coerce(*b, &t);
                }
                t
            }
            HirExpr::Tuple(v) => {
                let exp: Vec<Option<Ty>> = match expected.map(|x| self.cx.shallow(x)) {
                    Some(x) => match x.as_ref() {
                        TyKind::Tuple(xs) if xs.len() == v.len() => {
                            xs.iter().cloned().map(Some).collect()
                        }
                        _ => vec![None; v.len()],
                    },
                    None => vec![None; v.len()],
                };
                let tys = v
                    .iter()
                    .zip(exp)
                    .map(|(x, ex)| match ex {
                        Some(ex) => {
                            self.expr_coerce(*x, &ex);
                            ex
                        }
                        None => self.expr(*x, None),
                    })
                    .collect();
                mk(TyKind::Tuple(tys))
            }
            HirExpr::Array(v) => {
                let elem = match expected.map(|x| self.cx.shallow(x)) {
                    Some(x) => match x.as_ref() {
                        TyKind::Array(t, _) | TyKind::Slice(t) => t.clone(),
                        _ => self.cx.new_var(),
                    },
                    None => self.cx.new_var(),
                };
                for x in v {
                    self.expr_coerce(*x, &elem);
                }
                mk(TyKind::Array(elem, v.len() as u64))
            }
            HirExpr::Repeat { value, count } => {
                let elem = match expected.map(|x| self.cx.shallow(x)) {
                    Some(x) => match x.as_ref() {
                        TyKind::Array(t, _) => t.clone(),
                        _ => self.cx.new_var(),
                    },
                    None => self.cx.new_var(),
                };
                self.expr_coerce(*value, &elem);
                mk(TyKind::Array(elem, *count))
            }
            HirExpr::Unary(op, x) => self.unary(e, *op, *x, expected),
            HirExpr::Binary(op, l, r) => self.binary(e, *op, *l, *r, expected),
            HirExpr::AssignOp(op, l, r) => {
                let lt = self.expr(*l, None);
                let lt_r = self.structurally_resolve(&lt);
                if is_scalar(&self.cx, &lt_r) {
                    let rhs_exp = if matches!(op, BinOp::Shl | BinOp::Shr) {
                        None
                    } else {
                        Some(lt.clone())
                    };
                    let rt = self.expr(*r, rhs_exp.as_ref());
                    if !matches!(op, BinOp::Shl | BinOp::Shr) {
                        self.coerce(*r, &rt, &lt);
                    }
                } else {
                    // `a op= b` via the binary operator trait on the place type.
                    let rt = self.expr(*r, None);
                    let _ = self.op_trait(e, *op, &lt, &rt);
                }
                unit()
            }
            HirExpr::Assign(l, r) => {
                let lt = self.expr(*l, None);
                self.expr_coerce(*r, &lt);
                unit()
            }
            HirExpr::Ref(m, x) => {
                let inner_exp = expected.and_then(|x| match self.cx.shallow(x).as_ref() {
                    TyKind::Ref(_, i) => Some(i.clone()),
                    _ => None,
                });
                let t = self.expr(*x, inner_exp.as_ref());
                reference(*m, t)
            }
            HirExpr::Cast(x, t) => {
                let t = self.cx.fill_holes(t);
                let src = self.expr(*x, None);
                let s = self.structurally_resolve(&src);
                // Integer literals take the cast target when it is an int.
                if let (TyKind::Var(v), TyKind::Int(_)) = (s.as_ref(), t.as_ref())
                    && self.cx.var_kind(*v) == VarKind::Integer
                {
                    // Keep the literal's own default (rustc: i32), not the target.
                }
                t
            }
            HirExpr::Block { stmts, tail } => {
                let mut diverges = false;
                for s in stmts {
                    match s {
                        HirStmt::Let {
                            pat,
                            ty,
                            init,
                            else_block,
                        } => {
                            let ty = self.cx.fill_holes(ty);
                            let ty = self.normalize(&ty);
                            if let Some(i) = init {
                                let it = self.expr(*i, Some(&ty));
                                self.coerce(*i, &it, &ty);
                            }
                            if let Some(b) = else_block {
                                self.expr(*b, None);
                            }
                            self.pat(pat, &ty, None);
                        }
                        HirStmt::Expr(x) => {
                            let t = self.expr(*x, None);
                            if matches!(self.cx.shallow(&t).as_ref(), TyKind::Never) {
                                diverges = true;
                            }
                        }
                    }
                }
                match tail {
                    Some(t) => self.expr(*t, expected),
                    None if diverges => mk(TyKind::Never),
                    None => unit(),
                }
            }
            HirExpr::If { cond, then, els } => {
                self.expr_coerce(*cond, &bool_ty());
                match els {
                    Some(el) => {
                        let tt = self.expr(*then, expected);
                        let et = self.expr(*el, expected.or(Some(&tt)));
                        self.lub(e, &tt, &et)
                    }
                    None => {
                        let tt = self.expr(*then, Some(&unit()));
                        self.coerce(*then, &tt, &unit());
                        unit()
                    }
                }
            }
            HirExpr::Match { scrutinee, arms } => {
                let st = self.expr(*scrutinee, None);
                let mut result: Option<Ty> = expected.cloned().filter(|_| false);
                for arm in arms {
                    self.pat(&arm.pat, &st, Some(*scrutinee));
                    if let Some(g) = arm.guard {
                        self.expr_coerce(g, &bool_ty());
                    }
                    let at = self.expr(arm.body, result.as_ref().or(expected));
                    result = Some(match result {
                        None => at,
                        Some(r) => self.lub(arm.body, &r, &at),
                    });
                }
                result.unwrap_or_else(|| mk(TyKind::Never))
            }
            HirExpr::Loop { body: b, label } => {
                let t = self.cx.new_var();
                self.loops.push((*label, t.clone()));
                self.expr(*b, None);
                self.loops.pop();
                // A loop without a break value of its own diverges.
                if self.cx.has_unresolved(&t) && !self.loop_has_break(*b, *label) {
                    mk(TyKind::Never)
                } else {
                    t
                }
            }
            HirExpr::While {
                cond,
                body: b,
                label,
            } => {
                self.loops.push((*label, unit()));
                self.expr_coerce(*cond, &bool_ty());
                self.expr(*b, None);
                self.loops.pop();
                unit()
            }
            HirExpr::For {
                pat,
                iter,
                body: b,
                label,
            } => {
                let it = self.expr(*iter, None);
                let item = self.infer_into_iter_item(*iter, &it);
                self.pat(pat, &item, None);
                self.loops.push((*label, unit()));
                self.expr(*b, None);
                self.loops.pop();
                unit()
            }
            HirExpr::Break { label, value } => {
                let target = self
                    .loops
                    .iter()
                    .rev()
                    .find(|(l, _)| l == label)
                    .map(|(_, t)| t.clone());
                match (value, target) {
                    (Some(v), Some(t)) => {
                        self.expr_coerce(*v, &t);
                    }
                    (None, Some(t)) => {
                        let _ = self.cx.try_unify(&t, &unit());
                    }
                    (Some(v), None) => {
                        self.expr(*v, None);
                    }
                    _ => {}
                }
                mk(TyKind::Never)
            }
            HirExpr::Continue => mk(TyKind::Never),
            HirExpr::Return(v) => {
                let r = self
                    .closure_ret
                    .last()
                    .cloned()
                    .unwrap_or_else(|| self.ret.clone());
                match v {
                    Some(v) => {
                        self.expr_coerce(*v, &r);
                    }
                    None => self.unify_at(e, &r, &unit()),
                }
                mk(TyKind::Never)
            }
            HirExpr::Closure {
                params,
                ret,
                body: b,
                id,
            } => self.closure(e, params, ret, *b, *id, expected),
            HirExpr::Range {
                start,
                end,
                inclusive,
            } => {
                let l = self.db.check_lang();
                let def = if *inclusive {
                    l.range_inclusive
                } else {
                    l.range
                };
                let elem = self.cx.new_var();
                for x in start.iter().chain(end.iter()) {
                    self.expr_coerce(*x, &elem);
                }
                match def {
                    Some(d) => adt(d, vec![elem]),
                    None => {
                        self.err(
                            Some(e),
                            CheckErrorKind::Unsupported("range without lang item"),
                        );
                        error()
                    }
                }
            }
        }
    }

    fn loop_has_break(&self, _b: ExprId, _label: u32) -> bool {
        // Conservative: scan the body expressions for a break to this label.
        self.body
            .exprs
            .iter()
            .any(|x| matches!(x, HirExpr::Break { label, .. } if *label == _label))
    }

    /// Least upper bound for branches: coerce each into the other.
    fn lub(&mut self, e: ExprId, a: &Ty, b: &Ty) -> Ty {
        let (sa, sb) = (self.cx.shallow(a), self.cx.shallow(b));
        if matches!(sa.as_ref(), TyKind::Never) {
            return b.clone();
        }
        if matches!(sb.as_ref(), TyKind::Never) {
            return a.clone();
        }
        if self.try_coerce(b, a).is_ok() {
            return a.clone();
        }
        if self.try_coerce(a, b).is_ok() {
            return b.clone();
        }
        self.err(
            Some(e),
            CheckErrorKind::Mismatch {
                expected: self.cx.resolve(a),
                found: self.cx.resolve(b),
            },
        );
        error()
    }

    fn ctor(&mut self, a: DefId, variant: u32) -> Ty {
        let data = self.db.adt(a).clone();
        let args = self.cx.fresh_args(data.generics as usize);
        let t = adt(a, args.clone());
        let fields = &data.variants[variant as usize].fields;
        if fields.is_empty() {
            return t;
        }
        let inputs = fields.iter().map(|(_, f)| subst(f, &args)).collect();
        mk(TyKind::FnPtr(inputs, t))
    }

    /// Instantiate a function item. Returns (fn type, generic args).
    fn fn_item(
        &mut self,
        e: ExprId,
        def: DefId,
        self_ty: Option<&Ty>,
        written: &[Ty],
    ) -> (Ty, Vec<Ty>) {
        let sig = self.db.fn_sig(def).clone();
        let mut args = self.cx.fresh_args(sig.generics as usize);
        // Written method generics fill the trailing own parameters.
        let own_start = sig.parent_generics as usize;
        for (i, w) in written.iter().enumerate() {
            let w = self.cx.fill_holes(w);
            if let Some(a) = args.get(own_start + i).cloned() {
                self.unify_at(e, &a, &w);
            }
        }
        if let Some(st) = self_ty {
            let st = self.cx.fill_holes(st);
            match sig.owner {
                Some(Owner::Impl(i)) => {
                    let it = subst(&self.db.impl_data(i).self_ty, &args);
                    self.unify_at(e, &st, &it);
                }
                Some(Owner::Trait(_)) => {
                    if let Some(a) = args.first().cloned() {
                        self.unify_at(e, &a, &st);
                    }
                }
                None => {}
            }
        }
        if let Some(Owner::Impl(i)) = sig.owner {
            let preds = self.db.impl_data(i).predicates.clone();
            for p in preds {
                self.register(super::solve::pred_goal(&subst_pred(&p, &args)), e);
            }
        }
        if let Some(Owner::Trait(t)) = sig.owner {
            let ngen = self.db.trait_data(t).generics as usize;
            let tr = TraitRef {
                trait_id: t,
                self_ty: args[0].clone(),
                args: args[1..=ngen].to_vec(),
            };
            self.register(Goal::Trait(tr), e);
        }
        for p in &sig.predicates {
            self.register(super::solve::pred_goal(&subst_pred(p, &args)), e);
        }
        let inputs: Vec<Ty> = sig.inputs.iter().map(|t| subst(t, &args)).collect();
        let inputs = inputs.iter().map(|t| self.normalize(t)).collect();
        let output = self.normalize(&subst(&sig.output, &args));
        args = args.into_iter().collect();
        (mk(TyKind::FnPtr(inputs, output)), args)
    }

    /// Inherent associated constant `Self::NAME`: the first inherent impl of
    /// the simplified self type whose head unifies and that declares `name`.
    /// Trait-provided constants are not modeled yet (explicit Unsupported).
    fn resolve_assoc_const(&mut self, self_ty: &Ty, name: &str) -> Option<Ty> {
        let key = simplify(self_ty)?;
        for &i in self.db.inherent_impls(key).to_vec().iter() {
            let Some((_, t)) = self
                .db
                .impl_consts(i)
                .iter()
                .find(|(n, _)| n == name)
                .cloned()
            else {
                continue;
            };
            let data = self.db.impl_data(i).clone();
            let args = self.cx.fresh_args(data.generics as usize);
            let head = subst(&data.self_ty, &args);
            if self.cx.unify(&head, self_ty).is_err() {
                continue;
            }
            return Some(self.normalize(&subst(&t, &args)));
        }
        None
    }

    fn resolve_assoc(&mut self, e: ExprId, self_ty: &Ty, name: &str) -> Option<Ty> {
        let st = self.cx.shallow(self_ty);
        if let Some(key) = simplify(&st) {
            for &i in self.db.inherent_impls(key).to_vec().iter() {
                if let Some((_, f)) = self
                    .db
                    .impl_items(i)
                    .iter()
                    .find(|(n, _)| n == name)
                    .cloned()
                {
                    let (t, _) = self.fn_item(e, f, Some(self_ty), &[]);
                    return Some(t);
                }
            }
        }
        for &t in self.traits_in_scope {
            if let Some((_, f)) = self
                .db
                .trait_data(t)
                .methods
                .iter()
                .find(|(n, _)| n == name)
                .cloned()
            {
                let (ty, _) = self.fn_item(e, f, Some(self_ty), &[]);
                return Some(ty);
            }
        }
        None
    }

    fn call(&mut self, e: ExprId, callee: ExprId, args: &[ExprId], _expected: Option<&Ty>) -> Ty {
        let ct = self.expr(callee, None);
        let ct = self.structurally_resolve(&ct);
        let (inputs, output) = match ct.as_ref() {
            TyKind::FnPtr(i, o) => (i.clone(), o.clone()),
            TyKind::Closure(c) => (c.inputs.clone(), c.output.clone()),
            TyKind::Param(_) | TyKind::Var(_) | TyKind::Adt(..) | TyKind::Ref(..) => {
                // Callable through Fn* bounds.
                let l = self.db.lang().clone();
                let (Some(fo), Some(out_item)) = (l.fn_once, l.fn_once_output) else {
                    self.err(Some(e), CheckErrorKind::NotCallable(ct.clone()));
                    return error();
                };
                let mut callee_ty = ct.clone();
                if let TyKind::Ref(_, inner) = ct.as_ref() {
                    callee_ty = inner.clone();
                }
                let inputs: Vec<Ty> = args.iter().map(|_| self.cx.new_var()).collect();
                let tuple = mk(TyKind::Tuple(inputs.clone()));
                let out = self.cx.new_var();
                let tr = TraitRef {
                    trait_id: fo,
                    self_ty: callee_ty,
                    args: vec![tuple],
                };
                self.register(Goal::Trait(tr.clone()), e);
                self.register(
                    Goal::Projection(
                        ProjectionTy {
                            trait_ref: tr,
                            item: out_item,
                        },
                        out.clone(),
                    ),
                    e,
                );
                (inputs, out)
            }
            TyKind::Error => return error(),
            _ => {
                self.err(Some(e), CheckErrorKind::NotCallable(ct.clone()));
                return error();
            }
        };
        if inputs.len() != args.len() {
            self.err(
                Some(e),
                CheckErrorKind::Unsupported("wrong number of arguments"),
            );
            return output;
        }
        self.check_args(args, &inputs);
        output
    }

    /// Check arguments in two passes: everything but closures first, then
    /// closures, so closure parameter types come from solved bounds.
    fn check_args(&mut self, args: &[ExprId], inputs: &[Ty]) {
        for pass in 0..2 {
            for (a, t) in args.iter().zip(inputs) {
                let is_closure = matches!(self.body.exprs[*a as usize], HirExpr::Closure { .. });
                if (pass == 0) == is_closure {
                    continue;
                }
                self.expr_coerce(*a, t);
            }
            if pass == 0 {
                self.select();
            }
        }
    }

    fn closure(
        &mut self,
        _e: ExprId,
        params: &[(HirPat, Ty)],
        ret: &Ty,
        body: ExprId,
        id: u32,
        expected: Option<&Ty>,
    ) -> Ty {
        let mut exp_inputs: Option<Vec<Ty>> = None;
        let mut exp_out: Option<Ty> = None;
        // The closure kind comes from how the body uses its captures, which
        // is decided by capture analysis after type checking, not from the
        // expected bound. Until that analysis feeds this checker, `Fn` is
        // used: it satisfies every Fn* bound, and moves out of captures are
        // rejected later by ownership analysis.
        let kind = ClosureKind::Fn;
        if let Some(x) = expected {
            let xs = self.cx.shallow(x);
            match xs.as_ref() {
                TyKind::FnPtr(i, o) => {
                    exp_inputs = Some(i.clone());
                    exp_out = Some(o.clone());
                }
                TyKind::Var(_) => {
                    let solver = self.solver();
                    if let Some((i, o, _)) =
                        self.fulfill.expected_closure_sig(&solver, &self.cx, &xs)
                    {
                        exp_inputs = Some(i);
                        exp_out = o;
                    }
                }
                _ => {}
            }
        }
        let mut inputs = Vec::new();
        for (i, (p, t)) in params.iter().enumerate() {
            let t = self.cx.fill_holes(t);
            if let Some(ei) = exp_inputs.as_ref().and_then(|v| v.get(i)) {
                let _ = self.cx.try_unify(&t, ei);
            }
            self.pat(p, &t, None);
            inputs.push(t);
        }
        let out = self.cx.fill_holes(ret);
        if let Some(o) = &exp_out {
            let _ = self.cx.try_unify(&out, o);
        }
        self.closure_ret.push(out.clone());
        let saved_loops = std::mem::take(&mut self.loops);
        self.expr_coerce(body, &out);
        self.loops = saved_loops;
        self.closure_ret.pop();
        mk(TyKind::Closure(Arc::new(ClosureTy {
            id,
            inputs,
            output: out,
            kind,
        })))
    }

    fn method_call(
        &mut self,
        e: ExprId,
        recv: ExprId,
        name: &str,
        turbofish: &[Ty],
        args: &[ExprId],
    ) -> Ty {
        let rt = self.expr(recv, None);
        let rt = self.structurally_resolve(&rt);
        let probed = self.probe_method(e, &rt, name);
        if probed.is_none() && name == "len" && args.is_empty() && turbofish.is_empty() {
            // Builtin length of `[T]`, `[T; N]` and `str` (`ExprKind::SliceLen`
            // and the native str length); receivers autoderef through
            // references. Nominal `len` (Vec, String) are source methods.
            let mut cur = rt.clone();
            let mut derefs = 0u32;
            while let TyKind::Ref(_, inner) = cur.as_ref() {
                let inner = inner.clone();
                cur = self.structurally_resolve(&inner);
                derefs += 1;
            }
            if matches!(
                cur.as_ref(),
                TyKind::Slice(_) | TyKind::Array(..) | TyKind::Str
            ) {
                self.slice_lens.insert(e, derefs);
                return int(IntTy::Usize);
            }
        }
        if probed.is_none() && name == "iter" && args.is_empty() && turbofish.is_empty() {
            // Native array iteration (`ForAdapter::Iter`, FOR-LOANS.md):
            // `xs.iter()` iterates like `&xs`, yielding shared element loans.
            // Typed as the shared array reference so the native `for` rule
            // applies. Slices are not admitted: the native compiler rejects
            // `slice.iter()` today and there is no source `slice::Iter` yet.
            let mut cur = rt.clone();
            while let TyKind::Ref(_, inner) = cur.as_ref() {
                let inner = inner.clone();
                cur = self.structurally_resolve(&inner);
            }
            if let TyKind::Array(..) = cur.as_ref() {
                self.native_iters.insert(e);
                return reference(Mutbl::Not, cur.clone());
            }
        }
        if probed.is_none()
            && let Some(out) = self.intrinsic_call(e, &rt, name, args)
        {
            return out;
        }
        let Some(res) = probed else {
            if !matches!(rt.as_ref(), TyKind::Error) {
                let msg = format!("{name} on {}", self.cx.resolve(&rt));
                let kind = if matches!(rt.as_ref(), TyKind::Var(_)) {
                    CheckErrorKind::AnnotationsNeeded
                } else {
                    CheckErrorKind::NoMethod(msg)
                };
                self.err(Some(e), kind);
            }
            for a in args {
                self.expr(*a, None);
            }
            return error();
        };
        let (def, adjusted, derefs, autoref, step_ty) = res;
        let sig = self.db.fn_sig(def).clone();
        let gargs = self.cx.fresh_args(sig.generics as usize);
        let own = sig.parent_generics as usize;
        for (i, w) in turbofish.iter().enumerate() {
            let w = self.cx.fill_holes(w);
            if let Some(a) = gargs.get(own + i).cloned() {
                self.unify_at(e, &a, &w);
            }
        }
        match sig.owner {
            Some(Owner::Impl(i)) => {
                let d = self.db.impl_data(i).clone();
                let it = subst(&d.self_ty, &gargs);
                self.unify_at(e, &it, &step_ty);
                for p in &d.predicates {
                    self.register(super::solve::pred_goal(&subst_pred(p, &gargs)), e);
                }
            }
            Some(Owner::Trait(t)) => {
                let a0 = gargs[0].clone();
                self.unify_at(e, &a0, &step_ty);
                let ngen = self.db.trait_data(t).generics as usize;
                let tr = TraitRef {
                    trait_id: t,
                    self_ty: gargs[0].clone(),
                    args: gargs[1..=ngen].to_vec(),
                };
                self.register(Goal::Trait(tr), e);
            }
            None => {}
        }
        for p in &sig.predicates {
            self.register(super::solve::pred_goal(&subst_pred(p, &gargs)), e);
        }
        let inputs: Vec<Ty> = sig.inputs.iter().map(|t| subst(t, &gargs)).collect();
        let inputs: Vec<Ty> = inputs.iter().map(|t| self.normalize(t)).collect();
        let output = self.normalize(&subst(&sig.output, &gargs));
        if let Some(r) = inputs.first() {
            self.unify_at(recv, r, &adjusted);
        }
        if inputs.len() != args.len() + 1 {
            self.err(
                Some(e),
                CheckErrorKind::Unsupported("wrong number of method arguments"),
            );
            return output;
        }
        self.check_args(args, &inputs[1..]);
        self.methods.insert(
            e,
            MethodRes {
                def,
                args: gargs,
                derefs,
                autoref,
            },
        );
        output
    }

    /// Inherent primitive methods (`ir::intrinsics`), resolved after every
    /// source candidate failed. The receiver autoderefs through references to
    /// an exact scalar type. One shared table defines the signature, so the
    /// semantic core and the backend cannot disagree about it. An integer or
    /// float variable is not yet a fixed receiver: rustc reports E0689 there.
    fn intrinsic_call(&mut self, e: ExprId, rt: &Ty, name: &str, args: &[ExprId]) -> Option<Ty> {
        use crate::compiler::ir::{Intrinsic, ScalarTy};
        if !crate::compiler::ir::IntrinsicOp::candidate_name(name) {
            return None;
        }
        let mut cur = self.structurally_resolve(rt);
        let mut derefs = 0u32;
        while let TyKind::Ref(_, inner) = cur.as_ref() {
            let inner = inner.clone();
            cur = self.structurally_resolve(&inner);
            derefs += 1;
        }
        let scalar = match cur.as_ref() {
            TyKind::Int(i) => ScalarTy::Int(*i),
            TyKind::Float(f) => ScalarTy::Float(*f),
            TyKind::Char => ScalarTy::Char,
            _ => return None,
        };
        let intrinsic = Intrinsic::resolve(scalar, name)?;
        let sig = intrinsic.signature()?;
        if let Some(reason) = intrinsic.unsupported_reason() {
            self.err(Some(e), CheckErrorKind::Unsupported(reason));
        }
        let lower = |t: &crate::compiler::ir::Ty| -> Ty {
            match t {
                crate::compiler::ir::Ty::Int(i) => int(*i),
                crate::compiler::ir::Ty::Float(f) => mk(TyKind::Float(*f)),
                crate::compiler::ir::Ty::Char => mk(TyKind::Char),
                crate::compiler::ir::Ty::Bool => bool_ty(),
                _ => error(),
            }
        };
        if sig.params.len() != args.len() + 1 {
            self.err(
                Some(e),
                CheckErrorKind::Unsupported("wrong number of method arguments"),
            );
            return Some(lower(&sig.result));
        }
        let params: Vec<Ty> = sig.params[1..].iter().map(lower).collect();
        self.check_args(args, &params);
        self.intrinsics.insert(e, (intrinsic, derefs));
        Some(lower(&sig.result))
    }

    /// Method probe. Returns (method, adjusted receiver type, derefs, autoref,
    /// self type at the matching step).
    fn probe_method(
        &mut self,
        e: ExprId,
        recv: &Ty,
        name: &str,
    ) -> Option<(DefId, Ty, u32, Option<Mutbl>, Ty)> {
        let mut steps = vec![recv.clone()];
        let mut cur = recv.clone();
        for _ in 0..8 {
            match self.deref_step(&cur) {
                Some(n) => {
                    let n = self.structurally_resolve(&n);
                    steps.push(n.clone());
                    cur = n;
                }
                None => break,
            }
        }
        if let TyKind::Array(t, _) = self.cx.shallow(&cur).as_ref() {
            steps.push(mk(TyKind::Slice(t.clone())));
        }
        for (d, step) in steps.iter().enumerate() {
            let step = self.cx.shallow(step);
            if matches!(step.as_ref(), TyKind::Var(_)) {
                return None;
            }
            for autoref in [None, Some(Mutbl::Not), Some(Mutbl::Mut)] {
                let adjusted = match autoref {
                    None => step.clone(),
                    Some(m) => reference(m, step.clone()),
                };
                for def in self.method_candidates(&step, name) {
                    if self.receiver_matches(e, def, &step, &adjusted) {
                        return Some((def, adjusted, d as u32, autoref, step));
                    }
                }
            }
        }
        None
    }

    fn method_candidates(&mut self, step: &Ty, name: &str) -> Vec<DefId> {
        let mut out = Vec::new();
        if let Some(key) = simplify(step) {
            for &i in self.db.inherent_impls(key) {
                for (n, f) in self.db.impl_items(i) {
                    if n == name && self.db.fn_sig(*f).has_self {
                        out.push(*f);
                    }
                }
            }
        }
        let inherent = out.len();
        for &t in self.traits_in_scope {
            for (n, f) in &self.db.trait_data(t).methods {
                if n == name && self.db.fn_sig(*f).has_self {
                    out.push(*f);
                }
            }
        }
        // Inherent methods take priority; keep trait ones only as fallback.
        if inherent > 0 {
            let (a, b) = out.split_at(inherent);
            let mut v = a.to_vec();
            v.extend_from_slice(b);
            return v;
        }
        out
    }

    fn receiver_matches(&mut self, _e: ExprId, def: DefId, step: &Ty, adjusted: &Ty) -> bool {
        let sig = self.db.fn_sig(def).clone();
        let solver = self.solver();
        let db = self.db;
        self.cx.can(|cx| {
            let args = cx.fresh_args(sig.generics as usize);
            let mut f = Fulfill::default();
            match sig.owner {
                Some(Owner::Impl(i)) => {
                    let it = subst(&db.impl_data(i).self_ty, &args);
                    cx.unify(&it, step).map_err(|_| ())?;
                }
                Some(Owner::Trait(t)) => {
                    cx.unify(&args[0], step).map_err(|_| ())?;
                    let ngen = db.trait_data(t).generics as usize;
                    f.register(
                        Goal::Trait(TraitRef {
                            trait_id: t,
                            self_ty: args[0].clone(),
                            args: args[1..=ngen].to_vec(),
                        }),
                        0,
                    );
                }
                None => return Err(()),
            }
            let recv = subst(&sig.inputs[0], &args);
            let mut goals = Vec::new();
            let recv = solver.normalize(cx, &recv, &mut goals);
            cx.unify(&recv, adjusted).map_err(|_| ())?;
            for g in goals {
                f.register(g, 0);
            }
            let errs = f.select_where_possible(&solver, cx);
            if errs.is_empty() { Ok(()) } else { Err(()) }
        })
    }

    fn field(&mut self, e: ExprId, base: ExprId, name: &str) -> Ty {
        let bt = self.expr(base, None);
        let mut cur = self.structurally_resolve(&bt);
        for _ in 0..8 {
            if let TyKind::Adt(a, args) = cur.as_ref() {
                let data = self.db.adt(*a);
                if data.variants.len() == 1
                    && let Some((_, t)) = data.variants[0].fields.iter().find(|(n, _)| n == name)
                {
                    let t = subst(t, args);
                    return self.normalize(&t);
                }
            }
            match self.deref_step(&cur) {
                Some(n) => cur = self.structurally_resolve(&n),
                None => break,
            }
        }
        if matches!(cur.as_ref(), TyKind::Var(_)) {
            self.err(Some(e), CheckErrorKind::AnnotationsNeeded);
        } else if !matches!(cur.as_ref(), TyKind::Error) {
            self.err(Some(e), CheckErrorKind::NoField(name.to_string()));
        }
        error()
    }

    fn index(&mut self, e: ExprId, base: ExprId, index: ExprId) -> Ty {
        let bt = self.expr(base, None);
        let it = self.expr(index, None);
        let mut cur = self.structurally_resolve(&bt);
        for _ in 0..8 {
            match cur.as_ref() {
                TyKind::Array(t, _) | TyKind::Slice(t) => {
                    let ri = self.structurally_resolve(&it);
                    if let TyKind::Adt(..) = ri.as_ref() {
                        // Range indexing yields a slice.
                        return mk(TyKind::Slice(t.clone()));
                    }
                    self.coerce(index, &it, &int(IntTy::Usize));
                    return t.clone();
                }
                TyKind::Adt(..) => {
                    if let Some((itr, out)) = self.db.check_lang().index {
                        let tr = TraitRef {
                            trait_id: itr,
                            self_ty: cur.clone(),
                            args: vec![it.clone()],
                        };
                        // Only use Index if some impl can apply.
                        let o = self.cx.new_var();
                        self.register(Goal::Trait(tr.clone()), e);
                        self.register(
                            Goal::Projection(
                                ProjectionTy {
                                    trait_ref: tr,
                                    item: out,
                                },
                                o.clone(),
                            ),
                            e,
                        );
                        self.select();
                        return o;
                    }
                    break;
                }
                _ => {}
            }
            match self.deref_step(&cur) {
                Some(n) => cur = self.structurally_resolve(&n),
                None => break,
            }
        }
        self.err(Some(e), CheckErrorKind::Unsupported("index on this type"));
        error()
    }

    fn unary(&mut self, e: ExprId, op: UnOp, x: ExprId, expected: Option<&Ty>) -> Ty {
        match op {
            UnOp::Deref => {
                let t = self.expr(x, None);
                let t = self.structurally_resolve(&t);
                match t.as_ref() {
                    TyKind::Ref(_, i) | TyKind::RawPtr(_, i) => i.clone(),
                    _ => match self.deref_step(&t) {
                        Some(n) => n,
                        None => {
                            self.err(Some(e), CheckErrorKind::Unsupported("deref of non-pointer"));
                            error()
                        }
                    },
                }
            }
            UnOp::Neg | UnOp::Not => self.expr(x, expected),
        }
    }

    fn binary(&mut self, e: ExprId, op: BinOp, l: ExprId, r: ExprId, expected: Option<&Ty>) -> Ty {
        use BinOp::*;
        match op {
            And | Or => {
                self.expr_coerce(l, &bool_ty());
                self.expr_coerce(r, &bool_ty());
                bool_ty()
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                let lt = self.expr(l, None);
                let rt = self.expr(r, Some(&lt));
                let (ls, rs) = (self.cx.shallow(&lt), self.cx.shallow(&rt));
                if is_scalar(&self.cx, &ls) || is_scalar(&self.cx, &rs) {
                    self.coerce(r, &rt, &lt);
                } else {
                    self.unify_at(e, &lt, &rt);
                }
                bool_ty()
            }
            Shl | Shr => {
                let lt = self.expr(l, expected);
                self.expr(r, None);
                lt
            }
            _ => {
                let lt = self.expr(l, expected);
                let ls = self.structurally_resolve(&lt);
                let lp = peel_ref(&self.cx, &ls);
                if is_scalar(&self.cx, &lp) {
                    let rt = self.expr(r, Some(&lp));
                    let rs = self.structurally_resolve(&rt);
                    let rp = peel_ref(&self.cx, &rs);
                    self.unify_at(r, &lp, &rp);
                    return lp;
                }
                let rt = self.expr(r, None);
                self.op_trait(e, op, &lt, &rt)
            }
        }
    }

    fn op_trait(&mut self, e: ExprId, op: BinOp, lt: &Ty, rt: &Ty) -> Ty {
        let Some((_, t, out)) = self
            .db
            .check_lang()
            .binops
            .iter()
            .find(|(o, ..)| *o == op)
            .cloned()
        else {
            self.err(
                Some(e),
                CheckErrorKind::Unsupported("operator without lang item"),
            );
            return error();
        };
        let tr = TraitRef {
            trait_id: t,
            self_ty: lt.clone(),
            args: vec![rt.clone()],
        };
        let o = self.cx.new_var();
        self.register(Goal::Trait(tr.clone()), e);
        self.register(
            Goal::Projection(
                ProjectionTy {
                    trait_ref: tr,
                    item: out,
                },
                o.clone(),
            ),
            e,
        );
        o
    }

    /// `x?` on `Option<T>` or `Result<T, E>`: yields `T`; the enclosing
    /// function (or closure) must return the same family. `Result` errors
    /// convert through `From` in Rust; this checker requires equal error
    /// types and reports anything else as unsupported, never accepting it.
    fn try_expr(&mut self, e: ExprId, x: ExprId) -> Ty {
        let t = self.expr(x, None);
        let t = self.structurally_resolve(&t);
        let l = self.db.check_lang().clone();
        let ret = self
            .closure_ret
            .last()
            .cloned()
            .unwrap_or_else(|| self.ret.clone());
        let ret = self.structurally_resolve(&ret);
        match t.as_ref() {
            TyKind::Adt(d, args) if Some(*d) == l.option => {
                let inner = args[0].clone();
                let want = adt(*d, vec![self.cx.new_var()]);
                if self.cx.try_unify(&ret, &want).is_err() {
                    self.err(
                        Some(e),
                        CheckErrorKind::Unsupported(
                            "? on Option in a function not returning Option",
                        ),
                    );
                }
                inner
            }
            TyKind::Adt(d, args) if Some(*d) == l.result => {
                let (ok, er) = (args[0].clone(), args[1].clone());
                let want = adt(*d, vec![self.cx.new_var(), er]);
                if self.cx.try_unify(&ret, &want).is_err() {
                    self.err(
                        Some(e),
                        CheckErrorKind::Unsupported("? with error conversion"),
                    );
                }
                ok
            }
            TyKind::Error => error(),
            _ => {
                self.err(
                    Some(e),
                    CheckErrorKind::Unsupported("? on a type that is not Option or Result"),
                );
                error()
            }
        }
    }

    fn infer_into_iter_item(&mut self, e: ExprId, it: &Ty) -> Ty {
        let l = self.db.lang().clone();
        let (Some(ii), Some(item)) = (l.into_iterator, l.into_iter_item) else {
            return self.builtin_iter_item(e, it);
        };
        // Integer ranges, arrays and slices are iterated natively by the
        // backend and have no source `IntoIterator` impl in the bootstrap
        // prelude. Their item type is fixed by the language rule.
        if self.is_native_iterable(it) {
            return self.builtin_iter_item(e, it);
        }
        let tr = TraitRef {
            trait_id: ii,
            self_ty: it.clone(),
            args: Vec::new(),
        };
        let v = self.cx.new_var();
        self.register(Goal::Trait(tr.clone()), e);
        self.register(
            Goal::Projection(
                ProjectionTy {
                    trait_ref: tr,
                    item,
                },
                v.clone(),
            ),
            e,
        );
        self.select();
        v
    }

    fn is_native_iterable(&mut self, it: &Ty) -> bool {
        let t = self.structurally_resolve(it);
        let cl = self.db.check_lang();
        match t.as_ref() {
            TyKind::Adt(d, _) => Some(*d) == cl.range || Some(*d) == cl.range_inclusive,
            TyKind::Array(..) | TyKind::Slice(_) => true,
            TyKind::Ref(_, inner) => {
                let inner = self.structurally_resolve(inner);
                matches!(inner.as_ref(), TyKind::Array(..) | TyKind::Slice(_))
            }
            _ => false,
        }
    }

    /// Without an `IntoIterator` lang item (older bootstrap preludes),
    /// `for` admits exactly the native iteration forms: integer ranges,
    /// arrays, slices and references to them.
    fn builtin_iter_item(&mut self, e: ExprId, it: &Ty) -> Ty {
        let t = self.structurally_resolve(it);
        let range = self.db.check_lang().range;
        let range_incl = self.db.check_lang().range_inclusive;
        match t.as_ref() {
            TyKind::Adt(d, args) if Some(*d) == range || Some(*d) == range_incl => args[0].clone(),
            TyKind::Array(el, _) | TyKind::Slice(el) => el.clone(),
            TyKind::Ref(m, inner) => {
                let inner = self.structurally_resolve(inner);
                match inner.as_ref() {
                    TyKind::Array(el, _) | TyKind::Slice(el) => reference(*m, el.clone()),
                    _ => {
                        self.err(
                            Some(e),
                            CheckErrorKind::Unsupported("for loop over this type"),
                        );
                        error()
                    }
                }
            }
            _ => {
                self.err(
                    Some(e),
                    CheckErrorKind::Unsupported("for loop over this type"),
                );
                error()
            }
        }
    }

    // ---------------------------------------------------------------- patterns

    fn pat(&mut self, p: &HirPat, ty: &Ty, cause: Option<ExprId>) {
        let cause_e = cause.unwrap_or(self.body.root);
        match p {
            HirPat::Wild => {}
            HirPat::Binding { local, by_ref } => {
                let t = match by_ref {
                    Some(m) => reference(*m, ty.clone()),
                    None => ty.clone(),
                };
                let l = self.locals[*local as usize].clone();
                self.unify_at(cause_e, &l, &t);
            }
            HirPat::Tuple(ps) => {
                let t = self.peel_for_pattern(ty);
                let elems: Vec<Ty> = match self.cx.shallow(&t).as_ref() {
                    TyKind::Tuple(v) if v.len() == ps.len() => v.clone(),
                    _ => {
                        let v: Vec<Ty> = ps.iter().map(|_| self.cx.new_var()).collect();
                        self.unify_at(cause_e, &t, &mk(TyKind::Tuple(v.clone())));
                        v
                    }
                };
                let wrap = self.binding_mode(ty);
                for (p, t) in ps.iter().zip(elems) {
                    let t = match wrap {
                        Some(m) => reference(m, t),
                        None => t,
                    };
                    self.pat(p, &t, cause);
                }
            }
            HirPat::Adt {
                adt: a,
                variant,
                fields,
            } => {
                let t = self.peel_for_pattern(ty);
                let data = self.db.adt(*a).clone();
                let args = self.cx.fresh_args(data.generics as usize);
                self.unify_at(cause_e, &t, &adt(*a, args.clone()));
                let wrap = self.binding_mode(ty);
                for (fi, fp) in fields {
                    let Some((_, ft)) = data.variants[*variant as usize].fields.get(*fi as usize)
                    else {
                        continue;
                    };
                    let ft = subst(ft, &args);
                    let ft = self.normalize(&ft);
                    let ft = match wrap {
                        Some(m) => reference(m, ft),
                        None => ft,
                    };
                    self.pat(fp, &ft, cause);
                }
            }
            HirPat::Ref(m, inner) => {
                let i = self.cx.new_var();
                self.unify_at(cause_e, ty, &reference(*m, i.clone()));
                self.pat(inner, &i, cause);
            }
            HirPat::Lit(l) | HirPat::Range(l) => {
                let lt = self.lit(l, Some(ty));
                let t = self.peel_for_pattern(ty);
                self.unify_at(cause_e, &t, &lt);
            }
            HirPat::Or(alts) => {
                for a in alts {
                    self.pat(a, ty, cause);
                }
            }
        }
    }

    /// Default binding modes: matching a non-reference pattern against
    /// `&T` peels the reference and binds by reference.
    fn peel_for_pattern(&mut self, ty: &Ty) -> Ty {
        let mut t = self.structurally_resolve(ty);
        while let TyKind::Ref(_, i) = t.as_ref() {
            t = self.structurally_resolve(i);
        }
        t
    }
    fn binding_mode(&mut self, ty: &Ty) -> Option<Mutbl> {
        let mut t = self.structurally_resolve(ty);
        let mut mode = None;
        while let TyKind::Ref(m, i) = t.as_ref() {
            mode = Some(match (mode, m) {
                (Some(Mutbl::Not), _) | (_, Mutbl::Not) => Mutbl::Not,
                _ => Mutbl::Mut,
            });
            t = self.structurally_resolve(i);
        }
        mode
    }

    // ---------------------------------------------------------------- finish

    fn finish(mut self) -> Typeck {
        self.select();
        if self.cx.default_numeric_vars() {
            self.select();
        }
        let solver = self.solver();
        let errs = self.fulfill.finish(&solver, &mut self.cx);
        for (ob, e) in errs {
            self.solve_err(ob.cause, e);
        }
        let mut expr_tys = Vec::with_capacity(self.expr_tys.len());
        let mut unresolved = None;
        for (i, t) in self.expr_tys.iter().enumerate() {
            let t = t.clone().unwrap_or_else(error);
            let r = self.cx.resolve(&t);
            if unresolved.is_none() && any(&r, &mut |x| matches!(x.as_ref(), TyKind::Var(_))) {
                unresolved = Some(i as u32);
            }
            expr_tys.push(r);
        }
        if let Some(i) = unresolved
            && self.errors.is_empty()
        {
            self.err(Some(i), CheckErrorKind::AnnotationsNeeded);
        }
        let local_tys = self.locals.iter().map(|t| self.cx.resolve(t)).collect();
        let methods = self
            .methods
            .iter()
            .map(|(k, m)| {
                (
                    *k,
                    MethodRes {
                        args: m.args.iter().map(|a| self.cx.resolve(a)).collect(),
                        ..m.clone()
                    },
                )
            })
            .collect();
        Typeck {
            expr_tys,
            local_tys,
            methods,
            intrinsics: self.intrinsics,
            slice_lens: self.slice_lens,
            native_iters: self.native_iters,
            errors: self.errors,
        }
    }
}

fn is_scalar(cx: &InferCtxt, t: &Ty) -> bool {
    match cx.shallow(t).as_ref() {
        TyKind::Int(_) | TyKind::Float(_) | TyKind::Bool | TyKind::Char => true,
        TyKind::Var(v) => cx.var_kind(*v) != VarKind::General,
        _ => false,
    }
}

fn peel_ref(cx: &InferCtxt, t: &Ty) -> Ty {
    let s = cx.shallow(t);
    match s.as_ref() {
        TyKind::Ref(_, i) if is_scalar(cx, i) => cx.shallow(i),
        _ => s,
    }
}
