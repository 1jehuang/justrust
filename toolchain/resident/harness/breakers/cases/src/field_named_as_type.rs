//! rand_xorshift imports `Wrapping` as `w`, and its generator has a field `w` of that type.

use core::num::Wrapping as w;

#[derive(Clone, PartialEq, Eq, Debug)]
struct Generator {
    x: w<u32>,
    w: w<u32>,
}

pub fn run() -> (u32, u32, bool) {
    let first = Generator { x: w(1), w: w(u32::MAX) };
    let second = first.clone();
    ((second.x + second.w).0, second.w.0, first == second)
}
