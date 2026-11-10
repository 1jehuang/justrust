//! Builtin array operations that stock provides through const-generic impls.
//! The length is concrete on every `Ty::Array`, so each use is checked here
//! against that length and lowered to the matching slice operation.
use super::super::operators::First;
use super::*;

fn literal_len(e: &Expr) -> Option<usize> {
    match e {
        Expr::Array(elements) => Some(elements.len()),
        Expr::Repeat { len, .. } => Some(*len),
        _ => None,
    }
}

fn shared_slice(element: &ir::Ty) -> ir::Ty {
    ir::Ty::Ref {
        kind: ir::BorrowKind::Shared,
        referent: Arc::new(ir::Ty::Slice {
            element: Arc::new(element.clone()),
        }),
    }
}

impl Checker<'_> {
    fn array_operand_hint(&self, e: &Expr) -> Result<Option<(ir::Ty, usize)>, String> {
        if let Some(ir::Ty::Array { element, len }) = self.method_receiver_hint(e)? {
            return Ok(Some((element.as_ref().clone(), len)));
        }
        // `[1.0, f64::NAN]`: the literal as a whole has no standalone type,
        // but one element that does fixes the element type.
        let elements = match e {
            Expr::Array(elements) => elements.as_slice(),
            Expr::Repeat { value, .. } => std::slice::from_ref(value.as_ref()),
            _ => return Ok(None),
        };
        let Some(len) = literal_len(e) else {
            return Ok(None);
        };
        for (index, element) in elements.iter().enumerate() {
            self.infer_work(index)?;
            if matches!(element, Expr::Integer(_) | Expr::Float(_)) {
                continue;
            }
            if let Some(ty) = self.method_receiver_hint(element)? {
                return Ok(Some((ty, len)));
            }
        }
        Ok(None)
    }

    /// `a == b` / `a != b` with both operands `[T; N]` and `[U; N]`: stock's
    /// `impl PartialEq<[U; N]> for [T; N]` compares the two as slices, so
    /// this is `<[T] as PartialEq<[U]>>::eq(&a[..], &b[..])` once the equal
    /// lengths are proven. Each operand is evaluated once, left first.
    pub(in crate::compiler::frontend) fn array_equality(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<Option<ir::ExprId>, String> {
        let method = match op {
            BinaryOp::Eq => "eq",
            BinaryOp::Ne => "ne",
            _ => return Ok(None),
        };
        // A literal operand whose elements have no standalone type
        // (`a == [1.0, f64::NAN]`) is typed by the other operand.
        let (mut l, mut r) = match (
            self.array_operand_hint(left)?,
            self.array_operand_hint(right)?,
        ) {
            (Some(l), Some(r)) => (l, r),
            (Some(l), None) => match literal_len(right) {
                Some(len) => (l.clone(), (l.0, len)),
                None => return Ok(None),
            },
            (None, Some(r)) => match literal_len(left) {
                Some(len) => ((r.0.clone(), len), r),
                None => return Ok(None),
            },
            (None, None) => return Ok(None),
        };
        // An array literal takes its element type from the other operand, as
        // the selected impl would give it, not from literal fallback.
        match (literal_len(left), literal_len(right)) {
            (_, Some(_)) => r.0 = l.0.clone(),
            (Some(_), None) => l.0 = r.0.clone(),
            (None, None) => {}
        }
        if l.1 != r.1 {
            return Err(format!(
                "E0277: can't compare an array of {} elements with an array of {} elements",
                l.1, r.1
            ));
        }
        if matches!(l.0, ir::Ty::Array { .. }) || matches!(r.0, ir::Ty::Array { .. }) {
            return Err("unsupported: comparing arrays whose elements are arrays".into());
        }
        let first = if let Ok(place) = static_source_place(left)
            && place_expression(left)
        {
            let (place, _, _) = self.checked_place(&place)?;
            self.borrow_expression(ir::BorrowKind::Shared, place)?
        } else {
            self.borrow_value(ir::BorrowKind::Shared, left, Some(&shared_slice(&l.0)))?
        };
        let first = self.array_reference_as_slice(first)?;
        let slice = |element: ir::Ty| ir::Ty::Slice {
            element: Arc::new(element),
        };
        self.operator_call(
            ("cmp", "PartialEq", method),
            slice(l.0),
            Some(slice(r.0)),
            First::Lowered(first),
            std::slice::from_ref(right),
        )
        .map(Some)
    }
}
