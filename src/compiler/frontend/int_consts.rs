//! Primitive integer associated constants `<int>::MAX`, `::MIN`, `::BITS`
//! as typed literals (`i32::MIN` is `-2147483648i32`), and the float
//! associated constants (`f64::MAX`, `f32::EPSILON`, `f64::INFINITY`,
//! `f64::NAN`, ...) as typed literal expressions.
use super::*;

impl Parser<'_, '_> {
    /// `f32`/`f64` associated constant at the cursor, if any.
    fn float_const_expr(&mut self) -> Option<Expr> {
        let token = self.token();
        if !token.is_ident() || token.string {
            return None;
        }
        let ty = float_type(token.text)?;
        if !self.cursor.peek(1).is_some_and(|t| t.matches("::")) {
            return None;
        }
        let name = self.cursor.peek(2).map(|t| t.text)?;
        // A following `(`/`::` would be some other path form; leave it alone.
        if self
            .cursor
            .peek(3)
            .is_some_and(|t| t.matches("(") || t.matches("::"))
        {
            return None;
        }
        let single = ty == ir::FloatTy::F32;
        let float = |digits: &str| {
            Expr::Float(FloatLiteral {
                digits: digits.to_string(),
                suffix: Some(ty),
            })
        };
        let neg = |value: Expr| Expr::Unary {
            op: UnaryOp::Neg,
            value: Box::new(value),
            site: None,
        };
        let div = |left: Expr, right: Expr| Expr::Binary {
            op: BinaryOp::Div,
            left: Box::new(left),
            right: Box::new(right),
            site: None,
        };
        let int = |magnitude: u128, suffix| {
            Expr::Integer(IntegerLiteral {
                magnitude,
                suffix: Some(suffix),
            })
        };
        let text = |f32_value: &'static str, f64_value: &'static str| {
            if single { f32_value } else { f64_value }
        };
        let pick = |f32_value: u128, f64_value: u128| if single { f32_value } else { f64_value };
        let max = text("3.4028235e38", "1.7976931348623157e308");
        let expr = match name {
            "MAX" => float(max),
            "MIN" => neg(float(max)),
            "MIN_POSITIVE" => float(text("1.1754944e-38", "2.2250738585072014e-308")),
            "EPSILON" => float(text("1.1920929e-7", "2.220446049250313e-16")),
            "INFINITY" => div(float("1.0"), float("0.0")),
            "NEG_INFINITY" => div(neg(float("1.0")), float("0.0")),
            // The invalid quotient is the negative default NaN on x86-64;
            // the constant is its positive counterpart.
            "NAN" => neg(div(float("0.0"), float("0.0"))),
            "RADIX" => int(2, ir::IntTy::U32),
            "MANTISSA_DIGITS" => int(pick(24, 53), ir::IntTy::U32),
            "DIGITS" => int(pick(6, 15), ir::IntTy::U32),
            "MAX_EXP" => int(pick(128, 1024), ir::IntTy::I32),
            "MAX_10_EXP" => int(pick(38, 308), ir::IntTy::I32),
            "MIN_EXP" => neg(int(pick(125, 1021), ir::IntTy::I32)),
            "MIN_10_EXP" => neg(int(pick(37, 307), ir::IntTy::I32)),
            _ => return None,
        };
        self.cursor.bump();
        self.cursor.bump();
        self.cursor.bump();
        Some(expr)
    }

    pub(super) fn int_const_expr(&mut self) -> Result<Option<Expr>, ParseError> {
        if let Some(expr) = self.float_const_expr() {
            return Ok(Some(expr));
        }
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
