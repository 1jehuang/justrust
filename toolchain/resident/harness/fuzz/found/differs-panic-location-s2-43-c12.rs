// Found by fuzz.py: seed s2-43, case c12, edition 2021, backend cranelift.
// differs: panic location
// Reduced in 205 builds. Again: fuzz.py check differs-panic-location-s2-43-c12.rs
// rustc:
// panic: panicked at src/main.rs:450:612:
// index out of bounds: the len is 3 but the index is 18446744073709551615
// c12 b6eb33012c305708 ok
// ra-jit:
// panic: panicked at src/main.rs:450:611:
// index out of bounds: the len is 3 but the index is 18446744073709551615
// c12 b6eb33012c305708 ok
mod c12 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub struct S1 { pub f0: char, pub f1: u8 }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub enum E2<A> {
        V0 { f0: char },
        V1 { f0: NonZeroU8, f1: bool, f2: Box<A> },
        V2 { f0: isize, f1: char, f2: NonZeroU8 },
    }
    impl<A: Mix> Mix for E2<A> {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub enum E3 {
        V0(&'static str),
        V1 { f0: S1, f1: S1, f2: char },
        V2 { f0: char },
    }
    impl Mix for E3 {
        fn mix(&self) {
        }
    }
    static K4: bool = true;
    static K5: S1 = S1 { f0: '\n', f1: 8u8 };
    pub trait T6 {
        fn m7(&self) -> E3;
        fn m8(&self, a0: char) -> u8;
        fn m9(&mut self, a0: &'static str) -> E2<i16>;
    }
    impl T6 for i16 {
        fn m7(&self) -> E3 {
            ('b12: { if ((twice::<bool>(&false).1 || (!false))) { break 'b12 (if (K4) { sel::<E3>(K4, E3::V2 { f0: '\n' }, E3::V2 { f0: '_' }) } else { (if (K4) { E3::V2 { f0: ' ' } } else { E3::V1 { f0: S1 { f0: '中', f1: 255u8 }, f1: S1 { f0: ' ', f1: 9u8 }, f2: '\u{80}' } }) }); } E3::V2 { f0: 'ß' } })
        }
        fn m8(&self, a13: char) -> u8 {
            let v20: u8 = 'l19: loop {
            };
        }
        fn m9(&mut self, a22: &'static str) -> E2<i16> {
            (match (&{ K5 }) { S1 { f0: p30 @ '\0'..='\u{7f}', f1: p31 } => E2::<i16>::V2 { f0: (-12isize), f1: (*p30), f2: NonZeroU8::new(37u8).unwrap() }, _ => E2::<i16>::V1 { f0: NonZeroU8::new(255u8).unwrap(), f1: K4, f2: Box::new((-1i16)) } })
        }
    }
    impl T6 for E3 {
        fn m7(&self) -> E3 {
            E3::V1 { f0: S1 { f0: '\u{80}', ..S1 { f0: '\u{10FFFF}', f1: K5.f1 } }, f1: S1 { f0: char::from_u32(1u32).unwrap_or('é'), f1: ({ let t32: E2<i16> = E2::<i16>::V2 { f0: 9223372036854775806isize, f1: ' ', f2: NonZeroU8::new(255u8).unwrap() }; K5.f1 }) }, f2: K5.f0 }
        }
        fn m8(&self, a33: char) -> u8 {
            10u8
        }
        fn m9(&mut self, a41: &'static str) -> E2<i16> {
            E2::<i16>::V1 { f0: NonZeroU8::new(1u8).unwrap(), f1: ({ let t44: Option<&'static str> = Some("héllo"); false }), f2: Box::new((-16i16)) }
        }
    }
    #[inline(never)]
    pub fn f45(a46: &Option<&'static str>, a47: &mut [&'static str], a48: &i64, a49: &[Option<&'static str>]) -> S1 {
        { let value = (loop { break "x y"; }); if let Some(slot) = a47.first_mut() { *slot = value; } }
        if let [Some(_), None, Some(_)] = (a49.to_vec().as_slice()) {
            let mut n51 = 0u32;
            let v53: S1 = 'l52: loop {
                n51 += 1;
                if n51 > 8 { break (if (false) { K5 } else { S1 { f0: '\u{7f}', f1: 14u8 } }); }
                let S1 { f1: p54, .. }: S1 = K5;
                let mut o55: Vec<Box<dyn T6>> = vec![Box::new(larger::<E3>((if (K4) { E3::V2 { f0: 'Z' } } else { E3::V0("3.5") }), E3::V2 { f0: '\u{10FFFF}' })), Box::new(larger::<i16>(16385i16.wrapping_rem_euclid((-13649i16) | 1).wrapping_rem(i16::from_ne_bytes((-4i16).to_le_bytes()) | 1), i16::try_from(0u128).unwrap_or(7735i16).wrapping_div((10i8 as i16) | 1)))];
                for o56 in o55.iter() { o56.m7().mix(); }
                let S1 { f1: p57, .. }: S1 = S1 { f0: id::<char>(K5.f0), f1: (12u16 as u8) };
            };
            match (&{ "a,b,,c" }) {
                p58 if K4 => {
                    let v59: usize = (64usize.wrapping_div_euclid(39usize | 1).abs_diff(a47.len().checked_sub(14usize).unwrap_or(a47.len())) as usize);
                }
                p60 => {
                    let mut o61: Box<dyn T6> = Box::new(E3::V0((*p60)));
                }
                _ => {
                    char::from_digit(({ let t62: S1 = v53; 0u32 }) % 36, 36).unwrap_or('\\').mix();
                }
                _ => 7i8.checked_sub((-101i8)).unwrap_or((-35i8)).mix(),
            }
        } else {
            match (sel::<Vec<usize>>(K4, vec![9223372036854775807usize, 8245092562891560917usize, 8usize], vec![9395035852827271114usize, 67108864usize, 0usize]).as_slice()) {
                [p65, .., 1459462625671978854usize] if Some((-12i64)).is_some() => {
                    let mut o66: Vec<Box<dyn T6>> = vec![Box::new((8i16.unsigned_abs() as i16)), Box::new(E3::V2 { f0: 'z' })];
                    for o67 in o66.iter_mut() { o67.m9((if (false) { "0123456789" } else { "a" })).mix(); }
                }
                [p69, p68 @ ..] => {
                    let S1 { f1: _, .. }: S1 = K5;
                }
                [] => {
                    let mut v71: [char; 8] = <[i64; 8]>::try_from(vec![100i64, 7i64, 11i64]).unwrap_or([8i64, 1125899906842623i64, 1i64, 0i64, (-14i64), 2713623622333716005i64, 8i64, 68719476735i64]).map(|x74: i64| -> char { K5.f0 });
                    { let value = sel::<char>((false || K4), char::from_digit(2u32 % 36, 36).unwrap_or(K5.f0), char::from_digit(1048577u32 % 36, 36).unwrap_or(K5.f0)); let raw = Box::into_raw(Box::new(v71)); unsafe { (*raw)[5] = value; Box::from_raw(raw).mix(); } }
                    v71.mix();
                }
                _ => 32767i16.saturating_add(129i16).mix(),
            }
        }
        S1 { f0: (if (K4) { '😀' } else { K5.f0 }), f1: K5.f1.rotate_right(4194305u32) }
    }
    #[inline(never)]
    pub fn f77(a78: &mut i8, a79: &mut char) -> bool {
        let mut v80: BTreeMap<bool, E2<i16>> = BTreeMap::new();
        v80.insert(twice::<bool>(&true).1, E2::<i16>::V2 { f0: 513isize, f1: (loop { break (*a79); }), f2: ('b81: { if (K4) { break 'b81 NonZeroU8::new(25u8).unwrap(); } NonZeroU8::new(1u8).unwrap() }) }).mix();
        v80.insert(K4, (if (false) { E2::<i16>::V0 { f0: (*a79) } } else { E2::<i16>::V1 { f0: NonZeroU8::new(100u8).unwrap(), f1: K4, f2: Box::new(18i16) } })).mix();
        v80.insert(char::from_u32(1059400459u32).unwrap_or((*a79)).is_alphanumeric(), E2::<i16>::V1 { f0: (if (K4) { NonZeroU8::new(127u8).unwrap() } else { NonZeroU8::new(3u8).unwrap() }), f1: false, f2: pick::<Box<i16>>(true, &Box::new((-17i16)), &Box::new(2047i16)).clone() }).mix();
        v80.insert((if (sel::<bool>(K4, K4, K4)) { Some(E2::<i16>::V2 { f0: (-3022215390378039251isize), f1: 'a', f2: NonZeroU8::new(254u8).unwrap() }).is_some() } else { sel::<bool>(K4, K4, K4) }), E2::<i16>::V0 { f0: (*a79) }).mix();
        for (key, value) in v80.iter().rev() { key.mix(); value.mix(); }
        v80.len().mix();
        let mut v83: [i32; 8] = <[i32; 8]>::try_from(vec![1870359858i32, 3i32]).unwrap_or([33554432i32; 8]);
        { let at = ((10usize as usize) % 8); let value = sel::<i32>(false, 17i32, (-5i32)).checked_add(1126485858i32.reverse_bits()).unwrap_or(2i32.overflowing_shr(8388607u32).0); unsafe { raw_set(v83.as_mut_ptr(), at, value); raw_get(v83.as_ptr(), at).mix(); } }
        v83.mix();
        (K5.f1.leading_ones() as u8).overflowing_sub(u8::try_from(16i64).unwrap_or(32u8)).1
    }
    pub fn run() {
        let r234 = catch_unwind(AssertUnwindSafe(|| -> [bool; 3] {
                    let S1 { f0: p253, f1: p254 }: S1 = ({ let mut m245: Vec<&'static str> = larger::<Vec<&'static str>>(sel::<Vec<&'static str>>(false, vec!["x y"], Vec::<&'static str>::new()), vec!["ff", "héllo", "ff", "a"]); let out = f45(&pick::<Option<&'static str>>(({ let mut m241: i8 = 11i8; let mut m242: char = K5.f0; let out = f77(&mut m241, &mut m242); m241.mix(); m242.mix(); out }), &(if (true) { Some("  pad  ") } else { None::<&'static str> }), &Some(Some(Vec::<&'static str>::new())).and_then(|x244: Option<Vec<&'static str>>| -> Option<&'static str> { Some("Abc Def") })).clone(), &mut m245, &[(64i32 as i64), (549755813888i64.abs_diff(6609053410108741367i64) as i64), pick::<i64>(K4, &20i64, &15i64).clone()][larger::<usize>(35184372088832usize, 18446744073709551615usize)], &[true.then(|| -> &'static str { "x y" }), (if (K4) { None::<&'static str> } else { Some("line\n") })]); m245.mix(); out });
            [false, false, true]
        }));
    }
}
