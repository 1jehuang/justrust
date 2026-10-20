//! General range expressions in every expression position:
//! `a..b`, `a..`, `..b`, `..`, `a..=b`, `..=b`.
//!
//! Each form desugars to a struct literal of the matching prelude type in
//! `std::ops` (`Range`, `RangeFrom`, `RangeTo`, `RangeFull`,
//! `RangeInclusive`, `RangeToInclusive`). Ranges bind looser than every
//! binary operator, so they are only recognised at minimum precedence 0;
//! positions that own `..` themselves (slice indexing, `for` ranges) parse
//! their operands at precedence 1.
use super::*;

impl Parser<'_, '_> {
    /// True when the token after `..` cannot start a range end bound.
    fn range_end_absent(&self) -> bool {
        let t = self.token();
        !t.string
            && (matches!(t.text, ")" | "]" | "," | ";" | "}" | "=>" | "=")
                || (t.text == "{" && !self.allow_struct))
    }

    /// At `..` / `..=` with an already parsed optional start: build the range.
    pub(super) fn range_expression(&mut self, start: Option<Expr>) -> Result<Expr, ParseError> {
        let site = panic_callsite(self.names.session(), self.token().span)
            .map_err(native_adapter::transport_error)?;
        let inclusive = self.eat("..=");
        if !inclusive {
            self.expect("..")?;
        }
        let end = if self.range_end_absent() {
            if inclusive {
                return Err(self.error("inclusive range requires an end bound"));
            }
            None
        } else {
            Some(self.expr(1)?)
        };
        let ty = match (&start, &end, inclusive) {
            (Some(_), Some(_), false) => "Range",
            (Some(_), Some(_), true) => "RangeInclusive",
            (Some(_), None, _) => "RangeFrom",
            (None, Some(_), false) => "RangeTo",
            (None, Some(_), true) => "RangeToInclusive",
            (None, None, _) => "RangeFull",
        };
        let mut idents = Vec::new();
        for segment in ["std", "ops", ty] {
            idents.push(
                self.names
                    .intern_root(segment, site)
                    .map_err(native_adapter::name_error)?,
            );
        }
        if inclusive && let Some(start) = start {
            idents.push(
                self.names
                    .intern_root("new", site)
                    .map_err(native_adapter::name_error)?,
            );
            let name = AstPath::new(self.names, PathAnchor::Relative, &idents)
                .map_err(native_adapter::name_error)?;
            return Ok(Expr::Call {
                name,
                args: vec![
                    start,
                    end.ok_or_else(|| self.error("inclusive range requires an end"))?,
                ],
            });
        }
        let name = AstPath::new(self.names, PathAnchor::Relative, &idents)
            .map_err(native_adapter::name_error)?;
        let mut fields = Vec::new();
        for (field, value) in [("start", start), ("end", end)] {
            if let Some(value) = value {
                let field = self
                    .names
                    .intern_root(field, site)
                    .map_err(native_adapter::name_error)?;
                fields.push((field, value));
            }
        }
        let expr = Expr::Struct { name, fields };
        validate_depth(DepthNode::Expr(&expr)).map_err(|e| self.depth_error(&e))?;
        Ok(expr)
    }

    /// `nested_expr` that leaves a top-level `..` for the caller.
    pub(super) fn nested_expr_no_range(&mut self) -> Result<Expr, ParseError> {
        let previous = self.allow_struct;
        let previous_purpose = self.condition_purpose;
        self.allow_struct = true;
        self.condition_purpose = ConditionPurpose::Ordinary;
        let result = self.expr(1);
        self.allow_struct = previous;
        self.condition_purpose = previous_purpose;
        result
    }
}
