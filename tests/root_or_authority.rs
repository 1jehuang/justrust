//! Flat OR rows retain exact tuple domain/projections and ordinary checked facts.
use ir::{ExprKind, FactsCtx, MatchPattern, PatternTestKind, Projection};
use justrust_engine::compiler::{cfg, frontend, ir};

fn rows_mut(function: &mut ir::CheckedFunction) -> &mut Vec<Vec<ir::PatternTest>> {
    function
        .exprs
        .iter_mut()
        .find_map(|value| match &mut value.kind {
            ExprKind::Match { arms, .. } => {
                arms.iter_mut().find_map(|arm| match &mut arm.pattern {
                    MatchPattern::Alternatives { rows, .. } => Some(rows),
                    _ => None,
                })
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn malformed_later_rows_never_gain_ir_or_cfg_authority() {
    let program =
        frontend::parse("fn pick(p:(bool,u8))->u8{match p{(true,0)|(false,1)=>7,_=>0}}fn main(){}")
            .unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let original =
        frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan
        .instance_facts(&ir::CallTarget {
            declaration,
            type_args: [].into(),
        })
        .unwrap();
    original
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    cfg::lower_with_facts(&original, &facts, facts.budget()).unwrap();
    for mutation in 0..6 {
        let mut changed = original.clone();
        let rows = rows_mut(&mut changed);
        match mutation {
            0 => rows[1][0].projection = vec![Projection::Field(99)],
            1 => rows[1][0].projection = vec![Projection::Field(1)],
            2 => rows[1][0].projection = vec![Projection::Deref],
            3 => rows[1][0].kind = PatternTestKind::Char(ir::CharValue { value: 65 }),
            4 => rows.truncate(1),
            5 => {
                let duplicate = rows[1][0].clone();
                rows[1].push(duplicate);
            }
            _ => unreachable!(),
        }
        assert!(
            changed
                .validate_with_facts(FactsCtx::Validated(&facts))
                .is_err(),
            "mutation {mutation}"
        );
        assert!(
            cfg::lower_with_facts(&changed, &facts, facts.budget()).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn root_or_cfg_has_one_shared_body_and_ordered_row_cleanup() {
    let source = "fn pick(p:(bool,u8))->u8{match p{(true,0)|(false,1)=>7,_=>0}}fn main(){}";
    for workers in [1, 8] {
        let compilation = justrust_engine::compiler::compile(source.into(), workers, true).unwrap();
        assert!(!compilation.report.published_kind(103));
        assert_eq!(compilation.control_flow.len(), 2);
        assert!(
            compilation
                .analyses
                .iter()
                .all(|analysis| analysis.passed())
        );
        let pick = compilation
            .control_flow
            .iter()
            .find(|f| f.name.ends_with("pick"))
            .unwrap();
        let sevens = pick.blocks.iter().flat_map(|b| &b.statements).filter(|s| matches!(
            &s.value, cfg::Statement::Assign { value: cfg::Rvalue::Use(cfg::Operand::Constant(cfg::Constant::Int(v))), .. } if v.bits == 7
        )).count();
        assert_eq!(
            sevens, 1,
            "OR alternatives duplicated the selected source body"
        );
        for function in &compilation.control_flow {
            function.validate().unwrap();
        }
    }
}

#[test]
fn actual_outer_row_and_projection_capacity_is_paid_before_clone() {
    let program =
        frontend::parse("fn pick(p:bool)->u8{match p{true|false=>7}}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let original =
        frontend::type_callable_with_plan(&program, plan.function_decl(0).unwrap(), &plan, &budget)
            .unwrap();
    let mut enlarged = original.clone();
    let first = ir::PlanBudget::default();
    enlarged.preflight_plan(FactsCtx::Concrete, &first).unwrap();
    let base = first.usage_snapshot().bytes;
    let rows = rows_mut(&mut enlarged);
    let old = rows.capacity() * std::mem::size_of::<Vec<ir::PatternTest>>()
        + rows[0].capacity() * std::mem::size_of::<ir::PatternTest>()
        + rows[0][0].projection.capacity() * std::mem::size_of::<Projection>();
    rows.reserve_exact(128);
    rows[0].reserve_exact(128);
    rows[0][0].projection.reserve_exact(128);
    let new = rows.capacity() * std::mem::size_of::<Vec<ir::PatternTest>>()
        + rows[0].capacity() * std::mem::size_of::<ir::PatternTest>()
        + rows[0][0].projection.capacity() * std::mem::size_of::<Projection>();
    let second = ir::PlanBudget::default();
    enlarged
        .preflight_plan(FactsCtx::Concrete, &second)
        .unwrap();
    assert_eq!(second.usage_snapshot().bytes - base, new - old);
    let small = ir::PlanBudget::new(ir::PlanLimits {
        bytes: base + new - old - 1,
        ..ir::PlanLimits::default()
    });
    assert!(enlarged.preflight_plan(FactsCtx::Concrete, &small).is_err());
    assert!(small.first_resource_failure().is_some());
    assert!(original.preflight_plan(FactsCtx::Concrete, &small).is_err());
    let paid = second.usage_snapshot().bytes;
    drop(enlarged);
    assert_eq!(second.usage_snapshot().bytes, paid);
}
