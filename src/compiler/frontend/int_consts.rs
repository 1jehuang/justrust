//! Primitive integer associated constants `<int>::MAX`, `::MIN`, `::BITS`
//! as typed literals (`i32::MIN` is `-2147483648i32`).
use super::*;

impl Parser<'_, '_> {
    pub(super) fn int_const_expr(&mut self) -> Result<Option<Expr>, ParseError> {
        let Some(ty) = (self.token().is_ident() && !self.token().string)
            .then(|| integer_type(self.token().text))
            .flatten()
        else {
            return Ok(None);
        };
        if !self.cursor.peek(1).is_some_and(|t| t.matches("::")) {
            return Ok(None);
        }
        let Some(name) = self.cursor.peek(2).map(|t| t.text) else {
            return Ok(None);
        };
        if !matches!(name, "MAX" | "MIN" | "BITS") {
            return Ok(None);
        }
        // A following `(`/`::` would be some other path form; leave it alone.
        if self
            .cursor
            .peek(3)
            .is_some_and(|t| t.matches("(") || t.matches("::"))
        {
            return Ok(None);
        }
        self.cursor.bump();
        self.cursor.bump();
        self.cursor.bump();
        let bits = ty.bits();
        let signed = ty.is_signed();
        let literal = |magnitude: u128, suffix| {
            Expr::Integer(IntegerLiteral {
                magnitude,
                suffix: Some(suffix),
            })
        };
        Ok(Some(match name {
            "BITS" => literal(u128::from(bits), ir::IntTy::U32),
            "MAX" if signed => literal((1u128 << (bits - 1)) - 1, ty),
            "MAX" => literal(
                if bits == 128 {
                    u128::MAX
                } else {
                    (1u128 << bits) - 1
                },
                ty,
            ),
            _ if signed => Expr::Unary {
                op: UnaryOp::Neg,
                value: Box::new(literal(1u128 << (bits - 1), ty)),
                site: None,
            },
            _ => literal(0, ty),
        }))
    }
}
