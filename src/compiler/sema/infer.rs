//! Inference context: union-find type variables, unification, snapshots.
//!
//! Local scratch for one body check. Never published as a graph fact.

use super::ty::*;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    General,
    Integer,
    Float,
}

#[derive(Clone, Debug)]
struct VarData {
    parent: u32,
    rank: u8,
    kind: VarKind,
    value: Option<Ty>,
}

#[derive(Clone, Debug)]
enum Undo {
    NewVar,
    SetParent(u32, u32, u8),
    SetValue(u32, Option<Ty>, VarKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeError {
    Mismatch(Ty, Ty),
    Cyclic(Ty, Ty),
    Limit,
}

/// Deterministic per-node work meter. Limits scale with the node's input and
/// never touch shared state.
#[derive(Clone, Debug)]
pub struct Meter {
    pub used: u64,
    pub limit: u64,
}
impl Meter {
    pub fn new(limit: u64) -> Self {
        Self { used: 0, limit }
    }
    pub fn charge(&mut self, n: u64) -> Result<(), TypeError> {
        self.used = self.used.saturating_add(n);
        if self.used > self.limit {
            Err(TypeError::Limit)
        } else {
            Ok(())
        }
    }
}

pub struct Snapshot {
    undo_len: usize,
}

#[derive(Clone, Debug)]
pub struct InferCtxt {
    vars: Vec<VarData>,
    undo: Vec<Undo>,
    snapshots: usize,
    pub meter: Meter,
}

impl InferCtxt {
    pub fn new(limit: u64) -> Self {
        Self {
            vars: Vec::new(),
            undo: Vec::new(),
            snapshots: 0,
            meter: Meter::new(limit),
        }
    }

    pub fn var_count(&self) -> usize {
        self.vars.len()
    }

    pub fn new_var_of(&mut self, kind: VarKind) -> Ty {
        let id = self.vars.len() as u32;
        self.vars.push(VarData {
            parent: id,
            rank: 0,
            kind,
            value: None,
        });
        if self.snapshots > 0 {
            self.undo.push(Undo::NewVar);
        }
        mk(TyKind::Var(TyVid(id)))
    }
    pub fn new_var(&mut self) -> Ty {
        self.new_var_of(VarKind::General)
    }
    pub fn new_int_var(&mut self) -> Ty {
        self.new_var_of(VarKind::Integer)
    }
    pub fn new_float_var(&mut self) -> Ty {
        self.new_var_of(VarKind::Float)
    }

    fn find(&self, mut v: u32) -> u32 {
        while self.vars[v as usize].parent != v {
            v = self.vars[v as usize].parent;
        }
        v
    }

    pub fn var_kind(&self, v: TyVid) -> VarKind {
        self.vars[self.find(v.0) as usize].kind
    }

    pub fn probe_var(&self, v: TyVid) -> Option<Ty> {
        self.vars[self.find(v.0) as usize].value.clone()
    }

    pub fn root_var(&self, v: TyVid) -> TyVid {
        TyVid(self.find(v.0))
    }

    /// Resolve the outermost variable chain only.
    pub fn shallow(&self, ty: &Ty) -> Ty {
        let mut t = ty.clone();
        loop {
            let TyKind::Var(v) = t.as_ref() else {
                return t;
            };
            let root = self.find(v.0);
            match &self.vars[root as usize].value {
                Some(val) => t = val.clone(),
                None => {
                    return if root == v.0 {
                        t
                    } else {
                        mk(TyKind::Var(TyVid(root)))
                    };
                }
            }
        }
    }

    /// Replace every bound variable, recursively.
    pub fn resolve(&self, ty: &Ty) -> Ty {
        fold(ty, &mut |t| match t.as_ref() {
            TyKind::Var(_) => {
                let s = self.shallow(t);
                if matches!(s.as_ref(), TyKind::Var(_)) {
                    Some(s)
                } else {
                    Some(self.resolve(&s))
                }
            }
            _ => None,
        })
    }

    pub fn has_unresolved(&self, ty: &Ty) -> bool {
        let r = self.resolve(ty);
        any(&r, &mut |t| matches!(t.as_ref(), TyKind::Var(_)))
    }

    pub fn snapshot(&mut self) -> Snapshot {
        self.snapshots += 1;
        Snapshot {
            undo_len: self.undo.len(),
        }
    }
    pub fn rollback(&mut self, s: Snapshot) {
        while self.undo.len() > s.undo_len {
            match self.undo.pop().unwrap() {
                Undo::NewVar => {
                    self.vars.pop();
                }
                Undo::SetParent(v, p, r) => {
                    let d = &mut self.vars[v as usize];
                    d.parent = p;
                    d.rank = r;
                }
                Undo::SetValue(v, val, kind) => {
                    let d = &mut self.vars[v as usize];
                    d.value = val;
                    d.kind = kind;
                }
            }
        }
        self.snapshots -= 1;
    }
    pub fn commit(&mut self, _s: Snapshot) {
        self.snapshots -= 1;
        if self.snapshots == 0 {
            self.undo.clear();
        }
    }
    /// Run `f`; keep its effects only if it succeeds.
    pub fn probe<T, E>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, E>) -> Result<T, E> {
        let s = self.snapshot();
        match f(self) {
            Ok(v) => {
                self.commit(s);
                Ok(v)
            }
            Err(e) => {
                self.rollback(s);
                Err(e)
            }
        }
    }
    /// Run `f` and always roll back. Reports only whether it succeeded.
    pub fn can<T, E>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, E>) -> bool {
        let s = self.snapshot();
        let ok = f(self).is_ok();
        self.rollback(s);
        ok
    }

    fn set_value(&mut self, root: u32, value: Ty) {
        let d = &self.vars[root as usize];
        if self.snapshots > 0 {
            self.undo
                .push(Undo::SetValue(root, d.value.clone(), d.kind));
        }
        self.vars[root as usize].value = Some(value);
    }
    fn set_kind(&mut self, root: u32, kind: VarKind) {
        let d = &self.vars[root as usize];
        if self.snapshots > 0 {
            self.undo
                .push(Undo::SetValue(root, d.value.clone(), d.kind));
        }
        self.vars[root as usize].kind = kind;
    }

    fn union(&mut self, a: u32, b: u32) -> Result<(), TypeError> {
        let (ka, kb) = (self.vars[a as usize].kind, self.vars[b as usize].kind);
        let kind = match (ka, kb) {
            (VarKind::General, k) | (k, VarKind::General) => k,
            (x, y) if x == y => x,
            _ => {
                return Err(TypeError::Mismatch(
                    mk(TyKind::Var(TyVid(a))),
                    mk(TyKind::Var(TyVid(b))),
                ));
            }
        };
        let (ra, rb) = (self.vars[a as usize].rank, self.vars[b as usize].rank);
        let (child, root) = if ra < rb { (a, b) } else { (b, a) };
        if self.snapshots > 0 {
            let c = &self.vars[child as usize];
            self.undo.push(Undo::SetParent(child, c.parent, c.rank));
            let r = &self.vars[root as usize];
            self.undo.push(Undo::SetParent(root, r.parent, r.rank));
        }
        self.vars[child as usize].parent = root;
        if ra == rb {
            self.vars[root as usize].rank += 1;
        }
        self.set_kind(root, kind);
        Ok(())
    }

    fn occurs(&self, v: u32, ty: &Ty) -> bool {
        let r = self.resolve(ty);
        any(&r, &mut |t| match t.as_ref() {
            TyKind::Var(w) => self.find(w.0) == v,
            _ => false,
        })
    }

    fn bind(&mut self, root: u32, ty: &Ty) -> Result<(), TypeError> {
        let kind = self.vars[root as usize].kind;
        let ok = matches!(
            (kind, ty.as_ref()),
            (VarKind::General, _)
                | (_, TyKind::Error)
                | (VarKind::Integer, TyKind::Int(_))
                | (VarKind::Float, TyKind::Float(_))
        );
        let var = mk(TyKind::Var(TyVid(root)));
        if !ok {
            return Err(TypeError::Mismatch(var, ty.clone()));
        }
        if self.occurs(root, ty) {
            return Err(TypeError::Cyclic(var, ty.clone()));
        }
        self.set_value(root, ty.clone());
        Ok(())
    }

    /// Structural equality with variable binding. Projections are expected to
    /// have been normalized by the caller; rigid projections compare
    /// structurally.
    pub fn unify(&mut self, a: &Ty, b: &Ty) -> Result<(), TypeError> {
        self.meter.charge(1)?;
        let a = self.shallow(a);
        let b = self.shallow(b);
        if Arc::ptr_eq(&a, &b) {
            return Ok(());
        }
        use TyKind::*;
        match (a.as_ref(), b.as_ref()) {
            (Error, _) | (_, Error) => Ok(()),
            (Var(x), Var(y)) => {
                let (x, y) = (self.find(x.0), self.find(y.0));
                if x == y { Ok(()) } else { self.union(x, y) }
            }
            (Var(x), _) => {
                let r = self.find(x.0);
                self.bind(r, &b)
            }
            (_, Var(y)) => {
                let r = self.find(y.0);
                self.bind(r, &a)
            }
            (Bool, Bool) | (Char, Char) | (Str, Str) | (Never, Never) => Ok(()),
            (Int(x), Int(y)) if x == y => Ok(()),
            (Float(x), Float(y)) if x == y => Ok(()),
            (Param(x), Param(y)) if x == y => Ok(()),
            (Tuple(x), Tuple(y)) if x.len() == y.len() => self.unify_all(x, y),
            (Adt(d, x), Adt(e, y)) if d == e && x.len() == y.len() => self.unify_all(x, y),
            (Ref(m, x), Ref(n, y)) | (RawPtr(m, x), RawPtr(n, y)) if m == n => self.unify(x, y),
            (Array(x, n), Array(y, m)) if n == m => self.unify(x, y),
            (Slice(x), Slice(y)) => self.unify(x, y),
            (FnPtr(i, o), FnPtr(j, p)) if i.len() == j.len() => {
                self.unify_all(i, j)?;
                self.unify(o, p)
            }
            (Closure(x), Closure(y)) if x.id == y.id => {
                self.unify_all(&x.inputs, &y.inputs)?;
                self.unify(&x.output, &y.output)
            }
            (Projection(p), Projection(q)) if p.item == q.item => {
                self.unify(&p.trait_ref.self_ty, &q.trait_ref.self_ty)?;
                self.unify_all(&p.trait_ref.args, &q.trait_ref.args)
            }
            _ => Err(TypeError::Mismatch(a.clone(), b.clone())),
        }
    }

    fn unify_all(&mut self, a: &[Ty], b: &[Ty]) -> Result<(), TypeError> {
        for (x, y) in a.iter().zip(b) {
            self.unify(x, y)?;
        }
        Ok(())
    }

    /// Unify, rolling back on failure.
    pub fn try_unify(&mut self, a: &Ty, b: &Ty) -> Result<(), TypeError> {
        self.probe(|cx| cx.unify(a, b))
    }

    /// Apply integer/float literal defaults (i32, f64). Returns true if any
    /// variable changed.
    pub fn default_numeric_vars(&mut self) -> bool {
        let mut changed = false;
        for i in 0..self.vars.len() as u32 {
            let r = self.find(i);
            if r != i || self.vars[r as usize].value.is_some() {
                continue;
            }
            let t = match self.vars[r as usize].kind {
                VarKind::Integer => int(IntTy::I32),
                VarKind::Float => mk(TyKind::Float(FloatTy::F64)),
                VarKind::General => continue,
            };
            self.set_value(r, t);
            changed = true;
        }
        changed
    }

    /// Instantiate `Param(i)` in a generic type with the given arguments.
    pub fn instantiate(&self, ty: &Ty, args: &[Ty]) -> Ty {
        subst(ty, args)
    }

    pub fn fresh_args(&mut self, n: usize) -> Vec<Ty> {
        (0..n).map(|_| self.new_var()).collect()
    }

    /// Replace holes with fresh variables.
    pub fn fill_holes(&mut self, ty: &Ty) -> Ty {
        let mut out = Vec::new();
        let r = fold(ty, &mut |t| {
            if matches!(t.as_ref(), TyKind::Hole) {
                out.push(());
                None
            } else {
                None
            }
        });
        if out.is_empty() {
            return r;
        }
        self.fill_holes_inner(ty)
    }
    fn fill_holes_inner(&mut self, ty: &Ty) -> Ty {
        let mut vars = Vec::new();
        let count = {
            let mut c = 0;
            any(ty, &mut |t| {
                if matches!(t.as_ref(), TyKind::Hole) {
                    c += 1;
                }
                false
            });
            c
        };
        for _ in 0..count {
            vars.push(self.new_var());
        }
        let mut it = vars.into_iter();
        fold(ty, &mut |t| {
            if matches!(t.as_ref(), TyKind::Hole) {
                it.next()
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unify_binds_and_resolves() {
        let mut cx = InferCtxt::new(10_000);
        let a = cx.new_var();
        let v = adt(7, vec![a.clone()]);
        cx.unify(&v, &adt(7, vec![int(IntTy::U8)])).unwrap();
        assert_eq!(cx.resolve(&a), int(IntTy::U8));
    }

    #[test]
    fn integer_var_rejects_bool_and_defaults() {
        let mut cx = InferCtxt::new(10_000);
        let a = cx.new_int_var();
        assert!(cx.try_unify(&a, &bool_ty()).is_err());
        let b = cx.new_var();
        cx.unify(&a, &b).unwrap();
        assert!(cx.try_unify(&b, &bool_ty()).is_err());
        cx.default_numeric_vars();
        assert_eq!(cx.resolve(&b), int(IntTy::I32));
    }

    #[test]
    fn later_use_decides_literal() {
        let mut cx = InferCtxt::new(10_000);
        let a = cx.new_int_var();
        cx.unify(&a, &int(IntTy::Usize)).unwrap();
        cx.default_numeric_vars();
        assert_eq!(cx.resolve(&a), int(IntTy::Usize));
    }

    #[test]
    fn occurs_check() {
        let mut cx = InferCtxt::new(10_000);
        let a = cx.new_var();
        let r = cx.unify(&a, &adt(1, vec![a.clone()]));
        assert!(matches!(r, Err(TypeError::Cyclic(..))));
    }

    #[test]
    fn rollback_restores() {
        let mut cx = InferCtxt::new(10_000);
        let a = cx.new_var();
        let n = cx.var_count();
        let ok = cx.can(|cx| {
            let _ = cx.new_var();
            cx.unify(&a, &bool_ty())
        });
        assert!(ok);
        assert_eq!(cx.var_count(), n);
        assert!(matches!(cx.shallow(&a).as_ref(), TyKind::Var(_)));
        assert!(
            cx.try_unify(&adt(1, vec![a.clone()]), &adt(2, vec![]))
                .is_err()
        );
        assert!(matches!(cx.shallow(&a).as_ref(), TyKind::Var(_)));
    }

    #[test]
    fn meter_limits() {
        let mut cx = InferCtxt::new(3);
        let t = mk(TyKind::Tuple(vec![int(IntTy::I8); 5]));
        assert_eq!(cx.unify(&t, &t.clone()), Ok(()));
        let u = mk(TyKind::Tuple(vec![int(IntTy::I8); 5]));
        assert_eq!(cx.unify(&t, &u), Err(TypeError::Limit));
    }
}
