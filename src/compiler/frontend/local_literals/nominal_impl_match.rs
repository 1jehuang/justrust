//! Conservative structural compatibility for unique nominal impl equations.
//! This never binds a term or proves that an implementation applies.
use super::*;

impl Solver<'_, '_, '_> {
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
