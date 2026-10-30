// Found by fuzz.py: seed S1-2, case c7, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 59 builds. Again: fuzz.py check differs-checksum-S1-2-c7.rs
// rustc:
// panic: panicked at src/main.rs:362:9
// c7 d39e62595e590c78 ok
// ra-jit:
// panic: panicked at src/main.rs:362:9
// c7 6209e2595e58e1d6 ok
mod c7 {
    use super::*;
    pub fn run() {
            for armed in [false, true] { caught(|| { let _v = { let _local = Noisy(57); (Noisy(58), trip(armed, Noisy(47))) }; }); }
    }
}
