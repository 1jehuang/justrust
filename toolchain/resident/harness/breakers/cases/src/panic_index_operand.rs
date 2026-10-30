//! Found by the fuzzer (`fuzz/found/differs-panic-location-s2-3-c10.rs`): an index out of
//! bounds that is an operand of something else, an argument of a macro or of a call. rustc
//! reports the index expression; ra-jit reported the statement the index was read in, and
//! `<ra-jit>:0:0` when that statement was written by a macro.

use std::hint::black_box;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn caught(f: impl FnOnce() -> u32) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_err()
}

#[rustfmt::skip]
#[allow(unconditional_panic)]
pub fn run() -> Vec<bool> {
    let far = black_box(16usize);
    let items = black_box([1u32, 2, 3]);
    let held = black_box(true);
    vec![
        caught(|| { assert!(held, "held {}", [1u32, 2, 3][far]); 0 }),
        caught(|| { let value = [items[0], items[far], 7][0]; value }),
        caught(|| u32::max(items[far], 1)),
        caught(|| format!("{}", items[far]).len() as u32),
        caught(|| { let mut copy = items; copy[0] = copy[far]; copy[0] }),
    ]
}
