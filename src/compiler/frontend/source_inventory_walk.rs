use super::*;

impl SourceInventory {
    /// Walk transparent operand chains without retaining the large branch
    /// dispatch frame for every source occurrence. Keep occurrence paths and
    /// per-occurrence resource checks identical to recursive traversal.
    pub(super) fn expr(
        &mut self,
        mut expr: &Expr,
        node: &sr::SourceNode,
        mut application: Option<&GenericPath>,
        mut direct_let: Option<sr::SourceNode>,
        budget: &ir::PlanBudget,
        mut depth: usize,
    ) -> Result<(), FactsError> {
        let mut node = node.clone();
        loop {
            sr::depth(budget, depth)?;
            sr::work(budget, 1)?;
            if deref_coercion::candidate(expr)
                && !self
                    .call_pointers
                    .contains_key(&deref_coercion::site(expr as *const Expr as usize, 1))
            {
                for step in 1..=deref_coercion::MAX_DEREF_STEPS {
                    self.operator_site(
                        deref_coercion::site(expr as *const Expr as usize, step),
                        &node,
                        budget,
                    )?;
                }
            }
            match expr {
                Expr::Cast { value, .. } | Expr::Repeat { value, .. } => {
                    node = node.child(11, 0, budget)?;
                    expr = value;
                    application = None;
                    direct_let = None;
                    depth += 1;
                }
                _ => {
                    return self.expr_branch(expr, &node, application, direct_let, budget, depth);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_operand_chain_preserves_original_call_authority() {
        let text = format!(
            "fn source()->i32{{7}}fn main(){{let values=[source(){};2];}}",
            " as u8".repeat(120)
        );
        let program = parse(&text).unwrap();
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
        assert_eq!(
            body.source()
                .exprs
                .iter()
                .filter(|expr| matches!(expr.kind, ir::ExprKind::Call { .. }))
                .count(),
            1
        );
        let mut forged = body.source().as_ref().clone();
        let cast = forged
            .exprs
            .iter_mut()
            .find(|expr| matches!(expr.kind, ir::ExprKind::IntCast { .. }))
            .unwrap();
        let ir::ExprKind::IntCast { to, .. } = &mut cast.kind else {
            unreachable!();
        };
        *to = ir::IntTy::U16;
        assert!(
            body.validate_source(&Arc::new(forged), body.facts(), &budget)
                .is_err()
        );
    }
}
