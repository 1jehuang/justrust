//! Public guarded matches! authority and CFG topology, not runtime execution.
//! These are acceptance controls for GUARDED-MATCHES.md, including pending support.
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

fn plan(source: &str) -> (frontend::Program, frontend::PlanCtx) {
    let program = frontend::parse(source).unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    (program, plan)
}

fn declaration(program: &frontend::Program, plan: &frontend::PlanCtx) -> ir::DeclId {
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == "f"
        })
        .unwrap();
    plan.function_decl(index).unwrap()
}

fn checked(program: &frontend::Program, plan: &frontend::PlanCtx) -> ir::CheckedFunction {
    frontend::check(program).unwrap();
    let id = declaration(program, plan);
    let facts = plan.callable_facts(id).unwrap();
    let body = frontend::type_callable_with_plan(program, id, plan, facts.budget()).unwrap();
    body.validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    body
}

fn validate_cfg(program: &frontend::Program, plan: &frontend::PlanCtx, body: ir::CheckedFunction) {
    let facts = plan.callable_facts(declaration(program, plan)).unwrap();
    let template = cfg::lower_template(Arc::new(body), facts.clone(), facts.budget()).unwrap();
    template
        .function()
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}

fn guarded(body: &ir::CheckedFunction) -> (ir::ExprId, ir::ExprId, ir::ExprId) {
    let matches: Vec<_> = body
        .exprs
        .iter()
        .enumerate()
        .filter_map(|(index, expr)| {
            if let ir::ExprKind::Match {
                scrutinee, arms, ..
            } = &expr.kind
            {
                Some((ir::ExprId(index), *scrutinee, arms))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(matches.len(), 1);
    let (id, scrutinee, arms) = matches[0];
    assert_eq!(arms.len(), 2);
    assert_eq!(body.exprs[id.0].ty, ir::Ty::Bool);
    assert_eq!(body.exprs[id.0].completion, ir::Completion::Returns);
    let ir::ExprKind::If {
        condition,
        then_block,
        else_block,
    } = &body.exprs[arms[0].body.0].kind
    else {
        panic!("matched arm must contain the ordinary guard If")
    };
    assert!(matches!(
        body.exprs[then_block.tail.unwrap().0].kind,
        ir::ExprKind::Bool(true)
    ));
    assert!(matches!(
        body.exprs[else_block.tail.unwrap().0].kind,
        ir::ExprKind::Bool(false)
    ));
    assert!(matches!(
        body.exprs[arms[1].body.0].kind,
        ir::ExprKind::Bool(false)
    ));
    (scrutinee, arms[0].body, *condition)
}

fn source_guard_mut(program: &mut frontend::Program) -> &mut frontend::Expr {
    let frontend::Expr::Match { arms, .. } = program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        panic!("expected source match")
    };
    let frontend::Expr::If { condition, .. } = &mut arms[0].body else {
        panic!("expected source guard If")
    };
    condition
}

// Follow actual public CFG edges. No fabricated lowering proof or execution claim.
fn reachable(
    function: &cfg::Function,
    start: cfg::BlockId,
    blocked: Option<cfg::BlockId>,
) -> Vec<bool> {
    let mut seen = vec![false; function.blocks.len()];
    let mut pending = vec![start];
    while let Some(id) = pending.pop() {
        if Some(id) == blocked || std::mem::replace(&mut seen[id.0], true) {
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
            cfg::Terminator::Return | cfg::Terminator::Panic(_) | cfg::Terminator::Unreachable => {}
        }
    }
    seen
}

#[test]
fn bool_guards_use_two_arm_match_and_ordinary_if_across_delimiters() {
    for expression in [
        "matches!(1, 1 if true)",
        "matches![1, 1 if false,]",
        "matches!{1, 2 if true,}",
        "matches!(1, 2 if false)",
    ] {
        let (program, plan) = plan(&format!("fn f()->bool {{{expression}}} fn main(){{}}"));
        let body = checked(&program, &plan);
        let (_, _, guard) = guarded(&body);
        assert_eq!(body.exprs[guard.0].ty, ir::Ty::Bool);
        validate_cfg(&program, &plan, body);
    }
}

#[test]
fn pattern_bindings_are_available_to_guard_including_nested_aggregates() {
    for source in [
        "fn f()->bool {matches!(3, value if value == 3)} fn main(){}",
        "fn f()->bool {matches!((3,true), (value, flag) if flag && value == 3)} fn main(){}",
        "enum E {A((i32,bool)),B} fn f(value:E)->bool {matches!(value,E::A((n,flag)) if flag && n == 3)} fn main(){}",
        "fn f()->bool {let value = false; matches!(3, value if value == 3)} fn main(){}",
    ] {
        let (program, plan) = plan(source);
        let body = checked(&program, &plan);
        guarded(&body);
        validate_cfg(&program, &plan, body);
    }
}

#[test]
fn pattern_binding_does_not_escape_or_replace_outer_local() {
    let (program, plan) = plan(
        "fn f()->bool {let value = false; let result = matches!(3,value if value == 3); value} fn main(){}",
    );
    let body = checked(&program, &plan);
    guarded(&body);
    validate_cfg(&program, &plan, body);
    let program = frontend::parse(
        "fn f()->bool {let result = matches!(3,value if value == 3); value == 3} fn main(){}",
    )
    .unwrap();
    assert!(
        frontend::check(&program).is_err(),
        "guard binding escaped its arm"
    );
}

#[test]
fn scrutinee_once_and_guard_call_only_on_successful_pattern_path() {
    let (program, plan) = plan(
        "fn value()->i32 {4} fn guard()->bool {false} fn f()->bool {matches!(value(),4 if guard())} fn main(){}",
    );
    let body = checked(&program, &plan);
    let (scrutinee, conditional, guard) = guarded(&body);
    assert!(matches!(
        body.exprs[scrutinee.0].kind,
        ir::ExprKind::Call { .. }
    ));
    assert!(matches!(
        body.exprs[guard.0].kind,
        ir::ExprKind::Call { .. }
    ));
    assert_eq!(
        body.exprs
            .iter()
            .filter(|expr| matches!(expr.kind, ir::ExprKind::Call { .. }))
            .count(),
        2
    );
    let facts = plan.callable_facts(declaration(&program, &plan)).unwrap();
    let template = cfg::lower_template(Arc::new(body), facts.clone(), facts.budget()).unwrap();
    let function = template.function();
    function
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let calls: Vec<_> = function
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            if let cfg::Terminator::Call { name, next, .. } = &block.terminator.value {
                Some((
                    cfg::BlockId(index),
                    name.as_str(),
                    *next,
                    block.terminator.origin,
                ))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls.len(), 2);
    let value = calls.iter().find(|call| call.1 == "crate::value").unwrap();
    let guard_call = calls.iter().find(|call| call.1 == "crate::guard").unwrap();
    assert_eq!(value.3, Some(scrutinee));
    assert_eq!(guard_call.3, Some(guard));
    assert!(reachable(function, function.entry, None)[value.0.0]);
    assert!(!reachable(function, function.entry, Some(value.0))[guard_call.0.0]);
    // A pattern decision must have one route to the guard and a bypass route
    // to Return. The latter cannot enter the guard even through later joins.
    assert!(function.blocks.iter().any(|block| {
        if block.terminator.origin == Some(conditional) {
            return false;
        }
        let (yes, no) = match block.terminator.value {
            cfg::Terminator::Switch {
                then_block,
                else_block,
                ..
            }
            | cfg::Terminator::SwitchVariant {
                then_block,
                else_block,
                ..
            } => (then_block, else_block),
            _ => return false,
        };
        let yes_reach = reachable(function, yes, None);
        let no_reach = reachable(function, no, None);
        yes_reach[guard_call.0.0] != no_reach[guard_call.0.0]
            && function.blocks.iter().enumerate().any(|(index, block)| {
                matches!(block.terminator.value, cfg::Terminator::Return)
                    && if yes_reach[guard_call.0.0] {
                        no_reach[index]
                    } else {
                        yes_reach[index]
                    }
            })
    }));
    let after_guard = &function.blocks[guard_call.2.0];
    assert_eq!(after_guard.terminator.origin, Some(conditional));
    let cfg::Terminator::Switch {
        then_block,
        else_block,
        ..
    } = after_guard.terminator.value
    else {
        panic!("guard result must control an ordinary conditional")
    };
    for branch in [then_block, else_block] {
        let seen = reachable(function, branch, None);
        assert!(!seen[guard_call.0.0], "guard must not be retried");
        assert!(!seen[value.0.0], "scrutinee must not be retried");
        assert!(
            function
                .blocks
                .iter()
                .enumerate()
                .any(|(index, block)| seen[index]
                    && matches!(block.terminator.value, cfg::Terminator::Return))
        );
    }
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}

#[test]
fn never_guard_is_checked_but_mismatch_keeps_bool_result() {
    for pattern in ["1", "2"] {
        let (program, plan) = plan(&format!(
            "fn f()->bool {{matches!(1,{pattern} if panic!(\"guard\"))}} fn main(){{}}"
        ));
        let body = checked(&program, &plan);
        let (_, _, guard) = guarded(&body);
        assert_eq!(body.exprs[guard.0].ty, ir::Ty::Never);
        assert_eq!(body.exprs[guard.0].completion, ir::Completion::Never);
        validate_cfg(&program, &plan, body);
    }
}

#[test]
fn invalid_guards_are_checked_even_when_dead_or_mismatched() {
    for source in [
        "fn f()->bool {matches!(1,1 if 3)} fn main(){}",
        "fn f()->bool {matches!(1,2 if 3)} fn main(){}",
        "fn f()->bool {matches!(1,2 if missing)} fn main(){}",
        "fn f()->bool {if false {matches!(1,1 if missing)} else {false}} fn main(){}",
        "fn f()->bool {return false; matches!(1,1 if 3)} fn main(){}",
        "fn f()->bool {matches!(1,2 if {let x = (1,2); x})} fn main(){}",
        "fn f()->bool {let mut value = 1; let held = &value; matches!(1,1 if {value = 2; *held == 1})} fn main(){}",
        "fn f()->bool {let mut value = 1; let held = &value; matches!(1,2 if {value = 2; *held == 1})} fn main(){}",
    ] {
        let program = frontend::parse(source).expect("invalid guard must reach semantic checking");
        assert!(
            frontend::check(&program).is_err(),
            "accepted invalid guard: {source}"
        );
    }
}

#[test]
fn non_copy_pattern_binding_cannot_be_consumed_by_guard() {
    // Ordinary arm-body If checking is insufficient: a real pattern guard
    // cannot move a non-Copy binding, even when the matched arm returns Bool.
    // Keep this as a rejection control, not a claim of general guard support.
    let control = "struct Token {value:i32} enum E {Some(Token),None} fn consume(value:Token)->bool {true} fn f(value:E)->bool {match value {E::Some(bound) => consume(bound), E::None => false}} fn main(){}";
    let (program, plan) = plan(control);
    let body = checked(&program, &plan);
    validate_cfg(&program, &plan, body);
    let source = "struct Token {value:i32} enum E {Some(Token),None} fn consume(value:Token)->bool {true} fn f(value:E)->bool {matches!(value,E::Some(bound) if consume(bound))} fn main(){}";
    let program = frontend::parse(source).expect("guard move must reach semantic checking");
    assert!(
        frontend::check(&program).is_err(),
        "non-Copy bound value was moved out of a pattern guard"
    );
}

#[test]
fn bounded_guards_allow_non_copy_wildcards_but_reject_non_copy_or_mutable_bindings() {
    let (program, plan) = plan(
        "struct Token {value:i32} fn f(value:Token)->bool {matches!(value,_ if true)} fn main(){}",
    );
    let body = checked(&program, &plan);
    guarded(&body);
    validate_cfg(&program, &plan, body);
    for source in [
        "struct Token {value:i32} fn f(value:Token)->bool {matches!(value,bound if true)} fn main(){}",
        "fn f()->bool {matches!(1,mut bound if true)} fn main(){}",
    ] {
        // The bounded policy may reject during parsing or semantic checking.
        // Do not treat that policy as general Rust guard compatibility.
        if let Ok(program) = frontend::parse(source) {
            assert!(
                frontend::check(&program).is_err(),
                "accepted out-of-scope guard binding: {source}"
            );
        }
    }
}

#[test]
fn cloned_source_retains_guard_authority() {
    let (program, retained) = plan("fn f(value:bool)->bool {matches!(1,1 if value)} fn main(){}");
    let cloned = program.clone();
    let body = checked(&cloned, &retained);
    guarded(&body);
    validate_cfg(&cloned, &retained, body);
    let fresh = frontend::PlanCtx::from_program(&cloned, &ir::PlanBudget::default()).unwrap();
    let body = checked(&cloned, &fresh);
    validate_cfg(&cloned, &fresh, body);
}

#[test]
fn foreign_guard_name_and_panic_site_cannot_acquire_source_authority() {
    for source in [
        "fn f(value:bool)->bool {matches!(1,1 if value)} fn main(){}",
        "fn f()->bool {matches!(1,1 if panic!(\"guard\"))} fn main(){}",
    ] {
        let (mut program, plan) = plan(source);
        let mut foreign = frontend::parse(source).unwrap();
        let id = declaration(&program, &plan);
        let facts = plan.callable_facts(id).unwrap();
        frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        *source_guard_mut(&mut program) = source_guard_mut(&mut foreign).clone();
        assert!(frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).is_err());
        assert!(frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).is_err());
    }
}

#[test]
fn candidate_guard_edit_does_not_replace_retained_checked_body() {
    let (mut program, plan) = plan("fn f()->bool {matches!(1,1 if true)} fn main(){}");
    let id = declaration(&program, &plan);
    let facts = plan.callable_facts(id).unwrap();
    *source_guard_mut(&mut program) = frontend::Expr::Bool(false);
    let candidate = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let (_, _, guard) = guarded(&candidate);
    assert!(matches!(
        candidate.exprs[guard.0].kind,
        ir::ExprKind::Bool(false)
    ));
    let target = ir::CallTarget {
        declaration: id,
        type_args: Arc::from([]),
    };
    let retained = plan.checked_body(&target, &facts, facts.budget()).unwrap();
    let (_, _, guard) = guarded(&retained);
    assert!(matches!(
        retained.exprs[guard.0].kind,
        ir::ExprKind::Bool(true)
    ));
    validate_cfg(&program, &plan, retained);
}

#[test]
fn ordinary_match_guards_remain_unsupported() {
    assert!(
        frontend::parse("fn f()->bool {match 1 {1 if true => true, _ => false}} fn main(){}")
            .is_err()
    );
}
