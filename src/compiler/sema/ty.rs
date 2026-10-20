//! Semantic types for the general inference core.
//!
//! These types are independent of `ir::Ty`: inference variables, holes and
//! rigid projections live here, and only fully resolved types are converted
//! back to the published IR by an adapter. Every value is immutable and
//! shared through `Arc`, so a type can be cloned freely inside one node.

use std::fmt;
use std::sync::Arc;

pub use super::super::ir::{FloatTy, IntTy};

/// Declaration identity as seen by the semantic core. The adapter maps graph
/// declaration identities to these dense numbers for one body check.
pub type DefId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TyVid(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mutbl {
    Not,
    Mut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClosureKind {
    Fn,
    FnMut,
    FnOnce,
}

/// A closure type. `id` is local to the body that defines it. Its signature
/// may still contain inference variables while the body is being checked.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClosureTy {
    pub id: u32,
    pub inputs: Vec<Ty>,
    pub output: Ty,
    pub kind: ClosureKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitRef {
    pub trait_id: DefId,
    pub self_ty: Ty,
    pub args: Vec<Ty>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProjectionTy {
    pub trait_ref: TraitRef,
    pub item: DefId,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TyKind {
    Bool,
    Char,
    Str,
    Int(IntTy),
    Float(FloatTy),
    Never,
    /// The empty tuple is unit.
    Tuple(Vec<Ty>),
    Adt(DefId, Vec<Ty>),
    Ref(Mutbl, Ty),
    RawPtr(Mutbl, Ty),
    Array(Ty, u64),
    Slice(Ty),
    FnPtr(Vec<Ty>, Ty),
    Closure(Arc<ClosureTy>),
    /// Generic parameter by position in the item's full parameter list
    /// (parent parameters first, then the item's own).
    Param(u32),
    /// A projection that could not be normalized further because it is
    /// rigid in the current environment, for example `<T as Iterator>::Item`.
    Projection(Arc<ProjectionTy>),
    /// General, integer or float inference variable.
    Var(TyVid),
    /// A written `_` or an omitted annotation, replaced by a fresh variable.
    Hole,
    /// A type whose error was already reported. Unifies with anything.
    Error,
}

pub type Ty = Arc<TyKind>;

pub fn mk(kind: TyKind) -> Ty {
    Arc::new(kind)
}
pub fn unit() -> Ty {
    mk(TyKind::Tuple(Vec::new()))
}
pub fn int(i: IntTy) -> Ty {
    mk(TyKind::Int(i))
}
pub fn bool_ty() -> Ty {
    mk(TyKind::Bool)
}
pub fn param(i: u32) -> Ty {
    mk(TyKind::Param(i))
}
pub fn reference(m: Mutbl, t: Ty) -> Ty {
    mk(TyKind::Ref(m, t))
}
pub fn adt(d: DefId, args: Vec<Ty>) -> Ty {
    mk(TyKind::Adt(d, args))
}
pub fn error() -> Ty {
    mk(TyKind::Error)
}
pub fn hole() -> Ty {
    mk(TyKind::Hole)
}

/// Generic predicate over parameters, as declared by an item.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Predicate {
    Trait(TraitRef),
    /// `<trait_ref>::item == ty`
    Projection(ProjectionTy, Ty),
}

/// Substitute `Param(i)` by `args[i]`. Parameters beyond `args` are left
/// unchanged; callers that require completeness check arity first.
pub fn subst(ty: &Ty, args: &[Ty]) -> Ty {
    fold(ty, &mut |t| match t.as_ref() {
        TyKind::Param(i) => args.get(*i as usize).cloned(),
        _ => None,
    })
}

pub fn subst_trait_ref(tr: &TraitRef, args: &[Ty]) -> TraitRef {
    TraitRef {
        trait_id: tr.trait_id,
        self_ty: subst(&tr.self_ty, args),
        args: tr.args.iter().map(|a| subst(a, args)).collect(),
    }
}

pub fn subst_pred(p: &Predicate, args: &[Ty]) -> Predicate {
    match p {
        Predicate::Trait(tr) => Predicate::Trait(subst_trait_ref(tr, args)),
        Predicate::Projection(proj, ty) => Predicate::Projection(
            ProjectionTy {
                trait_ref: subst_trait_ref(&proj.trait_ref, args),
                item: proj.item,
            },
            subst(ty, args),
        ),
    }
}

/// Structural map. `f` returns `Some` to replace a node, else children are
/// folded. Shares unchanged subtrees.
pub fn fold(ty: &Ty, f: &mut impl FnMut(&Ty) -> Option<Ty>) -> Ty {
    if let Some(t) = f(ty) {
        return t;
    }
    let list = |v: &Vec<Ty>, f: &mut dyn FnMut(&Ty) -> Ty| -> (Vec<Ty>, bool) {
        let mut changed = false;
        let out = v
            .iter()
            .map(|t| {
                let n = f(t);
                changed |= !Arc::ptr_eq(&n, t);
                n
            })
            .collect();
        (out, changed)
    };
    let mut rec = |t: &Ty| fold(t, f);
    let new = match ty.as_ref() {
        TyKind::Tuple(v) => {
            let (v, c) = list(v, &mut rec);
            if !c {
                return ty.clone();
            }
            TyKind::Tuple(v)
        }
        TyKind::Adt(d, v) => {
            let (v, c) = list(v, &mut rec);
            if !c {
                return ty.clone();
            }
            TyKind::Adt(*d, v)
        }
        TyKind::Ref(m, t) => TyKind::Ref(*m, rec(t)),
        TyKind::RawPtr(m, t) => TyKind::RawPtr(*m, rec(t)),
        TyKind::Array(t, n) => TyKind::Array(rec(t), *n),
        TyKind::Slice(t) => TyKind::Slice(rec(t)),
        TyKind::FnPtr(i, o) => {
            let (i, _) = list(i, &mut rec);
            TyKind::FnPtr(i, rec(o))
        }
        TyKind::Closure(c) => {
            let (inputs, _) = list(&c.inputs, &mut rec);
            TyKind::Closure(Arc::new(ClosureTy {
                id: c.id,
                inputs,
                output: rec(&c.output),
                kind: c.kind,
            }))
        }
        TyKind::Projection(p) => {
            let tr = &p.trait_ref;
            let (args, _) = list(&tr.args, &mut rec);
            TyKind::Projection(Arc::new(ProjectionTy {
                trait_ref: TraitRef {
                    trait_id: tr.trait_id,
                    self_ty: rec(&tr.self_ty),
                    args,
                },
                item: p.item,
            }))
        }
        _ => return ty.clone(),
    };
    Arc::new(new)
}

/// Visit every node, stopping early when `f` returns true.
pub fn any(ty: &Ty, f: &mut impl FnMut(&Ty) -> bool) -> bool {
    if f(ty) {
        return true;
    }
    match ty.as_ref() {
        TyKind::Tuple(v) | TyKind::Adt(_, v) => v.iter().any(|t| any(t, f)),
        TyKind::Ref(_, t) | TyKind::RawPtr(_, t) | TyKind::Array(t, _) | TyKind::Slice(t) => {
            any(t, f)
        }
        TyKind::FnPtr(i, o) => i.iter().any(|t| any(t, f)) || any(o, f),
        TyKind::Closure(c) => c.inputs.iter().any(|t| any(t, f)) || any(&c.output, f),
        TyKind::Projection(p) => {
            any(&p.trait_ref.self_ty, f) || p.trait_ref.args.iter().any(|t| any(t, f))
        }
        _ => false,
    }
}

/// Key for inherent impl lookup, so a graph adapter can serve one fact per
/// type constructor instead of scanning every impl.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimplifiedType {
    Bool,
    Char,
    Str,
    Int(IntTy),
    Float(FloatTy),
    Adt(DefId),
    Slice,
    Array,
    Ref,
    RawPtr,
    Tuple(usize),
}

pub fn simplify(ty: &Ty) -> Option<SimplifiedType> {
    Some(match ty.as_ref() {
        TyKind::Bool => SimplifiedType::Bool,
        TyKind::Char => SimplifiedType::Char,
        TyKind::Str => SimplifiedType::Str,
        TyKind::Int(i) => SimplifiedType::Int(*i),
        TyKind::Float(f) => SimplifiedType::Float(*f),
        TyKind::Adt(d, _) => SimplifiedType::Adt(*d),
        TyKind::Slice(_) => SimplifiedType::Slice,
        TyKind::Array(..) => SimplifiedType::Array,
        TyKind::Ref(..) => SimplifiedType::Ref,
        TyKind::RawPtr(..) => SimplifiedType::RawPtr,
        TyKind::Tuple(v) => SimplifiedType::Tuple(v.len()),
        _ => return None,
    })
}

pub fn int_name(i: IntTy) -> &'static str {
    match i {
        IntTy::I8 => "i8",
        IntTy::I16 => "i16",
        IntTy::I32 => "i32",
        IntTy::I64 => "i64",
        IntTy::I128 => "i128",
        IntTy::Isize => "isize",
        IntTy::U8 => "u8",
        IntTy::U16 => "u16",
        IntTy::U32 => "u32",
        IntTy::U64 => "u64",
        IntTy::U128 => "u128",
        IntTy::Usize => "usize",
    }
}

impl fmt::Display for TyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, v: &[Ty]| -> fmt::Result {
            for (i, t) in v.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{t}")?;
            }
            Ok(())
        };
        match self {
            TyKind::Bool => write!(f, "bool"),
            TyKind::Char => write!(f, "char"),
            TyKind::Str => write!(f, "str"),
            TyKind::Int(i) => write!(f, "{}", int_name(*i)),
            TyKind::Float(FloatTy::F32) => write!(f, "f32"),
            TyKind::Float(FloatTy::F64) => write!(f, "f64"),
            TyKind::Never => write!(f, "!"),
            TyKind::Tuple(v) => {
                write!(f, "(")?;
                list(f, v)?;
                if v.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")
            }
            TyKind::Adt(d, v) => {
                write!(f, "adt#{d}")?;
                if !v.is_empty() {
                    write!(f, "<")?;
                    list(f, v)?;
                    write!(f, ">")?;
                }
                Ok(())
            }
            TyKind::Ref(Mutbl::Not, t) => write!(f, "&{t}"),
            TyKind::Ref(Mutbl::Mut, t) => write!(f, "&mut {t}"),
            TyKind::RawPtr(Mutbl::Not, t) => write!(f, "*const {t}"),
            TyKind::RawPtr(Mutbl::Mut, t) => write!(f, "*mut {t}"),
            TyKind::Array(t, n) => write!(f, "[{t}; {n}]"),
            TyKind::Slice(t) => write!(f, "[{t}]"),
            TyKind::FnPtr(i, o) => {
                write!(f, "fn(")?;
                list(f, i)?;
                write!(f, ") -> {o}")
            }
            TyKind::Closure(c) => write!(f, "closure#{}", c.id),
            TyKind::Param(i) => write!(f, "P{i}"),
            TyKind::Projection(p) => write!(
                f,
                "<{} as trait#{}>::item#{}",
                p.trait_ref.self_ty, p.trait_ref.trait_id, p.item
            ),
            TyKind::Var(v) => write!(f, "?{}", v.0),
            TyKind::Hole => write!(f, "_"),
            TyKind::Error => write!(f, "{{error}}"),
        }
    }
}
