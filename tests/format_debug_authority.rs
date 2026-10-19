//! Public source, immutable-plan, IR and CFG authority for native format modes.
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

fn plan(source: &str) -> (frontend::Program, frontend::PlanCtx, ir::DeclId) {
    let program = frontend::parse(source).unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let id = plan.function_decl(0).unwrap();
    (program, plan, id)
}

#[test]
fn malformed_format_modes_counts_and_unsupported_types_are_rejected() {
    for source in [
        r#"fn main(){println!("{:?}");}"#,
        r#"fn main(){println!("{} {:?}",1);}"#,
        r#"fn main(){println!("{:?}",1,2);}"#,
        r#"fn main(){println!("{:#?}",1);}"#,
        r#"fn main(){println!("{:?x}",1);}"#,
        r#"fn main(){println!("{:}",1);}"#,
        r#"fn main(){println!("{0:?}",1);}"#,
    ] {
        assert!(frontend::parse(source).is_err(), "{source}");
    }
    for source in [
        r#"fn unused(){println!("{:?}",'x');} fn main(){}"#,
        r#"fn unused(){println!("{:?}","text");} fn main(){}"#,
        r#"fn unused(){println!("{:?}",1.25f64);} fn main(){}"#,
        r#"fn unused(){println!("{}",1.25f64);} fn main(){}"#,
        r#"struct S;fn unused(){println!("{:?}",(S,));}fn main(){}"#,
        r#"struct S;fn unused(){let a:[S;0]=[];println!("{:?}",a);}fn main(){}"#,
        r#"trait Debug{}struct S;impl Debug for S{}fn unused(){println!("{:?}",S);}fn main(){}"#,
    ] {
        let program = frontend::parse(source).unwrap();
        let error = frontend::check(&program).unwrap_err();
        assert!(
            error.contains("Debug") || error.contains("Display"),
            "{source}: {error}"
        );
    }
}

#[test]
fn mode_and_piece_arity_are_checked_in_live_and_unused_ir_and_cfg() {
    let (program, plan, id) = plan(
        r#"fn f(){println!("{} {:?}",identity(1),(2,true));}fn identity(v:i32)->i32{v}fn main(){}"#,
    );
    let facts = plan.callable_facts(id).unwrap();
    let original = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let index = original
        .exprs
        .iter()
        .position(|expr| matches!(expr.kind, ir::ExprKind::Format { .. }))
        .unwrap();
    for unused in [false, true] {
        for mutation in 0..3 {
            let mut forged = original.clone();
            let index = if unused {
                forged.exprs.push(original.exprs[index].clone());
                forged.exprs.len() - 1
            } else {
                index
            };
            let ir::ExprKind::Format { pieces, modes, .. } = &mut forged.exprs[index].kind else {
                unreachable!()
            };
            match mutation {
                0 => {
                    modes.pop();
                }
                1 => {
                    pieces.pop();
                }
                _ => {
                    modes[1] = ir::FormatMode::Display;
                }
            }
            assert!(
                forged
                    .validate_with_facts(ir::FactsCtx::Validated(&facts))
                    .is_err()
            );
            assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
        }
    }
    let template = cfg::lower_template(Arc::new(original), facts.clone(), facts.budget()).unwrap();
    assert!(template.function().lowering_proof.is_some());
    template
        .function()
        .validate_provenance(&facts, facts.budget())
        .unwrap();
    for mutation in 0..3 {
        let mut function = template.function().as_ref().clone();
        let value = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match &mut statement.value {
                cfg::Statement::Assign {
                    value: cfg::Rvalue::Format { pieces, modes, .. },
                    ..
                } => Some((pieces, modes)),
                _ => None,
            })
            .unwrap();
        match mutation {
            0 => {
                value.1.pop();
            }
            1 => {
                value.0.pop();
            }
            _ => {
                value.1[1] = ir::FormatMode::Display;
            }
        }
        assert!(
            function
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err()
        );
    }
    for unused in [false, true] {
        let mut function = template.function().as_ref().clone();
        let block = function
            .blocks
            .iter()
            .position(|block| {
                block.statements.iter().any(|statement| {
                    matches!(
                        statement.value,
                        cfg::Statement::Assign {
                            value: cfg::Rvalue::Format { .. },
                            ..
                        }
                    )
                })
            })
            .unwrap();
        let block = if unused {
            function.blocks.push(function.blocks[block].clone());
            function.blocks.len() - 1
        } else {
            block
        };
        let modes = function.blocks[block]
            .statements
            .iter_mut()
            .find_map(|statement| match &mut statement.value {
                cfg::Statement::Assign {
                    value: cfg::Rvalue::Format { modes, .. },
                    ..
                } => Some(modes),
                _ => None,
            })
            .unwrap();
        modes[0] = ir::FormatMode::Debug;
        // An integer admits both modes. Type admissibility does not authorize
        // replacing the actual source operation, including in unreachable rows.
        assert!(
            function
                .validate_provenance(&facts, facts.budget())
                .is_err()
        );
    }
}

#[test]
fn candidate_mode_edits_cannot_replace_immutable_retained_source_mode() {
    let (mut program, plan, id) = plan(r#"fn f(){println!("{}",1)}fn main(){}"#);
    let facts = plan.callable_facts(id).unwrap();
    let frontend::Expr::Format { modes, .. } =
        program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        unreachable!()
    };
    modes[0] = ir::FormatMode::Debug;
    let candidate = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    assert!(candidate.exprs.iter().any(|expr|matches!(&expr.kind,ir::ExprKind::Format{modes,..} if modes == &[ir::FormatMode::Debug])));
    let target = ir::CallTarget {
        declaration: id,
        type_args: Arc::from([]),
    };
    let retained = plan.checked_body(&target, &facts, facts.budget()).unwrap();
    assert!(retained.exprs.iter().any(|expr|matches!(&expr.kind,ir::ExprKind::Format{modes,..} if modes == &[ir::FormatMode::Display])));
    let frontend::Expr::Format { modes, .. } =
        program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        unreachable!()
    };
    modes.clear();
    assert!(frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).is_err());
}
