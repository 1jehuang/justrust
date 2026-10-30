// Found by fuzz.py: seed s2-28, case c1, edition 2024, backend cranelift.
// differs: panic location
// Reduced in 56 builds. Again: fuzz.py check differs-panic-location-s2-28-c1.rs
// rustc:
// panic: panicked at src/main.rs:345:37:
// index out of bounds: the len is 3 but the index is 17764076402644541682
// c1 0000000000000000 ok
// ra-jit:
// panic: panicked at src/main.rs:345:24:
// index out of bounds: the len is 3 but the index is 17764076402644541682
// c1 0000000000000000 ok
mod c1 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub enum E2 {
        V0,
        V1,
    }
    pub fn run() {
        let r448 = catch_unwind(AssertUnwindSafe(|| -> E2 {
            if (({ let t449: u128 = [1u128, 25u128, 288230376151711744u128][17764076402644541682usize]; sel::<bool>(false, false, false) })) {
            }
            E2::V0
        }));
    }
}
