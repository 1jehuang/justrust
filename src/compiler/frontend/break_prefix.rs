use super::*;

impl<'input> Parser<'input, '_> {
    /// A break value recursively parses another expression. Keep that recursion
    /// out of the ordinary prefix frame and its unrelated literal/borrow scratch.
    #[inline(never)]
    pub(super) fn break_prefix(&mut self) -> Result<Expr, ParseError> {
        let label = if self.token().is_lifetime() {
            let label = self.input.mint_name(&self.token(), self.names)?;
            self.cursor.bump();
            Some(label)
        } else {
            None
        };
        let value = if matches!(self.token().text, ";" | "}" | ")" | "]" | "," | "") {
            None
        } else {
            Some(Box::new(self.expr(0)?))
        };
        Ok(Expr::Break { label, value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn break_value_depth_and_original_target_authority_are_preserved() {
        let within = format!("fn main(){{{}0;}}", "break ".repeat(120));
        parse(&within).unwrap();
        let beyond = format!("fn main(){{{}0;}}", "break ".repeat(140));
        assert!(parse(&beyond).unwrap_err().contains("nesting"));

        let program =
            parse("fn value()->i32{7}fn main(){let result='outer:loop{break 'outer value();};}")
                .unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let PublishedCallableBody::Source(body) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected original source");
        };
        body.validate_source(body.source(), body.facts(), &budget)
            .unwrap();
        let mut forged = body.source().as_ref().clone();
        let expression = forged
            .exprs
            .iter_mut()
            .find(|expr| matches!(expr.kind, ir::ExprKind::Break { .. }))
            .unwrap();
        let ir::ExprKind::Break { target, .. } = &mut expression.kind else {
            unreachable!();
        };
        *target = ir::LoopId(999);
        assert!(
            body.validate_source(&Arc::new(forged), body.facts(), &budget)
                .is_err()
        );
    }
}
