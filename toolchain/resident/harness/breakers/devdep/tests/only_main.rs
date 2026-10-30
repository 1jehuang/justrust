//! A criterion bench listed as a test too, as ahash's `tests/bench.rs` is, the bench named as
//! the library. The test harness has a `main` of its own and leaves this one unused; ra-jit
//! took the bench for the library's crate, a program rustc built nothing of, and wrote the C
//! `main` to call this one.
fn main() {
    println!("not the main of the test harness");
    std::process::exit(3);
}
