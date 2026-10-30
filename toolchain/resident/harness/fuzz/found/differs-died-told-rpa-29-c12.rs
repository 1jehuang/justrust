// Found by fuzz.py: seed told-rpa-29, case c12, edition 2021, backend llvm.
// differs: died
// Reduced in 61 builds. Again: fuzz.py check differs-died-told-rpa-29-c12.rs
// ra-jit's program ended with status -11 in this case
// 
mod c12 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub struct S1<A> { pub f0: Vec<A>, pub f1: (char,), pub f2: Option<[f64; 8]> }
    impl<A: Mix> Mix for S1<A> {
        fn mix(&self) {
            feed(3);
            self.f0.mix();
            self.f1.mix();
            self.f2.mix();
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    pub enum E2 {
        V0(&'static str, f32),
        V1,
        V2(char),
    }
    impl Mix for E2 {
        fn mix(&self) {
            match self {
                E2::V0(a0, a1) => { feed(10); a0.mix(); a1.mix(); }
                E2::V1 => { feed(11); }
                E2::V2(a0) => { feed(12); a0.mix(); }
            }
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(u16)]
    pub enum E3 {
        V0,
        V1,
    }
    impl Mix for E3 {
        fn mix(&self) {
            match self {
                E3::V0 => { feed(10); }
                E3::V1 => { feed(11); }
            }
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    pub struct S4<A, B>(pub Option<A>, pub E3, pub (E2, f32), pub [B; 2]);
    impl<A: Mix, B: Mix> Mix for S4<A, B> {
        fn mix(&self) {
            feed(4);
            self.0.mix();
            self.1.mix();
            self.2.mix();
            self.3.mix();
        }
    }
    static K5: usize = 67108864usize.saturating_add(7usize);
    static K6: char = 'ß';
    pub trait T7 {
        fn m8(&self) -> u128;
        fn m9(&self, a10: (char,)) -> char {
            self.m8().mix();
            a10.0
        }
    }
    impl T7 for S1<Option<(f32, char)>> {
        fn m8(&self) -> u128 {
            let mut n11 = 0u32;
            'l12: while n11 < 8 && ((match (E3::V1) { E3::V1 if true => true, _ => true })) {
                n11 += 1;
                let mut v13: f32 = larger::<f32>((E2::V1, 891.1402f32).1, (match (None::<(f32, char)>) { Some((_, ..)) if true => f32::MAX, _ => f32::NEG_INFINITY }));
                (match (vec![('Z', [""; 4], 1.0f64), ('_', ["42"; 4], 1e30f64), ('\u{d7ff}', ["-7"; 4], (-9223372036854775808.0f64))].as_slice()) { [p14 @ .., ('A'..='Z', [p15, _, p16, _], _)] => S1 { f0: vec![Some(((-0.0f32), ' ')), Some(((-26.9996f32), '\'')), Some((1e30f32, '\u{10FFFF}')), Some((255.5f32, '中')), Some((0.2f32, '\\'))], f1: ('\u{7f}',), f2: Some([3.25f64, f64::INFINITY, 0.0f64, 1e30f64, 1e30f64, 1.5f64, 393.7817f64, (-55.1065f64)]) }, [(p18 @ '0'..='9', [.., p19], p20), p17 @ ..] if true => S1 { f0: vec![Some((1.0f32, '\u{d7ff}')), None::<(f32, char)>, Some((1.0f32, '\n')), None::<(f32, char)>, None::<(f32, char)>], f1: ('\0',), f2: Some([0.0f64, 65535.0f64, (-0.0f64), 196.7216f64, (-171.4627f64), 0.2f64, 628.7152f64, 1.5f64]) }, _ => (*self).clone() }).mix();
            }
            340282366920938463463374607431768211454u128.wrapping_sub(170141183460469231731687303715884105727u128)
        }
    }
    impl T7 for (E2, f32) {
        fn m8(&self) -> u128 {
            175646531598377572367638404390227386397u128.swap_bytes().checked_shl(3167624774u32.wrapping_mul(0u32)).unwrap_or(u128::from(10u64))
        }
    }
    impl T7 for Option<(f32, char)> {
        fn m8(&self) -> u128 {
            (match (Vec::<E3>::new().as_slice()) { [E3::V1, p21 @ ..] => ('a' as u128), [E3::V1, ..] => 35184372088833u128.wrapping_add(127u128), _ => (1u128.checked_ilog2().unwrap_or(0) as u128) }).checked_add(1u128.wrapping_add(2u128.checked_mul(4u128).unwrap_or(8u128))).unwrap_or((39u128.overflowing_add(209936986603950888224486022495625010077u128).1 as u128).rotate_right((false as u32)))
        }
    }
    #[inline]
    pub fn f23(a24: f32, a25: char, a26: ((bool, f32, f64), f32, [f32; 4]), a27: &mut [E3], a28: f32, a29: Option<[f64; 8]>, a30: &'static str, a31: u128, a32: char, a33: ((bool, f32, f64), f32, [f32; 4])) -> char {
        { let value = (if (pick::<bool>(true, &a26.0.0, &false).clone()) { (match (vec![('9', ["😀", "", "Abc Def", "-7"], 1.5f64), ('Z', ["a", "line\n", "-7", "a"], f64::MAX), ('\0', ["3.5", "3.5", "😀", "line\n"], 1.5f64)].as_slice()) { [_, ..] => E3::V0, [(p36 @ '\u{80}'..='\u{10FFFF}', [p37, _, _, _], _), p35 @ ..] => E3::V1, _ => E3::V0 }) } else { E3::V0 }); if let Some(slot) = a27.first_mut() { *slot = value; } }
        (match (vec!["42", "ff", "😀", "hello", "42"].as_slice()) { [p41 @ .., _] => Ok::<Option<[f64; 8]>, (char,)>(Some([4294967296.0f64; 8])), _ => Ok::<Option<[f64; 8]>, (char,)>(None::<[f64; 8]>) }).unwrap_or_else(|x39: (char,)| -> Option<[f64; 8]> { None::<((bool, f32, f64), f32, [f32; 4])>.map(|x42: ((bool, f32, f64), f32, [f32; 4])| -> [f64; 8] { [0.1f64, 900.5850f64, 0.1f64, (-1.0f64), 1e-40f64, f64::MAX, 0.5f64, 1.5f64] }) }).mix();
        let r43 = catch_unwind(AssertUnwindSafe(|| -> Option<[f64; 8]> {
            'l45: for i44 in ((44i8..46i8).rev()) {
                let S4(_, p46, (_, _), [p47, ..]): S4<E3, f32> = S4(Some(E3::V0), E3::V1, (E2::V0("ff", 65535.0f32), 232.1822f32), [0.2f32, (-128.9f32)]);
                (Some(65535.0f32), E2::V1).mix();
                let mut v48: Vec<f32> = (if (a33.0.0) { vec![4294967296.0f32, (-1.0f32), 3.25f32, f32::MIN, f32::NAN] } else { vec![0.0f32] });
            }
            match (&{ (if (false) { (Some(318.3113f32), E2::V2('z')) } else { (Some((-1.0f32)), E2::V2('\u{10FFFF}')) }) }) {
                (Some(p50), E2::V0(p51, _)) => {
                    (a33.0.0 as u128).mix();
                }
                _ => {}
            }
            {
                async fn work(seed: u32) -> u64 { let h52 = (seed as f64 * 0.5, seed as u8, seed % 3 == 0); let mut w53 = 0u64; for i in 0..1u64 { let _g = Noisy(i as u32); w53 += later((i % 2) as u32, i).await; } let h54 = [seed as u8; 17]; let w55 = later(0, [seed as u64; 9]).await[0]; let h56 = (seed as f64 * 0.5, seed as u8, seed % 3 == 0); let w57 = match later(1, seed % 3).await { 0 => later(0, 10u64).await, 1 => 11, _ => later(2, 12u64).await }; (h52.0 as u64 + h52.1 as u64 + h52.2 as u64).rotate_left(0) ^ (w53).rotate_left(1) ^ (h54[0] as u64).rotate_left(2) ^ (w55).rotate_left(3) ^ (h56.0 as u64 + h56.1 as u64 + h56.2 as u64).rotate_left(4) ^ (w57).rotate_left(5) }
                poll_some(work(9), 6).mix();
                poll_some(work(94), 3).mix();
                poll_some(work(93), 1).mix();
            }
            S1 { f0: ({ let mut items = Vec::<Option<(f32, char)>>::new(); items.retain(|x59: &Option<(f32, char)>| -> bool { true }); items }), f1: Some(('é',)).unwrap_or_else(|| -> (char,) { (' ',) }), f2: None::<[f64; 8]> }.f2
        }));
        match r43 {
            Ok(value) => value.mix(),
            Err(caught) => payload(caught),
        }
        (match (Some((0.5f32, '\''))) { Some((p62, ..)) => K6, Some((p63, p64)) => a25, _ if false => a32, _ => a25 })
    }
    pub fn f65(a66: &Option<(f32, char)>, a67: (char, [&'static str; 4], f64), a68: Option<(f32, char)>, a69: usize, a70: i128, a71: i8, a72: u16, a73: &char) -> i16 {
        match ((E2::V0("  pad  ", 0.0f32), 1.5f32)) {
            (E2::V1, _) if 2i16.overflowing_sub((-31363i16)).1 => {
                {
                    for armed in [false, true] { let acts: Vec<Box<dyn Fn() -> u32>> = vec![Box::new(|| 58), Box::new(move || trip(armed, 35)), Box::new(|| 3)]; caught(|| { let _g = Noisy(58); for act in &acts { feed(act() as u64); } }); }
                }
                let (_, _, _): (char, [&'static str; 4], f64) = (match (({ let mut items = [vec![E3::V0, E3::V0], vec![E3::V0, E3::V0]].concat(); items.push(E3::V1); items }).as_slice()) { [E3::V0, p76 @ ..] => (match ((match (vec![E2::V2('0'), E2::V1].as_slice()) { [E2::V1, p77 @ .., E2::V0(_, p78)] if true => String::from("0123456789"), _ => String::from("a,b,,c") }).as_str()) { "a,b,,c" => (match (vec![('ß',), ('\\',)].as_slice()) { [p79 @ .., (p80 @ '\0'..='\u{7f}',)] => a67, _ => a67 }), "line\n" => a67, _ => a67 }), _ => a67 });
            }
            (_, p81) => {
                usize::try_from(Ok::<u64, f64>(13u64).unwrap_or_else(|x82: f64| -> u64 { 18014398509481984u64.wrapping_shr(3442973535u32) })).unwrap_or(a69.isqrt()).mix();
            }
            (E2::V0("a", p83), ..) => {
                let (_, p84): (E2, f32) = (if (true) { (E2::V1, (-701.6272f32)) } else { (E2::V1, f32::NAN) });
            }
            _ => {}
        }
        let mut n85 = 0u32;
        'l86: while n85 < 3 && ((loop { break false; })) {
            n85 += 1;
            let mut v87: i128 = (loop { break (if (true) { a70 } else { a70 }); }).wrapping_sub((if (Some(2i8).is_some()) { a70 } else { i128::from(a72) }));
            {
                enum Loud { I(i32, Noisy), C(char, Noisy) }
                #[inline(never)] fn items(k: u32, last: char) -> [Loud; 3] { black_box([Loud::I(k as i32 % 4, Noisy(k)), Loud::C('q', Noisy(k + 1)), if k % 3 == 0 { Loud::I(1, Noisy(k + 2)) } else { Loud::C(last, Noisy(k + 2)) }]) }
                #[inline(never)] fn pick0(k: u32, last: char, flag: bool) -> u8 { match items(k, last) { [_, Loud::C(..), Loud::C('Z', _)] if black_box(flag) => 1, [x, Loud::C(..), _] => 2, _ => 0 } }
                for k in 0..4u32 { for last in ['a', 'Z', 'q'] { for flag in [false, true] { feed(pick0(k, last, flag) as u64); feed(0xf00d); } } }
                #[inline(never)] fn pair(k: u32, last: char, flag: bool) -> u8 { match (items(k, last), k % 3, flag) { ([Loud::C('a'..='c', _), ..], 2, true) => 1, ([.., Loud::C('a'..='c', _) | Loud::I(1, _)], 2, _) => 2, ([x, m @ ..], 2, false) => 3, (all, _, _) if black_box(flag) => 4, _ => 0 } }
                for k in 0..4u32 { for last in ['a', 'Z'] { for flag in [false, true] { feed(pair(k, last, flag) as u64); feed(0xf00d); } } }
            }
            {
                let mut items: Vec<Box<dyn Any>> = vec![Box::new(()), Box::new(Some(60i64)), Box::new(String::from("owned"))];
                for item in &items { if let Some(it) = item.downcast_ref::<u32>() { feed(*it as u64); } else if let Some(it) = item.downcast_ref::<&str>() { it.mix(); } else if item.is::<Noisy>() { feed(1); } else if let Some(it) = item.downcast_ref::<(u8, bool)>() { feed(it.0 as u64 + it.1 as u64); } else { feed(2); } }
                { let first = items.swap_remove(0); match first.downcast::<u32>() { Ok(it) => feed(*it as u64), Err(_) => feed(3) } }
            }
            {
                for armed in [false, true] { caught(|| { let t = (Noisy(2), Noisy(43), Noisy(23)); let _x = t.1; trip(armed, ()); let _y = t.0; }); }
                for armed in [false, true] { caught(|| { let t = (Some(Noisy(2)), Duo { a: Noisy(43), b: Noisy(23) }); let _x = t.1.b; trip(armed, ()); let _late = Noisy(57); }); }
            }
        }
        match ((if (true) { ('_', ["ff", "42", "3.5", "  pad  "], (-1.0f64)) } else { a67 })) {
            p88 => {
                let mut v89: BTreeMap<i8, bool> = BTreeMap::new();
                v89.insert(a71, f64::from(a72).is_infinite()).mix();
                v89.insert((loop { break 0i8.wrapping_add(a71); }), (if (K5.overflowing_sub(a69).1) { (2097152usize > 26usize) } else { true })).mix();
                v89.get(&(a71.overflowing_add(13i8).1 as i8)).cloned().mix();
                v89.len().mix();
                {
                    trait Shape { fn area(&self) -> u64; fn scaled(&self, by: u64) -> u64 where Self: Sized { self.area() * by } }
                    trait Conv<T> { fn conv(&self) -> T; }
                    struct Sq(u64); struct Rect(u64, u64);
                    impl Shape for Sq { fn area(&self) -> u64 { self.0 * self.0 } }
                    impl Shape for Rect { fn area(&self) -> u64 { self.0 * self.1 } }
                    impl<T: Shape + ?Sized> Shape for Box<T> { fn area(&self) -> u64 { (**self).area() + 1 } }
                    impl Conv<u64> for Sq { fn conv(&self) -> u64 { self.0 } } impl Conv<String> for Sq { fn conv(&self) -> String { self.0.to_string() } }
                    impl dyn Shape { fn doubled(&self) -> u64 { self.area() * 2 } }
                    fn total<T: Shape + ?Sized>(it: &T) -> u64 { it.area() }
                    fn sum(shapes: &[&dyn Shape]) -> u64 { shapes.iter().map(|it| total(*it)).sum() }
                    let (sq, rect) = (Sq(4), Rect(4, 19));
                    feed(total(&sq) + total(&rect as &dyn Shape) + sq.scaled(3));
                    { let mut counts: Box<dyn Iterator<Item = u64>> = Box::new((1..4u64).map(|it| it * 3)); feed(counts.next().unwrap_or(0)); feed(counts.sum()); }
                    { let mut calls = 0u64; let mut bump: Box<dyn FnMut(u64) -> u64> = Box::new(|by| { calls += by; calls }); bump(2); feed(bump(5)); }
                    { let nested: Box<Box<dyn Shape>> = Box::new(Box::new(Sq(3))); feed(nested.area()); }
                    feed(sum(&[&sq, &rect, &Sq(2)]));
                }
            }
            (p90, ..) => {
                let mut v91: i8 = (if (true) { 2i8 } else { a71 }).wrapping_mul((-11i8));
            }
            _ => {}
        }
        i16::from(a71)
    }
    pub fn f92(a93: (char, [&'static str; 4], f64), a94: &'static str, a95: f32, a96: S4<(char, [&'static str; 4], f64), &'static str>) -> [(char, f64, char, f32, bool, isize); 1] {
        {
            for armed in [false, true] { caught(|| { let mut kept = Vec::new(); let mut i = 0u32; 'out: loop { if i >= 3 { break 'out; } let n = Noisy(i); if i == 3 { trip(armed, ()); } if i % 2 == 1 { kept.push(n); i += 1; continue; } let _m = Noisy(10 + i); if i == 3 { break 'out; } i += 1; } feed(kept.len() as u64); }); }
        }
        (match (vec![false, false, false].as_slice()) { [p101 @ .., true] if false => Some([('é', 1e10f64, '\'', (-0.0f32), false, 128isize); 1]), [false, .., true] => Some([('0', f64::INFINITY, ' ', (-2.5e-3f32), true, 8809598092302795428isize)]), _ => Some([('\u{d7ff}', f64::MIN_POSITIVE, 'z', 372.1694f32, false, 2097153isize)]) }).map_or((match (vec![7i128, 4i128, (-10i128), 80016023144940287867770406196194043189i128, (-12i128)].as_slice()) { [(100i128 | ..=7i128), p105 @ ..] => [('\u{10FFFF}', 0.0f64, '\'', (-2.5e-3f32), false, 9isize); 1], _ => [('_', 0.5f64, 'Z', 3.25f32, true, (-9223372036854775807isize))] }), |x99: [(char, f64, char, f32, bool, isize); 1]| -> [(char, f64, char, f32, bool, isize); 1] { x99 })
    }
    pub fn f106(a107: &(char, f64, S4<E3, f32>, E3), a108: u32, a109: &'static str, a110: i16, a111: f64, a112: f64, a113: i8, a114: S4<(char, [&'static str; 4], f64), &'static str>) -> i32 {
        4194303i32.overflowing_sub((-2147483648i32)).0.saturating_pow(Ok::<u32, E2>(10u32).unwrap_or(a108) % 7)
    }
    pub fn f116(a117: S4<E3, f32>, a118: u16, a119: &(char, f64, S4<E3, f32>, E3), a120: i16, a121: Option<(f32, char)>, a122: &(char,)) -> (E2, f32) {
        let mut v123: BTreeMap<i16, (char, [&'static str; 4], f64)> = BTreeMap::new();
        v123.insert(a120, ('\\', ["x y", "hello", "x y", ""], f64::MIN)).mix();
        v123.get(&a120.wrapping_mul(a120)).cloned().mix();
        v123.len().mix();
        a117.2
    }
    pub fn run() {
        match (&{ (if (false) { ('\n', 0.0f64, S4(Some(E3::V0), E3::V0, (E2::V1, 1.0f32), [1e30f32, (-128.9f32)]), E3::V1) } else { ('9', 1e10f64, S4(Some(E3::V1), E3::V0, (E2::V0("x y", 3.25f32), 1.0f32), [1e30f32, (-9223372036854775808.0f32)]), E3::V1) }) }) {
            (p124, p125, _, E3::V1) if '9'.is_uppercase() => {
                let mut v126: Option<[f64; 8]> = None::<[f64; 8]>;
            }
            _ => 0i128.mix(),
        }
        let mut v128: char = ({ let mut m129: Vec<E3> = vec![E3::V1, E3::V0, E3::V0]; let out = f23(f32::MAX, K6, ((false, 0.2f32, 1e-40f64), 65535.0f32, [f32::INFINITY, f32::MAX, 0.0f32, (-821.9547f32)]), &mut m129, 526.0621f32, Some([f64::NAN; 8]), "  pad  ", 192257228606652253690795924890865248196u128, K6, ((true, 4294967296.0f32, f64::INFINITY), (-2.5e-3f32), [1e10f32; 4])); m129.mix(); out });
        let (_, [p131, ..], p132): (char, [&'static str; 4], f64) = apply(|x130: [E2; 2]| -> (char, [&'static str; 4], f64) { ('_', [""; 4], 0.5f64) }, [E2::V1, E2::V0("", 1e10f32)]);
        match (({ let mut text = String::from("a,b,,c"); text.push_str(p131); text }).as_str()) {
            "-7" => {
                let mut n133 = 0u32;
                let v135: S4<E3, f32> = 'l134: loop {
                    n133 += 1;
                    if n133 > 1 { break (if (true) { S4(Some(E3::V0), E3::V1, (E2::V0("😀", 1e10f32), 819.1213f32), [(-95.7424f32), 3.25f32]) } else { S4(Some(E3::V0), E3::V1, (E2::V1, (-0.0f32)), [(-1.0f32), f32::MIN_POSITIVE]) }); }
                    {
                        for armed in [false, true] { caught(|| { let t = [Noisy(45), Noisy(27), Noisy(21)]; trip(armed, ()); let [_x, ..] = t; }); }
                    }
                    match (&{ Err::<[f64; 2], Option<(f32, char)>>(Some(((-692.2605f32), 'ß'))).unwrap_or([(-9223372036854775808.0f64); 2]) }) {
                        [p139, p140] if (match (((false, 4294967296.0f32, f64::EPSILON), 0.1f32, [f32::EPSILON, (-2.5e-3f32), f32::MIN_POSITIVE, 7.0f32])) { ((p141, ..), p142, [p143, p144, p145, p146]) if false => p141, (p147, p148, [p149, _, _, p150]) => p147.0, p151 if p151.0.0 => p151.0.0, _ => true }) => {
                            {
                                for armed in [false, true] { caught(|| { let t = (Some(Noisy(13)), Duo { a: Noisy(15), b: Noisy(18) }); trip(armed, ()); let _x = t.1.b; let _y = t.0; let _late = Noisy(24); }); }
                                for armed in [false, true] { caught(|| { let t = [Noisy(13), Noisy(15), Noisy(18)]; trip(armed, ()); let [_, _x @ ..] = t; let _late = Noisy(43); }); }
                                for armed in [false, true] { caught(|| { let t = [Noisy(13), Noisy(15), Noisy(18)]; let [_x, ..] = t; trip(armed, ()); let _late = Noisy(19); }); }
                            }
                            {
                                { let x = black_box((2147483647.0) as f32); for word in [(x as isize) as i128 as u128 as u64, (x as u16) as i128 as u128 as u64, (x as u128) as i128 as u128 as u64, (x as i8) as i128 as u128 as u64, (x as u8) as i128 as u128 as u64, (x as i128) as i128 as u128 as u64] { feed(word); } }
                                { let x = black_box((f64::MIN) as f64); for word in [(x as u32) as i128 as u128 as u64, (x as i8) as i128 as u128 as u64, (x as u16) as i128 as u128 as u64, (x as i128) as i128 as u128 as u64] { feed(word); } }
                                { let x = black_box((1.5) as f64); for word in [(x as i8) as i128 as u128 as u64, (x as i16) as i128 as u128 as u64, (x as i32) as i128 as u128 as u64, (x as i128) as i128 as u128 as u64] { feed(word); } }
                                { let x = black_box((2147483648.0) as f64); for word in [(x as i16) as i128 as u128 as u64, (x as i64) as i128 as u128 as u64, (x as u16) as i128 as u128 as u64, (x as u32) as i128 as u128 as u64, (x as i32) as i128 as u128 as u64] { feed(word); } }
                                { let x = black_box((3.4028236692093846e38) as f32); for word in [(x as usize) as i128 as u128 as u64, (x as u64) as i128 as u128 as u64, (x as i128) as i128 as u128 as u64, (x as i64) as i128 as u128 as u64, (x as i16) as i128 as u128 as u64] { feed(word); } }
                                { let x = black_box(77 as u16); (x as f32).mix(); (x as f64).mix(); ((x as f32) as u16).mix(); ((x as f64) as u16).mix(); (x as f64 as f32).mix(); }
                                { let x = black_box(i32::MIN as i32); (x as f32).mix(); (x as f64).mix(); ((x as f32) as i32).mix(); ((x as f64) as i32).mix(); (x as f64 as f32).mix(); }
                            }
                        }
                        _ => <[f64; 2]>::try_from(Vec::<f64>::new()).unwrap_or([0.1f64, 65535.0f64]).mix(),
                    }
                };
            }
            "Abc Def" if '_'.is_uppercase() => {
                {
                    let mut items: Vec<Box<dyn Any>> = vec![Box::new(String::from("owned")), Box::new(30u32), Box::new("text"), Box::new(Some(30i64)), Box::new(Noisy(2))];
                    for item in &items { if let Some(it) = item.downcast_ref::<u32>() { feed(*it as u64); } else if let Some(it) = item.downcast_ref::<&str>() { it.mix(); } else if item.is::<Noisy>() { feed(1); } else if let Some(it) = item.downcast_ref::<(u8, bool)>() { feed(it.0 as u64 + it.1 as u64); } else { feed(2); } }
                    { let first = items.swap_remove(0); match first.downcast::<u32>() { Ok(it) => feed(*it as u64), Err(_) => feed(3) } }
                }
                let ((p158, p159, _), p160, [p161, p162, _, _]): ((bool, f32, f64), f32, [f32; 4]) = (match (None::<Option<(f32, char)>>.and_then(|x154: Option<(f32, char)>| -> Option<[f64; 8]> { None::<[f64; 8]> })) { Some([p156, p155 @ .., p157]) => (if (true) { ((true, 0.0f32, f64::NEG_INFINITY), 1.5f32, [255.5f32, 255.5f32, 1e30f32, (-511.0121f32)]) } else { ((true, (-797.6906f32), 0.0f64), f32::MIN_POSITIVE, [255.5f32; 4]) }), _ => ((false, 16777217.0f32, 1.0f64), (-0.0f32), [(-1.0f32); 4]) });
            }
            "a,b,,c" => {
                (if (Ok::<bool, u128>(false).unwrap_or((false || false))) { (if let Some(x164) = (Some(false)) { x164 } else { 4u16.overflowing_sub(47069u16).1 }) } else { v128.is_uppercase() }).mix();
            }
            _ => p131.mix(),
        }
        let mut n166 = 0u32;
        'l167: while n166 < 3 && (id::<bool>(true)) {
            n166 += 1;
            i8::try_from(65401740448376406282824263752455291959u128.wrapping_shl(1u32)).unwrap_or(i8::try_from((-18i8)).unwrap_or(1i8)).mix();
            match (String::from("ff").as_str()) {
                p168 if '中'.is_alphanumeric() => {
                    "héllo".mix();
                }
                "3.5" => {
                    (if (1024u16.overflowing_sub(100u16).1) { 255.5f32 } else { apply_mut(|x169: E3| -> f32 { (-128.9f32) }, E3::V0).1 }).mix();
                    let (p170, p171): (E2, f32) = id::<(E2, f32)>((E2::V2(' '), (-67.0044f32)));
                }
                _ => {}
            }
                async fn work(seed: u32) -> u64 { let h172 = (seed as f64 * 0.5, seed as u8, seed % 3 == 0); let mut w173 = 0u64; for i in 0..3u64 { let _g = Noisy(i as u32); w173 += later((i % 2) as u32, i).await; } (h172.0 as u64 + h172.1 as u64 + h172.2 as u64).rotate_left(0) ^ (w173).rotate_left(1) }
                block_on(work(31)).mix();
            'l175: for i174 in (20i8..24i8) {
                (if (false) { 0.1f32 } else { f32::NEG_INFINITY }).mix();
            }
        }
        {
            for (i, f) in TABLE.iter().enumerate() { feed(f(i as u32 + 10) as u64); }
            PICKED.mix(); feed(std::ptr::eq(*PICKED, WORDS[2]) as u64);
            LISTS.0.mix(); for part in LISTS.1 { part.mix(); } feed(LISTS.2[0].map_or(0, |it| *it) as u64 + LISTS.2[1].is_none() as u64);
        }
        K6.mix();
        let mut n176 = 0u32;
        let v178: (Option<f32>, E2) = 'l177: loop {
            n176 += 1;
            if (true) { break 'l177 (match (vec![E3::V0].as_slice()) { [E3::V0, p179 @ ..] => (Some(1.0f32), E2::V1), _ => (Some((-128.9f32)), E2::V1) }); }
            match ([E2::V2('\u{80}'), E2::V0("line\n", 0.2f32)][1]) {
                E2::V0(p180, p181) => {
                    if (((match (Some(((-128.9f32), '\u{80}'))) { None => true, _ => false }) && 8i16.overflowing_add(0i16).1)) { break 'l177 (Some(f32::EPSILON), E2::V2('\u{e000}')); }
                    match (larger::<Vec<E2>>(vec![E2::V0("a,b,,c", (-128.9f32))], vec![E2::V2('\u{10FFFF}'), E2::V1]).as_slice()) {
                        [E2::V0(p184, p185), p183 @ ..] => {
                            let p189: (Option<f32>, E2) = (if ((match (Some(false)) { None => true, Some(x186) => x186 })) { (match (String::from("😀").as_str()) { p187 => (None::<f32>, E2::V0("42", 3.25f32)), _ => (None::<f32>, E2::V0("0123456789", 65535.0f32)) }) } else { (match (String::from("").as_str()) { p188 => (Some(4294967296.0f32), E2::V1), _ => (Some(255.5f32), E2::V2('中')) }) });
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            (if (v128.is_control()) { S4(Some(E3::V0), E3::V0, (E2::V0("", 16777217.0f32), 16777217.0f32), [65535.0f32, f32::MIN]) } else { (match ([1e30f64, f64::INFINITY]) { [.., p190] if true => S4(None::<E3>, E3::V0, (E2::V1, (-384.3245f32)), [106.1326f32; 2]), [p191, .., _] => S4(None::<E3>, E3::V0, (E2::V2('\u{80}'), 1.5f32), [(-308.2611f32); 2]), [.., _] => S4(Some(E3::V0), E3::V1, (E2::V2('\u{d7ff}'), (-1.0f32)), [1e10f32, f32::INFINITY]), _ => S4(Some(E3::V0), E3::V1, (E2::V2('é'), f32::INFINITY), [f32::MIN, f32::INFINITY]) }) }).mix();
        };
        (match (id::<Vec<E3>>(vec![E3::V0, E3::V0, E3::V0, E3::V0, E3::V1]).as_slice()) { [E3::V1, p194 @ ..] => S4(Some(E3::V1), E3::V0, (E2::V2('_'), f32::EPSILON), [1e30f32; 2]), _ => S4(Some(E3::V1), E3::V1, (E2::V2('\u{80}'), 3.25f32), [16777217.0f32, 0.1f32]) }).mix();
            static NUMBERS: [u32; 4] = [13, 15, 27, 4];
            static SECOND: &u32 = &NUMBERS[1];
            static TAIL: &[u32] = match &NUMBERS { [_, _, rest @ ..] => rest };
            static WORDS: [&str; 3] = ["one", "two", "three"];
            static PICKED: &&str = &WORDS[2];
            struct Link { value: u32, next: Option<&'static Link>, call: fn(u32) -> u32, name: &'static str, shown: &'static (dyn Debug + Sync) }
            fn double(x: u32) -> u32 { x.wrapping_mul(2) }
            static END: Link = Link { value: 3, next: None, call: double, name: "end", shown: &NUMBERS };
            static MID: Link = Link { value: 13, next: Some(&END), call: |x| x + 1, name: WORDS[0], shown: &"mid" };
            static HEAD: Link = Link { value: 15, next: Some(&MID), call: double, name: "head", shown: &MID.value };
            static TABLE: [fn(u32) -> u32; 3] = [double, |x| x ^ 5, HEAD.call];
            static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            static LISTS: (&[u8], &[&[u8]], [Option<&u32>; 2]) = (b"bytes", &[b"a", b"", &[13, 2]], [Some(&NUMBERS[3]), None]);
            const VIEW: &[u32; 4] = &NUMBERS;
            thread_local! { static LOCAL: Cell<u32> = const { Cell::new(5) }; static NOTES: RefCell<Vec<&'static u32>> = const { RefCell::new(Vec::new()) }; }
            LOCAL.with(|it| { it.set(it.get() + 1); feed(it.get() as u64 % 2 + 40); }); NOTES.with(|it| { it.borrow_mut().push(SECOND); feed(*it.borrow()[0] as u64); });
            { let mut at = Some(&HEAD); while let Some(link) = at { feed((link.call)(link.value) as u64); link.name.mix(); format!("{:?}", link.shown).mix(); at = link.next; } }
            { let before = COUNTER.fetch_add(3, Ordering::Relaxed); feed((COUNTER.load(Ordering::Relaxed) - before) as u64); }
            for (i, f) in TABLE.iter().enumerate() { feed(f(i as u32 + 10) as u64); }
            PICKED.mix(); feed(std::ptr::eq(*PICKED, WORDS[2]) as u64);
        (if (((true || false) && (match (vec![(-9223372036854775808.0f32), f32::MIN_POSITIVE, 623.1350f32, (-128.9f32), 3.25f32].as_slice()) { [p196, p195 @ ..] => false, _ => true }))) { (match (&{ ((true, (-128.9f32), 4294967296.0f64), 65535.0f32, [f32::NEG_INFINITY, f32::NAN, 990.4192f32, f32::NAN]) }) { ((true, _, p197), _, [p199, p198 @ .., p200]) => K5, _ => K5 }).isqrt() } else { (if (true) { (34359738367i64 as usize) } else { K5.rotate_right(2u32) }) }).mix();
    }
}
