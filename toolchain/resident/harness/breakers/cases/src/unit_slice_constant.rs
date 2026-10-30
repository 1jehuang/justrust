//! library/coretests slice.rs: `&[(); usize::MAX]` is a slice in a constant, and a reference
//! to that constant made ra-jit go over each of the elements looking for addresses to carry
//! over, which with that many of no size never ended.

const UNITS: &[()] = &[(); usize::MAX];

#[inline(never)]
fn length(units: &&[()]) -> usize {
    units.len()
}

#[inline(never)]
fn inside(units: &Option<&[()]>) -> Option<usize> {
    units.map(|units| units.len())
}

pub fn run() -> (usize, Option<usize>) {
    (length(&UNITS), inside(&Some(UNITS)))
}
