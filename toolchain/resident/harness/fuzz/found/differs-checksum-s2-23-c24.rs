// Found by fuzz.py: seed s2-23, case c24, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 101 builds. Again: fuzz.py check differs-checksum-s2-23-c24.rs
// rustc:
// panic: panicked at src/main.rs:366:271:
// called `Option::unwrap()` on a `None` value
// c24 96891f8561ae43eb ok
// ra-jit:
// panic: panicked at src/main.rs:366:271:
// called `Option::unwrap()` on a `None` value
// c24 b3d5124b1345f240 ok
mod c24 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub enum E1 {
        V0(f32, NonZeroU8),
    }
    impl Mix for E1 {
        fn mix(&self) {
            match self {
                E1::V0(a0, a1) => { feed(10); a0.mix(); a1.mix(); }
            }
        }
    }
    impl Drop for E1 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    const K2: i128 = 10i128;
    static K3: bool = false;
    pub trait T4 {
        fn m5(&self, a0: E1) -> [Box<NonZeroU32>; 4];
    }
    impl T4 for E1 {
        fn m5(&self, a6: E1) -> [Box<NonZeroU32>; 4] {
            [Box::new(NonZeroU32::new(1u32).unwrap()), Box::new(NonZeroU32::new(262143u32).unwrap()), Box::new(NonZeroU32::new(5u32).unwrap()), Box::new(NonZeroU32::new(2u32).unwrap())]
        }
    }
    pub fn f7(a8: f32, a9: f32, a10: f32, a11: bool, a12: E1, a13: f32, a14: i128, a15: [E1; 8], a16: &dyn T4, a17: i128) -> bool {
        let r18 = catch_unwind(AssertUnwindSafe(|| -> E1 {
                let mut v29: [E1; 8] = [E1::V0((-9223372036854775808.0f32), NonZeroU8::new(128u8).unwrap()), E1::V0(0.2f32, NonZeroU8::new(2u8).unwrap()), pick::<E1>(K3, &a12.clone(), &a12.clone()).clone(), E1::V0(174.7038f32, NonZeroU8::new(27u8).unwrap()), None::<E1>.unwrap(), vec![E1::V0(1e-40f32, NonZeroU8::new(1u8).unwrap()), E1::V0(0.5f32, NonZeroU8::new(124u8).unwrap()), E1::V0(3.25f32, NonZeroU8::new(127u8).unwrap())][2305843009213693952usize].clone(), apply_dyn(&|x32: bool| -> E1 { a12.clone() }, false), apply_dyn_mut(&mut |x33: [(bool, char); 1]| -> E1 { a12.clone() }, [(false, '0'); 1])];
            apply_dyn_mut(&mut |x46: Vec<[char; 2]>| -> E1 { E1::V0((-461.1561f32), NonZeroU8::new(8u8).unwrap()) }, vec![['\u{80}', 'Z'], ['\u{7f}', '\u{80}'], ['\''; 2], ['a', '\\']])
        }));
        false
    }
    pub fn run() {
        let [p119, p120, _, p121]: [u8; 4] = [221u8.wrapping_sub(91u8), 1u8.clamp(64u8, 85u8), 25u8.clamp(16u8, 254u8), 4u8];
        let [[p124, p125, _]]: [[bool; 3]; 1] = [[true; 3]; 1];
        match (pick::<(Option<()>, [i128; 2])>(p124, &(None::<()>, [13i128, 7i128]), &(Some(()), [3i128, (-12i128)])).clone()) {
            (_, p159) => {
                let v160: bool = f7((-2.5e-3f32).mul_add((-7.0f32), 0.1f32), (-(700.9669f64 as f32)), "inf".parse::<f32>().unwrap_or(sel::<f32>(p125, 3.25f32, (-2.5e-3f32))), (pick::<Box<[bool; 2]>>(K3, &Box::new([false; 2]), &Box::new([false; 2])).clone() == (match (Vec::<[Box<NonZeroU32>; 4]>::new().as_slice()) { [] if false => Box::new([false, false]), [[p163, p164, p165, p166], .., p167] => Box::new([false, false]), _ => Box::new([false, true]) })), E1::V0((4u32 as f32), NonZeroU8::new(p121).unwrap_or(NonZeroU8::new(8u8).unwrap())), (59222u16 as f32).clamp(-1.5f32, 2.5f32), (unsafe { *(&K2 as *const i128) }).swap_bytes(), [E1::V0(1e30f32, NonZeroU8::new(152u8).unwrap()), E1::V0(1e30f32, NonZeroU8::new(7u8).unwrap()), E1::V0(4294967296.0f32, NonZeroU8::new(7u8).unwrap()), E1::V0(0.2f32, NonZeroU8::new(33u8).unwrap()), E1::V0(f32::NAN, NonZeroU8::new(15u8).unwrap()), E1::V0((-2.5e-3f32), NonZeroU8::new(17u8).unwrap()), E1::V0(0.5f32, NonZeroU8::new(132u8).unwrap()), E1::V0((-128.9f32), NonZeroU8::new(10u8).unwrap())], &(vec![(-1.0f32), (-1.0f32), (-128.9f32), 1e10f32, (-1.0f32)], E1::V0(0.2f32, NonZeroU8::new(20u8).unwrap()), (263199038672797030810441640593950499469u128, false)).1, (1e30f32 as i128).to_be());
            }
        }
    }
}
