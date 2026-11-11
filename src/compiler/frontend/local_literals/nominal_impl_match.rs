//! Conservative structural compatibility for unique nominal impl equations.
//! This never binds a term or proves that an implementation applies.
use super::*;

impl Solver<'_, '_, '_> {
    /// Match the whole impl head without changing the inference graph. Repeated
    /// parameters are correlated across Self and trait arguments, so a blanket
    /// `impl<T> From<T> for T` cannot compete with an array-to-map conversion
    /// merely because each occurrence of T matches in isolation.
    pub(super) fn impl_head_compatible(
        &self,
        receiver: Term,
        arguments: &[Term],
        header: &tr::ImplHeader,
    ) -> Result<bool, String> {
        if arguments.len() != header.trait_ref.args.len() {
            return Ok(false);
        }
        let mut bindings = HashMap::new();
        for (actual, pattern) in std::iter::once((receiver, &header.trait_ref.self_ty))
            .chain(arguments.iter().copied().zip(header.trait_ref.args.iter()))
        {
            if !self.correlated_pattern_compatible(
                actual,
                pattern,
                &header.schema_env,
                &mut bindings,
                0,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn correlated_pattern_compatible(
        &self,
        actual: Term,
        pattern: &ir::Ty,
        env: &ir::GenericEnv,
        bindings: &mut HashMap<ir::ParamId, Term>,
        depth: usize,
    ) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        if let ir::Ty::Param(parameter) = pattern
            && env.contains(*parameter)
        {
            if let Some(previous) = bindings.get(parameter) {
                return self.impl_terms_compatible(*previous, actual, depth + 1);
            }
            self.checker.infer_storage(1)?;
            bindings.insert(*parameter, actual);
            return Ok(true);
        }
        if !self.impl_pattern_compatible(actual, pattern, env, depth + 1)? {
            return Ok(false);
        }
        let pairs = match (self.domain(actual), pattern) {
            (Domain::Nominal(_, actuals), _) => {
                let Some((_, patterns)) = nominal_parts(pattern) else {
                    return Ok(true);
                };
                actuals.into_iter().zip(patterns).collect::<Vec<_>>()
            }
            (Domain::Tuple(actuals), ir::Ty::Tuple(patterns)) => {
                actuals.into_iter().zip(patterns.iter()).collect()
            }
            (Domain::Ref(_, actual), ir::Ty::Ref { referent, .. }) => {
                vec![(actual, referent.as_ref())]
            }
            (Domain::RawPtr(_, actual), ir::Ty::RawPtr { pointee, .. }) => {
                vec![(actual, pointee.as_ref())]
            }
            (
                Domain::Array(actual, _) | Domain::DependentArray(actual, _),
                ir::Ty::Array { element, .. } | ir::Ty::DependentArray { element, .. },
            ) => vec![(actual, element.as_ref())],
            (Domain::Slice(actual), ir::Ty::Slice { element }) => vec![(actual, element.as_ref())],
            (Domain::PhantomData(_, actual), ir::Ty::PhantomData(pattern)) => {
                vec![(actual, pattern.argument())]
            }
            (Domain::ManuallyDrop(_, actual), ir::Ty::ManuallyDrop(pattern)) => {
                vec![(actual, pattern.argument())]
            }
            (Domain::Cell(_, actual), ir::Ty::Cell(pattern)) => vec![(actual, pattern.argument())],
            _ => return Ok(true),
        };
        for (actual, pattern) in pairs {
            if !self.correlated_pattern_compatible(actual, pattern, env, bindings, depth + 1)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Open terms retain all possible competitors. Only incompatible known
    /// constructors eliminate a head, never defaulting or a speculative proof.
    fn impl_terms_compatible(&self, left: Term, right: Term, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        if self.root(left) == self.root(right) {
            return Ok(true);
        }
        let pairs = match (self.domain(left), self.domain(right)) {
            (
                Domain::Unknown
                | Domain::AssociatedProjection(_)
                | Domain::Callable(_)
                | Domain::Opaque(..),
                _,
            )
            | (
                _,
                Domain::Unknown
                | Domain::AssociatedProjection(_)
                | Domain::Callable(_)
                | Domain::Opaque(..),
            ) => return Ok(true),
            (Domain::Integer, Domain::Integer | Domain::Fixed(ir::Ty::Int(_)))
            | (Domain::Fixed(ir::Ty::Int(_)), Domain::Integer)
            | (Domain::Float, Domain::Float | Domain::Fixed(ir::Ty::Float(_)))
            | (Domain::Fixed(ir::Ty::Float(_)), Domain::Float) => return Ok(true),
            (Domain::Fixed(left), Domain::Fixed(right)) => return Ok(left == right),
            (Domain::Nominal(left, a), Domain::Nominal(right, b))
                if left == right && a.len() == b.len() =>
            {
                a.into_iter().zip(b).collect::<Vec<_>>()
            }
            (Domain::Tuple(a), Domain::Tuple(b)) if a.len() == b.len() => {
                a.into_iter().zip(b).collect()
            }
            (Domain::Ref(a, x), Domain::Ref(b, y)) if a == b => vec![(x, y)],
            (Domain::RawPtr(a, x), Domain::RawPtr(b, y)) if a == b => vec![(x, y)],
            (Domain::Array(x, n), Domain::Array(y, m)) if n == m => vec![(x, y)],
            (Domain::DependentArray(x, n), Domain::DependentArray(y, m)) => vec![(x, y), (n, m)],
            (Domain::Array(x, _), Domain::DependentArray(y, _))
            | (Domain::DependentArray(x, _), Domain::Array(y, _)) => vec![(x, y)],
            (Domain::Slice(x), Domain::Slice(y)) => vec![(x, y)],
            (Domain::PhantomData(a, x), Domain::PhantomData(b, y))
            | (Domain::ManuallyDrop(a, x), Domain::ManuallyDrop(b, y))
            | (Domain::Cell(a, x), Domain::Cell(b, y))
                if a == b =>
            {
                vec![(x, y)]
            }
            (Domain::FnPointer(..), _) | (_, Domain::FnPointer(..)) => return Ok(true),
            _ => return Ok(false),
        };
        for (left, right) in pairs {
            if !self.impl_terms_compatible(left, right, depth + 1)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn impl_pattern_compatible(
        &self,
        actual: Term,
        pattern: &ir::Ty,
        env: &ir::GenericEnv,
        depth: usize,
    ) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        if matches!(pattern, ir::Ty::Param(parameter) if env.contains(*parameter))
            || matches!(pattern, ir::Ty::AssociatedType(_) | ir::Ty::Opaque(_))
        {
            return Ok(true);
        }
        Ok(match (self.domain(actual), pattern) {
            // Keep all candidates at an unresolved frontier. Inference may
            // use equations only if precisely one complete head remains.
            (
                Domain::Unknown
                | Domain::AssociatedProjection(_)
                | Domain::Callable(_)
                | Domain::Opaque(..),
                _,
            ) => true,
            (Domain::Integer, ir::Ty::Int(_)) | (Domain::Float, ir::Ty::Float(_)) => true,
            (Domain::Fixed(actual), _) => actual == *pattern,
            (Domain::Nominal(owner, actuals), _) => {
                let Some((declaration, patterns)) = nominal_parts(pattern) else {
                    return Ok(false);
                };
                if owner != declaration || actuals.len() != patterns.len() {
                    return Ok(false);
                }
                for (actual, pattern) in actuals.iter().zip(patterns) {
                    if !self.impl_pattern_compatible(*actual, pattern, env, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            (Domain::Tuple(actuals), ir::Ty::Tuple(patterns)) => {
                if actuals.len() != patterns.len() {
                    return Ok(false);
                }
                for (actual, pattern) in actuals.iter().zip(patterns.iter()) {
                    if !self.impl_pattern_compatible(*actual, pattern, env, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            (Domain::Ref(actual_kind, actual), ir::Ty::Ref { kind, referent }) => {
                actual_kind == *kind
                    && self.impl_pattern_compatible(actual, referent, env, depth + 1)?
            }
            (Domain::RawPtr(actual_kind, actual), ir::Ty::RawPtr { kind, pointee }) => {
                actual_kind == *kind
                    && self.impl_pattern_compatible(actual, pointee, env, depth + 1)?
            }
            (Domain::Array(actual, actual_len), ir::Ty::Array { element, len }) => {
                actual_len == *len
                    && self.impl_pattern_compatible(actual, element, env, depth + 1)?
            }
            (Domain::Array(..), ir::Ty::DependentArray { .. }) => true,
            (Domain::DependentArray(actual, _), ir::Ty::DependentArray { element, .. })
            | (Domain::DependentArray(actual, _), ir::Ty::Array { element, .. }) => {
                self.impl_pattern_compatible(actual, element, env, depth + 1)?
            }
            (Domain::Slice(actual), ir::Ty::Slice { element }) => {
                self.impl_pattern_compatible(actual, element, env, depth + 1)?
            }
            (Domain::PhantomData(owner, actual), ir::Ty::PhantomData(pattern)) => {
                owner == pattern.declaration()
                    && self.impl_pattern_compatible(actual, pattern.argument(), env, depth + 1)?
            }
            (Domain::ManuallyDrop(owner, actual), ir::Ty::ManuallyDrop(pattern)) => {
                owner == pattern.declaration()
                    && self.impl_pattern_compatible(actual, pattern.argument(), env, depth + 1)?
            }
            (Domain::Cell(owner, actual), ir::Ty::Cell(pattern)) => {
                owner == pattern.declaration()
                    && self.impl_pattern_compatible(actual, pattern.argument(), env, depth + 1)?
            }
            // Function pointers remain unsupported for execution, and an open
            // projected pattern must not eliminate a potential competitor.
            (Domain::FnPointer(..), _) => true,
            _ => false,
        })
    }
}
