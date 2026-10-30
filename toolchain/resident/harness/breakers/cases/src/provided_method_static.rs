//! bevy's `ScheduleLabel::intern`: `INTERNER` is a static in the body of a method a trait
//! provides, and functions left to rustc reach it, so ours has to be rustc's copy. The path
//! of a static's owner was only spelled for a method of an impl, so no symbol was found for
//! it, and every function of ours that interns a label (bevy's `main`) was left to rustc.
//! Here `rustcs::counted` is left to rustc for the constant `caller_in_constant` is about.
//! bevy's trait is written by `define_label!`, as `Labelled` is here.

use std::panic::Location;
use std::sync::atomic::{AtomicUsize, Ordering};

pub trait Counted {
    fn intern(&self) -> usize
    where
        Self: Sized,
    {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        COUNT.fetch_add(1, Ordering::Relaxed)
    }
}

macro_rules! define_label {
    ($name:ident) => {
        pub trait $name {
            fn intern(&self) -> usize
            where
                Self: Sized,
            {
                static INTERNER: AtomicUsize = AtomicUsize::new(0);
                INTERNER.fetch_add(1, Ordering::Relaxed)
            }
        }
    };
}

define_label!(Labelled);

pub struct Label;

impl Counted for Label {}

impl Labelled for Label {}

pub mod rustcs {
    use super::*;

    #[inline(never)]
    pub fn counted() -> (usize, usize) {
        const CALLER: &Location<'static> = Location::caller();
        let line = usize::from(CALLER.line() == 0);
        (Counted::intern(&Label) + line, Labelled::intern(&Label) + line)
    }
}

pub fn run() -> [(usize, usize); 3] {
    let ours = || (Counted::intern(&Label), Labelled::intern(&Label));
    [ours(), rustcs::counted(), ours()]
}
