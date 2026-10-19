//! Test the canonical publication boundary, not just structural IR typing.
use super::*;

#[test]
fn canonical_mutable_range_source_and_cfg_seal_exact_bounds() {
    let source =
        "fn view<'a>(s:&'a mut[i32],lo:usize,hi:usize)->&'a mut[i32]{&mut s[lo..hi]}fn main(){}";
    let program = parse(source).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let PublishedCallableBody::Source(publication) =
        plan.publish_callable_body(declaration, &budget).unwrap()
    else {
        panic!("source publication required")
    };
    let checked = publication.source();
    let index = checked
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ir::ExprKind::Subslice { .. }))
        .unwrap();
    publication
        .validate_source(checked, publication.facts(), &budget)
        .unwrap();
    for mode in 0..3 {
        let mut changed = checked.as_ref().clone();
        let ir::ExprKind::Subslice {
            start,
            end,
            inclusive,
            ..
        } = &mut changed.exprs[index].kind
        else {
            unreachable!()
        };
        if mode == 0 {
            std::mem::swap(start, end);
        } else if mode == 1 {
            *inclusive = !*inclusive;
        } else {
            *start = None;
        }
        assert!(
            publication
                .validate_source(&Arc::new(changed), publication.facts(), &budget)
                .is_err(),
            "canonical source mode{mode}"
        );
    }
    let lowered = super::super::cfg::lower_source_closed(&publication, &budget).unwrap();
    for mode in 0..3 {
        let mut changed = lowered.clone();
        let located = changed
            .blocks
            .iter_mut()
            .flat_map(|b| &mut b.statements)
            .find(|s| {
                matches!(
                    s.value,
                    super::super::cfg::Statement::Assign {
                        value: super::super::cfg::Rvalue::Subslice { .. },
                        ..
                    }
                )
            })
            .unwrap();
        if mode == 0 {
            located.origin = None;
        } else {
            let super::super::cfg::Statement::Assign {
                value:
                    super::super::cfg::Rvalue::Subslice {
                        start,
                        end,
                        inclusive,
                        ..
                    },
                ..
            } = &mut located.value
            else {
                unreachable!()
            };
            if mode == 1 {
                std::mem::swap(start, end.as_mut().unwrap());
            } else {
                *inclusive = !*inclusive;
            }
        }
        assert!(
            changed
                .validate_with_facts(ir::FactsCtx::Validated(publication.facts()))
                .is_err(),
            "canonical CFG mode{mode}"
        );
    }
}
