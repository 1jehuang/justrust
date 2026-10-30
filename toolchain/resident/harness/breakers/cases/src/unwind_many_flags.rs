//! Found by the fuzzer on AArch64 (`fuzz/found/differs-died-A1-17-c23.rs`): a closure with
//! enough values that are dropped only on some paths that, once LLVM has optimized it, the
//! flags of the drops fill the registers across the call that panics. With the quick
//! selection after optimization, LLVM 20.1's fast register allocator gave one flag the
//! register the landing pad's selector arrives in and did not reload it, so the landing pad
//! dropped a vector that was never made and the program died in `free`.

use std::num::NonZeroI64;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn text(it: &str) -> String {
    String::from(it)
}

pub fn run() -> (bool, usize) {
    let mut rows: Vec<Vec<String>> = vec![vec![text("hello")]];
    let spare: Vec<Vec<String>> = vec![vec![text("0123456789")]];
    let caught = catch_unwind(AssertUnwindSafe(|| -> Vec<char> {
        rows = match (match text("a,b,,c")
            .chars()
            .map(|_| rows.clone())
            .map(|_| NonZeroI64::new(4).unwrap())
            .collect::<Vec<NonZeroI64>>()
            .as_slice()
        {
            [_first @ .., _last] => None::<Vec<Vec<String>>>,
            [_first, ..] => match vec![-4i8, -14i8, -127i8].as_slice() {
                [2i8..32i8, ..] => Some(vec![vec![text("3.5"), text("a,b,,c"), text("0123456789")]]),
                _ => None::<Vec<Vec<String>>>,
            },
            _ => Some(Some(vec![
                vec![text("  pad  "), text("-7"), text("0123456789")],
                vec![text("a")],
                vec![text("héllo"), text("  pad  ")],
            ]))
            .unwrap_or(Some(vec![
                vec![text("x y")],
                Vec::<String>::new(),
                vec![text("line\n"), text("x y")],
                vec![text("line\n")],
                Vec::<String>::new(),
            ])),
        }) {
            None => spare.clone(),
            Some(found) => found,
        };
        let held: bool = ({
            let mut items = vec![
                Vec::<Vec<String>>::new(),
                vec![
                    vec![text("a,b,,c"), text("😀"), text("ff")],
                    vec![text("0123456789"), text("Abc Def"), text("ff"), text("0123456789"), text("😀")],
                    Vec::<String>::new(),
                    vec![text("-7"), text("x y"), text("42")],
                    vec![text("x y"), text("  pad  ")],
                ],
            ];
            items.pop()
        })
        .is_some();
        assert!(!held, "held {}", 131073u32 + 268435455u32);
        vec!['a']
    }));
    (caught.is_err(), rows.len())
}
