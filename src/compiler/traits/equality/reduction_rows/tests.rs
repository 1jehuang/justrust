use super::*;

fn row(snapshot: SnapshotId, index: u64) -> DeclaredEqualityReduction {
    let world = WorldId(snapshot);
    DeclaredEqualityReduction {
        world,
        stage: DeclaredStageId(world),
        declaration: DeclId { snapshot, index: 0 },
        projection: ir::AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: DeclId { snapshot, index: 1 },
                self_ty: Ty::Bool,
                args: Arc::from([]),
            },
            item: DeclId { snapshot, index },
        },
        rhs: Ty::Bool,
    }
}

#[test]
fn declared_reduction_rows_move_then_append_preserves_order_duplicates_and_peak() {
    let budget = PlanBudget::default();
    let snapshot = SnapshotId::standalone().unwrap();
    let mut first = ReductionRows::default();
    first.push(row(snapshot, 2), &budget).unwrap();
    let pointer = first.values.as_ptr();
    let receipt = first.storage.as_ref().unwrap().bytes();
    let mut frozen = ReductionRows::default();
    frozen.append(first, &budget).unwrap();
    assert_eq!(frozen.values.as_ptr(), pointer);
    assert_eq!(frozen.storage.as_ref().unwrap().bytes(), receipt);
    let mut next = ReductionRows::default();
    next.push(row(snapshot, 3), &budget).unwrap();
    next.push(row(snapshot, 2), &budget).unwrap();
    let before = budget.usage_snapshot().bytes;
    frozen.append(next, &budget).unwrap();
    assert_eq!(
        frozen
            .values
            .iter()
            .map(|r| r.projection.item.index)
            .collect::<Vec<_>>(),
        [2, 3, 2]
    );
    let bytes = ReductionRows::bytes(frozen.values.capacity(), &budget).unwrap();
    assert!(frozen.storage.as_ref().unwrap().bytes() >= bytes);
    assert!(budget.usage_snapshot().reservations.peak_bytes >= before + bytes);
    frozen.check(&budget).unwrap();
}

#[test]
fn declared_reduction_rows_underpaid_replacement_is_transactional_and_recoverable() {
    let budget = PlanBudget::default();
    let snapshot = SnapshotId::standalone().unwrap();
    let mut rows = ReductionRows::default();
    rows.push(row(snapshot, 2), &budget).unwrap();
    let pointer = rows.values.as_ptr();
    let capacity = rows.values.capacity();
    let paid = rows.storage.as_ref().unwrap().bytes();
    let mut replacement = Vec::new();
    replacement.try_reserve_exact(capacity + 1).unwrap();
    let underpaid = budget.reserve_bytes(0).unwrap();
    assert!(
        rows.commit_replacement(replacement, underpaid, &budget)
            .is_err()
    );
    assert_eq!(rows.values.as_ptr(), pointer);
    assert_eq!(rows.values.capacity(), capacity);
    assert_eq!(rows.values.len(), 1);
    assert_eq!(rows.storage.as_ref().unwrap().bytes(), paid);
    rows.check(&budget).unwrap();
    rows.push(row(snapshot, 3), &budget).unwrap();
    assert_eq!(rows.values.len(), 2);
}

#[test]
fn declared_reduction_rows_failed_growth_and_poison_do_not_commit() {
    let budget = PlanBudget::new(ir::PlanLimits {
        bytes: 2 * size_of::<DeclaredEqualityReduction>(),
        ..Default::default()
    });
    let snapshot = SnapshotId::standalone().unwrap();
    let mut rows = ReductionRows::default();
    rows.push(row(snapshot, 2), &budget).unwrap();
    let pointer = rows.values.as_ptr();
    let capacity = rows.values.capacity();
    assert!(rows.push(row(snapshot, 3), &budget).is_err());
    assert!(budget.has_resource_failure());
    assert_eq!(rows.values.as_ptr(), pointer);
    assert_eq!(rows.values.capacity(), capacity);
    assert_eq!(rows.values.len(), 1);
    assert!(rows.check(&budget).is_err());
    assert!(ReductionRows::default().append(rows, &budget).is_err());
}

#[test]
fn declared_reduction_rows_reject_missing_receipt_foreign_payer_and_sticky_work() {
    let budget = PlanBudget::default();
    let other = PlanBudget::new(budget.limits());
    let snapshot = SnapshotId::standalone().unwrap();
    let mut rows = ReductionRows::default();
    rows.push(row(snapshot, 2), &budget).unwrap();
    assert!(rows.check(&other).is_err());
    assert!(ReductionRows::default().append(rows, &other).is_err());
    let mut rows = ReductionRows::default();
    rows.push(row(snapshot, 2), &budget).unwrap();
    let receipt = rows.storage.take();
    assert!(rows.check(&budget).is_err());
    rows.storage = receipt;
    assert!(budget.charge_work(usize::MAX).is_err());
    assert!(rows.push(row(snapshot, 3), &budget).is_err());
    assert_eq!(rows.values.len(), 1);
    assert!(ReductionRows::default().check(&budget).is_err());
}
