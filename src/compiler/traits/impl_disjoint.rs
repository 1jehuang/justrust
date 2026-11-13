//! Negative reasoning for impl overlap (E0119).
//!
//! Two impl heads that unify still do not overlap when a where-clause of
//! either impl can never hold for the unified type: `impl<I: Iterator>
//! IntoIterator for I` and `impl<T> IntoIterator for Vec<T>` coexist because
//! `Vec<T>: Iterator` has no implementation and none can appear later.
//!
//! "Can never hold" is deliberately one-sided. A goal is refuted only when
//! every way of proving it is visible here and absent:
//! - catalog memberships are never refuted; builtin callable memberships are
//!   refuted only for intrinsically non-callable type constructors,
//! - a type that is, or hides, a parameter, projection, opaque, object or
//!   callable type is never refuted,
//! - a type a downstream crate could make its own (`&T`, `Box<T>` over an
//!   unbound parameter) is never refuted,
//! - a trait of an upstream crate (the library, seen from a user crate) is
//!   only refuted for a type the checked crate declares, since upstream may
//!   add an implementation for any other type later,
//! - an implementation whose head unifies refutes nothing unless one of its
//!   own where-clauses is refuted by the same rules.
//!
//! Anything undecided keeps the overlap error, so a wrong answer here can
//! only reject a program, never admit two applicable implementations.
use super::*;

/// Nested where-clause refutations followed before giving up as "may hold".
const MAX_NESTING: usize = 4;

/// Which crate declares a module. One program graph holds the user crate,
/// the embedded library and mounted extern crates, but coherence still
/// reasons per crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrateOrigin {
    /// The embedded standard library.
    Library,
    /// A mounted dependency, identified by its mount module.
    Extern(ModuleId),
    /// The crate being compiled.
    User,
}

impl ValidatedTraitWorld {
    /// Whether a where-clause of `a` (side 0) or `b` (side 1) is refuted
    /// under the head unifier already recorded in `bindings`.
    pub(super) fn heads_disjoint_by_bounds<'a>(
        &'a self,
        a: &'a ImplHeader,
        b: &'a ImplHeader,
        bindings: &mut Vec<Binding<'a>>,
    ) -> Result<bool> {
        // The crate being checked is the most downstream of the two impls.
        // Without crate metadata the world is one closed crate (`None`).
        let mut local = None;
        for implementation in [a, b] {
            let origin = self
                .schema
                .crate_origin(self.info(implementation.declaration)?.module)?;
            local = match (local, origin) {
                (None, origin) => origin,
                (Some(CrateOrigin::Library), Some(origin)) => Some(origin),
                (Some(CrateOrigin::Extern(_)), Some(CrateOrigin::User)) => origin,
                (local, _) => local,
            };
        }
        // An implicitly `Sized` impl parameter can never be bound to an
        // intrinsically unsized type (`str`, `[T]`, `dyn P`): `impl<T: Clone>
        // ToOwned for T` and `impl ToOwned for str` are disjoint (stock rule).
        for (side, implementation) in [(0u8, a), (1u8, b)] {
            for parameter in implementation.schema_env.ordered_params() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if implementation.schema_env.relaxed_sized(parameter) {
                    continue;
                }
                let mut bound = None;
                for binding in bindings.iter() {
                    if binding.side == side && binding.parameter == parameter {
                        bound = Some(binding.value);
                        break;
                    }
                }
                let Some(bound) = bound else {
                    continue;
                };
                let bound = self.dereference(bound, bindings, 0)?;
                if matches!(bound.ty, Ty::Str | Ty::Slice { .. } | Ty::Dyn(_)) {
                    return Ok(true);
                }
            }
        }
        for (side, implementation) in [(0, a), (1, b)] {
            for predicate in implementation.predicates.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                let Predicate::Implements(required) = predicate else {
                    continue;
                };
                if !self.goal_may_hold(required, side, bindings, 0, local)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// False only when `goal` (written with the parameters of `side`) has no
    /// possible proof for any instance of the current unifier.
    fn goal_may_hold<'a>(
        &'a self,
        goal: &'a TraitRef,
        side: u8,
        bindings: &mut Vec<Binding<'a>>,
        depth: usize,
        local: Option<CrateOrigin>,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if depth >= MAX_NESTING {
            return Ok(true);
        }
        let subject = Term {
            side,
            ty: &goal.self_ty,
        };
        if self.closure_trait_kind(goal.declaration)?.is_some() {
            if self.intrinsically_non_callable(subject, bindings, 0)? {
                return Ok(false);
            }
            // A struct, enum or reference gets a call capability only from a
            // written impl (builtin call impls cover closures, fn items and
            // fn pointers; `&F` forwards through the library impls). Decide
            // it by the ordinary impl enumeration below.
            let resolved = self.dereference(subject, bindings, 0)?;
            if !matches!(resolved.ty, Ty::Struct(_) | Ty::Enum(_) | Ty::Ref { .. }) {
                return Ok(true);
            }
        } else if self.catalog.declaration(goal.declaration).is_some() {
            return Ok(true);
        }
        if !self.decidable_subject(subject, bindings, 0)? {
            return Ok(true);
        }
        for argument in goal.args.iter() {
            let argument = Term { side, ty: argument };
            if !self.decidable_subject(argument, bindings, 0)? {
                return Ok(true);
            }
        }
        if let Some(local) = local
            && self
                .schema
                .crate_origin(self.info(goal.declaration)?.module)?
                != Some(local)
        {
            let mut has_local = self.local_subject(subject, bindings, local)?;
            for argument in goal.args.iter() {
                has_local |= self.local_subject(Term { side, ty: argument }, bindings, local)?;
            }
            if !has_local {
                // An upstream crate may implement its own trait for these
                // types later: the orphan rules leave that open. A local
                // nominal in Self or a trait argument closes that possibility.
                return Ok(true);
            }
        }
        let candidates = self
            .trait_impl_index
            .get(&goal.declaration)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let candidate_side = u8::try_from(depth + 2)
            .map_err(|_| FactsError::invalid(&self.budget, "impl overlap nesting overflow"))?;
        for &position in candidates {
            charged(&self.budget, self.budget.charge_work(1))?;
            let candidate = &self.headers.impls[position];
            if candidate.trait_ref.args.len() != goal.args.len() {
                return Ok(true);
            }
            let head = Term {
                side: candidate_side,
                ty: &candidate.trait_ref.self_ty,
            };
            // Distinct concrete outer constructors never unify, whatever
            // (possibly non-structural) arguments they carry.
            if self.outer_constructors_differ(head, subject, bindings)? {
                continue;
            }
            // A head that hides a projection may equal the subject only after
            // normalization: it cannot be excluded structurally.
            if !self.structural(head, bindings, 0)? {
                return Ok(true);
            }
            let mark = bindings.len();
            let mut applies = self.unify(head, subject, bindings, 0)?;
            for (left, right) in candidate.trait_ref.args.iter().zip(goal.args.iter()) {
                if !applies {
                    break;
                }
                let left = Term {
                    side: candidate_side,
                    ty: left,
                };
                if !self.structural(left, bindings, 0)? {
                    bindings.truncate(mark);
                    return Ok(true);
                }
                applies = self.unify(left, Term { side, ty: right }, bindings, 0)?;
            }
            let mut refuted = !applies;
            if applies {
                for predicate in candidate.predicates.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if let Predicate::Implements(required) = predicate
                        && !self.goal_may_hold(
                            required,
                            candidate_side,
                            bindings,
                            depth + 1,
                            local,
                        )?
                    {
                        refuted = true;
                        break;
                    }
                }
            }
            bindings.truncate(mark);
            if !refuted {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// These constructors cannot acquire a builtin Fn/FnMut/FnOnce call
    /// implementation. References only forward a referent's call capability.
    /// In particular this is not absence-of-impl reasoning for a nominal,
    /// parameter, projection, opaque type, function pointer or closure.
    fn intrinsically_non_callable<'a>(
        &self,
        term: Term<'a>,
        bindings: &[Binding<'a>],
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let term = self.dereference(term, bindings, depth)?;
        match term.ty {
            Ty::Ref { referent, .. } => self.intrinsically_non_callable(
                Term {
                    side: term.side,
                    ty: referent,
                },
                bindings,
                depth + 1,
            ),
            Ty::Int(_)
            | Ty::Char
            | Ty::Str
            | Ty::Float(_)
            | Ty::Bool
            | Ty::Unit
            | Ty::Tuple(_)
            | Ty::Array { .. }
            | Ty::Slice { .. }
            | Ty::DependentArray { .. }
            | Ty::RawPtr { .. }
            | Ty::Never => Ok(true),
            _ => Ok(false),
        }
    }

    /// `&T`, `&mut T` and `Box<T>` are local to whichever crate owns `T`.
    fn fundamental_referent<'a>(&self, term: Term<'a>) -> Result<Option<Term<'a>>> {
        let inner = match term.ty {
            Ty::Ref { referent, .. } => referent.as_ref(),
            Ty::Struct(nominal)
                if self
                    .schema
                    .native_box_source(nominal.def().source_decl)?
                    .is_some() =>
            {
                match nominal.def().type_args.first() {
                    Some(argument) => argument,
                    None => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(Term {
            side: term.side,
            ty: inner,
        }))
    }

    /// True only when both terms resolve to concrete type constructors that
    /// can never be equal (`Box<dyn E>` vs `u8`). Parameters, projections,
    /// opaque, object and callable types are never called different.
    fn outer_constructors_differ<'a>(
        &self,
        left: Term<'a>,
        right: Term<'a>,
        bindings: &[Binding<'a>],
    ) -> Result<bool> {
        charged(&self.budget, self.budget.charge_work(1))?;
        let left = self.dereference(left, bindings, 0)?;
        let right = self.dereference(right, bindings, 0)?;
        fn class(ty: &Ty) -> Option<u8> {
            Some(match ty {
                Ty::Int(_) => 0,
                Ty::Char => 1,
                Ty::Str => 2,
                Ty::Float(_) => 3,
                Ty::Bool => 4,
                Ty::Unit => 5,
                Ty::Struct(_) => 6,
                Ty::Enum(_) => 7,
                Ty::Tuple(_) => 8,
                Ty::Array { .. } | Ty::DependentArray { .. } => 9,
                Ty::Slice { .. } => 10,
                Ty::Ref { .. } => 11,
                Ty::RawPtr { .. } => 12,
                Ty::Never => 13,
                _ => return None,
            })
        }
        let (Some(a), Some(b)) = (class(left.ty), class(right.ty)) else {
            return Ok(false);
        };
        if a != b {
            return Ok(true);
        }
        Ok(match (left.ty, right.ty) {
            (Ty::Int(x), Ty::Int(y)) => x != y,
            (Ty::Float(x), Ty::Float(y)) => x != y,
            (Ty::Struct(x), Ty::Struct(y)) => x.def().source_decl != y.def().source_decl,
            (Ty::Enum(x), Ty::Enum(y)) => x.def().source_decl != y.def().source_decl,
            (Ty::Tuple(x), Ty::Tuple(y)) => x.len() != y.len(),
            (Ty::Ref { kind: x, .. }, Ty::Ref { kind: y, .. }) => x != y,
            _ => false,
        })
    }

    /// A subject whose trait memberships are all given by implementation
    /// headers: structural, and not a type some downstream crate could own.
    fn decidable_subject<'a>(
        &self,
        term: Term<'a>,
        bindings: &[Binding<'a>],
        depth: usize,
    ) -> Result<bool> {
        let mut term = self.dereference(term, bindings, depth)?;
        let mut hops = depth;
        while let Some(inner) = self.fundamental_referent(term)? {
            hops += 1;
            charged(&self.budget, self.budget.check_depth(hops))?;
            term = self.dereference(inner, bindings, hops)?;
        }
        if matches!(term.ty, Ty::Param(_)) {
            return Ok(false);
        }
        // `Box<dyn P>` / `&dyn P` is not the object type: only a written impl
        // for the wrapper applies (builtin object impls cover `dyn P` alone),
        // and candidate enumeration keeps any impl whose bound stays
        // undecided. The object's own arguments must still be structural.
        if hops > depth
            && let Ty::Dyn(object) = term.ty
        {
            for argument in object
                .args
                .iter()
                .chain(object.associated_types.iter().map(|(_, ty)| ty))
            {
                let argument = Term {
                    side: term.side,
                    ty: argument,
                };
                if !self.structural(argument, bindings, hops + 1)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        self.structural(term, bindings, hops)
    }

    /// Only scalars, references, pointers, arrays, slices, tuples, nominals
    /// and parameters, at every depth. Everything else (projections, opaque
    /// and object types, callables, wrappers) may equal or implement more
    /// than its syntax shows.
    fn structural<'a>(
        &self,
        term: Term<'a>,
        bindings: &[Binding<'a>],
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let term = self.dereference(term, bindings, depth)?;
        let child = |ty| Term {
            side: term.side,
            ty,
        };
        match term.ty {
            Ty::Param(_) | Ty::Int(_) | Ty::Char | Ty::Str | Ty::Float(_) | Ty::Bool | Ty::Unit => {
                Ok(true)
            }
            Ty::Ref {
                referent: inner, ..
            }
            | Ty::RawPtr { pointee: inner, .. }
            | Ty::Array { element: inner, .. }
            | Ty::DependentArray { element: inner, .. }
            | Ty::Slice { element: inner } => self.structural(child(inner), bindings, depth + 1),
            Ty::Tuple(_) | Ty::Struct(_) | Ty::Enum(_) => {
                for argument in type_arguments(term.ty) {
                    if !self.structural(child(argument), bindings, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Orphan-rule locality: a nominal declared by the crate being checked,
    /// possibly behind `&`, `&mut` or `Box`.
    fn local_subject<'a>(
        &self,
        term: Term<'a>,
        bindings: &[Binding<'a>],
        local: CrateOrigin,
    ) -> Result<bool> {
        let mut term = self.dereference(term, bindings, 0)?;
        let mut hops = 0;
        while let Some(inner) = self.fundamental_referent(term)? {
            hops += 1;
            charged(&self.budget, self.budget.check_depth(hops))?;
            term = self.dereference(inner, bindings, hops)?;
        }
        let module = match term.ty {
            Ty::Struct(nominal) => nominal.def().module,
            Ty::Enum(nominal) => nominal.def().module,
            _ => return Ok(false),
        };
        Ok(self.schema.crate_origin(module)? == Some(local))
    }
}
