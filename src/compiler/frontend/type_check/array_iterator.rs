//! Immediate-for array iterators retain their original method occurrence while
//! reusing the ordinary shared element-place loop and borrow checker.
use super::*;

impl Checker<'_> {
    pub(super) fn builtin_array_iter_receiver<'a>(
        &self,
        expression: &'a Expr,
    ) -> Result<Option<&'a Expr>, String> {
        let Some((receiver, _)) = iterator_source::array_iter_candidate(expression, self.names()?)?
        else {
            return Ok(None);
        };
        if !self
            .method_receiver_hint(receiver)?
            .is_some_and(|ty| array_len_receiver::receiver_is_array(&ty))
            || self.array_len_shadowed_by_method(expression)?
        {
            return Ok(None);
        }
        Ok(Some(receiver))
    }

    pub(super) fn for_array_iter_value(
        &mut self,
        source: &ForSource,
        expression: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        let Some(receiver) = self.builtin_array_iter_receiver(expression)? else {
            return Ok(None);
        };
        let value = self
            .array_len_receiver(receiver)?
            .ok_or("array iterator receiver lost its checked array type")?;
        if let Some(cap) = self.canonical {
            cap.record_array_iter(source, value, &self.exprs)?;
        }
        Ok(Some(value))
    }
}
