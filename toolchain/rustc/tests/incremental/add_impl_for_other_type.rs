// Adding a trait impl for one local type while other functions select impls
// of the same trait for other types.
//
// Candidate assembly reads the impls that are not direct impls for a local
// ADT (`trait_impls_excluding_direct`) plus the direct impls for the self
// type's own ADT (`local_direct_impls_of_adt`), so that one new `#[derive]`
// does not re-run type checking of every function that uses the trait. This
// test checks that the split finds the right impls across sessions: both
// the old ones and the one added in the second session. `rustc_clean` cannot
// tell a re-run with an unchanged result from a reused one, so it does not
// check that the re-run is avoided.

//@ revisions: bpass1 bpass2
//@ compile-flags: -Z query-dep-graph
//@ ignore-backends: gcc

#![feature(rustc_attrs)]
#![allow(dead_code)]
#![crate_type = "rlib"]

pub trait Describe {
    fn describe(&self) -> u32;
}

pub struct First;
pub struct Second;

#[derive(Clone, Debug, PartialEq)]
pub struct Third(u32);

impl Describe for First {
    fn describe(&self) -> u32 {
        1
    }
}

impl Describe for Third {
    fn describe(&self) -> u32 {
        self.0
    }
}

// In a module of its own: an impl added at the crate root would renumber the
// derived impls of `Third` there (`{impl#N}`), which changes them for real.
#[cfg(bpass2)]
mod second_impl {
    impl super::Describe for super::Second {
        fn describe(&self) -> u32 {
            2
        }
    }
}

#[cfg(bpass2)]
#[derive(Clone, Debug, PartialEq)]
pub struct Fourth(u32);

fn takes_describe<T: Describe>(value: &T) -> u32 {
    value.describe()
}

#[rustc_clean(cfg = "bpass2")]
pub fn uses_first() -> u32 {
    takes_describe(&First) + First.describe()
}

#[rustc_clean(cfg = "bpass2")]
pub fn uses_third() -> bool {
    let third = Third(3);
    let copy = third.clone();
    format!("{:?}", copy).len() > takes_describe(&third) as usize && third == copy
}

#[cfg(bpass2)]
pub fn uses_second() -> u32 {
    takes_describe(&Second)
}
