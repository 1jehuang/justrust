//! Trait solving: candidate assembly, confirmation and lazy projection
//! normalization over a `TraitDb`.
//!
//! The database is the only source of program knowledge. Each query is keyed
//! by one declaration or one simplified type, so a graph adapter can serve it
//! from one per-item fact (see SEMANTIC-CORE.md). No query enumerates the
//! whole program.

use super::infer::{InferCtxt, TypeError};
use super::ty::*;

/// Recursion bound for nested obligations. Failure past it is `Limit`.
pub const MAX_DEPTH: u32 = 64;

#[derive(Clone, Debug)]
pub struct ImplData {
    pub id: DefId,
    /// Number of generic parameters on the impl.
    pub generics: u32,
    /// `None` for an inherent impl.
    pub trait_ref: Option<TraitRef>,
    pub self_ty: Ty,
    pub predicates: Vec<Predicate>,
    /// Associated type values: (trait item id, type over impl params).
    pub assoc_types: Vec<(DefId, Ty)>,
}

#[derive(Clone, Debug)]
pub struct TraitData {
    pub id: DefId,
    /// Generic parameters excluding `Self` (`Self` is `Param(0)` in the
    /// trait's own scope, the rest follow).
    pub generics: u32,
    /// Supertrait bounds over `Param(0) = Self` and the trait's params.
    pub supertraits: Vec<TraitRef>,
    pub assoc_types: Vec<DefId>,
    /// Method names provided by the trait (including defaults).
    pub methods: Vec<(String, DefId)>,
}

/// Language items the solver treats specially. Absent items disable the
/// corresponding builtin rule.
#[derive(Clone, Debug, Default)]
pub struct LangItems {
    pub sized: Option<DefId>,
    pub copy: Option<DefId>,
    pub clone: Option<DefId>,
    pub fn_: Option<DefId>,
    pub fn_mut: Option<DefId>,
    pub fn_once: Option<DefId>,
    /// `FnOnce::Output` associated type.
    pub fn_once_output: Option<DefId>,
    pub deref: Option<DefId>,
    pub deref_target: Option<DefId>,
    pub iterator: Option<DefId>,
    pub iterator_item: Option<DefId>,
    pub into_iterator: Option<DefId>,
    pub into_iter_item: Option<DefId>,
    pub into_iter_into_iter: Option<DefId>,
}

pub trait TraitDb {
    fn trait_data(&self, t: DefId) -> &TraitData;
    /// Every impl of trait `t`: an explicit completeness fact.
    fn impls_of(&self, t: DefId) -> &[DefId];
    fn impl_data(&self, i: DefId) -> &ImplData;
    /// Inherent impls whose self type simplifies to `key`.
    fn inherent_impls(&self, key: SimplifiedType) -> &[DefId];
    fn lang(&self) -> &LangItems;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Goal {
    Trait(TraitRef),
    /// `<projection> == ty`
    Projection(ProjectionTy, Ty),
}

#[derive(Clone, Debug)]
pub struct Obligation {
    pub goal: Goal,
    pub depth: u32,
    /// Opaque cause index chosen by the caller, for diagnostics.
    pub cause: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolveError {
    Unsatisfied(Goal),
    Ambiguous(Goal),
    Type(TypeError),
    Limit,
}
impl From<TypeError> for SolveError {
    fn from(e: TypeError) -> Self {
        match e {
            TypeError::Limit => SolveError::Limit,
            e => SolveError::Type(e),
        }
    }
}

/// Outcome of one selection attempt.
#[derive(Debug)]
enum Selected {
    /// Confirmed with nested obligations.
    Done(Vec<Goal>),
    /// Not yet decidable: more inference is needed.
    Ambiguous,
    NoMatch,
}

#[derive(Clone, Copy, Debug)]
enum Candidate {
    Param(usize),
    Impl(DefId),
    Builtin,
}

/// Caller bounds of the item being checked, over its own `Param`s.
#[derive(Clone, Debug, Default)]
pub struct ParamEnv {
    pub predicates: Vec<Predicate>,
}

impl ParamEnv {
    /// Add supertrait closure of every trait bound.
    pub fn elaborate(&mut self, db: &dyn TraitDb) {
        let mut i = 0;
        while i < self.predicates.len() {
            if let Predicate::Trait(tr) = self.predicates[i].clone() {
                let data = db.trait_data(tr.trait_id);
                let mut args = vec![tr.self_ty.clone()];
                args.extend(tr.args.iter().cloned());
                for sup in &data.supertraits {
                    let p = Predicate::Trait(subst_trait_ref(sup, &args));
                    if !self.predicates.contains(&p) && self.predicates.len() < 4096 {
                        self.predicates.push(p);
                    }
                }
            }
            i += 1;
        }
    }
}

pub struct Solver<'a> {
    pub db: &'a dyn TraitDb,
    pub env: &'a ParamEnv,
}

impl<'a> Solver<'a> {
    pub fn new(db: &'a dyn TraitDb, env: &'a ParamEnv) -> Self {
        Self { db, env }
    }

    /// Normalize every projection in `ty` that can be decided now. Undecided
    /// projections are replaced by a fresh variable and a pending projection
    /// goal is returned.
    pub fn normalize(&self, cx: &mut InferCtxt, ty: &Ty, out: &mut Vec<Goal>) -> Ty {
        let ty = cx.resolve(ty);
        if !any(&ty, &mut |t| matches!(t.as_ref(), TyKind::Projection(_))) {
            return ty;
        }
        fold(&ty, &mut |t| {
            let TyKind::Projection(p) = t.as_ref() else {
                return None;
            };
            // Normalize inner types first.
            let self_ty = self.normalize(cx, &p.trait_ref.self_ty, out);
            let args: Vec<Ty> = p
                .trait_ref
                .args
                .iter()
                .map(|a| self.normalize(cx, a, out))
                .collect();
            let proj = ProjectionTy {
                trait_ref: TraitRef {
                    trait_id: p.trait_ref.trait_id,
                    self_ty,
                    args,
                },
                item: p.item,
            };
            // A projection on a bare parameter with no env equality is rigid.
            if self.is_rigid(cx, &proj) {
                return Some(mk(TyKind::Projection(std::sync::Arc::new(proj))));
            }
            let v = cx.new_var();
            out.push(Goal::Projection(proj, v.clone()));
            Some(v)
        })
    }

    fn is_rigid(&self, cx: &InferCtxt, proj: &ProjectionTy) -> bool {
        let s = cx.shallow(&proj.trait_ref.self_ty);
        let rigid_self = matches!(s.as_ref(), TyKind::Param(_) | TyKind::Projection(_));
        if !rigid_self {
            return false;
        }
        // Rigid unless the environment supplies an equality for it.
        !self.env.predicates.iter().any(|p| match p {
            Predicate::Projection(q, _) => q.item == proj.item && q.trait_ref.self_ty == s,
            _ => false,
        })
    }

    /// Try to make progress on one goal.
    fn select(&self, cx: &mut InferCtxt, goal: &Goal, depth: u32) -> Result<Selected, SolveError> {
        cx.meter.charge(1)?;
        if depth > MAX_DEPTH {
            return Err(SolveError::Limit);
        }
        match goal {
            Goal::Trait(tr) => self.select_trait(cx, tr),
            Goal::Projection(proj, ty) => self.select_projection(cx, proj, ty),
        }
    }

    fn assemble(
        &self,
        cx: &mut InferCtxt,
        tr: &TraitRef,
    ) -> Result<(Vec<Candidate>, bool), SolveError> {
        let self_ty = cx.shallow(&tr.self_ty);
        let mut cands = Vec::new();
        // Self is an unresolved general variable: anything could match.
        if let TyKind::Var(v) = self_ty.as_ref()
            && cx.var_kind(*v) == super::infer::VarKind::General
        {
            return Ok((cands, true));
        }
        for (i, p) in self.env.predicates.iter().enumerate() {
            if let Predicate::Trait(b) = p
                && b.trait_id == tr.trait_id
                && cx.can(|cx| {
                    cx.unify(&b.self_ty, &tr.self_ty)?;
                    for (x, y) in b.args.iter().zip(&tr.args) {
                        cx.unify(x, y)?;
                    }
                    Ok::<(), TypeError>(())
                })
            {
                cands.push(Candidate::Param(i));
            }
        }
        if self.builtin_applies(cx, tr) && !cands.iter().any(|c| matches!(c, Candidate::Param(_))) {
            cands.push(Candidate::Builtin);
        }
        // Inference variables cannot match an impl head for a rigid param
        // self type when a param candidate exists (rustc prefers where-clauses).
        if !cands.iter().any(|c| matches!(c, Candidate::Param(_))) {
            for &i in self.db.impls_of(tr.trait_id) {
                cx.meter.charge(1)?;
                if self.impl_may_apply(cx, i, tr) {
                    cands.push(Candidate::Impl(i));
                }
            }
        }
        Ok((cands, false))
    }

    fn impl_may_apply(&self, cx: &mut InferCtxt, i: DefId, tr: &TraitRef) -> bool {
        let data = self.db.impl_data(i);
        let Some(itr) = &data.trait_ref else {
            return false;
        };
        cx.can(|cx| {
            let args = cx.fresh_args(data.generics as usize);
            let itr = subst_trait_ref(itr, &args);
            cx.unify(&itr.self_ty, &tr.self_ty)?;
            for (x, y) in itr.args.iter().zip(&tr.args) {
                cx.unify(x, y)?;
            }
            Ok::<(), TypeError>(())
        })
    }

    fn builtin_applies(&self, cx: &mut InferCtxt, tr: &TraitRef) -> bool {
        let l = self.db.lang();
        let t = cx.shallow(&tr.self_ty);
        let id = Some(tr.trait_id);
        if id == l.sized {
            return !matches!(t.as_ref(), TyKind::Str | TyKind::Slice(_));
        }
        if id == l.copy || id == l.clone {
            return match t.as_ref() {
                TyKind::Bool
                | TyKind::Char
                | TyKind::Int(_)
                | TyKind::Float(_)
                | TyKind::Never
                | TyKind::RawPtr(..)
                | TyKind::FnPtr(..)
                | TyKind::Ref(Mutbl::Not, _) => true,
                TyKind::Var(v) => cx.var_kind(*v) != super::infer::VarKind::General,
                // Composite builtins are confirmed by nested goals.
                TyKind::Tuple(_) | TyKind::Array(..) => true,
                _ => false,
            };
        }
        if id == l.fn_ || id == l.fn_mut || id == l.fn_once {
            return matches!(t.as_ref(), TyKind::Closure(_) | TyKind::FnPtr(..));
        }
        false
    }

    fn confirm(
        &self,
        cx: &mut InferCtxt,
        tr: &TraitRef,
        c: Candidate,
    ) -> Result<Vec<Goal>, SolveError> {
        match c {
            Candidate::Param(i) => {
                let Predicate::Trait(b) = &self.env.predicates[i] else {
                    unreachable!()
                };
                cx.unify(&b.self_ty, &tr.self_ty)?;
                for (x, y) in b.args.iter().zip(&tr.args) {
                    cx.unify(x, y)?;
                }
                Ok(Vec::new())
            }
            Candidate::Impl(i) => {
                let data = self.db.impl_data(i);
                let args = cx.fresh_args(data.generics as usize);
                let itr = subst_trait_ref(data.trait_ref.as_ref().unwrap(), &args);
                cx.unify(&itr.self_ty, &tr.self_ty)?;
                for (x, y) in itr.args.iter().zip(&tr.args) {
                    cx.unify(x, y)?;
                }
                Ok(data
                    .predicates
                    .iter()
                    .map(|p| pred_goal(&subst_pred(p, &args)))
                    .collect())
            }
            Candidate::Builtin => self.confirm_builtin(cx, tr),
        }
    }

    fn confirm_builtin(&self, cx: &mut InferCtxt, tr: &TraitRef) -> Result<Vec<Goal>, SolveError> {
        let l = self.db.lang();
        let t = cx.shallow(&tr.self_ty);
        let id = Some(tr.trait_id);
        let nested = |tys: Vec<Ty>| -> Vec<Goal> {
            tys.into_iter()
                .map(|t| {
                    Goal::Trait(TraitRef {
                        trait_id: tr.trait_id,
                        self_ty: t,
                        args: Vec::new(),
                    })
                })
                .collect()
        };
        if id == l.copy || id == l.clone {
            return Ok(match t.as_ref() {
                TyKind::Tuple(v) => nested(v.clone()),
                TyKind::Array(e, _) => nested(vec![e.clone()]),
                _ => Vec::new(),
            });
        }
        if id == l.fn_ || id == l.fn_mut || id == l.fn_once {
            let (inputs, kind) = match t.as_ref() {
                TyKind::Closure(c) => (c.inputs.clone(), c.kind),
                TyKind::FnPtr(i, _) => (i.clone(), ClosureKind::Fn),
                _ => return Err(SolveError::Unsatisfied(Goal::Trait(tr.clone()))),
            };
            let ok = match kind {
                ClosureKind::Fn => true,
                ClosureKind::FnMut => id != l.fn_,
                ClosureKind::FnOnce => id == l.fn_once,
            };
            if !ok {
                return Err(SolveError::Unsatisfied(Goal::Trait(tr.clone())));
            }
            // Fn traits take their arguments as one tuple.
            let args_tuple = mk(TyKind::Tuple(inputs));
            if let Some(a) = tr.args.first() {
                cx.unify(a, &args_tuple)?;
            }
            return Ok(Vec::new());
        }
        Ok(Vec::new())
    }

    fn select_trait(&self, cx: &mut InferCtxt, tr: &TraitRef) -> Result<Selected, SolveError> {
        let (cands, unknown_self) = self.assemble(cx, tr)?;
        if unknown_self {
            return Ok(Selected::Ambiguous);
        }
        match cands.len() {
            0 => Ok(Selected::NoMatch),
            1 => Ok(Selected::Done(self.confirm(cx, tr, cands[0])?)),
            _ => {
                // Several candidates: if they all agree after winnowing by
                // nested goals that already fail, pick the survivor.
                let mut live = Vec::new();
                for c in cands {
                    let ok = cx.can(|cx| {
                        let nested = self.confirm(cx, tr, c)?;
                        for g in nested {
                            if let Goal::Trait(n) = &g
                                && matches!(self.select_trait(cx, n)?, Selected::NoMatch)
                            {
                                return Err(SolveError::Unsatisfied(g.clone()));
                            }
                        }
                        Ok::<(), SolveError>(())
                    });
                    if ok {
                        live.push(c);
                    }
                }
                match live.len() {
                    0 => Ok(Selected::NoMatch),
                    1 => Ok(Selected::Done(self.confirm(cx, tr, live[0])?)),
                    _ => Ok(Selected::Ambiguous),
                }
            }
        }
    }

    fn select_projection(
        &self,
        cx: &mut InferCtxt,
        proj: &ProjectionTy,
        ty: &Ty,
    ) -> Result<Selected, SolveError> {
        let tr = &proj.trait_ref;
        let l = self.db.lang();
        // Environment equalities first.
        for p in &self.env.predicates {
            if let Predicate::Projection(q, rhs) = p
                && q.item == proj.item
                && cx.can(|cx| {
                    cx.unify(&q.trait_ref.self_ty, &tr.self_ty)?;
                    for (x, y) in q.trait_ref.args.iter().zip(&tr.args) {
                        cx.unify(x, y)?;
                    }
                    Ok::<(), TypeError>(())
                })
            {
                cx.unify(&q.trait_ref.self_ty, &tr.self_ty)?;
                for (x, y) in q.trait_ref.args.iter().zip(&tr.args) {
                    cx.unify(x, y)?;
                }
                cx.unify(rhs, ty)?;
                return Ok(Selected::Done(Vec::new()));
            }
        }
        let self_ty = cx.shallow(&tr.self_ty);
        if matches!(self_ty.as_ref(), TyKind::Param(_) | TyKind::Projection(_)) {
            // Rigid: the projection is its own normal form.
            cx.unify(
                &mk(TyKind::Projection(std::sync::Arc::new(proj.clone()))),
                ty,
            )?;
            return Ok(Selected::Done(vec![Goal::Trait(tr.clone())]));
        }
        // Builtin `Output` for closures and fn pointers. The prelude declares
        // `Output` on each of Fn, FnMut and FnOnce, so any associated type of
        // a Fn-family trait is the callable's return type.
        let fn_family = [l.fn_, l.fn_mut, l.fn_once].contains(&Some(tr.trait_id));
        if Some(proj.item) == l.fn_once_output
            || (fn_family
                && self
                    .db
                    .trait_data(tr.trait_id)
                    .assoc_types
                    .contains(&proj.item))
        {
            match self_ty.as_ref() {
                TyKind::Closure(c) => {
                    cx.unify(&c.output, ty)?;
                    return Ok(Selected::Done(vec![Goal::Trait(tr.clone())]));
                }
                TyKind::FnPtr(_, o) => {
                    cx.unify(o, ty)?;
                    return Ok(Selected::Done(vec![Goal::Trait(tr.clone())]));
                }
                _ => {}
            }
        }
        let (cands, unknown) = self.assemble(cx, tr)?;
        if unknown {
            return Ok(Selected::Ambiguous);
        }
        let impls: Vec<DefId> = cands
            .iter()
            .filter_map(|c| match c {
                Candidate::Impl(i) => Some(*i),
                _ => None,
            })
            .collect();
        if impls.len() > 1 {
            return Ok(Selected::Ambiguous);
        }
        let Some(&i) = impls.first() else {
            return Ok(if cands.is_empty() {
                Selected::NoMatch
            } else {
                Selected::Ambiguous
            });
        };
        let data = self.db.impl_data(i);
        let args = cx.fresh_args(data.generics as usize);
        let itr = subst_trait_ref(data.trait_ref.as_ref().unwrap(), &args);
        cx.unify(&itr.self_ty, &tr.self_ty)?;
        for (x, y) in itr.args.iter().zip(&tr.args) {
            cx.unify(x, y)?;
        }
        let Some((_, value)) = data.assoc_types.iter().find(|(d, _)| *d == proj.item) else {
            return Ok(Selected::NoMatch);
        };
        let mut nested: Vec<Goal> = data
            .predicates
            .iter()
            .map(|p| pred_goal(&subst_pred(p, &args)))
            .collect();
        let value = subst(value, &args);
        let value = self.normalize(cx, &value, &mut nested);
        cx.unify(&value, ty)?;
        Ok(Selected::Done(nested))
    }
}

pub fn pred_goal(p: &Predicate) -> Goal {
    match p {
        Predicate::Trait(t) => Goal::Trait(t.clone()),
        Predicate::Projection(p, t) => Goal::Projection(p.clone(), t.clone()),
    }
}

/// Pending obligations of one body. Local scratch, never published.
#[derive(Default)]
pub struct Fulfill {
    pub pending: Vec<Obligation>,
}

impl Fulfill {
    pub fn register(&mut self, goal: Goal, cause: u32) {
        self.pending.push(Obligation {
            goal,
            depth: 0,
            cause,
        });
    }

    /// Process obligations until no further progress. Returns errors for
    /// goals that definitely fail; ambiguous goals stay pending.
    pub fn select_where_possible(
        &mut self,
        solver: &Solver<'_>,
        cx: &mut InferCtxt,
    ) -> Vec<(Obligation, SolveError)> {
        let mut errors = Vec::new();
        loop {
            let mut progress = false;
            let pending = std::mem::take(&mut self.pending);
            for ob in pending {
                // Normalize projections inside the goal before selecting.
                let mut extra = Vec::new();
                let goal = match &ob.goal {
                    Goal::Trait(tr) => Goal::Trait(TraitRef {
                        trait_id: tr.trait_id,
                        self_ty: solver.normalize(cx, &tr.self_ty, &mut extra),
                        args: tr
                            .args
                            .iter()
                            .map(|a| solver.normalize(cx, a, &mut extra))
                            .collect(),
                    }),
                    g => g.clone(),
                };
                for g in extra {
                    self.pending.push(Obligation {
                        goal: g,
                        depth: ob.depth + 1,
                        cause: ob.cause,
                    });
                }
                match cx.probe(|cx| solver.select(cx, &goal, ob.depth)) {
                    Ok(Selected::Done(nested)) => {
                        progress = true;
                        for g in nested {
                            self.pending.push(Obligation {
                                goal: g,
                                depth: ob.depth + 1,
                                cause: ob.cause,
                            });
                        }
                    }
                    Ok(Selected::Ambiguous) => self.pending.push(Obligation { goal, ..ob }),
                    Ok(Selected::NoMatch) => {
                        let g = goal.clone();
                        errors.push((Obligation { goal, ..ob }, SolveError::Unsatisfied(g)));
                    }
                    Err(e) => errors.push((Obligation { goal, ..ob }, e)),
                }
            }
            if !progress || !errors.is_empty() {
                return errors;
            }
        }
    }

    /// After all inference: remaining obligations are ambiguity errors.
    pub fn finish(
        &mut self,
        solver: &Solver<'_>,
        cx: &mut InferCtxt,
    ) -> Vec<(Obligation, SolveError)> {
        let mut errors = self.select_where_possible(solver, cx);
        if errors.is_empty() {
            for ob in self.pending.drain(..) {
                let g = ob.goal.clone();
                errors.push((ob, SolveError::Ambiguous(g)));
            }
        }
        errors
    }

    /// Closure signature deduction: find a pending `F: Fn*(Args)` bound and
    /// matching `<F as FnOnce>::Output == R` on the given closure variable.
    pub fn expected_closure_sig(
        &self,
        solver: &Solver<'_>,
        cx: &InferCtxt,
        closure_var: &Ty,
    ) -> Option<(Vec<Ty>, Option<Ty>, ClosureKind)> {
        let l = solver.db.lang();
        let target = cx.shallow(closure_var);
        let same = |t: &Ty| cx.shallow(t) == target;
        let mut sig: Option<(Vec<Ty>, Option<Ty>, ClosureKind)> = None;
        for ob in &self.pending {
            match &ob.goal {
                Goal::Trait(tr) if same(&tr.self_ty) => {
                    let kind = if Some(tr.trait_id) == l.fn_ {
                        ClosureKind::Fn
                    } else if Some(tr.trait_id) == l.fn_mut {
                        ClosureKind::FnMut
                    } else if Some(tr.trait_id) == l.fn_once {
                        ClosureKind::FnOnce
                    } else {
                        continue;
                    };
                    let args = tr.args.first().map(|a| cx.shallow(a));
                    if let Some(a) = args
                        && let TyKind::Tuple(v) = a.as_ref()
                    {
                        let out = sig.as_ref().and_then(|s| s.1.clone());
                        sig = Some((v.clone(), out, kind));
                    }
                }
                Goal::Projection(p, rhs)
                    if Some(p.item) == l.fn_once_output && same(&p.trait_ref.self_ty) =>
                {
                    if let Some(s) = &mut sig {
                        s.1 = Some(rhs.clone());
                    } else if let Some(a) = p.trait_ref.args.first()
                        && let TyKind::Tuple(v) = cx.shallow(a).as_ref()
                    {
                        sig = Some((v.clone(), Some(rhs.clone()), ClosureKind::FnOnce));
                    }
                }
                _ => {}
            }
        }
        sig
    }
}
