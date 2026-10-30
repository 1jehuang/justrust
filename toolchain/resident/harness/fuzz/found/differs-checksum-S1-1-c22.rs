// Found by fuzz.py: seed S1-1, case c22, edition 2024, backend cranelift.
// differs: checksum
// Reduced in 25 builds. Again: fuzz.py check differs-checksum-S1-1-c22.rs
// rustc:
// panic: panicked at src/main.rs:362:9
// panic: panicked at src/main.rs:362:9
// panic: panicked at src/main.rs:362:9
// c22 69cd4e290aa883a6 ok
// ra-jit:
// panic: panicked at src/main.rs:362:9
// panic: panicked at src/main.rs:362:9
// panic: panicked at src/main.rs:362:9
// c22 81ed96063a0346f6 ok
mod c22 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub enum E1<A, B> {
        V0 { f0: char, f1: f64 },
        V1 { f0: char, f1: i8, f2: Option<B>, f3: Vec<A>, f4: f64 },
    }
    impl<A: Mix, B: Mix> Mix for E1<A, B> {
        fn mix(&self) {
            match self {
                E1::V0 { f0: a0, f1: a1 } => { feed(10); a0.mix(); a1.mix(); }
                E1::V1 { f0: a0, f1: a1, f2: a2, f3: a3, f4: a4 } => { feed(11); a0.mix(); a1.mix(); a2.mix(); a3.mix(); a4.mix(); }
            }
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    #[repr(u64)]
    pub enum E2 {
        V0,
        V1(i8, i8),
        V2 { f0: E1<i8, i8>, f1: E1<i8, i8> },
    }
    impl Mix for E2 {
        fn mix(&self) {
            match self {
                E2::V0 => { feed(10); }
                E2::V1(a0, a1) => { feed(11); a0.mix(); a1.mix(); }
                E2::V2 { f0: a0, f1: a1 } => { feed(12); a0.mix(); a1.mix(); }
            }
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    #[repr(C, u8)]
    pub enum E3 {
        V0(char),
        V1(f64, u64),
        V2,
        V3(E1<u128, i8>),
    }
    impl Mix for E3 {
        fn mix(&self) {
            match self {
                E3::V0(a0) => { feed(10); a0.mix(); }
                E3::V1(a0, a1) => { feed(11); a0.mix(); a1.mix(); }
                E3::V2 => { feed(12); }
                E3::V3(a0) => { feed(13); a0.mix(); }
            }
        }
    }
    const K4: i8 = (-5i8);
    pub trait T5 {
        fn m6(&self) -> E1<i8, i8>;
    }
    impl T5 for E1<u128, i8> {
        fn m6(&self) -> E1<i8, i8> {
            let mut v7: u64 = ((E1::<u128, i8>::V1 { f0: '_', f1: 0i8, f2: Some((-84i8)), f3: vec![0u128, 293494295621791653072558943050373376442u128, 7u128], f4: (-20.7652f64) } == id::<E1<u128, i8>>(E1::<u128, i8>::V1 { f0: 'ß', f1: (-17i8), f2: Some((-18i8)), f3: vec![2u128, 51998320914251126868468250579414542722u128, 1180591620717411303424u128, 47870778265687094419458166880671707414u128, 0u128], f4: 1.5f64 })) as u64);
            {
                async fn work(seed: u32) -> u64 { let mut h8 = seed as u64; let h8r = &mut h8; *h8r += 49; let w9 = later(0, seed as u64 + 58).await; let h10: u128 = (seed as u128) << 54; let w11 = match later(1, seed % 3).await { 0 => later(0, 10u64).await, 1 => 11, _ => later(2, 12u64).await }; let h12 = format!("{seed}-51"); let w13 = later(0, [seed as u64; 2]).await[0]; let h14: u128 = (seed as u128) << 60; let w15 = later(2, seed as u64 + 5).await; (*h8r).rotate_left(0) ^ (w9).rotate_left(1) ^ ((h10 >> 3) as u64).rotate_left(2) ^ (w11).rotate_left(3) ^ (h12.len() as u64).rotate_left(4) ^ (w13).rotate_left(5) ^ ((h14 >> 3) as u64).rotate_left(6) ^ (w15).rotate_left(7) }
                block_on(work(84)).mix();
                block_on(work(60)).mix();
            }
            apply_once(|| -> E1<i8, i8> { E1::<i8, i8>::V0 { f0: '\u{e000}', f1: f64::MIN } })
        }
    }
    impl T5 for f64 {
        fn m6(&self) -> E1<i8, i8> {
            E1::<i8, i8>::V0 { f0: '😀', f1: (*self) }
        }
    }
    pub extern "C" fn f17(a18: f64, a19: u64, a20: f64, a21: u64) -> f64 {
        if let E1::V1 { f0: '\u{10FFFF}', f1: 51i8, f2: Some(3i8), f3: p22, f4: p23 } = (&{ E1::<i8, i8>::V1 { f0: '9', f1: K4, f2: None::<i8>, f3: vec![100i8, 3i8], f4: 0.1f64 } }) {
            let mut v24: char = char::from_u32(apply_dyn_mut(&mut |x25: [Box<u16>; 3]| -> u32 { 4272035822u32 }, [Box::new(25312u16), Box::new(29u16), Box::new(14u16)])).unwrap_or((117u8 as char));
        }
        (a20 + (99.0139f64 % a20))
    }
    pub fn f28<G26: Mix + Clone + Debug + PartialOrd, G27: Mix + Clone + Debug + PartialOrd>(a29: &G26, a30: G27, a31: char, a32: (G26, u8), a33: [G27; 2], a34: i8) -> Option<G26> {
        {
            { let mut d = Duo { a: 23u32, b: Noisy(18) }; let mut inc = || d.a += 1; inc(); inc(); let take = move || d.b; feed(d.a as u64); take().mix(); }
            { let d = Duo { a: Noisy(23), b: Noisy(18) }; let only_a = move || d.a.0; feed(only_a() as u64); d.b.mix(); }
        }
        let v35: [Box<u16>; 3] = apply_dyn_mut(&mut |x38: char| -> [Box<u16>; 3] { std::array::from_fn::<Box<u16>, 3, _>(|i39: usize| -> Box<u16> { Box::new(3u16) }) }, a31);
        None::<G26>
    }
    pub extern "C" fn f41(a42: i8, a43: i8, a44: u64, a45: i8, a46: i8, a47: f64) -> i8 {
        0i8
    }
    #[inline(never)]
    pub fn f49<G48: Mix + Clone + Debug + PartialOrd>(a50: &G48) -> [Box<u16>; 3] {
        let mut v51: G48 = (if (('\u{10FFFF}' <= '\n')) { (*a50).clone() } else { (*a50).clone() });
        {
            struct Outer(Noisy, Bomb, Noisy);
            impl Drop for Outer { fn drop(&mut self) { feed(0x0d0d + self.0.0 as u64); } }
            caught(|| { let _a = Noisy(18); let _ = Some(Bomb(54)); let _z = Noisy(55); });
        }
        (41i8..47i8).flat_map(|x52: i8| -> Vec<[Box<u16>; 3]> { (if (([Box::new(0u16), Box::new(36u16), Box::new(35u16)] != [Box::new(39434u16), Box::new(9u16), Box::new(26u16)])) { id::<Vec<[Box<u16>; 3]>>(vec![[Box::new(10u16), Box::new(56065u16), Box::new(7u16)]]) } else { apply_dyn_mut(&mut |x56: char| -> Vec<[Box<u16>; 3]> { vec![[Box::new(32u16), Box::new(11u16), Box::new(2u16)]] }, 'z') }) }).flat_map(|x57: [Box<u16>; 3]| -> Vec<char> { vec!['\0', '9', '\0'] }).fold((loop { break K4.wrapping_rem_euclid(K4 | 1); }), |acc58: i8, x59: char| -> i8 { ((Some(NonZeroU8::new(255u8).unwrap()), (false,)).cmp(&(Some(NonZeroU8::new(2u8).unwrap()), (true,))) as i8).saturating_sub(((-18i8) << (2906701455u32 % 8))) }).mix();
        [[(134217727i32, '\'')]; 3].map(|x61: [(i32, char); 1]| -> Box<u16> { ({ let t62: i8 = 8i8; Box::new(17u16) }) })
    }
    pub extern "C" fn f63(a64: i8, a65: i8) -> f64 {
        926.6783f64.recip().clamp(-1.5f64, 2.5f64)
    }
    #[inline(never)]
    pub extern "C" fn f66(a67: i8, a68: i8, a69: i8) -> f64 {
        f64::NEG_INFINITY
    }
    pub fn run() {
        'l71: for i70 in ((1i8..3i8).step_by(2)) {
            let mut v72: f64 = (3.25f64 % 0.0f64);
            let v73: Option<(char, bool, bool)> = f28::<(char, bool, bool), E1<i8, i8>>(&(char::from_u32(4294967295u32).unwrap_or('\\'), apply_once(|| -> bool { false }), vec![[(10i32, '😀')], [(2i32, '\u{d7ff}'); 1], [(8i32, '\u{e000}')], [(0i32, '\\')], [((-13i32), '0')]].contains(&[(8i32, '\'')])), (Box::new((unsafe { let mut slot = MaybeUninit::<f64>::uninit(); slot.write(v72); slot.assume_init() })) as Box<dyn T5>).m6(), id::<char>(id::<char>('\'')), (('\u{d7ff}', true, true), 0u8), std::array::from_fn::<[(i32, char); 1], 2, _>(|i78: usize| -> [(i32, char); 1] { [(2038488027i32, '9')] }).map(|x77: [(i32, char); 1]| -> E1<i8, i8> { E1::<i8, i8>::V0 { f0: '_', f1: f64::MIN_POSITIVE } }), i70);
        }
        let v80: char = None::<char>.unwrap_or_else(|| -> char { (unsafe { *(&id::<char>('ß') as *const char) }) });
        ("ff".parse::<i8>().unwrap_or(7i8) >> (((true ^ true) as u32) % 8)).mix();
        let mut o83: Box<dyn T5> = Box::new((f64::MAX.fract() - ((-9223372036854775808.0f64) / (-483.9223f64))).sqrt());
        let mut v84: Vec<E1<i8, i8>> = ({ let mut items = vec![E1::<i8, i8>::V1 { f0: '中', f1: 8i8, f2: Some(17i8), f3: Vec::<i8>::new(), f4: f64::MIN_POSITIVE }, E1::<i8, i8>::V1 { f0: 'a', f1: 2i8, f2: Some((-127i8)), f3: vec![(-69i8)], f4: 1e10f64 }, E1::<i8, i8>::V0 { f0: '9', f1: 1.5f64 }]; items.dedup(); items });
        v84.truncate(6);
        'l86: while let Some(x85) = v84.pop() {
            let mut v88: E1<i8, i8> = E1::<i8, i8>::V0 { f0: String::from("a").chars().next().unwrap_or(v80), f1: 0.0f64.round() };
            let v89: E1<i8, i8> = v88;
        }
        {
            let f90 = || -> [Box<u16>; 3] { [Box::new(16u16), Box::new(1u16), Box::new(8u16)] };
            if let Some(E1::V1 { f2: None, f3: p92, f4: p93, .. }) = (vec![E1::<i8, i8>::V1 { f0: 'a', f1: (-7i8), f2: Some(9i8), f3: Vec::<i8>::new(), f4: (-9223372036854775808.0f64) }, E1::<i8, i8>::V0 { f0: '\'', f1: f64::EPSILON }].first().cloned()) {
                let mut v94: i8 = f41(17i8, K4, 2825207557934187069u64, (-9i8), K4, p93);
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
                    let (sq, rect) = (Sq(19), Rect(19, 14));
                    { let mut counts: Box<dyn Iterator<Item = u64>> = Box::new((1..4u64).map(|it| it * 3)); feed(counts.next().unwrap_or(0)); feed(counts.sum()); }
                    feed(total(&sq) + total(&rect as &dyn Shape) + sq.scaled(3));
                    { let nested: Box<Box<dyn Shape>> = Box::new(Box::new(Sq(3))); feed(nested.area()); }
                    feed(sum(&[&sq, &rect, &Sq(2)]));
                }
            }
            let mut v95: i8 = (K4 >> (3u32 % 8));
            let mut v96: i8 = (-10i8).wrapping_div(v95 | 1);
            f90().mix();
        }
        let mut v97: E1<i8, i8> = E1::<i8, i8>::V1 { f0: 'z', f1: 120i8, f2: Some(1i8), f3: vec![14i8], f4: 7.0f64 };
        let mut v99: [Box<u16>; 3] = std::array::from_fn::<Box<u16>, 3, _>(|i101: usize| -> Box<u16> { Box::new(2u16) });
        {
            let mut f100 = |a98: E3| -> Option<(char, bool, bool)> { v99 = ({ let t107: E1<i8, i8> = v97.clone(); [[(14i32, '\\')], [(67108865i32, '\u{7f}'); 1], [((-18i32), '\''); 1]] }).map(|x104: [(i32, char); 1]| -> Box<u16> { Box::new((*v99[2])) }); None::<(char, bool, bool)> };
            {
                { let mut t = (51u64, (15u64, String::from("s"))); let mut c = || { t.0 += 1; t.1.0 *= 2; }; c(); c(); let s = &t.1.1; s.mix(); feed(t.0 + t.1.0); }
                { let nested = Duo { a: Duo { a: Noisy(51), b: 2u8 }, b: Noisy(15) }; let c = move || { let Duo { a: inner, .. } = nested; inner.b }; feed(c() as u64); }
                { let boxed = Box::new((Noisy(51), Noisy(15))); let c = move || boxed.0.0; feed(c() as u64); }
            }
            let mut v109: Vec<[Box<u16>; 3]> = ({ let mut items = ({ let mut items = vec![[Box::new(27u16), Box::new(65534u16), Box::new(0u16)], [Box::new(1u16), Box::new(512u16), Box::new(7u16)]]; items.reverse(); items }); items.push([Some((1042976772i32, 'a')), Some((19i32, '\u{e000}')), None::<(i32, char)>].map(|x114: Option<(i32, char)>| -> Box<u16> { Box::new(6u16) })); items });
            v109.truncate(6);
            'l111: while let Some(x110) = v109.pop() {
                let v116: Box<[Box<u16>; 3]> = Box::new(x110);
                let v117: u128 = "a".parse::<u128>().unwrap_or(2u128).min(apply_once(|| -> u128 { 10u128 }));
            }
            let [(p123, _)]: [(i32, char); 1] = (unsafe { let mut slot = MaybeUninit::<[(i32, char); 1]>::uninit(); slot.write([(16777216i32, v80); 1]); slot.assume_init() });
            f100((loop { break E3::V1(1e-40f64, 18u64); })).mix();
        }
        v99.mix();
        let mut v124: Vec<Option<(char, bool, bool)>> = ({ let mut items = ({ let mut items = (if (true) { vec![Some(('_', true, true)), None::<(char, bool, bool)>] } else { vec![Some(('a', false, true)), Some(('é', false, true)), Some(('é', true, false))] }); items.push(None::<(char, bool, bool)>); items }); items.dedup(); items });
        v124.truncate(6);
        'l126: while let Some(x125) = v124.pop() {
            let v131: E1<u128, i8> = larger::<E1<u128, i8>>(larger::<E1<u128, i8>>((|x132: u128| -> E1<u128, i8> { E1::<u128, i8>::V1 { f0: '\u{7f}', f1: 14i8, f2: Some(7i8), f3: vec![14u128, 213245541507142596530165215779759640912u128, 82011795300394268280903365918443904654u128, 16u128, 34u128], f4: 4294967296.0f64 } })(17592186044417u128), larger::<E1<u128, i8>>(E1::<u128, i8>::V0 { f0: '0', f1: 7.0f64 }, E1::<u128, i8>::V0 { f0: '\0', f1: (-9223372036854775808.0f64) })), E1::<u128, i8>::V0 { f0: String::from("a").chars().nth(2).unwrap_or(' '), f1: apply_dyn_mut(&mut |x133: f64| -> f64 { f64::MIN_POSITIVE }, 16777217.0f64) });
            let mut v134: char = v80;
            let mut v135: Option<(i32, char)> = Some('😀').map(|x137: char| -> [Box<u16>; 3] { v99.clone() }).map(|x136: [Box<u16>; 3]| -> (i32, char) { ((-2i32), v134) });
        }
        let mut o138: Box<dyn T5> = Box::new(f63(K4.wrapping_shr(10u32), 2i8));
        {
            #[derive(Clone, Copy, Debug, PartialEq)] enum Small { A, B, C }
            #[derive(Clone, Copy, Debug, PartialEq)] enum Outer { One(Small), Two, Three(bool), Four(char) }
            #[derive(Debug, PartialEq)] enum Deep { Ref(&'static u8), Boxed(Box<u16>), Nz(NonZeroU32), Nothing, Other(Option<Option<bool>>) }
            #[derive(Clone, Copy, Debug)] #[repr(u8)] enum Tagged { X(u8) = 3, Y = 9, Z(bool, u16) = 200 }
            enum Never {}
            static BYTE: u8 = 7;
            fn tell(it: &Outer) -> u64 { match *it { Outer::One(Small::A) => 1, Outer::One(Small::B) => 2, Outer::One(Small::C) => 3, Outer::Two => 4, Outer::Three(flag) => 5 + flag as u64, Outer::Four(c) => 7 + c as u64 } }
            fn deep(it: &Deep) -> u64 { match it { Deep::Ref(r) => **r as u64, Deep::Boxed(b) => **b as u64, Deep::Nz(n) => n.get() as u64, Deep::Nothing => 0, Deep::Other(None) => 1, Deep::Other(Some(None)) => 2, Deep::Other(Some(Some(flag))) => 3 + *flag as u64 } }
            feed((size_of::<Deep>() + size_of::<Option<Deep>>() * 64 + size_of::<Option<Box<u8>>>() * 4096 + size_of::<Option<&u8>>() * 65536 + size_of::<Result<Box<u8>, ()>>() * 1048576) as u64);
            { let mut slot = black_box(Outer::Two); slot = if 48 % 2 == 0 { Outer::Three(true) } else { Outer::One(Small::B) }; feed(tell(&slot)); let mut opt = black_box(Some(slot)); if let Some(Outer::Three(flag)) = &mut opt { *flag = false; } feed(opt.map_or(0, |it| tell(&it))); opt = None; feed(opt.is_some() as u64); }
            for it in black_box([Deep::Ref(&BYTE), Deep::Boxed(Box::new(48)), Deep::Nz(NonZeroU32::new(12).unwrap()), Deep::Nothing, Deep::Other(None), Deep::Other(Some(None)), Deep::Other(Some(Some(true)))]) { feed(deep(&it)); let wrapped = Some(it); feed(wrapped.as_ref().map_or(99, deep)); }
            { let items: Vec<Option<NonZeroU8>> = black_box(vec![NonZeroU8::new(0), NonZeroU8::new(48), NonZeroU8::new(255)]); for it in &items { feed(it.map_or(0, |n| n.get() as u64)); } feed(size_of::<Option<NonZeroU8>>() as u64); }
        }
        let mut o139: Box<dyn T5> = Box::new((if (false) { E1::<u128, i8>::V1 { f0: larger::<char>('é', 'a'), f1: K4.wrapping_pow(4294967294u32 % 7), f2: apply_once(|| -> Option<i8> { Some(64i8) }), f3: [vec![39614081257132168796771975168u128, 3u128, 7u128], vec![8u128, 267068090336936348565217151115960251246u128, 36u128, 29u128, 170141183460469231731687303715884105727u128]].concat(), f4: (f64::NAN + f64::NAN) } } else { E1::<u128, i8>::V0 { f0: char::from_u32(2851321446u32).unwrap_or(v80), f1: f64::MIN_POSITIVE.trunc() } }));
        let mut v143: BTreeMap<[Box<u16>; 3], Option<(char, bool, bool)>> = BTreeMap::new();
            for armed in [false, true] { caught(|| { let _v = (Noisy(27), Duo { a: Noisy(39), b: trip(armed, Noisy(15)) }); }); }
            for armed in [false, true] { caught(|| { let _v = { take3(Noisy(1), &Noisy(30), trip(armed, Noisy(15))) }; }); }
            for armed in [false, true] { caught(|| { let _v = Some(Noisy(31)).map(|it| (trip(armed, Noisy(29)), it)); }); }
    }
}
