// The standard library vendors foldhash 0.2.0 too, and rust-analyzer's graph of it has that
// copy, which rustc does not build and which sees no `core`: `--crate foldhash` is this one.
use core::hash::BuildHasher;

fn main() {
    println!("{}", foldhash::fast::FixedState::with_seed(7).hash_one(42u64));
}
