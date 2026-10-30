// Found by fuzz.py: seed s2-36, case c24, edition 2021, backend cranelift.
// differs: panic location
// Reduced in 190 builds. Again: fuzz.py check differs-panic-location-s2-36-c24.rs
// rustc:
// panic: panicked at src/main.rs:408:32:
// index out of bounds: the len is 3 but the index is 10
// c24 dead786553ac6654 ok
// ra-jit:
// panic: panicked at src/main.rs:408:21:
// index out of bounds: the len is 3 but the index is 10
// c24 dead786553ac6654 ok
mod c24 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub struct S1 { pub f0: i8, pub f1: u8 }
    impl Mix for S1 {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub struct S2 { pub f0: f32, pub f1: u16, pub f2: i128, pub f3: u8, pub f4: f32, pub f5: u8, pub f6: u16 }
    impl Mix for S2 {
        fn mix(&self) {
        }
    }
    static K3: f32 = 0.0f32;
    pub trait T4 {
        fn m5(&self, a0: S2) -> f32;
        fn m6(&self, a0: f32) -> (S1, Box<u128>);
    }
    impl T4 for (S1, Box<u128>) {
        fn m5(&self, a10: S2) -> f32 {
            a10.f4.clamp(-1.5f32, 2.5f32)
        }
        fn m6(&self, a23: f32) -> (S1, Box<u128>) {
            (S1 { f0: 5i8, f1: (*self).0.f1 }, (match (&{ S2 { f0: (-528.3791f32), f1: 16u16, f2: 0i128, f3: 1u8, f4: 1e10f32, f5: 1u8, f6: 28u16 } }) { S2 { f1: _, f2: _, f4: p24, f5: _, .. } => (*self).1.clone(), S2 { f1: _, f4: _, f5: _, f6: _, .. } if true => (*self).1.clone(), S2 { f0: _, f1: _, f2: _, f3: _, f4: p25, f6: _, .. } => (*self).1.clone(), _ => Box::new(0u128) }))
        }
    }
    #[inline(never)]
    pub fn f26() -> (S1, Box<&'static str>, String, S1, i16, Box<u128>) {
        let mut v27: S1 = S1 { f0: (if (true) { (-19i8) } else { (-128i8) }), f1: (38u8 as u8) };
        {
            let mut f28 = || -> (String, S2, S1) { v27 = (unsafe { *(&v27 as *const S1) }); (if (true) { (unsafe { let mut slot = MaybeUninit::<(String, S2, S1)>::uninit(); slot.write((String::from("3.5"), S2 { f0: f32::EPSILON, f1: 19098u16, f2: 15651665891891058289227796009728910052i128, f3: 8u8, f4: 1e30f32, f5: 7u8, f6: 8u16 }, S1 { f0: 63i8, f1: 254u8 })); slot.assume_init() }) } else { (match (vec![(-100i8), (-15i8)].as_slice()) { [_, .., _] if false => (String::from("line\n"), S2 { f0: f32::NEG_INFINITY, f1: 127u16, f2: 18446744073709551616i128, f3: 0u8, f4: (-9223372036854775808.0f32), f5: 72u8, f6: 65534u16 }, S1 { f0: 63i8, f1: 255u8 }), [_, ..] => (String::from("a"), S2 { f0: f32::MIN, f1: 100u16, f2: 67108864i128, f3: 0u8, f4: 16777217.0f32, f5: 157u8, f6: 1024u16 }, S1 { f0: 10i8, f1: 0u8 }), [_, ..] if false => (String::from("0123456789"), S2 { f0: 0.1f32, f1: 8u16, f2: (-9i128), f3: 1u8, f4: 16777217.0f32, f5: 0u8, f6: 3u16 }, S1 { f0: (-1i8), f1: 100u8 }), _ => (String::from("a"), S2 { f0: (-0.0f32), f1: 39u16, f2: (-28328757152414864359460161502913326956i128), f3: 35u8, f4: 1.5f32, f5: 22u8, f6: 39720u16 }, S1 { f0: 82i8, f1: 17u8 }) }) }) };
            let mut v33: Box<Box<u128>> = apply_dyn_mut(&mut |x35: Box<(i128, f32)>| -> Box<Box<u128>> { (if (false) { Box::new(Box::new(1u128)) } else { Box::new(Box::new(53698287211153581624539719850331903414u128)) }) }, pick::<Box<(i128, f32)>>(false, &Box::new(((-19i128), f32::NEG_INFINITY)), &Box::new((15i128, 0.5f32))).clone());
            {
                let mut f34 = |a32: (S1, Box<u128>)| -> i8 { v33 = apply_mut(|x36: S1| -> Box<Box<u128>> { (match (String::from("3.5").as_str()) { p37 if true => v33.clone(), "x y" => v33.clone(), _ => Box::new(Box::new(9u128)) }) }, S1 { f0: a32.0.f0, f1: a32.0.f1 }).1; (3i8.wrapping_rem(a32.0.f0 | 1) & (a32.0.f0 ^ 127i8)) };
                let v38: (S1, Box<&'static str>, String, S1, i16, Box<u128>) = (S1 { f0: 10i8, f1: 7u8 }, Box::new(""), String::from("Abc Def"), S1 { f0: 1i8, f1: 8u8 }, 8i16, Box::new(22u128));
                let S2 { f0: _, f2: _, f3: _, f4: p42, f6: p43, .. }: S2 = S2 { f0: (unsafe { let mut slot = MaybeUninit::<f32>::uninit(); slot.write(apply_mut(|x39: String| -> f32 { (-9223372036854775808.0f32) }, v38.2.clone()).1); slot.assume_init() }), f1: sel::<u16>((if (false) { false } else { true }), (2u16 as u16), 51768u16), f2: 12i128, f3: (7u8.abs_diff(apply_dyn_mut(&mut |x40: f32| -> u8 { 0u8 }, K3)) as u8), f4: K3, f5: (if (apply_dyn_mut(&mut |x41: (S1, Box<u128>)| -> bool { false }, (S1 { f0: 17i8, f1: 0u8 }, Box::new(33u128)))) { 17u8 } else { v38.3.f1.clamp(32u8, 127u8) }), f6: ((19452u16 ^ 2048u16).abs_diff((31100u16.abs_diff(8u16) as u16)) as u16) };
                let v44: S2 = S2 { f0: (f64::NAN as f32), f1: p43, f2: apply_boxed(Box::new(|| -> i128 { 1329227995784915872903807060280344575i128 })), f3: 0u8.reverse_bits(), f4: p42, f5: u8::from_be_bytes(v38.3.f1.to_be_bytes()), f6: (if (false) { p43 } else { p43 }) };
                let f46 = move || -> (S1, Box<u128>) { let v45 = v44; let out = (match (&{ (match (vec![vec![S1 { f0: 19i8, f1: 16u8 }, S1 { f0: 19i8, f1: 17u8 }]].as_slice()) { [p49, .., _] => (S1 { f0: 28i8, f1: 54u8 }, Box::new(170141183460469231731687303715884105727u128)), [_, .., _] if true => (S1 { f0: 103i8, f1: 2u8 }, Box::new(50086039283637583555983704270784099770u128)), _ => (S1 { f0: (-1i8), f1: 34u8 }, Box::new(302231454903657293676543u128)) }) }) { (S1 { f0: _, f1: _ }, _) if true => (if (false) { (S1 { f0: 1i8, f1: 255u8 }, Box::new(21u128)) } else { (S1 { f0: 33i8, f1: 81u8 }, Box::new(2u128)) }), (S1 { f1: _, .. }, p51) if (v45.f3 < v45.f5) => (match (vec![Box::new(Box::new(4503599627370497u128))].as_slice()) { [_, .., _] => (S1 { f0: (-106i8), f1: 219u8 }, Box::new(8589934591u128)), [_, ..] if false => (S1 { f0: (-18i8), f1: 1u8 }, Box::new(27u128)), [p55, ..] => (S1 { f0: 15i8, f1: 28u8 }, Box::new(8u128)), _ => (S1 { f0: 33i8, f1: 254u8 }, Box::new(8u128)) }), p56 if true => (if (false) { (*p56).clone() } else { (*p56).clone() }), _ => sel::<(S1, Box<u128>)>(true, (S1 { f0: 9i8, f1: 1u8 }, Box::new(7u128)), (S1 { f0: 10i8, f1: 213u8 }, Box::new(5u128))) }); drop(v45); out };
                apply_boxed(Box::new(f46)).mix();
                f34((v38.0, Box::new(38829231846171062429160316598752355182u128))).mix();
            }
            v33.mix();
            [[vec![63u32, 16777215u32, 4294967295u32, 3509435714u32, 25u32], vec![14u32, 2u32, 20u32]].concat(), Some(S1 { f0: (-97i8), f1: 193u8 }).into_iter().map(|x60: S1| -> u32 { 1u32 }).collect::<Vec<u32>>()].concat().mix();
            f28().mix();
        }
        v27.mix();
        match &mut v27 {
            S1 { .. } => {
                let S2 { f0: p76, f1: p77, f2: _, f4: p78, f5: _, f6: _, .. }: S2 = (match ((match (&{ S2 { f0: 1e10f32, f1: 16u16, f2: 262144i128, f3: 29u8, f4: K3, f5: 1u8, f6: 19u16 } }) { S2 { f1: _, f3: _, f4: p62, f5: _, f6: _, .. } => (match (Vec::<Box<Box<u128>>>::new().as_slice()) { [p65, .., p66] => Vec::<f32>::new(), [p68, .., p69] => vec![(-1.0f32), f32::EPSILON], [_, .., _] => vec![f32::NAN, (-128.9f32)], _ => vec![f32::EPSILON, 1.0f32, 1e10f32, (-19.7554f32), (-128.9f32)] }), S2 { f0: _, f1: _, f2: _, f4: _, f5: _, .. } => vec![4294967296.0f32, K3, K3, K3], _ => ({ let mut items = vec![f32::MIN, f32::NEG_INFINITY]; items.extend(Vec::<f32>::new()); items }) }).as_slice()) { [_, .., p74] => sel::<S2>(1u8.is_power_of_two(), S2 { f0: K3, f1: 32u16, f2: 295147905179352825856i128, f3: 254u8, f4: K3, f5: 31u8, f6: 2u16 }, (|x75: Box<(i128, f32)>| -> S2 { S2 { f0: 1.0f32, f1: 64u16, f2: 170141183460469231731687303715884105727i128, f3: 8u8, f4: (-2.5e-3f32), f5: 96u8, f6: 512u16 } })(Box::new((32769i128, 1e10f32)))), _ => (if ((unsafe { let mut slot = MaybeUninit::<bool>::uninit(); slot.write(true); slot.assume_init() })) { S2 { f0: K3, f1: 6338u16, f2: (-98955887992152909411252405513385439182i128), f3: 1u8, f4: K3, f5: 63u8, f6: 3u16 } } else { (match (&{ S2 { f0: (-0.0f32), f1: 3u16, f2: 7i128, f3: 254u8, f4: (-2.5e-3f32), f5: 254u8, f6: 19844u16 } }) { S2 { f3: _, .. } => S2 { f0: (-782.5675f32), f1: 16385u16, f2: 117216463522407854219531579039803166205i128, f3: 32u8, f4: 65535.0f32, f5: 142u8, f6: 1u16 }, _ => S2 { f0: (-0.0f32), f1: 0u16, f2: 18i128, f3: 24u8, f4: (-1.0f32), f5: 0u8, f6: 32767u16 } }) }) });
            }
            S1 { f0: _, f1: _ } => {
                let mut v81: isize = 2isize;
                {
                    let mut f82 = |a79: Vec<(char, u128, i128, char)>, a80: u16| -> [String; 17] { v81 = (isize::try_from(a80).unwrap_or(v81).abs_diff((18446744073709551615u64 as isize)) as isize); [(-3i128), (-17i128), 170141183460469231731687303715884105727i128, 136611494860054582344366710139722205693i128, 149436140619836398949760157412344837175i128, 316912650057057350374175801344i128, 7i128, 144212258498891856353375706253788236560i128, (-131901774787150347708050767652848313505i128), 7i128, (-1i128), 20282409603651670423947251286016i128, 166153499473114484112975882535043072i128, (-5i128), 288230376151711744i128, 47773205114168390484002858716185444508i128, 7i128].map(|x84: i128| -> String { pick::<String>(false, &String::from("Abc Def"), &String::from("  pad  ")).clone() }) };
                    let mut v87: Vec<Vec<Box<f32>>> = vec![(loop { break vec![Box::new((-9223372036854775808.0f32))]; })];
                    let mut v90: [i128; 4] = [618970019642690137449562112i128, 3i128, 8191i128, 100i128];
                    { let v91: [i128; 4] = <[i128; 4]>::try_from([vec![295147905179352825856i128, 170141183460469231731687303715884105726i128, 170141183460469231731687303715884105727i128], vec![(-19i128), 7i128, (-9i128), (-170141183460469231731687303715884105728i128), 8193i128]].concat()).unwrap_or(sel::<[i128; 4]>(false, [33554432i128, (-33767374335972448850965792285651417393i128), (-118434278892625897589258386565084537806i128), 110689510026500535606162206999771460223i128], [0i128; 4])); unsafe { std::ptr::copy_nonoverlapping(v91.as_ptr(), v90.as_mut_ptr(), 4) }; }
                    v90.mix();
                    if ((match (&{ S1 { f0: (-8i8), f1: 46u8 } }) { p98 => String::from("ff").starts_with("hello"), _ => sel::<bool>(false, false, true) })) { return (unsafe { let mut slot = MaybeUninit::<(S1, Box<&'static str>, String, S1, i16, Box<u128>)>::uninit(); slot.write((match (vec![String::from("x y"), String::from("a,b,,c")].as_slice()) { [_, .., p100] => (S1 { f0: 65i8, f1: 31u8 }, Box::new("line\n"), String::from("  pad  "), S1 { f0: 88i8, f1: 3u8 }, (-10912i16), Box::new(23510445562828953577104230067540744675u128)), _ => (S1 { f0: (-14i8), f1: 160u8 }, Box::new("0123456789"), String::from("ff"), S1 { f0: 30i8, f1: 100u8 }, 255i16, Box::new(14u128)) })); slot.assume_init() }); }
                    f82(({ let mut items = Vec::<(char, u128, i128, char)>::new(); items.reverse(); items }), 16u16.saturating_sub(0u16)).mix();
                }
                v81.mix();
            }
            _ => {}
        }
        (match (&{ v27 }) { p102 if (false || (unsafe { let mut slot = MaybeUninit::<bool>::uninit(); slot.write(false); slot.assume_init() })) => (match (vec![vec![('\n', 77371252455336267181195265u128, (-12i128), '\u{e000}'), ('ß', 34u128, 1i128, '\\'), ('\n', 0u128, (-155065876353981595584121938251696651887i128), '\n')], Vec::<(char, u128, i128, char)>::new()].as_slice()) { [p104, ..] => (if (true) { (S1 { f0: 1i8, f1: 127u8 }, Box::new("Abc Def"), String::from("héllo"), S1 { f0: 2i8, f1: 97u8 }, 7i16, Box::new(281474976710655u128)) } else { (S1 { f0: 15i8, f1: 195u8 }, Box::new("line\n"), String::from("x y"), S1 { f0: 100i8, f1: 1u8 }, 11i16, Box::new(37u128)) }), _ => (match (&{ (S1 { f0: (-16i8), f1: 8u8 }, Box::new("3.5"), String::from("0123456789"), S1 { f0: 19i8, f1: 115u8 }, 11661i16, Box::new(149563348122484853400602790022649056022u128)) }) { (S1 { f0: _, f1: _ }, ..) => (S1 { f0: 2i8, f1: 254u8 }, Box::new("line\n"), String::from("a"), S1 { f0: (-127i8), f1: 40u8 }, (-3i16), Box::new(9u128)), _ => (S1 { f0: (-33i8), f1: 25u8 }, Box::new("😀"), String::from("héllo"), S1 { f0: 7i8, f1: 255u8 }, 8i16, Box::new(23u128)) }) }), _ => (loop { break apply_boxed(Box::new(|| -> (S1, Box<&'static str>, String, S1, i16, Box<u128>) { (S1 { f0: 78i8, f1: 8u8 }, Box::new("0123456789"), String::from("x y"), S1 { f0: 127i8, f1: 0u8 }, (-32768i16), Box::new(30u128)) })); }) })
    }
    pub fn f106(a107: &str, a108: isize, a109: u16, a110: &str, a111: (S1, Box<u128>), a112: S2, a113: Vec<S1>, a114: &Box<Box<u128>>, a115: u128, a116: Vec<Box<NonZeroI64>>, a117: &Box<NonZeroU32>, a118: S1, a119: [String; 17]) -> i128 {
        if let (S1 { f0: _, .. }, p133, _, S1 { f0: _, .. }, _, _) = (&{ f26() }) {
            let r139 = catch_unwind(AssertUnwindSafe(|| -> S1 {
                let v143: i8 = [3i8.saturating_add(a111.0.f0), (-13i8).checked_sub((-30i8)).unwrap_or(63i8), (a118.f0.abs_diff(a111.0.f0) as i8)][usize::try_from(10u32).unwrap_or(1usize)];
                (if ((-9223372036854775808.0f64).is_nan()) { S1 { f0: v143, f1: a111.0.f1 } } else { a118 })
            }));
        }
        a112.f2.clamp((-170141183460469231731687303715884105727i128), (-170141183460469231731687303715884105727i128))
    }
    pub fn f174(a175: Vec<Box<u128>>, a176: u128) -> impl Fn(i8, u8) -> [String; 17] {
        move |x177: i8, x178: u8| -> [String; 17] { [String::from("0123456789"), String::from("x y"), String::from("0123456789"), String::from("3.5"), String::from("😀"), String::from("ff"), String::from("😀"), String::from("a"), String::from("-7"), String::from("-7"), String::from("héllo"), String::from("Abc Def"), String::from("-7"), String::from(""), String::from("a"), String::from("a"), String::from("a")] }
    }
    #[inline(never)]
    pub fn f185(a186: fn() -> (S1, Box<&'static str>, String, S1, i16, Box<u128>)) -> (S1, Box<u128>) {
        (&(S1 { f0: 127i8, f1: 3u8 }, Box::new(239711895695831511478750680771773722687u128)) as &dyn T4).m6(1e30f32)
    }
    pub fn run() {
            let mut v196: (S1, Box<u128>) = (if ((if (false) { true } else { true })) { (match (Vec::<S1>::new().as_slice()) { [S1 { f0: _, f1: _ }, .., S1 { f0: _, f1: _ }] => (S1 { f0: 4i8, f1: 7u8 }, Box::new(39614081257132168796771975168u128)), _ => (S1 { f0: 32i8, f1: 31u8 }, Box::new(10u128)) }) } else { f185(f26) });
            let r229 = catch_unwind(AssertUnwindSafe(|| -> (NonZeroU32, S2, Vec<f64>) {
                let mut v234: i128 = (unsafe { let mut slot = MaybeUninit::<i128>::uninit(); slot.write(9444732965739290427393i128); slot.assume_init() });
                    let mut f235 = |a232: i128, a233: i128| -> ([i64; 2], S1, (u64, i8, f64), [i128; 2], ((), u16), Box<i128>) { v234 = f106(format!("{:?}", 1024u32).as_str(), 17isize.saturating_abs(), u16::from_be_bytes(43380u16.to_ne_bytes()), (unsafe { let mut slot = MaybeUninit::<String>::uninit(); slot.write(String::from("Abc Def")); slot.assume_init() }).as_str(), f185((|| -> (S1, Box<&'static str>, String, S1, i16, Box<u128>) { (unsafe { let mut slot = MaybeUninit::<(S1, Box<&'static str>, String, S1, i16, Box<u128>)>::uninit(); slot.write((S1 { f0: 1i8, f1: 35u8 }, Box::new("Abc Def"), String::from("3.5"), S1 { f0: (-11i8), f1: 255u8 }, 2i16, Box::new(39u128))); slot.assume_init() }) })), S2 { f0: f32::MAX, f1: 18u16, f2: (-6i128), f3: 0u8, f4: 0.5f32, f5: 254u8, f6: 23u16 }, [vec![S1 { f0: 32i8, f1: 127u8 }, S1 { f0: (-18i8), f1: 14u8 }, S1 { f0: (-7i8), f1: 100u8 }, S1 { f0: (-10i8), f1: 8u8 }, S1 { f0: (-95i8), f1: 15u8 }], Vec::<S1>::new()].concat(), &apply_dyn_mut(&mut |x237: Box<Box<u128>>| -> Box<Box<u128>> { x237.clone() }, Box::new(Box::new(19u128))), ((-81.8545f64) as u128), ({ let mut items = vec![Box::new(NonZeroI64::new(2i64).unwrap())]; items.sort(); items }), &Box::new(NonZeroU32::new(3896434501u32).unwrap()), S1 { f0: v196.0.f0, f1: v196.0.f1 }, f174(Vec::<Box<u128>>::new(), (*v196.1))(v196.0.f0, 64u8)); (loop { break ([4294967297i64, 11i64], S1 { f0: (-14i8), f1: 2u8 }, (13u64, (-19i8), (-59.2674f64)), [(-9i128); 2], ((), 1u16), Box::new(47215324870192818937812181111954518128i128)); }) };
                    let v241: (NonZeroU32, S2, Vec<f64>) = (if (apply_dyn_mut(&mut |x242: Vec<Box<u128>>| -> bool { true }, vec![Box::new(340282366920938463463374607431768211454u128), Box::new(8u128), Box::new(34u128), Box::new(649037107316853453566312041152512u128), Box::new(7u128)])) { (unsafe { let mut slot = MaybeUninit::<(NonZeroU32, S2, Vec<f64>)>::uninit(); slot.write((NonZeroU32::new(32u32).unwrap(), S2 { f0: f32::NAN, f1: 11u16, f2: 170141183460469231731687303715884105727i128, f3: 35u8, f4: 1e10f32, f5: 14u8, f6: 28u16 }, Vec::<f64>::new())); slot.assume_init() }) } else { (if (true) { (NonZeroU32::new(13u32).unwrap(), S2 { f0: (-128.9f32), f1: 2u16, f2: (-168438918276705259651494617610777250399i128), f3: 17u8, f4: (-1.0f32), f5: 63u8, f6: 65365u16 }, vec![(-2.5e-3f64), 1e10f64]) } else { (NonZeroU32::new(1322007452u32).unwrap(), S2 { f0: 1e-40f32, f1: 40u16, f2: 2i128, f3: 7u8, f4: 1.5f32, f5: 17u8, f6: 15u16 }, vec![f64::NEG_INFINITY, 1e30f64, 0.2f64]) }) });
                    f235(apply_boxed(Box::new(|| -> i128 { v241.1.f2 })), (v241.1.f2 / 11i128)).mix();
                apply_dyn_mut(&mut |x249: u128| -> (NonZeroU32, S2, Vec<f64>) { (NonZeroU32::new(32u32).unwrap(), S2 { f0: (-0.0f32), f1: 127u16, f2: (-13i128), f3: 129u8, f4: f32::INFINITY, f5: 1u8, f6: 35u16 }, vec![f64::NEG_INFINITY, 3.25f64, 0.2f64]) }, 340282366920938463463374607431768211454u128)
            }));
    }
}
