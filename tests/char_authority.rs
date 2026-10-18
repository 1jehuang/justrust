//! Public char authority from real source through retained plans and lowering.
//! These are structural checks, not native execution or layout claims.
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

fn plan(source: &str) -> (frontend::Program, frontend::PlanCtx) {
    let program = frontend::parse(source).unwrap();
    frontend::check(&program).unwrap();
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

fn typed(
    program: &frontend::Program,
    plan: &frontend::PlanCtx,
    name: &str,
) -> (ir::CheckedFunction, Arc<ValidatedFacts>) {
    let id = declaration(program, plan, name);
    let facts = plan.callable_facts(id).unwrap();
    let checked = frontend::type_callable_with_plan(program, id, plan, facts.budget()).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    (checked, facts)
}

fn lower(checked: ir::CheckedFunction, facts: Arc<ValidatedFacts>) {
    let template = cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
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

fn reject(checked: ir::CheckedFunction, facts: &Arc<ValidatedFacts>) {
    assert!(
        checked
            .validate_with_facts(ir::FactsCtx::Validated(facts))
            .is_err()
    );
    assert!(cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).is_err());
}

#[test]
fn unicode_scalar_literals_preserve_exact_source_ir_and_lowered_payloads() {
    for scalar in [
        '\0',
        '\n',
        '\'',
        '\\',
        'λ',
        '🦀',
        '\u{d7ff}',
        '\u{e000}',
        '\u{10ffff}',
    ] {
        let source = format!("fn value()->char{{{scalar:?}}} fn main(){{}}");
        let (program, plan) = plan(&source);
        assert!(matches!(program.functions[0].result, frontend::Type::Char));
        assert!(
            matches!(program.functions[0].body.tail.as_deref(), Some(frontend::Expr::Char(value)) if *value == scalar)
        );
        let (checked, facts) = typed(&program, &plan, "value");
        assert_eq!(checked.result, ir::Ty::Char);
        let literals: Vec<_> = checked
            .exprs
            .iter()
            .filter_map(|expr| {
                if let ir::ExprKind::Char(value) = expr.kind {
                    assert_eq!(expr.ty, ir::Ty::Char);
                    assert_eq!(expr.completion, ir::Completion::Returns);
                    Some(value.value)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(literals, [u32::from(scalar)]);
        let target = ir::CallTarget {
            declaration: declaration(&program, &plan, "value"),
            type_args: Arc::from([]),
        };
        let retained = plan.checked_body(&target, &facts, facts.budget()).unwrap();
        let template =
            cfg::lower_template(Arc::new(retained), facts.clone(), facts.budget()).unwrap();
        let lowered: Vec<_> = template
            .function()
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.value {
                cfg::Statement::Assign {
                    value: cfg::Rvalue::Use(cfg::Operand::Constant(cfg::Constant::Char(value))),
                    ..
                } => Some(value.value),
                _ => None,
            })
            .collect();
        assert_eq!(lowered, literals);
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
        lower(checked, facts);
    }
}

#[test]
fn malformed_scalars_types_and_completion_fail_in_reachable_and_unused_rows() {
    let (program, plan) = plan("fn value()->char{'λ'} fn main(){}");
    let (checked, facts) = typed(&program, &plan, "value");
    let original = checked
        .exprs
        .iter()
        .position(|expr| matches!(expr.kind, ir::ExprKind::Char(_)))
        .unwrap();
    for unused in [false, true] {
        for value in [0xd800, 0xdbff, 0xdc00, 0xdfff, 0x110000, u32::MAX] {
            let mut changed = checked.clone();
            let index = if unused {
                changed.exprs.push(checked.exprs[original].clone());
                changed.exprs.len() - 1
            } else {
                original
            };
            changed.exprs[index].kind = ir::ExprKind::Char(ir::CharValue { value });
            let error = changed
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .unwrap_err();
            assert!(
                error.contains("Unicode scalar"),
                "{value:x}, unused={unused}: {error}"
            );
            reject(changed, &facts);
        }
        for ty in [ir::Ty::Int(ir::IntTy::U32), ir::Ty::Bool, ir::Ty::Never] {
            let mut changed = checked.clone();
            let index = if unused {
                changed.exprs.push(checked.exprs[original].clone());
                changed.exprs.len() - 1
            } else {
                original
            };
            changed.exprs[index].ty = ty;
            reject(changed, &facts);
        }
        let mut changed = checked.clone();
        let index = if unused {
            changed.exprs.push(checked.exprs[original].clone());
            changed.exprs.len() - 1
        } else {
            original
        };
        changed.exprs[index].completion = ir::Completion::Never;
        reject(changed, &facts);
    }
}

#[test]
fn comparisons_are_bool_and_arithmetic_does_not_gain_integer_authority() {
    for operator in ["==", "!=", "<", "<=", ">", ">="] {
        let (program, plan) = plan(&format!(
            "fn compare()->bool{{'λ'{operator}'🦀'}} fn main(){{}}"
        ));
        let (checked, facts) = typed(&program, &plan, "compare");
        let index = checked
            .exprs
            .iter()
            .position(|expr| matches!(expr.kind, ir::ExprKind::Binary { .. }))
            .unwrap();
        assert_eq!(checked.exprs[index].ty, ir::Ty::Bool);
        assert_eq!(checked.exprs[index].completion, ir::Completion::Returns);
        lower(checked.clone(), facts.clone());
        let mut changed = checked.clone();
        changed.exprs[index].ty = ir::Ty::Char;
        reject(changed, &facts);
        let mut changed = checked;
        let ir::ExprKind::Binary { right, .. } = changed.exprs[index].kind else {
            unreachable!()
        };
        changed.exprs[right.0].ty = ir::Ty::Int(ir::IntTy::U32);
        changed.exprs[right.0].kind = ir::ExprKind::Int(ir::IntValue {
            ty: ir::IntTy::U32,
            bits: 0x1f980,
        });
        reject(changed, &facts);
        for expression in [
            format!("'λ'{operator}955u32"),
            format!("955u32{operator}'λ'"),
        ] {
            let source = format!("fn bad()->bool{{{expression}}} fn main(){{}}");
            assert!(
                frontend::parse(&source)
                    .and_then(|program| frontend::check(&program))
                    .is_err(),
                "{source}"
            );
        }
    }
    for operator in ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"] {
        for operands in [("'a'", "'b'"), ("'a'", "1u32"), ("1u32", "'a'")] {
            let source = format!(
                "fn bad()->char{{{}{operator}{}}} fn main(){{}}",
                operands.0, operands.1
            );
            assert!(
                frontend::parse(&source)
                    .and_then(|program| frontend::check(&program))
                    .is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn genuine_copy_clone_and_sized_support_char_without_numeric_coercion() {
    let (program, plan) = plan(
        r#"
fn twice<T:Copy>(x:T)->(T,T){(x,x)}
fn sized<T:Sized>(x:T)->T{x}
fn cloned()->char{let c='🦀';<char as Clone>::clone(&c)}
fn copied()->(char,char){twice::<char>('λ')}
fn sized_char()->char{sized::<char>('λ')}
fn bound()->char{match 'λ' { c => c }}
fn wildcard()->char{match 'λ' { _ => '🦀' }}
fn main(){}
"#,
    );
    for name in ["cloned", "copied", "sized_char", "bound", "wildcard"] {
        let (checked, facts) = typed(&program, &plan, name);
        assert!(facts.must_copy(&ir::Ty::Char).unwrap());
        facts.require_sized(&ir::Ty::Char).unwrap();
        lower(checked, facts);
    }
    for name in ["twice", "sized"] {
        let target = ir::CallTarget {
            declaration: declaration(&program, &plan, name),
            type_args: Arc::from([ir::Ty::Char]),
        };
        let instance = plan.instance_facts(&target).unwrap();
        instance.require_sized(&ir::Ty::Char).unwrap();
        assert!(instance.must_copy(&ir::Ty::Char).unwrap());
        // checked_body publishes rigid universal templates, not instances.
        // Validate the generic source with its actual declaration facts.
        let (retained, universal) = typed(&program, &plan, name);
        lower(retained, universal);
    }
    for source in [
        "fn bad()->u32{'λ'} fn main(){}",
        "fn bad()->char{955u32} fn main(){}",
    ] {
        assert!(
            frontend::parse(source)
                .and_then(|program| frontend::check(&program))
                .is_err()
        );
    }
}

#[test]
fn foreign_world_and_callable_metadata_cannot_authorize_char_bodies() {
    let source = "fn value()->char{'λ'} fn peer()->char{'🦀'} fn main(){}";
    let (program, plan) = plan(source);
    let (_, facts) = typed(&program, &plan, "value");
    let (_, peer) = typed(&program, &plan, "peer");
    let (foreign_program, foreign_plan) = self::plan(source);
    let (_, foreign) = typed(&foreign_program, &foreign_plan, "value");
    let target = ir::CallTarget {
        declaration: declaration(&program, &plan, "value"),
        type_args: Arc::from([]),
    };
    for wrong in [peer, foreign] {
        // Scalar-only raw IR has no declaration handles. The retained
        // publication boundary, not a leaf-type walk, binds the callable.
        assert!(plan.checked_body(&target, &wrong, wrong.budget()).is_err());
    }
    // Exact owner-preserving source clones remain usable.
    let clone = program.clone();
    let cloned =
        frontend::type_callable_with_plan(&clone, target.declaration, &plan, facts.budget())
            .unwrap();
    lower(cloned, facts);
}
