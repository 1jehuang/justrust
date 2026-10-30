// Found by fuzz.py: seed s2-2, case c27, edition 2024, backend cranelift.
// differs: panic location
// Reduced in 76 builds. Again: fuzz.py check differs-panic-location-s2-2-c27.rs
// rustc:
// panic: panicked at src/main.rs:344:39:
// index out of bounds: the len is 3 but the index is 10
// c27 0000000000000000 ok
// ra-jit:
// panic: panicked at src/main.rs:344:31:
// index out of bounds: the len is 3 but the index is 10
// c27 0000000000000000 ok
mod c27 {
    use super::*;
    pub fn run() {
        let r270 = catch_unwind(AssertUnwindSafe(|| -> i32 {
            match (&{ [([10u8, 127u8, 86u8, 127u8],), ([15u8; 4],), ([254u8, 254u8, 0u8, 6u8],)] }) {
                [p281, p280 @ .., ([p283, p282 @ .., _],)] => {
                    'l285: loop {
                        let mut v287: [u32; 1] = (if ((loop { break false; })) { [7u32] } else { [1u32] });
                        { let value = [3019780276u32.wrapping_sub(1024u32), 2295207998u32.wrapping_sub(134217727u32), (39u64 as u32)][usize::try_from((*p281).0[0]).unwrap()]; let raw = Box::into_raw(Box::new(v287)); unsafe { (*raw)[0] = value; Box::from_raw(raw).mix(); } }
                    }
                }
            }
        }));
    }
}
