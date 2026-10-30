//! Found by the fuzzer (`fuzz/found/differs-panic-location-s1-1-c14.rs`): the column a panic
//! is reported at. rustc's span of an expression in parentheses starts at the parenthesis, so
//! an overflow in `(a + b) * c` is at the `(`; ra-jit reported the `a`. An index that is built
//! in (an array or a slice by a `usize`) is reported from where the indexed expression
//! starts, and only one through `Index` (a `Vec`, a range) from its bracket; ra-jit reported
//! the bracket for both. Parentheses a macro puts around what it was passed are not part of
//! the span, and a method call is reported from its name whatever is around it.

use std::hint::black_box;
use std::panic::{AssertUnwindSafe, catch_unwind};

macro_rules! passed {
    ($e:expr) => {
        $e
    };
}

macro_rules! wrapped {
    ($e:expr) => {
        ($e)
    };
}

/// Runs what panics and says whether it did; where it did is what the panic prints.
fn caught(f: impl FnOnce() -> u32) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_err()
}

#[rustfmt::skip]
#[allow(unused_parens, arithmetic_overflow, unconditional_panic)]
pub fn run() -> Vec<bool> {
    let (big, one, zero, far) = black_box((u32::MAX, 1u32, 0u32, 9usize));
    let array = black_box([1u32, 2, 3]);
    let vector = black_box(vec![1u32, 2, 3]);
    let slice: &[u32] = &array;
    let none: Option<u32> = black_box(None);
    vec![
        caught(|| big + one),
        caught(|| (big + one)),
        caught(|| ((big + one))),
        caught(|| 2 * (big + one)),
        caught(|| (big + one).count_ones()),
        caught(|| (-black_box(i32::MIN)) as u32),
        caught(|| (one / zero)),
        caught(|| (one << black_box(40u32))),
        caught(|| array[far]),
        caught(|| (array[far])),
        caught(|| [1u32, 2, 3][far]),
        caught(|| 2 * array[far]),
        caught(|| slice[far]),
        caught(|| (array, 1).0[far]),
        caught(|| array[far].count_ones()),
        caught(|| { let mut copy = array; (copy[far]) = 1; copy[0] }),
        caught(|| vector[far]),
        caught(|| (vector[far])),
        caught(|| 2 * vector[far]),
        caught(|| slice[1..far][0]),
        caught(|| none.unwrap()),
        caught(|| (none.unwrap())),
        caught(|| (none).unwrap()),
        caught(|| Option::unwrap(none)),
        caught(|| (Option::unwrap(none))),
        caught(|| passed!(big + one)),
        caught(|| passed!((big + one))),
        caught(|| wrapped!(big + one)),
        caught(|| (panic!("in parentheses"))),
        caught(|| ({ big + one })),
        caught(|| black_box((big + one))),
        caught(|| ((big + one), 1).0),
    ]
}
