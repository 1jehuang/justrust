//! A crate whose integration test needs a dev-dependency (arc-swap's `random` test and its
//! proptest). `cratetest.py harness/breakers/devdep` runs it.

pub fn double(n: u32) -> u32 {
    n * 2
}
