use resident_macro_fixture::counter;

pub const FIRST: (usize, usize) = counter!();
pub const SECOND: (usize, usize) = counter!();
const _: () = {
    assert!(FIRST.0 == 1 && FIRST.1 == 1);
    assert!(SECOND.0 == 2 && SECOND.1 == 2);
};
