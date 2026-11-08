//! Reference-returning assignment operands participate in body-local inference.
//! These are equations only: no carrier is lowered and no loan is acquired.
use super::*;

impl<'a: 's, 'b: 's, 's> Solver<'a, 'b, 's> {
    pub(super) fn reference_assignment(
        &mut self,
        statement: &'s Stmt,
        depth: usize,
    ) -> Result<(), String> {
        let Stmt::AssignDeref {
            carrier,
            fields,
            index,
            op,
            value,
            ..
        } = statement
        else {
            return Err("reference assignment inference requires its source statement".into());
        };
        // Match assignment's source order while collecting constraints. The
        // checked assignment still emits RHS, carrier and index exactly once.
        let value = self.expr(value, depth + 1)?;
        let mut target = self.expr(carrier, depth + 1)?;
        for field in fields {
            target = if matches!(field, FieldSelector::ConstantIndex(_)) {
                let index = self.node(Domain::Integer)?;
                self.operator_index(target, index, true)?
            } else if matches!(field, FieldSelector::Deref)
                && !matches!(
                    self.domain(target),
                    Domain::ManuallyDrop(..) | Domain::Ref(..) | Domain::RawPtr(..)
                )
            {
                self.operator_deref(target)?
            } else {
                self.field(target, field.clone())?
            };
        }
        if let Some(index) = index {
            let index = self.expr(index, depth + 1)?;
            target = self.operator_index(target, index, true)?;
        }
        match op {
            Some(op) => self.operator_assignment(*op, target, value),
            None => self.coerce(value, target),
        }
    }
}
