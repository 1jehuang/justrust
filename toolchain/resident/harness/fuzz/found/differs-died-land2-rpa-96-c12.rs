// Found by fuzz.py: seed land2-rpa-96, case c12, edition 2021, backend llvm.
// differs: died
// Reduced in 95 builds. Again: fuzz.py check differs-died-land2-rpa-96-c12.rs
// ra-jit's program ended with status -11 in this case
// 
mod c12 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S1 { pub f0: usize }
    impl Drop for S1 {
        fn drop(&mut self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(packed(2))]
    pub struct S2(pub u8, pub u128);
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(u8)]
    pub enum E3 {
        V0,
        V1,
    }
    impl Mix for E3 {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(u16)]
    pub enum E4 {
        V0 = 36,
        V1 = 211,
        V2 = 27,
        V3 = 96,
    }
    const K5: E3 = E3::V0;
    static K6: u128 = 10184321486607035073183947839867858968u128;
    const K7: u8 = 31u8;
    pub trait T8 {
        fn m9(&self, a0: E4, a1: S2) -> u128;
        fn m10(&self, a0: u128) -> u8;
    }
    impl T8 for bool {
        fn m9(&self, a11: E4, a12: S2) -> u128 {
            (9223372036854775807usize as u128)
        }
        fn m10(&self, a13: u128) -> u8 {
            u8::try_from(8u32).unwrap_or(34u8).min(String::from("a").as_bytes().first().copied().unwrap_or(32u8))
        }
    }
    impl T8 for S2 {
        fn m9(&self, a14: E4, a15: S2) -> u128 {
            9444732965739290427393u128
        }
        fn m10(&self, a17: u128) -> u8 {
            ({ (*self).0 }).saturating_pow(268435456u32 % 7).wrapping_shl(4294967295u32)
        }
    }
    impl T8 for i128 {
        fn m9(&self, a19: E4, a20: S2) -> u128 {
            ("2.5".parse::<f64>().unwrap_or(f64::MAX) as u128)
        }
        fn m10(&self, a21: u128) -> u8 {
            K7.wrapping_rem_euclid(K7 | 1).wrapping_add(K7)
        }
    }
    #[inline(never)]
    pub fn f47(a48: usize, a49: &str, a50: &NonZeroI64, a51: S2, a52: (u128,), a53: u128, a54: &usize, a55: u128) -> bool {
        let mut v56: u128 = ((a55 as u128) | u128::from(K7));
        sel::<Option<NonZeroI64>>(a55.overflowing_sub(({ a51.1 })).1, Some(NonZeroI64::new(2i64).unwrap()).xor(Some(NonZeroI64::new(1756166768740247640i64).unwrap())), Some(NonZeroI64::new(2i64).unwrap())).is_some()
    }
    #[inline]
    pub fn f60(a61: &dyn T8, a62: i128, a63: &S2, a64: E3, a65: i8, a66: u128, a67: i128, a68: &mut [i8], a69: S1, a70: &dyn T8, a71: bool, a72: i8, a73: (u128,)) -> E4 {
        { let value = (if let Some(x74) = (Some(64i8)) { x74 } else { a65.wrapping_pow(24u32 % 7) }); if let Some(slot) = a68.first_mut() { *slot = value; } }
        f47((32768i64 as usize), larger::<&'static str>("x y", "3.5"), &(match (vec![(-128.9f32)].as_slice()) { [p77, p76 @ ..] if a71 => NonZeroI64::new((-9i64)).unwrap(), [p79, .., p80] if true => NonZeroI64::new((-6884709551596777098i64)).unwrap(), [p82, p81 @ .., p83] if a71 => NonZeroI64::new(100i64).unwrap(), _ => NonZeroI64::new((-9223372036854775808i64)).unwrap() }), (*a63), a73, twice::<u128>(&15u128).0, &Some(0usize).unwrap_or(26usize), (E4::V2 as u128)).mix();
        (match (Some(E4::V3)) { None => (match (Some(E4::V0)) { None => Some(E4::V1).unwrap_or_else(|| -> E4 { E4::V3 }), Some(x86) => x86 }), Some(x85) => x85 })
    }
    pub fn f99<G98: Mix + Clone + Debug + PartialOrd>(a100: G98, a101: S1, a102: (G98, u8)) -> (G98, u8) {
        a102.clone()
    }
    #[inline(never)]
    pub fn f146<G145: Mix + Clone + Debug + PartialOrd>(a147: &G145, a148: &mut u8, a149: (G145, u8), a150: &f64, a151: G145) -> bool {
        let v152: E4 = ({ let mut m155: Vec<i8> = vec![6i8, (-20i8)]; let out = f60(&f47(17usize, "", &NonZeroI64::new((-11i64)).unwrap(), S2(10u8, 2u128), (340282366920938463463374607431768211455u128,), K6, &9475915641741549611usize, 273471229935339708561926799940253099009u128), (-105200666997641855134727505747278897515i128), &(match (Some(S2(129u8, 36u128))) { None => S2(46u8, 1u128), Some(x153) => x153 }), (match (vec![E3::V0, E3::V1].as_slice()) { [E3::V0, p154 @ .., E3::V1] => K5, _ => K5 }), 3i8.wrapping_div_euclid(63i8 | 1), K6, ((-219.4066f32) as i128), &mut m155, (match (vec![1173404392i32, 2147483647i32].as_slice()) { [p158 @ -10i32..=-7i32, p157 @ .., (0i32 | 10i32)] => S1 { f0: 12410296855511847507usize }, [p160 @ ..=-14i32, p159 @ ..] if false => S1 { f0: 2usize }, [p162 @ -1350692107i32..=6i32, p161 @ ..] => S1 { f0: 1usize }, _ => S1 { f0: 3usize } }), &257i128, Some("3.5").is_some(), 1i8.wrapping_div((-14i8) | 1), (2u128,)); m155.mix(); out });
        let mut v163: [bool; 1] = [false];
        { let at = (1usize.rotate_right(4294967295u32) % 1); let value = f47("0123456789".parse::<usize>().unwrap_or(8usize), "  pad  ", &twice::<NonZeroI64>(&NonZeroI64::new(20i64).unwrap()).1, Ok::<S2, S2>(S2(1u8, 224690982380441679513417481222232173965u128)).unwrap_or_else(|x165: S2| -> S2 { S2(27u8, 36u128) }), (match ((340282366920938463463374607431768211454u128,)) { p166 => (30u128,), _ => (9u128,) }), K6, &std::mem::align_of::<f64>(), (0.0f32 as u128)); unsafe { raw_set(v163.as_mut_ptr(), at, value); raw_get(v163.as_ptr(), at).mix(); } }
        v163.mix();
        v163[0] = v163[0];
        f47(24usize.to_le(), "  pad  ", &NonZeroI64::new((-15i64)).unwrap(), None::<S2>.unwrap_or_else(|| -> S2 { S2(0u8, 100u128) }), (3u128,), larger::<u128>(K6, 2475880078570760549798248447u128), &larger::<usize>(100usize, 3usize), K6)
    }
    pub fn run() {
        let mut v179: [(i32,); 4] = [(131073i32,), Err::<(i32,), (u128,)>((1u128,)).unwrap_or_else(|x183: (u128,)| -> (i32,) { (10i32,) }), ((-1046993336i32),), Some((3i32,)).unwrap_or((461910983i32,))];
        let mut v199: i8 = String::from("ff").bytes().fold(i8::try_from(K6).unwrap_or(vec![S1 { f0: 12usize }, S1 { f0: 33554432usize }, S1 { f0: 15891103444526052359usize }].into_iter().flat_map(|x203: S1| -> Vec<E3> { Vec::<E3>::new() }).map(|x204: E3| -> i32 { v179[1].0 }).skip(1).fold(35i8, |acc206: i8, x207: i32| -> i8 { acc206 })), |acc201: i8, x202: u8| -> i8 { acc201 });
            let mut f200 = |a198: E4| -> i16 { v199 = 1i8; Some(15i16).unwrap_or((2i16 << (1u32 % 16))) };
            let mut v210: [bool; 1] = [None::<i8>.is_some(); 1];
            { let value = ({ let mut m215: u8 = ({ let whole = S2(1u8, 3u128); whole.0 }); let out = f146::<E3>(&None::<E3>.map_or(E3::V0, |x214: E3| -> E3 { K5 }), &mut m215, f99::<E3>(K5, S1 { f0: 28usize }, (E3::V0, 32u8)), &(f64::EPSILON - 16777217.0f64), (match (Some(E3::V0)) { None => K5, Some(x216) => x216 })); m215.mix(); out }); let slot = &mut v210[0] as *mut bool; unsafe { *slot = value; (*slot).mix(); } }
            v210.mix();
    }
}
