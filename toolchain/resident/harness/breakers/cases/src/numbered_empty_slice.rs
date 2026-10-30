//! library/coretests slice.rs, `test_slice_from_raw_parts_in_const`: an empty slice made in a
//! constant at an address that is a number keeps the number. ra-jit put the alignment of the
//! element there, as for an empty slice of an allocation of the evaluator's.

const EMPTY: &[i32] = unsafe { std::slice::from_raw_parts(std::ptr::without_provenance(123456), 0) };
const NOTHING: &() = unsafe { &*std::ptr::without_provenance(24) };
static FANCY: i32 = 4;
static OF_STATIC: &[i32] = unsafe { std::slice::from_raw_parts(&FANCY, 1) };

#[inline(never)]
fn addresses() -> (usize, usize, usize, bool) {
    let nothing: *const () = NOTHING;
    (EMPTY.as_ptr().addr(), EMPTY.len(), nothing.addr(), OF_STATIC.as_ptr() == &raw const FANCY)
}

pub fn run() -> (usize, usize, usize, bool) {
    addresses()
}
