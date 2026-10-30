//! Found by the fuzzer: what `assert!` says when it fails. The message is the condition as
//! rustc's pretty printer writes it back, which breaks a condition longer than a line over
//! several lines and indents them. ra-jit writes the condition on one line. Open: it needs
//! rustc's printer (`rustc_ast_pretty`), which is not a crate ra_jit can depend on.

use std::hint::black_box;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub fn run() -> String {
    let items = black_box(vec![228892998618067312720644583277788876762u128, 12u128, 22u128]);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        assert!(items < ({ let mut sorted = vec![4398046511104u128, 340282366920938463463374607431768211455u128]; sorted.sort_unstable(); sorted }));
    }));
    match caught {
        Err(payload) => payload.downcast_ref::<&str>().map_or(String::new(), |it| it.to_string()),
        Ok(()) => String::new(),
    }
}
