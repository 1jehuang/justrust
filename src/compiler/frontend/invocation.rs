//! Genuine postfix invocation syntax. No callable member identity is minted by
//! the parser. Source resolution supplies the canonical callable trait member.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn invocation_after_open(&mut self, receiver: Expr) -> Result<Expr, ParseError> {
        let mut arguments = Vec::new();
        while !self.is(")") {
            self.reserve_parser_items(&mut arguments, 1)?;
            arguments.push(self.nested_expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        let argument = if arguments.is_empty() {
            Expr::Block(Block {
                statements: Vec::new(),
                tail: None,
            })
        } else {
            Expr::Tuple(arguments)
        };
        Ok(Expr::Invoke {
            receiver: Box::new(receiver),
            args: vec![argument],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_retains_receiver_arguments_and_original_source_inventory() {
        let parsed =
            parse_unresolved("fn make()->i32{1}fn arg()->i32{2}fn main(){(make())(arg(),3)}")
                .unwrap();
        let program = &parsed.program;
        let expression = program.functions[2].body.tail.as_deref().unwrap();
        let Expr::Invoke { receiver, args } = expression else {
            panic!("missing invocation")
        };
        assert!(matches!(receiver.as_ref(), Expr::Call { .. }));
        let [Expr::Tuple(arguments)] = args.as_slice() else {
            panic!("missing canonical tuple")
        };
        assert_eq!(arguments.len(), 2);
        assert!(matches!(arguments[0], Expr::Call { .. }));
        let budget = ir::PlanBudget::default();
        let identity = sr::OriginalBodyId::new(&budget).unwrap();
        let inventory =
            SourceInventory::build(&program, &program.functions[2], identity, &budget).unwrap();
        assert_eq!(
            inventory
                .calls
                .iter()
                .filter(|call| !call.operator_candidate)
                .count(),
            3
        );
        program.validate_name_handles().unwrap();
        preflight_program_clone(&program, &budget).unwrap();
        recipes::inventory(&program, &budget).unwrap();
    }

    #[test]
    fn blocklike_statement_does_not_become_a_callee_for_the_next_statement() {
        let parsed = parse_unresolved("fn main(){if true {} (f)();}").unwrap();
        let body = &parsed.program.functions[0].body;
        assert_eq!(body.statements.len(), 2);
        assert!(matches!(
            &body.statements[0],
            Stmt::ImplicitExpr(Expr::If { .. })
        ));
        assert!(matches!(
            &body.statements[1],
            Stmt::Expr(Expr::Invoke { .. })
        ));
    }

    #[test]
    fn zero_argument_invocation_uses_existing_unit_tuple_shape() {
        let parsed = parse_unresolved("fn main(){(f)()}").unwrap();
        let program = &parsed.program;
        let Expr::Invoke { args, .. } = program.functions[0].body.tail.as_deref().unwrap() else {
            panic!("missing invocation")
        };
        assert!(
            matches!(args.as_slice(), [Expr::Block(Block { statements, tail: None })] if statements.is_empty())
        );
    }
}
