// Found by fuzz.py: seed L2-154, case c15, edition 2021, backend cranelift.
// differs: panic location
// Reduced in 55 builds. Again: fuzz.py check differs-panic-location-L2-154-c15.rs
// rustc:
// panic: panicked at src/main.rs:340:23:
// index out of bounds: the len is 3 but the index is 5725960416961799095
// c15 0000000000000000 ok
// ra-jit:
// panic: panicked at src/main.rs:344:57:
// gave up at 15
// c15 0000000000000000 ok
mod c15 {
    use super::*;
    pub fn run() {
            let r70 = catch_unwind(AssertUnwindSafe(|| -> ([i128; 2], [isize; 1], (usize, u128, u64, i8)) {
                match ([5i8, 1i8, 12i8][5725960416961799095usize]) {
                    _ => {
                    }
                }
                let v75: bool = id::<bool>((if (true) { panic!("gave up at {}", 15u32) } else { true }));
                ([18i128, 1i128], (if (true) { [(-159190607391289555isize)] } else { [(-2103020517773012019isize); 1] }), (0usize, 293806303916579336202876889566461182704u128, 549755813887u64, (-10i8)))
            }));
    }
}
