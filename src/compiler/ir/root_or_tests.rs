use super::*;
#[test]
fn root_or_entry_preserves_prior_bytes_and_work_failure_on_retry() {
    for kind in [ResourceBudgetKind::Bytes, ResourceBudgetKind::Work] {
        let budget = PlanBudget::new(PlanLimits {
            bytes: 0,
            work: 100,
            ..PlanLimits::default()
        });
        match kind {
            ResourceBudgetKind::Bytes => assert!(budget.reserve_bytes(1).is_err()),
            ResourceBudgetKind::Work => assert!(budget.charge_work(101).is_err()),
            _ => unreachable!(),
        }
        let before = budget.usage_snapshot();
        let first = before.first_failure.unwrap();
        assert_eq!(first.kind, kind);
        // Both failures leave enough work for these otherwise valid inputs.
        assert_eq!(before.work, 0);
        for _ in 0..2 {
            assert_eq!(
                validate_alternative_domain(&Ty::Bool, Some(&budget)).unwrap_err(),
                "planning resource budget already exhausted"
            );
            assert_eq!(
                validate_alternative_rows(&Ty::Bool, &[Vec::new(), Vec::new()], Some(&budget))
                    .unwrap_err(),
                "planning resource budget already exhausted"
            );
            assert_eq!(budget.first_resource_failure(), Some(first));
            assert_eq!(budget.usage_snapshot().work, before.work);
            assert_eq!(budget.usage_snapshot().bytes, before.bytes);
        }
    }
}
fn rows() -> Vec<Vec<PatternTest>> {
    [true, false]
        .into_iter()
        .map(|value| {
            vec![
                PatternTest {
                    projection: vec![Projection::Field(0)],
                    kind: PatternTestKind::Bool(value),
                },
                PatternTest {
                    projection: vec![Projection::Field(1)],
                    kind: PatternTestKind::Int(IntValue {
                        ty: IntTy::U8,
                        bits: u128::from(value),
                    }),
                },
            ]
        })
        .collect()
}
#[test]
fn root_or_row_work_counts_pairs_projections_and_headers_cumulatively() {
    let ty = Ty::Tuple(vec![Ty::Bool, Ty::Int(IntTy::U8)].into());
    let rows = rows();
    let budget = PlanBudget::default();
    validate_alternative_rows(&ty, &rows, Some(&budget)).unwrap();
    assert_eq!(budget.usage_snapshot().work, 23);
    let tiny = PlanBudget::new(PlanLimits {
        work: 22,
        ..PlanLimits::default()
    });
    assert!(validate_alternative_rows(&ty, &rows, Some(&tiny)).is_err());
    assert!(tiny.first_resource_failure().is_some());
    assert!(validate_alternative_rows(&ty, &[Vec::new(), Vec::new()], Some(&tiny)).is_err());
}
#[test]
fn root_or_empty_row_headers_are_not_free_authority() {
    let rows = vec![
        Vec::new(),
        vec![PatternTest {
            projection: Vec::new(),
            kind: PatternTestKind::Bool(true),
        }],
    ];
    let budget = PlanBudget::default();
    validate_alternative_rows(&Ty::Bool, &rows, Some(&budget)).unwrap();
    assert_eq!(budget.usage_snapshot().work, 5);
    // Typed conjunction validity is not source correspondence. Only the
    // private canonical checked-body seal authenticates an empty source leaf.
}
