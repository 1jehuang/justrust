// Found by fuzz.py: seed smoke-1, case c16, edition 2021, backend llvm.
// differs: checksum
// Reduced in 203 builds. Again: fuzz.py check differs-checksum-smoke-1-c16.rs
// #418, recovered with the CI binary at 41fddbbb58 and a store primed by the smoke run.
// Fixed by 2a3a0a35: LLVM's store key includes asm! text. fuzz/store.py covers the collision.
// rustc:
// c16 f58220087df0d8bc ok
// ra-jit:
// c16 36432f6a1cd4f5a3 ok
mod c16 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S2<A, B>(pub bool, pub Vec<A>, pub B, pub NonZeroU32);
    static mut K4: bool = false;
    static mut K5: u128 = (match 33u128.checked_mul(2u128) { Some(sum) => sum, None => 2u128 });
    pub trait T7 {
        fn m8(&self, a0: Option<Option<&'static str>>) -> u128;
        fn m9(&self) -> NonZeroU8;
        fn m10(&self, a11: NonZeroU32) -> S2<i128, i128> {
            ('b12: { if ((if (true) { (unsafe { K4 }) } else { (unsafe { K4 }) })) { break 'b12 apply_dyn_mut(&mut |x13: Option<Option<&'static str>>| -> S2<i128, i128> { S2(true, vec![(-16i128), (-20i128), (-170141183460469231731687303715884105727i128)], 16384i128, NonZeroU32::new(3930435971u32).unwrap()) }, None::<Option<&'static str>>); } S2(false, Vec::<i128>::new(), (-170141183460469231731687303715884105727i128), NonZeroU32::new(28u32).unwrap()) })
        }
    }
    impl T7 for S2<i128, i128> {
        fn m8(&self, a14: Option<Option<&'static str>>) -> u128 {
            let v15: u128 = 340282366920938463463374607431768211454u128.checked_rem((unsafe { K5 })).unwrap_or(162259276829213363391578010288128u128);
            u128::try_from((unsafe { K5 }).saturating_add(v15).wrapping_sub(1u128)).unwrap_or((!(unsafe { K5 })).wrapping_sub((4398046511105i64 as u128)))
        }
        fn m9(&self) -> NonZeroU8 {
            id::<NonZeroU8>(NonZeroU8::new(121u8).unwrap())
        }
    }
    impl T7 for Option<String> {
        fn m8(&self, a20: Option<Option<&'static str>>) -> u128 {
            u128::from(0u8)
        }
        fn m9(&self) -> NonZeroU8 {
                    #[cfg(target_arch = "x86_64")] #[inline(never)] fn neg(mut x: u64, mut y: u64) -> u64 { unsafe { std::arch::asm!("neg {x}", "not {y}", "add {x}, {y}", x = inout(reg) x, y = inout(reg) y => _, options(pure, nomem, nostack)); } x }
                    #[cfg(target_arch = "aarch64")] #[inline(never)] fn neg(mut x: u64, mut y: u64) -> u64 { unsafe { std::arch::asm!("neg {x}, {x}", "mvn {y}, {y}", "add {x}, {x}, {y}", x = inout(reg) x, y = inout(reg) y => _, options(pure, nomem, nostack)); } x }
                    for (x, y) in [(16u64, 55u64), (u64::MAX, 1), (0, 0), (0x8000_0000_0000_0000, 16)] { let _g = Noisy(x as u32 % 7); feed(neg(black_box(x), black_box(y))); }
            ({ let t50: u128 = (unsafe { K5 }); NonZeroU8::new(181u8).unwrap() })
        }
    }
    impl T7 for Option<Option<&'static str>> {
        fn m8(&self, a51: Option<Option<&'static str>>) -> u128 {
            330113465687290239270631319222850333234u128.checked_rem(222264325171135853868704583197091959809u128).unwrap_or(3u128).saturating_add((unsafe { K5 })).max(id::<u128>((unsafe { K5 })).saturating_mul((unsafe { K5 }).checked_neg().unwrap_or_default()))
        }
        fn m9(&self) -> NonZeroU8 {
            (if ((unsafe { K4 })) { NonZeroU8::new(9u8).unwrap() } else { NonZeroU8::new(1u8).unwrap() })
        }
    }
    pub fn run() {
        let mut v358: [bool; 8] = (if (true) { [false, true, false, false, false, true, false, true] } else { [false, true, true, true, false, true, false, false] });
                    let v454: [bool; 8] = v358;
                    let mut o457: Vec<Box<dyn T7>> = vec![Box::new(Some(format!("{:?}", Box::new(7u128)))), Box::new((if ((if (v454[0]) { true } else { true })) { (Box::new(Some(Some("x y"))) as Box<dyn T7>).m10(NonZeroU32::new(3u32).unwrap()) } else { (if (v358[0]) { S2(false, vec![16i128, 107718348388036513543327295542164395752i128, 4916669389749383889951547897575684629i128], 4611686018427387903i128, NonZeroU32::new(22u32).unwrap()) } else { S2(true, vec![16i128, 67067297349758122679465517661832781325i128, 14i128, (-170141183460469231731687303715884105728i128), 116228739023378439588321620915645880955i128], 100i128, NonZeroU32::new(131071u32).unwrap()) }) }))];
                    for o458 in o457.iter() { o458.m9().mix(); }
    }
}
