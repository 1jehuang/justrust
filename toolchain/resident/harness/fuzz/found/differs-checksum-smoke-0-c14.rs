// Found by fuzz.py: seed smoke-0, case c14, edition 2024, backend cranelift.
// differs: checksum
// Reduced in 48 builds. Again: fuzz.py check differs-checksum-smoke-0-c14.rs
// rustc:
// panic: panicked at src/main.rs:362:9
// c14 31bf21542c5d8c60 ok
// rustd:
// panic: panicked at src/main.rs:362:9
// c14 7f91f6d436cf62dd ok
mod c14 {
    use super::*;
    pub fn run() {
            for armed in [false, true] { caught(|| { first_of(&Noisy(60), &trip(armed, Noisy(57))).mix(); }); }
    }
}
