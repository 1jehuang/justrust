//! Public equality-assertion source, retained-plan, IR and CFG authority.
//! Structural evaluation checks are not claims of runtime or general Rust support.
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

fn plan(source: &str) -> (frontend::Program, frontend::PlanCtx) {
    let program = frontend::parse(source).unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    (program, plan)
}

fn declaration(program: &frontend::Program, plan: &frontend::PlanCtx, name: &str) -> ir::DeclId {
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap();
    plan.function_decl(index).unwrap()
}

fn panic_mut(program: &mut frontend::Program) -> &mut frontend::SourcePanic {
    let frontend::Expr::If { else_block, .. } =
        program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        panic!("expected direct assertion If")
    };
    let frontend::Expr::Panic(payload) = else_block.tail.as_deref_mut().unwrap() else {
        panic!("expected assertion Panic")
    };
    payload
}

#[test]
fn comparison_operands_are_once_in_order_and_panic_has_no_result() {
    for (macro_name, op) in [
        ("assert_eq", frontend::BinaryOp::Eq),
        ("assert_ne", frontend::BinaryOp::Ne),
    ] {
        // false is an ordinary comparison operand, not a short-circuit guard.
        let source = format!(
            "fn left()->bool{{false}} fn right()->bool{{true}} fn f(){{{macro_name}!(left(),right(),\"cold\")}} fn main(){{}}"
        );
        let (program, plan) = plan(&source);
        frontend::check(&program).unwrap();
        let frontend::Expr::If { condition, .. } =
            program.functions[2].body.tail.as_deref().unwrap()
        else {
            panic!("expected source assertion If")
        };
        let frontend::Expr::Binary {
            op: source_op,
            left: source_left,
            right: source_right,
        } = condition.as_ref()
        else {
            panic!("expected source comparison")
        };
        assert_eq!(*source_op, op);
        assert!(matches!(source_left.as_ref(), frontend::Expr::Call { .. }));
        assert!(matches!(source_right.as_ref(), frontend::Expr::Call { .. }));
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        checked
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let assertions: Vec<_> = checked
            .exprs
            .iter()
            .filter_map(|expr| {
                if let ir::ExprKind::If { condition, .. } = expr.kind {
                    assert_eq!(expr.ty, ir::Ty::Unit);
                    assert_eq!(expr.completion, ir::Completion::Returns);
                    Some(condition)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(assertions.len(), 1);
        let comparison = assertions[0];
        assert_eq!(checked.exprs[comparison.0].ty, ir::Ty::Bool);
        let ir::ExprKind::Binary {
            op: actual,
            left,
            right,
        } = checked.exprs[comparison.0].kind
        else {
            panic!("assertion guard must be a direct comparison")
        };
        assert_eq!(actual, op);
        assert_ne!(left, right);
        let calls: Vec<_> = checked
            .exprs
            .iter()
            .enumerate()
            .filter_map(|(index, expr)| match &expr.kind {
                ir::ExprKind::Call { name, .. } => Some((ir::ExprId(index), name.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(calls, [(left, "crate::left"), (right, "crate::right")]);
        let panics: Vec<_> = checked
            .exprs
            .iter()
            .enumerate()
            .filter_map(|(index, expr)| {
                if let ir::ExprKind::Panic(payload) = &expr.kind {
                    assert_eq!(expr.ty, ir::Ty::Never);
                    assert_eq!(expr.completion, ir::Completion::Never);
                    assert_eq!(payload.message, "cold");
                    Some((ir::ExprId(index), payload.clone()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(panics.len(), 1);
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        let function = template.function();
        function
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let cfg_calls: Vec<_> = function
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| match &block.terminator.value {
                cfg::Terminator::Call {
                    name,
                    next,
                    destination,
                    ..
                } => Some((
                    index,
                    name.as_str(),
                    *next,
                    block.terminator.origin,
                    destination.clone(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(cfg_calls.len(), 2);
        let first = cfg_calls
            .iter()
            .find(|call| call.1 == "crate::left")
            .unwrap();
        let second = cfg_calls
            .iter()
            .find(|call| call.1 == "crate::right")
            .unwrap();
        assert_eq!(first.3, Some(left));
        assert_eq!(second.3, Some(right));
        // No Switch is allowed between the calls, including for false left.
        assert_eq!(first.2.0, second.0);
        let after_right = &function.blocks[second.2.0];
        assert!(after_right.statements.iter().any(|statement| {
            statement.origin == Some(comparison)
                && matches!(&statement.value, cfg::Statement::Assign { value: cfg::Rvalue::Binary { op: actual, left, right }, .. } if *actual == op && *left == cfg::Operand::Copy(first.4.clone()) && *right == cfg::Operand::Copy(second.4.clone()))
        }));
        assert!(matches!(
            after_right.terminator.value,
            cfg::Terminator::Switch { .. }
        ));
        let terminals: Vec<_> = function
            .blocks
            .iter()
            .filter(|block| matches!(block.terminator.value, cfg::Terminator::Panic(_)))
            .collect();
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].terminator.origin, Some(panics[0].0));
        let cfg::Terminator::Panic(payload) = &terminals[0].terminator.value else {
            unreachable!()
        };
        assert_eq!(payload, &panics[0].1);
        // body() may emit a tail transfer on the disconnected continuation
        // created after Panic. Such bookkeeping is not an evaluated result.
        // Prove this from the actual public CFG rather than origin alone.
        let mut reachable = vec![false; function.blocks.len()];
        let mut pending = vec![function.entry];
        while let Some(id) = pending.pop() {
            if std::mem::replace(&mut reachable[id.0], true) {
                continue;
            }
            match &function.blocks[id.0].terminator.value {
                cfg::Terminator::Goto(next)
                | cfg::Terminator::Call { next, .. }
                | cfg::Terminator::Drop { next, .. } => pending.push(*next),
                cfg::Terminator::Switch {
                    then_block,
                    else_block,
                    ..
                }
                | cfg::Terminator::SwitchVariant {
                    then_block,
                    else_block,
                    ..
                } => {
                    pending.push(*then_block);
                    pending.push(*else_block);
                }
                cfg::Terminator::Return
                | cfg::Terminator::Panic(_)
                | cfg::Terminator::Unreachable => {}
            }
        }
        assert!(reachable[first.0] && reachable[second.0]);
        for (index, block) in function.blocks.iter().enumerate() {
            if matches!(block.terminator.value, cfg::Terminator::Panic(_)) {
                assert!(reachable[index]);
            }
        }
        assert!(!terminals[0].statements.iter().any(|statement| {
            statement.origin == Some(panics[0].0)
                && matches!(statement.value, cfg::Statement::Assign { .. })
        }));
        assert!(
            !function
                .blocks
                .iter()
                .enumerate()
                .filter(|(index, _)| reachable[*index])
                .map(|(_, block)| block)
                .flat_map(|block| &block.statements)
                .any(|statement| {
                    statement.origin == Some(panics[0].0)
                        && matches!(statement.value, cfg::Statement::Assign { .. })
                })
        );
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
}

#[test]
fn malformed_comparison_and_if_metadata_fail_even_in_unused_arena_nodes() {
    for macro_name in ["assert_eq", "assert_ne"] {
        let (program, plan) = plan(&format!(
            "fn f(){{let unit=();{macro_name}!(1i32,2i32)}} fn main(){{}}"
        ));
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        let (assertion, comparison) = checked
            .exprs
            .iter()
            .enumerate()
            .find_map(|(index, expr)| match expr.kind {
                ir::ExprKind::If { condition, .. } => Some((index, condition)),
                _ => None,
            })
            .unwrap();
        let ir::ExprKind::Binary { left, right, .. } = checked.exprs[comparison.0].kind else {
            unreachable!()
        };
        for unused in [false, true] {
            for corruption in 0..6 {
                let mut forged = checked.clone();
                let original = if corruption == 0 || corruption == 2 {
                    assertion
                } else {
                    comparison.0
                };
                let index = if unused {
                    forged.exprs.push(checked.exprs[original].clone());
                    forged.exprs.len() - 1
                } else {
                    original
                };
                match corruption {
                    0 => forged.exprs[index].ty = ir::Ty::Bool,
                    1 => forged.exprs[index].ty = ir::Ty::Int(ir::IntTy::I32),
                    2 => {
                        let ir::ExprKind::If { condition, .. } = &mut forged.exprs[index].kind
                        else {
                            unreachable!()
                        };
                        *condition = left;
                    }
                    3 => {
                        let ir::ExprKind::Binary { right, .. } = &mut forged.exprs[index].kind
                        else {
                            unreachable!()
                        };
                        *right = ir::ExprId(usize::MAX);
                    }
                    4 => {
                        // A real Unit expression parsed independently of the assertion.
                        let unit = checked
                            .exprs
                            .iter()
                            .position(|expr| {
                                expr.ty == ir::Ty::Unit
                                    && matches!(expr.kind, ir::ExprKind::Block(_))
                            })
                            .unwrap();
                        let ir::ExprKind::Binary { right, .. } = &mut forged.exprs[index].kind
                        else {
                            unreachable!()
                        };
                        *right = ir::ExprId(unit);
                    }
                    5 => {
                        let ir::ExprKind::Binary { op, .. } = &mut forged.exprs[index].kind else {
                            unreachable!()
                        };
                        *op = frontend::BinaryOp::And;
                    }
                    _ => unreachable!(),
                }
                assert!(
                    forged
                        .validate_with_facts(ir::FactsCtx::Validated(&facts))
                        .is_err(),
                    "{macro_name}, corruption={corruption}, unused={unused}, right={right:?}"
                );
                assert!(
                    cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err()
                );
            }
        }
    }
}

#[test]
fn raw_candidate_changes_do_not_replace_the_immutable_plan_body() {
    for macro_name in ["assert_eq", "assert_ne"] {
        let (mut program, plan) = plan(&format!(
            "fn f(){{{macro_name}!(true,false,\"original\")}} fn main(){{}}"
        ));
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        let frontend::Expr::If { condition, .. } =
            program.functions[0].body.tail.as_deref_mut().unwrap()
        else {
            unreachable!()
        };
        let frontend::Expr::Binary { left, .. } = condition.as_mut() else {
            unreachable!()
        };
        **left = frontend::Expr::Bool(false);
        panic_mut(&mut program).message = "candidate".into();
        let candidate =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        assert!(candidate.exprs.iter().any(|expr| matches!(&expr.kind, ir::ExprKind::Panic(payload) if payload.message == "candidate")));
        let candidate_condition = candidate
            .exprs
            .iter()
            .find_map(|expr| match expr.kind {
                ir::ExprKind::If { condition, .. } => Some(condition),
                _ => None,
            })
            .unwrap();
        let ir::ExprKind::Binary { left, .. } = candidate.exprs[candidate_condition.0].kind else {
            unreachable!()
        };
        assert!(matches!(
            candidate.exprs[left.0].kind,
            ir::ExprKind::Bool(false)
        ));
        let target = ir::CallTarget {
            declaration: id,
            type_args: Arc::from([]),
        };
        let retained = plan.checked_body(&target, &facts, facts.budget()).unwrap();
        assert!(retained.exprs.iter().any(|expr| matches!(&expr.kind, ir::ExprKind::Panic(payload) if payload.message == "original")));
        let condition = retained
            .exprs
            .iter()
            .find_map(|expr| match expr.kind {
                ir::ExprKind::If { condition, .. } => Some(condition),
                _ => None,
            })
            .unwrap();
        let ir::ExprKind::Binary { left, .. } = retained.exprs[condition.0].kind else {
            unreachable!()
        };
        assert!(matches!(
            retained.exprs[left.0].kind,
            ir::ExprKind::Bool(true)
        ));
        let template =
            cfg::lower_template(Arc::new(retained), facts.clone(), facts.budget()).unwrap();
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
}

#[test]
fn genuine_foreign_panic_site_is_rejected_before_unsupported_operands() {
    for macro_name in ["assert_eq", "assert_ne"] {
        let source = format!("fn f(){{{macro_name}!(true,false)}} fn main(){{}}");
        for unsupported in [false, true] {
            let (mut program, plan) = plan(&source);
            let mut foreign = frontend::parse(&source).unwrap();
            let foreign_site = panic_mut(&mut foreign).site;
            let id = declaration(&program, &plan, "f");
            let facts = plan.callable_facts(id).unwrap();
            if unsupported {
                let frontend::Expr::If { condition, .. } =
                    program.functions[0].body.tail.as_deref_mut().unwrap()
                else {
                    unreachable!()
                };
                let frontend::Expr::Binary { left, .. } = condition.as_mut() else {
                    unreachable!()
                };
                **left = frontend::Expr::Array(vec![frontend::Expr::Bool(true)]);
            }
            panic_mut(&mut program).site = foreign_site;
            let error =
                frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap_err();
            assert!(
                error.contains("contract") || error.contains("foreign"),
                "foreign metadata must precede operand capability: {error}"
            );
            let error = match frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default())
            {
                Err(error) => error,
                Ok(_) => panic!("unused function imported foreign panic site into a new plan"),
            };
            assert!(
                error.contains("contract") || error.contains("foreign"),
                "new-plan foreign metadata must precede operand capability: {error}"
            );
        }
    }
}

#[test]
fn primitive_forms_and_original_expansion_locations_are_preserved() {
    for (source, suffix) in [
        ("fn f(){\nassert_eq!(1i32,1i32,)\n} fn main(){}", ":2:1"),
        (
            "fn f(){\nassert_ne![false,true,\"literal\",]\n} fn main(){}",
            ":2:1",
        ),
        ("fn f(){\nassert_eq!{(),()}\n} fn main(){}", ":2:1"),
        (
            "fn f(){\nmacro_rules! check {()=>{assert_eq!(1u8,1u8)};}\ncheck!()\n} fn main(){}",
            ":3:1",
        ),
    ] {
        let (program, plan) = plan(source);
        frontend::check(&program).unwrap();
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        let payload = checked
            .exprs
            .iter()
            .find_map(|expr| match &expr.kind {
                ir::ExprKind::Panic(payload) => Some(payload),
                _ => None,
            })
            .unwrap();
        assert!(payload.location.ends_with(suffix), "{}", payload.location);
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
}

#[test]
fn reference_aggregate_and_formatting_boundaries_remain_explicit() {
    for source in [
        "fn main(){let value=1i32;assert_eq!(&value,&value)}",
        "fn main(){assert_ne!((1i32,2i32),(2i32,1i32))}",
        "fn main(){assert_eq!([1i32],[1i32])}",
    ] {
        let program = frontend::parse(source).expect("operand capability is a semantic boundary");
        let error = match frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
            Err(error) => error,
            Ok(_) => {
                frontend::check(&program).expect_err("unsupported comparison gained authority")
            }
        };
        assert!(
            error.to_ascii_lowercase().contains("unsupported"),
            "{source}: {error}"
        );
    }
    for source in [
        "fn main(){assert_eq!(1i32,1i32,1i32)}",
        "fn main(){assert_ne!(1i32,2i32,\"{}\",1i32)}",
        "fn main(){assert_eq!(1i32,1i32,\"value {value}\")}",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.to_ascii_lowercase().contains("unsupported"),
            "{source}: {error}"
        );
    }
}

#[test]
fn mismatched_operands_and_nonunit_results_are_semantic_rejections() {
    for source in [
        "fn main(){assert_eq!(true,1i32)}",
        "fn main(){assert_ne!(1u8,1i32)}",
        "fn f()->bool{assert_eq!(true,true)} fn main(){}",
        "fn main(){let value:i32=assert_ne!(1i32,2i32);}",
        "fn main(){panic!();assert_eq!(true,1i32)}",
    ] {
        let program = frontend::parse(source).expect("semantic error must parse");
        let error = match frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
            Err(error) => error,
            Ok(_) => frontend::check(&program).expect_err("invalid equality assertion accepted"),
        };
        assert!(
            !error.to_ascii_lowercase().contains("unsupported"),
            "{source}: {error}"
        );
    }
}
