#![cfg(feature = "inventory")]

//! A query with cycle handling and an `lru`: a memo whose value was evicted, verified again by
//! a query that depends on it, is executed again when its value is asked for, not taken for a
//! cycle head that panicked (blaze's patch to salsa 0.28.5, blaze #261).

use salsa::{Database, Durability, Setter};

#[salsa::input]
struct Number {
    value: u32,
}

#[salsa::input]
struct Unrelated {
    value: u32,
}

#[salsa::tracked(returns(copy), cycle_result = doubled_cycle_result, lru = 1)]
fn doubled(db: &dyn Database, number: Number) -> u32 {
    number.value(db) * 2
}

fn doubled_cycle_result(_db: &dyn Database, _id: salsa::Id, _number: Number) -> u32 {
    0
}

#[salsa::tracked(returns(copy))]
fn doubled_plus_one(db: &dyn Database, number: Number) -> u32 {
    doubled(db, number) + 1
}

#[test]
fn evicted_value_is_executed_again() {
    let mut db = salsa::DatabaseImpl::default();
    let (one, two) = (Number::new(&db, 1), Number::new(&db, 2));
    let unrelated = Unrelated::new(&db, 0);
    assert_eq!(doubled_plus_one(&db, one), 3);
    assert_eq!(doubled_plus_one(&db, two), 5);
    // A new revision evicts all but the last `doubled` asked for.
    db.synthetic_write(Durability::LOW);
    // Verifies `doubled(one)`, whose value is gone, without executing it.
    assert_eq!(doubled_plus_one(&db, one), 3);
    // The memo was verified in this revision and has no value: it is executed again.
    assert_eq!(doubled(&db, one), 2);
    unrelated.set_value(&mut db).to(1);
    assert_eq!(doubled_plus_one(&db, two), 5);
    assert_eq!(doubled(&db, two), 4);
}
