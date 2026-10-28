//! Verifies the actual SUMMARY request-controller source, not a copied model.
//! Run with the pinned Verus version through check_summary.py.
#![allow(unexpected_cfgs)]

use vstd::prelude::*;

#[path = "../../src/compiler/summary_node/requests.rs"]
mod requests;

#[verus_spec]
fn one_request_then_done(instance: &[u64]) {
    let mut requests = requests::Requests::new(instance);
    let first = requests.next();
    let second = requests.next();
    let third = requests.next();
    proof! {
        assert(first.is_some());
        assert(first.unwrap()@ == instance@);
        assert(second.is_none());
        assert(third.is_none());
    }
}

fn main() {}
