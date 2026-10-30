// Found by fuzz.py: seed land2-rpa-85, case c26, edition 2021, backend llvm.
// differs: died
// Reduced in 89 builds. Again: fuzz.py check differs-died-land2-rpa-85-c26.rs
// ra-jit's program ended with status -11 in this case
// 
mod c26 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub enum E1<A> {
        V0((), (A, u8), u128),
    }
    impl<A: Mix> Mix for E1<A> {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub enum E2 {
        V0,
        V1,
        V2,
        V3,
    }
    impl Mix for E2 {
        fn mix(&self) {
        }
    }
    static mut K3: f64 = (-128.9f64);
    static mut K4: i128 = (std::mem::size_of::<Vec<E1<NonZeroI64>>>() as i128);
    pub trait T5 {
        fn m6(&self, a0: i128, a1: i128) -> E2;
        fn m7(&self, a0: Vec<E1<NonZeroI64>>) -> E2;
    }
    impl T5 for NonZeroI64 {
        fn m6(&self, a8: i128, a9: i128) -> E2 {
            (if let Some(x10) = (({ let mut items = vec![E2::V1, E2::V1, E2::V3, E2::V1, E2::V0]; items.pop() })) { x10 } else { (|x12: f32| -> E2 { E2::V0 })(f32::MAX) })
        }
        fn m7(&self, a13: Vec<E1<NonZeroI64>>) -> E2 {
            (if (false) { (if (false) { E2::V1 } else { E2::V2 }) } else { E2::V3 })
        }
    }
    #[inline(never)]
    pub fn f87() -> E1<NonZeroI64> {
        let mut v88: char = (if (true) { (if (false) { '0' } else { '\u{80}' }) } else { apply_boxed(Box::new(|| -> char { '\u{10FFFF}' })) });
        (match (vec![11i8].as_slice()) { [.., -91i8] if String::from("x y").ends_with("  pad  ") => (if (false) { E1::<NonZeroI64>::V0((), (NonZeroI64::new((-9i64)).unwrap(), 7u8), 10384593717069655257060992658440192u128) } else { E1::<NonZeroI64>::V0((), (NonZeroI64::new(100i64).unwrap(), 255u8), 9007199254740992u128) }), [_, .., 3i8] if apply(|x93: u128| -> bool { true }, 239957319476121195576328394880390580690u128) => (if (true) { E1::<NonZeroI64>::V0((), (NonZeroI64::new((-6575420245911674604i64)).unwrap(), 226u8), 3u128) } else { E1::<NonZeroI64>::V0((), (NonZeroI64::new(3507843583796144217i64).unwrap(), 127u8), 3u128) }), [-13i8..127i8, .., (..=8i8 | 16i8)] => (if (true) { E1::<NonZeroI64>::V0((), (NonZeroI64::new(7i64).unwrap(), 243u8), 162533115434329475741231919086479771583u128) } else { E1::<NonZeroI64>::V0((), (NonZeroI64::new(7i64).unwrap(), 116u8), 132465372213870423462073342828726122917u128) }), _ => ({ let t95: f64 = (unsafe { K3 }); E1::<NonZeroI64>::V0((), (NonZeroI64::new(14i64).unwrap(), 254u8), 8u128) }) })
    }
    pub fn f99(a100: i128, a101: fn() -> E1<NonZeroI64>, a102: &mut [E2], a103: Vec<E2>, a104: i8, a105: i128, a106: Vec<f32>, a107: &mut [(f64, NonZeroI64, char, i128)]) -> E2 {
        (if ((8u128.is_power_of_two() || String::from("42").ends_with("ff"))) { (match ((if (true) { vec!['\0', 'a'] } else { vec!['\u{7f}', '\u{80}'] }).as_slice()) { ['\'', ..] if String::from("  pad  ").contains("hello") => (if (false) { E2::V1 } else { E2::V2 }), [.., p129] if (match (String::from("héllo").as_str()) { p130 => true, _ => false }) => (&NonZeroI64::new(9223372036854775807i64).unwrap() as &dyn T5).m7(Vec::<E1<NonZeroI64>>::new()), [.., p132] => E2::V2, _ => E2::V1 }) } else { twice::<E2>(&apply_once(|| -> E2 { E2::V2 })).1 })
    }
    pub fn run() {
        let mut v134: f64 = (if ((E2::V3 != E2::V0)) { ((-128.9f64) - (-9223372036854775808.0f64)) } else { ((unsafe { K3 }) - (unsafe { K3 })) });
        let v135: f64 = std::mem::replace(&mut v134, f64::from((-118i8).clamp((-81i8), 33i8)));
        let r245 = catch_unwind(AssertUnwindSafe(|| -> (f64, NonZeroI64, char, i128) {
            let mut v246: BTreeMap<E2, E1<NonZeroI64>> = BTreeMap::new();
            v246.insert((match (['_', '\\'].to_vec().as_slice()) { ['Z', ..] if Some(false).unwrap() => (unsafe { *(&E2::V2 as *const E2) }), [.., '中'] => Some(E2::V0).unwrap(), _ => ({ let mut m254: Vec<E2> = vec![E2::V3]; let mut m255: Vec<(f64, NonZeroI64, char, i128)> = vec![(f64::NAN, NonZeroI64::new(256i64).unwrap(), '\u{7f}', (-170141183460469231731687303715884105727i128)), (f64::INFINITY, NonZeroI64::new(100i64).unwrap(), 'Z', (-12i128))]; let out = f99((unsafe { K4 }), (|| -> E1<NonZeroI64> { (match (&{ (Some(177163336100455281976558525137179712519u128), (-409.1420f32), E2::V3, f64::INFINITY, Some(1024i128), '\u{7f}') }) { (None, _, E2::V0, _, None, '\u{80}'..='\u{10FFFF}') if true => E1::<NonZeroI64>::V0((), (NonZeroI64::new(9223372036854775807i64).unwrap(), 24u8), 1u128), (Some(83076749736557242056487941267521536u128), p250, E2::V2, p251, Some(170141183460469231731687303715884105726i128), '\0') => E1::<NonZeroI64>::V0((), (NonZeroI64::new(9223372036854775807i64).unwrap(), 159u8), 8388607u128), (Some(317994326337493544324471720647273021344u128), p252, E2::V0, p253, None, '0'..='9') => E1::<NonZeroI64>::V0((), (NonZeroI64::new((-1i64)).unwrap(), 100u8), 100u128), _ => E1::<NonZeroI64>::V0((), (NonZeroI64::new(6i64).unwrap(), 6u8), 85070591730234615865843651857942052864u128) }) }), &mut m254, vec![E2::V1, E2::V3], 10i8, (-2i128), Vec::<f32>::new(), &mut m255); m254.mix(); m255.mix(); out }) }), (if ((match (Some(true)) { None => true, Some(x256) => x256 })) { E1::<NonZeroI64>::V0((), (NonZeroI64::new(9223372036854775807i64).unwrap(), 22u8), 0u128) } else { f87() })).mix();
            ((if (String::from("42").starts_with("")) { v135 } else { apply_dyn(&|x260: Vec<E2>| -> f64 { 3.25f64 }, vec![E2::V1, E2::V1, E2::V0, E2::V0, E2::V0]) }), NonZeroI64::new(15i64).unwrap(), Ok::<char, (f64, Vec<f32>, f32)>('ß').ok().expect("nothing there"), (unsafe { K4 }))
        }));
    }
}
