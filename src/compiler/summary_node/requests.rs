//! One-shot direct dependency protocol for the actual SUMMARY producer.
//!
//! Verus checks this same file via experiments/dependency_proof/summary.rs.
//! This proves the request protocol, not the async engine adapter, SSA summary
//! algorithm, necessity of the input, or transitive minimality of its producers.
//! No verifier dependency is needed for ordinary Rust compilation.
#![allow(unexpected_cfgs)]

#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg_attr(verus_keep_ghost, verus_verify)]
pub(super) struct Requests<'a> {
    instance: &'a [u64],
    requested: bool,
}

#[cfg(verus_keep_ghost)]
verus! {
impl Requests<'_> {
    pub(super) closed spec fn words(&self) -> Seq<u64> { self.instance@ }
    pub(super) closed spec fn issued(&self) -> bool { self.requested }
}
}

#[cfg_attr(verus_keep_ghost, verus_verify)]
impl<'a> Requests<'a> {
    #[cfg_attr(verus_keep_ghost, verus_spec(result =>
        ensures result.words() == instance@, !result.issued(),
    ))]
    pub(super) fn new(instance: &'a [u64]) -> Self {
        Self {
            instance,
            requested: false,
        }
    }

    #[cfg_attr(verus_keep_ghost, verus_spec(result =>
        ensures
            final(self).words() == old(self).words(),
            final(self).issued(),
            match result {
                Some(words) => !old(self).issued() && words@ == old(self).words(),
                None => old(self).issued(),
            },
    ))]
    pub(super) fn next(&mut self) -> Option<&'a [u64]> {
        if self.requested {
            None
        } else {
            self.requested = true;
            Some(self.instance)
        }
    }
}
