//! Assignment expressions use the same place operations as assignment statements.
//! Their value is unit, including in match arms and parenthesized expressions.
use super::{
    BinaryOp, Block, Expr, ParseError, Parser, Stmt, call_rooted_target, compound_operator,
    nested_runtime_target,
};

impl<'input, 'names> Parser<'input, 'names> {
    pub(super) fn assignment_expression(&mut self, target: Expr) -> Result<Expr, ParseError> {
        let op = compound_operator(self.token());
        if op.is_none() && !self.is("=") {
            return Ok(target);
        }
        self.cursor.bump();
        let value = self.expr(0)?;
        let statement = self.assignment_operation(target, op, value)?;
        Ok(Expr::Block(Block {
            statements: vec![statement],
            tail: None,
        }))
    }

    fn assignment_operation(
        &mut self,
        target: Expr,
        op: Option<BinaryOp>,
        value: Expr,
    ) -> Result<Stmt, ParseError> {
        let (target, value) = match call_rooted_target(target, op, value) {
            Ok(statement) => return Ok(statement),
            Err(unchanged) => *unchanged,
        };
        if nested_runtime_target(&target) {
            return Ok(Stmt::AssignNestedIndex { target, op, value });
        }
        if let Expr::DynamicIndex { base, index } = target {
            let array = self.assignment_place(*base)?;
            return Ok(match op {
                Some(op) => Stmt::AssignIndexOp {
                    array,
                    index: *index,
                    op,
                    value,
                },
                None => Stmt::AssignIndex {
                    array,
                    index: *index,
                    value,
                },
            });
        }
        if matches!(
            target,
            Expr::Integer(_)
                | Expr::Float(_)
                | Expr::Bool(_)
                | Expr::Char(_)
                | Expr::Str(_)
                | Expr::ByteStr(_)
                | Expr::Binary { .. }
                | Expr::Unary { .. }
        ) {
            return Err(self.error("invalid assignment target: expected a place expression"));
        }
        let place = self.assignment_place(target)?;
        Ok(match op {
            Some(op) => Stmt::AssignOp { place, op, value },
            None if place.fields.is_empty() => Stmt::Assign {
                name: place.local,
                value,
            },
            None => Stmt::AssignPlace { place, value },
        })
    }
}
