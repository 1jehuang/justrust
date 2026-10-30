//! What the 2021 edition does differently, for the cases that are about it.

use std::cell::RefCell;

struct Noisy<'a>(u8, &'a RefCell<Vec<u8>>);

impl Drop for Noisy<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

fn mark(n: u8, log: &RefCell<Vec<u8>>) {
    log.borrow_mut().push(n);
}

/// tests/ui/drop/drop-order-comparisons.rs: before the 2024 edition the temporaries of what
/// an `if let` matches on live to the end of the statement the `if` is in, and those of an
/// `if let` at the end of a branch to the end of that branch.
#[inline(never)]
pub fn if_let_outlives(log: &RefCell<Vec<u8>>) {
    _ = (
        if let Some(_) = Some(&Noisy(4, log)) {
            mark(1, log);
        },
        mark(2, log),
        Noisy(3, log),
    );
    _ = (
        if let Some(_) = None.or(Some(&Noisy(8, log))).filter(|_| false) {
            mark(0, log);
        } else {
            mark(5, log);
        },
        mark(6, log),
        Noisy(7, log),
    );    _ = {
        if let Some(_) = Some(Noisy(11, log)) {
            if let Some(_) = Some(Noisy(10, log)).as_ref() {
                mark(9, log);
            }
        }
    };
}

/// Makes a macro that says whether what it is given is of the named kind of fragment.
#[macro_export]
macro_rules! make_matcher {
    ($name:ident, $fragment:ident, $d:tt) => {
        macro_rules! $name {
            ($d _:$fragment) => {
                true
            };
            (const { 0 }) => {
                false
            };
            (A | B) => {
                false
            };
        }
    };
}

pub mod for_tail_temporaries;
pub mod if_branch_temporaries;
pub mod if_let_temporaries;
pub mod unwind_operand_tail;
pub mod unwind_tail_after_local;

/// A macro that panics over how its input is spelled, here so that the function left to
/// rustc for it is alone in its crate.
pub mod macro_reads_its_spelling {
    #[derive(literals::NamedByText)]
    struct Unit;

    #[inline(never)]
    pub fn name() -> &'static str {
        Unit::name()
    }
}

/// An `if` passed as an expression fragment and then a closure, here so that the function
/// left to rustc for it is alone in its crate.
pub mod fragment_ends_statement {
    #[inline(never)]
    pub fn closure_after_fragment() -> u32 {
        macro_rules! then_closure {
            ($e:expr) => {{
                let add: fn(u32) -> u32 = {
                    $e
                    |x| x + 1
                };
                add(1)
            }};
        }
        then_closure!(if true {})
    }
}
