//! Literal facts stay in ordinary per-body graph outputs; CHECK never requests objects.
use justrust_engine::compiler::{self, cfg, frontend, ir, ownership};
use std::sync::Arc;

#[test]
fn literal_payloads_are_immutable_operand_free_cfg_values() {
    for source in [
        r##"fn bytes()->&'static [u8;3]{br#"a\n"#}fn main(){}"##,
        r##"fn bytes()->[u8;3]{*br#"a\n"#}fn main(){}"##,
    ] {
        let program = frontend::parse(source).unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let facts = plan.callable_facts(declaration).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, declaration, &plan, facts.budget())
                .unwrap();
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        let function = template.function();
        function
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let values: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .filter_map(|s| {
                if let cfg::Statement::Assign {
                    value: cfg::Rvalue::ByteStr(bytes),
                    ..
                } = &s.value
                {
                    Some(bytes.as_ref())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(values, [b"a\\n".as_slice()]);
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
}

#[test]
fn check_literals_does_not_demand_native_codegen() {
    let source = r##"fn text()->&'static str{r#"水\n"#}fn bytes()->&'static [u8;2]{b"\0\xff"}fn copied()->[u8;2]{*b"\0\xff"}fn main(){let x:&[u8]=bytes();assert_eq!(x[1],255u8);let y=copied();assert_eq!(y[1],255u8);}"##;
    for workers in [1, 8] {
        let check = compiler::compile_observed(source.into(), workers, true).unwrap();
        assert!(check.report.capture_complete);
        assert!(check.executable.is_none());
        assert!(
            check
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130..=133))
        );
        let build = compiler::compile_observed(source.into(), workers, false).unwrap();
        assert!(
            build
                .report
                .node_evaluations
                .iter()
                .any(|(key, _)| key.kind == 103 && !key.params.is_empty())
        );
        assert!(build.executable.is_some());
    }
}
