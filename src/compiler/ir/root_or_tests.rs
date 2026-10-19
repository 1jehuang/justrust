use super::*;
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
