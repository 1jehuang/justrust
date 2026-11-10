//! Unsuffixed literals nested in an index operand take their type from the
//! receiver's `Index` impl: `grid[(1, 0)]` with `impl Index<(usize, usize)>`
//! types both literals `usize`, as stock does, instead of defaulting them to
//! `i32` and then failing selection. Only a unique shape-compatible formal
//! contributes. It is an expectation, never call authority.
use super::*;

impl Solver<'_, '_, '_> {
    /// An open numeric literal strictly inside an aggregate operand. A bare
    /// literal or `&literal` operand has its own direct hints.
    fn nested_open_numeric(&self, term: Term, nested: bool, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        Ok(match self.domain(term) {
            Domain::Integer | Domain::Float => nested,
            Domain::Ref(_, inner) => self.nested_open_numeric(inner, nested, depth + 1)?,
            Domain::Array(element, _)
            | Domain::DependentArray(element, _)
            | Domain::Slice(element) => self.nested_open_numeric(element, true, depth + 1)?,
            Domain::Tuple(fields) | Domain::Nominal(_, fields) => {
                for field in fields {
                    if self.nested_open_numeric(field, true, depth + 1)? {
                        return Ok(true);
                    }
                }
                false
            }
            _ => false,
        })
    }

    /// Could this operand term still become `formal`? Open terms fit anything
    /// of their literal class; settled structure must agree exactly.
    fn operand_fits(&self, term: Term, formal: &ir::Ty, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        Ok(match (self.domain(term), formal) {
            (Domain::Unknown, _) => true,
            (Domain::Integer, ir::Ty::Int(_)) | (Domain::Float, ir::Ty::Float(_)) => true,
            (Domain::Integer | Domain::Float, _) => false,
            (Domain::Tuple(fields), ir::Ty::Tuple(formals)) => {
                if fields.len() != formals.len() {
                    return Ok(false);
                }
                for (field, formal) in fields.iter().zip(formals.iter()) {
                    if !self.operand_fits(*field, formal, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            (
                Domain::Ref(kind, inner),
                ir::Ty::Ref {
                    kind: formal,
                    referent,
                },
            ) => kind == *formal && self.operand_fits(inner, referent, depth + 1)?,
            (
                Domain::Array(element, len),
                ir::Ty::Array {
                    element: formal,
                    len: n,
                },
            ) => len == *n && self.operand_fits(element, formal, depth + 1)?,
            (Domain::Slice(element), ir::Ty::Slice { element: formal }) => {
                self.operand_fits(element, formal, depth + 1)?
            }
            (Domain::Nominal(declaration, actuals), formal) => {
                let Some((owner, formals)) = nominal_parts(formal) else {
                    return Ok(false);
                };
                if owner != declaration || formals.len() != actuals.len() {
                    return Ok(false);
                }
                for (actual, formal) in actuals.iter().zip(formals) {
                    if !self.operand_fits(*actual, formal, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            _ => match self.resolve(term, depth + 1)? {
                Some(actual) => actual == *formal,
                None => true,
            },
        })
    }

    /// Types literals nested in `operand` from the unique fitting `Index`
    /// formal of the first autoderef step of `receiver` that has any.
    pub(super) fn hint_index_operand(
        &mut self,
        receiver: &ir::Ty,
        declaration: ir::DeclId,
        method: &AstIdent,
        operand: Term,
    ) -> Result<(), String> {
        if !self.nested_open_numeric(operand, false, 0)? {
            return Ok(());
        }
        let mut step = receiver.clone();
        for _ in 0..8 {
            let Some(formals) = self
                .checker
                .operator_rhs_formals(&step, declaration, method)?
            else {
                return Ok(());
            };
            if !formals.is_empty() {
                let mut fitting = None;
                for formal in &formals {
                    if self.operand_fits(operand, formal, 0)? && fitting.replace(formal).is_some() {
                        return Ok(());
                    }
                }
                if let Some(formal) = fitting {
                    let formal = self.fixed(formal.clone())?;
                    self.coerce(operand, formal)?;
                }
                return Ok(());
            }
            step = match &step {
                ir::Ty::Ref { referent, .. } => referent.as_ref().clone(),
                other => match self.checker.deref_target_hint(other)? {
                    Some(next) => next,
                    None => return Ok(()),
                },
            };
        }
        Ok(())
    }
}
