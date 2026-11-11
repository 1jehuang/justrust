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
            if matches!(expr, Expr::BorrowValue { .. })
                && let Some((_, place)) =
                    explicit_field_borrow::authored_static_borrow(expr, budget)
                        .map_err(|e| source_error(budget, &e))?
                && let Some(binding) = self.lookup(place.local, budget)?
            {
                reserve_source_map(&mut self.operand_nodes, budget)?;
                if self
                    .operand_nodes
                    .insert(node.path.clone(), binding)
                    .is_some()
                {
                    return Err(FactsError::invalid(budget, "duplicate lexical source node"));
                }
            }
            let call_receiver = match expr {
                Expr::MethodCall { receiver, .. } => Some((receiver.as_ref(), 10)),
                // A field may autoderef its owned call-result base. Its
                // operator roles belong to that exact authored operand.
                Expr::Field { base, .. } => Some((base.as_ref(), 13)),
                // `guard()[i]`: index autoderef may call `Deref` on the owned
                // call result, recorded at the base's receiver step.
                Expr::Index { base, .. } | Expr::DynamicIndex { base, .. } => {
                    Some((base.as_ref(), 12))
                }
                _ => None,
            };
            if let Some((receiver, tag)) = call_receiver
                && type_check::genuine_call(receiver)
            {
                let receiver_node = node.child(tag, 0, budget)?;
                for step in 1..=deref_coercion::MAX_DEREF_STEPS {
                    self.operator_site(
                        deref_coercion::site(receiver as *const Expr as usize, step),
                        &receiver_node,
                        budget,
                    )?;
                }
            }
            if (deref_coercion::candidate(expr)
                || matches!(expr, Expr::Index { .. } | Expr::DynamicIndex { .. }))
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
                Expr::Cast { value, .. }
                | Expr::Repeat { value, .. }
                | Expr::ParamRepeat { value, .. } => {
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
    fn projected_borrow_value_roles_keep_original_occurrence() {
        let program = parse("struct Owner{values:[i32;2]}fn main(){let owner=Owner{values:[1i32,2i32]};let borrowed=&owner.values[1];}").unwrap();
        let function = program.functions.last().unwrap();
        let Stmt::Let { value: source, .. } = &function.body.statements[1] else {
            panic!("borrow initializer")
        };
        assert!(matches!(source, Expr::BorrowValue { .. }));
        let budget = ir::PlanBudget::default();
        assert!(
            source_input_replay::copy_arguments(std::slice::from_ref(source), &budget)
                .unwrap()
                .is_some()
        );
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        let pointer = source as *const Expr as usize;
        let first = &inventory.calls[inventory.call_pointers[&deref_coercion::site(pointer, 1)]];
        assert_eq!(
            inventory.operand_nodes[&first.node.path],
            inventory.bindings[0].node
        );
        for step in 1..=deref_coercion::MAX_DEREF_STEPS {
            let role =
                &inventory.calls[inventory.call_pointers[&deref_coercion::site(pointer, step)]];
            assert!(role.operator_candidate);
            assert_eq!(role.node, first.node);
        }
        for source in [
            "struct Owner{values:[i32;2]}fn make()->Owner{Owner{values:[1i32,2i32]}}fn main(){&make().values[1]}",
            "struct Owner{values:[i32;2]}fn index()->usize{1usize}fn main(){let owner=Owner{values:[1i32,2i32]};&owner.values[index()]}",
            "struct Owner{values:[i32;2]}fn main(){let owner=Owner{values:[1i32,2i32]};&owner.values}",
        ] {
            let program = parse(source).unwrap();
            let source = program
                .functions
                .last()
                .unwrap()
                .body
                .tail
                .as_deref()
                .unwrap();
            if matches!(source, Expr::BorrowValue { .. }) {
                assert!(
                    source_input_replay::copy_arguments(std::slice::from_ref(source), &budget)
                        .unwrap()
                        .is_none()
                );
                assert!(
                    explicit_field_borrow::authored_static_borrow(source, &budget)
                        .unwrap()
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn field_call_receiver_roles_preserve_exact_operand_and_step() {
        let program = parse(
            "struct V{n:u32}fn make()->V{V{n:1u32}}fn main(){let n=make().n;let other=make();}",
        )
        .unwrap();
        let function = program.functions.last().unwrap();
        let Stmt::Let {
            value: Expr::Field { base, .. },
            ..
        } = &function.body.statements[0]
        else {
            panic!("expected authored call-base field");
        };
        let Stmt::Let { value: other, .. } = &function.body.statements[1] else {
            panic!("expected unrelated authored call");
        };
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory = SourceInventory::build(&program, function, identity, &budget).unwrap();
        let pointer = base.as_ref() as *const Expr as usize;
        let authored = &inventory.calls[inventory.call_pointers[&pointer]];
        assert!(!authored.operator_candidate);
        assert!(authored.node.path.ends_with(&[13, 0]));
        for step in 1..=deref_coercion::MAX_DEREF_STEPS {
            let tagged = deref_coercion::site(pointer, step);
            let role = &inventory.calls[inventory.call_pointers[&tagged]];
            assert!(role.operator_candidate);
            assert!(role.ordinary.is_none());
            assert_eq!(role.node, authored.node);
            // A different written call has no field-receiver operator role.
            let foreign = deref_coercion::site(other as *const Expr as usize, step);
            assert!(!inventory.call_pointers.contains_key(&foreign));
        }
        // A malformed high tag can alias the ordinary call address, but
        // must never identify a reserved operator role.
        let changed_step = deref_coercion::site(pointer, deref_coercion::MAX_DEREF_STEPS + 1);
        assert!(
            inventory
                .call_pointers
                .get(&changed_step)
                .is_none_or(|index| !inventory.calls[*index].operator_candidate)
        );
        let foreign_body = SourceInventory::build(
            &program,
            &program.functions[0],
            sr::OriginalBodyId::new(&budget).unwrap(),
            &budget,
        )
        .unwrap();
        assert!(!foreign_body.call_pointers.contains_key(&pointer));
        assert_ne!(foreign_body.owner, authored.node.owner);
    }

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
