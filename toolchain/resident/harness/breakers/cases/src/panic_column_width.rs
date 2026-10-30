//! Found by the fuzzer (`fuzz/found/differs-panic-location-s1-2-c13.rs`): the column of a
//! panic after characters that are not one column wide. rustc counts what a character takes
//! on a terminal: two for `中` and `😀`, none for a combining accent or a joiner, four for a
//! tab. ra-jit counted one for each.

use std::hint::black_box;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn caught(f: impl FnOnce() -> usize) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_err()
}

#[rustfmt::skip]
pub fn run() -> Vec<bool> {
    let big = black_box(usize::MAX);
    vec![
        caught(|| { let _ = "ab"; big + 1 }),
        caught(|| { let _ = "éb"; big + 1 }),
        caught(|| { let _ = "中b"; big + 1 }),
        caught(|| { let _ = "😀b"; big + 1 }),
        caught(|| { let _ = "éb"; big + 1 }),
        caught(|| { let _ = "a	b"; big + 1 }),
        caught(|| { let _ = "a‍b"; big + 1 }),
    ]
}
