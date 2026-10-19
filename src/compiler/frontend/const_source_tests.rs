use super::*;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn const_source_publication_rejects_swapped_initializer_and_same_handle_program_clone() {
        let program = Arc::new(parse("const A:u8=1;const B:u8=2;fn main(){}").unwrap());
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_shared_program(program.clone(), &budget).unwrap();
        let a = plan.constant_decl(0).unwrap();
        plan.publish_constant_declaration(&program, a, &budget)
            .unwrap();
        let swapped = ConstDecl {
            initializer: program.constants[1].initializer.clone(),
            ..program.constants[0].clone()
        };
        let error =
            super::super::super::const_source::project(&plan, a, &swapped, &budget).unwrap_err();
        assert!(error.contains("authoritative source"), "{error}");
        let clone = Arc::new(program.as_ref().clone());
        let error = plan
            .publish_constant_declaration(&clone, a, &budget)
            .unwrap_err();
        assert!(error.contains("publication changed"), "{error}");
        let b = plan.constant_decl(1).unwrap();
        let mut forged = plan
            .publish_constant_declaration(&program, a, &budget)
            .unwrap();
        let other = plan
            .publish_constant_declaration(&program, b, &budget)
            .unwrap();
        forged.expressions = other.expressions;
        let error =
            crate::compiler::const_check::check(Arc::new(forged), &[], &plan, &budget).unwrap_err();
        assert!(
            error.contains("frozen constant source publication changed"),
            "{error}"
        );
    }
    #[test]
    fn const_runtime_body_without_graph_inputs_fails_closed() {
        let program = Arc::new(parse("const A:u8=1;fn main(){let x:u8=A;}").unwrap());
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_shared_program(program, &budget).unwrap();
        let declaration = plan.callable_decl(0).unwrap();
        let error = match plan.publish_callable_body(declaration, &budget) {
            Ok(_) => panic!("missing value graph handle accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("graph input handles"), "{error}");
    }
}
