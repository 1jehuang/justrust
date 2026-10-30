//! ra-jit loaded a workspace without its dev-dependencies, so nothing of `helper` resolved here.
//! A block that ended in the macro it could not expand was lowered as one that ends in
//! nothing, and `range` was compiled to return `()`: the test read a `Range` that was never
//! written, where arc-swap's test crashed.

use helper::range;

fn range(to: u32) -> helper::Range {
    range!(1, to)
}

#[test]
fn sums() {
    assert_eq!(range(5).sum(devdep::double), 20);
}

#[test]
fn empty() {
    assert_eq!(range(1).sum(devdep::double), 0);
}
