//! The crate is named and none of its methods is used, as in downcast-rs's own tests: no symbol
//! of the link has the crate in it.
use blanket::AsAny;

#[allow(dead_code)]
trait Shape: AsAny {}

#[test]
fn links() {
    assert_eq!(std::mem::size_of::<&dyn Shape>(), 2 * std::mem::size_of::<usize>());
}
