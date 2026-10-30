// Found by fuzz.py: seed L12-20, case c25, edition 2024, backend cranelift.
// left: shares an `async` block with a function left to rustc
// Reduced in 48 builds. Again: fuzz.py check left-shares-an-async-block-with-a-function-left-to-rustc-L12-20-c25.rs
// fz::c25::run::work: unsupported: shares an `async` block with a function left to rustc
mod c25 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub struct S1 { pub f0: usize }
    impl Mix for S1 {
        fn mix(&self) {
            feed(1);
            self.f0.mix();
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    pub struct S2 { pub f0: f64 }
    impl Mix for S2 {
        fn mix(&self) {
            feed(1);
            self.f0.mix();
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S3(pub S1, pub usize);
    impl Mix for S3 {
        fn mix(&self) {
            feed(2);
            self.0.mix();
            self.1.mix();
        }
    }
    impl Drop for S3 {
        fn drop(&mut self) {
            feed(0xce1c8456);
            self.mix();
        }
    }
    pub trait T4 {
        fn m5(&self, a0: f64, a1: i128) -> S1;
        fn m6(&self) -> f64;
        fn m7(&self, a8: S2) -> i128 {
            self.m5((64i16 as f64), (-1i128)).mix();
            (7.0f64 as i128)
        }
    }
    impl T4 for f64 {
        fn m5(&self, a9: f64, a10: i128) -> S1 {
            id::<S1>(S1 { f0: 18446744073709551615usize })
        }
        fn m6(&self) -> f64 {
            let mut v11: S3 = (unsafe { let mut slot = MaybeUninit::<S3>::uninit(); slot.write(S3(S1 { f0: 34usize }, 25usize)); slot.assume_init() });
            (*self)
        }
    }
    impl T4 for &'static str {
        fn m5(&self, a12: f64, a13: i128) -> S1 {
            let v14: u32 = 4294967294u32;
            let mut v15: [S2; 4] = [S2 { f0: (-920.2118f64) }; 4];
            { let v16: [S2; 4] = id::<[S2; 4]>([S2 { f0: f64::MIN }, S2 { f0: (-0.0f64) }, S2 { f0: 3.25f64 }, S2 { f0: 7.0f64 }]); unsafe { std::ptr::copy_nonoverlapping(v16.as_ptr(), v15.as_mut_ptr(), 4) }; }
            v15.mix();
            pick::<S1>(Vec::<u32>::new().contains(&v14), &S1 { f0: 2usize, ..S1 { f0: 10usize } }, &None::<S1>.map_or(S1 { f0: 17592186044416usize }, |x23: S1| -> S1 { x23 })).clone()
        }
        fn m6(&self) -> f64 {
            65535.0f64
        }
    }
    impl T4 for String {
        fn m5(&self, a24: f64, a25: i128) -> S1 {
            {
                for armed in [false, true] { caught(|| { let t = (Noisy(31), Box::new(Duo { a: Noisy(20), b: Noisy(50) })); let _x = t.1.a; let _y = t.0; trip(armed, ()); }); }
                for armed in [false, true] { caught(|| { let t = Duo { a: Noisy(31), b: (Noisy(20), Noisy(50)) }; trip(armed, ()); let _x = t.b.0; let _y = t.a; }); }
                for armed in [false, true] { caught(|| { let t = [Noisy(31), Noisy(20), Noisy(50)]; trip(armed, ()); let [.., _x] = t; }); }
            }
            let mut v26: [bool; 3] = <[bool; 3]>::try_from(vec![true, false]).unwrap_or([false; 3]);
            { let value = (if (false) { (a24 == 7.0f64) } else { None::<S3>.is_some() }); let slot = &mut v26[1] as *mut bool; unsafe { *slot = value; (*slot).mix(); } }
            v26.mix();
            id::<S1>(pick::<S1>(v26[0], &S1 { f0: 549755813887usize }, &S1 { f0: 4194303usize }).clone())
        }
        fn m6(&self) -> f64 {
            {
                struct Outer(Noisy, Bomb, Noisy);
                impl Drop for Outer { fn drop(&mut self) { feed(0x0d0d + self.0.0 as u64); } }
                caught(|| { let _a = Noisy(2); let _h0 = Rc::new((Noisy(23), Bomb(31))); let _z = Noisy(42); });
                caught(|| { let _a = Noisy(31); let mut v0 = vec![Bomb(44), Bomb(39), Bomb(42)]; v0.truncate(1); let _z = Noisy(13); });
            }
            let mut v30: [u32; 8] = (if (true) { [10u32; 8] } else { [3090629085u32, 17u32, 4294967294u32, 7u32, 10u32, 377804800u32, 3u32, 2299557948u32] });
            { let (x, y) = (1, (37usize.wrapping_sub(27usize) % 8)); let start = v30.as_mut_ptr(); unsafe { std::ptr::swap(start.add(x), start.add(y)) }; }
            v30.mix();
            (unsafe { let mut slot = MaybeUninit::<f64>::uninit(); slot.write((7i128 as f64)); slot.assume_init() })
        }
    }
    impl T4 for S3 {
        fn m5(&self, a34: f64, a35: i128) -> S1 {
            {
                let (a, b): (u128, u128) = (black_box((65537) as u128), black_box((10) as u128));
                (a & b).mix(); (a | b).mix(); (a ^ b).mix(); feed((a < b) as u64 + 2 * (a == b) as u64); a.max(b).mix(); a.abs_diff(b).mix(); a.cmp(&b).mix();
                { let mut x = a; let mut steps = 0u32; while x != 0 && steps < 200 { x /= 10; steps += 1; } feed(steps as u64); }
                (a << 32).mix(); (a >> 32).mix(); a.rotate_left(32).mix(); a.checked_shl(32 + 64).mix(); a.wrapping_shr(32 + 100).mix(); (a >> (b as u32 % 128)).mix();
                a.checked_pow(3).mix(); a.wrapping_pow(b as u32 % 7).mix(); a.checked_neg().mix(); a.checked_next_multiple_of(b | 1).mix();
                { let (hi, lo) = ((a >> 64) as u64, a as u64); let again = ((hi as u128) << 64) | lo as u128; feed((again == a as u128) as u64); ((lo as u128) * (hi as u128)).mix(); }
                a.isqrt().mix(); a.checked_ilog2().mix(); a.checked_ilog10().mix();
            }
            {
                let (a, b) = (black_box((0.0) as f64), black_box((1e39) as f64));
                a.floor().mix(); a.ceil().mix(); a.round().mix(); a.trunc().mix(); a.fract().mix(); a.abs().mix(); a.round_ties_even().mix();
                (a + b).mix(); (a - b).mix(); (a * b).mix(); (a / b).mix(); (a % b).mix(); (-a).mix();
                let (mut acc, mut k) = (a, 0u32); while acc.is_finite() && acc.abs() < 1e30 && k < 40 { acc = acc * 1.5 + 1.0; k += 1; } feed(k as u64);
            }
            S1 { f0: 7usize, ..(*self).0 }
        }
        fn m6(&self) -> f64 {
            if ((f64::MAX.is_normal() ^ false)) {
                let mut v36: bool = ((*self).0 != (*self).0);
            } else {
                let mut v37: BTreeMap<S1, u32> = BTreeMap::new();
                v37.insert((*self).0, id::<u32>((unsafe { std::mem::transmute::<u128, [u64; 2]>(10u128) }[1] as u32))).mix();
                v37.insert((unsafe { *(&sel::<S1>(true, (*self).0, (*self).0) as *const S1) }), (3773999580u32 >> (3755585023u32.wrapping_pow(4294967294u32 % 7) % 32))).mix();
                v37.insert(S1 { f0: ((*self).0.f0.checked_ilog2().unwrap_or(0) as usize), ..(unsafe { let mut slot = MaybeUninit::<S1>::uninit(); slot.write((*self).0); slot.assume_init() }) }, (false as u32).saturating_mul((1u32.leading_zeros() as u32))).mix();
                v37.insert(pick::<S1>(Err::<bool, bool>(false).unwrap_or(true), &(*self).0, &S1 { f0: (*self).1, ..(*self).0 }).clone(), ({ let mut items = Vec::<u32>::new(); items.sort_unstable(); items }).into_iter().fold((2048u32.trailing_ones() as u32), |acc40: u32, x41: u32| -> u32 { (loop { break 25u32; }) })).mix();
                v37.get(&(match (Some(S1 { f0: 100usize })) { None => (*self).0, Some(x42) => x42 })).cloned().mix();
                v37.len().mix();
                {
                    #[derive(Clone, Copy, Debug, Default, PartialEq)] #[repr(C)] struct Plain { a: u8, b: u32, c: u16, d: u64 }
                    #[derive(Clone, Copy, Default)] #[repr(C, packed)] struct Tight { a: u8, b: u64, c: u16, d: u128 }
                    #[derive(Clone, Copy, Debug, Default, PartialEq)] #[repr(C, align(2))] struct Big { a: u8, b: [u16; 3] }
                    #[derive(Clone, Copy, Debug, Default, PartialEq)] #[repr(align(2))] struct Rusty(u8, u32);
                    #[derive(Clone, Copy, Debug, PartialEq)] #[repr(transparent)] struct Wrap(f32);
                    #[derive(Clone, Copy, Debug, PartialEq)] #[repr(i8)] enum Signed { Low = -128, Mid = -1, High = 127 }
                    #[derive(Clone, Copy, Debug, PartialEq)] #[repr(u16)] enum Wider { A = 1, B = 0x1234, C = 0xffff }
                    #[derive(Clone, Copy, Debug, PartialEq)] #[repr(C)] enum Cee { Unit, One(u8), Two { x: u16, y: u64 } }
                    #[derive(Clone, Copy, Debug, PartialEq)] #[repr(C, u8)] enum Both { P(u32), Q(u8, u8), R }
                    #[derive(Clone, Copy)] #[repr(C)] union Bits { word: u32, bytes: [u8; 4], half: (u16, u16) }
                    #[inline(never)] extern "C" fn c_plain(p: Plain, t: Tight, w: Wrap) -> Plain { Plain { a: p.a.wrapping_add(t.a), b: p.b ^ { t.c } as u32, c: p.c, d: p.d } }
                    #[inline(never)] extern "C" fn c_big(x: u8, big: Big, y: u8, rusty: Rusty) -> Big { Big { a: big.a ^ x ^ y ^ rusty.0, b: [big.b[2], big.b[0], rusty.1 as u16] } }
                    #[inline(never)] fn r_enums(s: Signed, w: Wider, c: Cee, b: Both) -> (i8, u16, Cee, Both) { (s as i8, w as u16, match c { Cee::One(v) => Cee::Two { x: v as u16, y: 1 }, other => other }, match b { Both::Q(x, y) => Both::Q(y, x), other => other }) }
                    feed((size_of::<Plain>() + align_of::<Plain>() * 256 + size_of::<Tight>() * 4096 + align_of::<Tight>() * 65536) as u64); feed((size_of::<Big>() + align_of::<Big>() * 256 + size_of::<Rusty>() * 4096 + size_of::<Cee>() * 65536 + size_of::<Both>() * 1048576) as u64);
                    { let w = black_box(Wrap(2.5)); let raw: f32 = unsafe { std::mem::transmute(w) }; raw.mix(); let items = black_box([Rusty(1, 2), Rusty(3, 4)]); feed(items[1].1 as u64 + size_of_val(&items) as u64 * 16); }
                    { let p = c_plain(black_box(Plain { a: 14, b: 55, c: 3, ..Default::default() }), black_box(Tight { a: 1, b: 2, c: 14, d: 4 }), Wrap(1.5)); feed(p.a as u64 + p.b as u64 * 256 + p.c as u64 * 65536); }
                    { let mut t = black_box(Tight { a: 14, b: 55, c: 7, d: u128::MAX }); t.b += 1; t.d = t.d.wrapping_add(14); let (b, c, d) = (t.b, t.c, t.d); feed(b + c as u64); d.mix(); feed(unsafe { std::ptr::addr_of!(t.d).read_unaligned() } as u64); }
                    { let got = r_enums(black_box(Signed::Low), black_box(Wider::B), black_box(Cee::One(14)), black_box(Both::Q(1, 2))); feed(got.0 as u8 as u64 + got.1 as u64 * 256); format!("{:?} {:?}", got.2, got.3).mix(); }
                    { let both = black_box([Both::P(7), Both::Q(1, 2), Both::R]); for it in both { feed(unsafe { *(&it as *const Both as *const u8) } as u64); feed(match it { Both::P(v) => v as u64, Both::Q(x, y) => (x + y) as u64, Both::R => 99 }); } }
                }
            }
            (if ((1e30f64.recip() < (f64::INFINITY * 1e30f64))) { (67108865u32 as f64) } else { (13i32 as f64).div_euclid(334.4388f64.round()) })
        }
    }
    pub fn f43(a44: &mut [u32]) -> f64 {
        { let value = (unsafe { std::mem::transmute::<u16, [u8; 2]>((7u16 << (0u32 % 16))) }[1] as u32); if let Some(slot) = a44.first_mut() { *slot = value; } }
        let mut o45: Box<dyn T4> = Box::new((if (Ok::<S1, bool>(S1 { f0: 8usize }).err().is_some()) { "3.5".get(3..5).unwrap_or("3.5").get(3..7).unwrap_or(Some("ff").map_or("", |x47: &'static str| -> &'static str { x47 })) } else { "0123456789" }));
        if (Err::<bool, i128>(10141204801825835211973625643009i128).unwrap_or_else(|x48: i128| -> bool { vec![18446744073709551614usize, 137438953472usize, 18446744073709551615usize].contains(&a44.len()) })) {
            {
                { let mut made: Vec<Box<dyn Fn(u32) -> u32>> = Vec::new(); for i in 0..2u32 { let k = i.wrapping_mul(54); made.push(Box::new(move |x| x.wrapping_add(k))); } for f in &made { f(1).mix(); } }
                { let mut log = Vec::new(); let mut i = 0u32; while i < 2 { let mut push = |it: u32| log.push(it * 54); push(i); push(i + 1); i += 1; } log.mix(); }
                { for i in 0..2u32 { let n = Noisy(i); let once = move || n.0 + 54; if i != 1 { once().mix(); } } }
                { let mut fs: Vec<Box<dyn FnMut() -> u32>> = Vec::new(); for i in 0..2u32 { let mut own = i; fs.push(Box::new(move || { own += 54; own })); } for round in 0..2 { for f in fs.iter_mut() { feed(f() as u64 + round); } } }
            }
        }
        (7isize as f64).div_euclid((1.5f64 * (-2.5e-3f64)))
    }
    pub extern "C" fn f50(a51: u32, a52: usize, a53: bool, a54: u32, a55: u8, a56: isize) -> bool {
        id::<S2>((loop { break larger::<S2>(S2 { f0: (-128.9f64) }, S2 { f0: f64::NEG_INFINITY }); })).mix();
        let mut v57: Vec<bool> = pick::<Vec<bool>>((false && a53), &({ let mut items = Vec::<bool>::new(); items.reverse(); items }), &id::<Vec<bool>>(vec![true, true, false])).clone();
        v57.truncate(6);
        'l59: while let Some(x58) = v57.pop() {
            let mut v63: isize = (unsafe { std::mem::transmute::<i16, [u8; 2]>(2i16.saturating_mul(0i16)) }[1] as isize).wrapping_sub((a56.saturating_add(a56).trailing_ones() as isize));
            {
                let mut f64 = || -> u8 { v63 = ((if (false) { 512i16 } else { 63i16 }) as isize); (if (true) { 36u8 } else { a55 }).overflowing_shl(a54.saturating_add(a54)).0 };
                let mut v65: (S2, S1, S2) = Some(None::<Result<(S2, S1, S2), S1>>).unwrap_or_else(|| -> Option<Result<(S2, S1, S2), S1>> { Some(Ok::<(S2, S1, S2), S1>((S2 { f0: f64::NAN }, S1 { f0: 18446744073709551615usize }, S2 { f0: 0.5f64 }))) }).unwrap_or_else(|| -> Result<(S2, S1, S2), S1> { Err::<(S2, S1, S2), S1>(S1 { f0: 205067702035494565usize }) }).unwrap_or_else(|x66: S1| -> (S2, S1, S2) { (S2 { f0: 0.1f64 }, S1 { f0: 31usize }, S2 { f0: (-1.0f64) }) });
                a56.mix();
                v65 = v65;
                f64().mix();
            }
            v63.mix();
        }
        String::from("Abc Def").ends_with("hello")
    }
    #[inline(never)]
    pub fn f72(a73: usize) -> S3 {
        if (f50((6u32 << (28u32 % 32)), String::from("hello").len(), Some(2u32).is_some(), None::<u32>.map_or(2097152u32, |x74: u32| -> u32 { x74 }), Ok::<u8, &'static str>(63u8).unwrap_or(8u8), (loop { break 7isize; }))) {
            if (f50(22u32.to_be(), String::from("héllo").len(), (3999902338u32 < 1198295273u32), (2147483647u32.leading_ones() as u32), 3u8, (unsafe { std::mem::transmute::<i16, i16>((-13i16)) } as isize))) {
                let v76: usize = 1usize.reverse_bits().saturating_sub((1e10f64 as usize));
                if ((String::from("a,b,,c") <= ({ let mut text = String::from("42"); text.push('\u{80}'); text }))) { return (match (Some(7.0f64).map(|x78: f64| -> S3 { S3(S1 { f0: 3usize }, 1152921504606846976usize) })) { None => sel::<S3>(false, S3(S1 { f0: 3usize }, 1usize), S3(S1 { f0: 14996528147412295818usize }, 4usize)), Some(x77) => x77 }); }
            } else {
                std::mem::size_of::<u32>().mix();
                let mut v79: u32 = (((-18i8).saturating_sub((-36i8)) as u32) << ((67108864u32.checked_div(2367918172u32).unwrap_or(4194303u32).leading_zeros() as u32) % 32));
            }
            let mut n80 = 0u32;
            'l81: loop {
                n80 += 1;
                if n80 > 2 { break; }
                {
                    for armed in [false, true] { let n = Noisy(60); let f = move |armed: bool| { let _l = Noisy(5); trip(armed, ()); n.0 }; caught(move || feed(f(armed) as u64)); }
                    for armed in [false, true] { let n = Noisy(60); let once = move || { let n = n; trip(armed, ()); drop(n); }; caught(once); }
                }
                let g83: extern "C" fn(u32, usize, bool, u32, u8, isize) -> bool = f50;
                let mut v84: [f64; 2] = (loop { break [String::from("😀"), String::from("a,b,,c")].map(|x89: String| -> f64 { (-2.5e-3f64) }); });
                { let bytes: [u8; 17] = std::array::from_fn::<S3, 17, _>(|i92: usize| -> S3 { (match (None::<S3>) { None => S3(S1 { f0: 40usize }, 32usize), Some(x94) => x94 }) }).map(|x91: S3| -> u8 { (unsafe { std::mem::transmute::<u32, [u16; 2]>(3470466883u32.overflowing_shl(37u32).0) }[0] as u8) }); (unsafe { (bytes.as_ptr().add(9) as *const u64).read_unaligned() }).mix(); }
                v84.mix();
                (match (Some(Some((S2 { f0: 0.2f64 }, S1 { f0: 4194304usize }, S2 { f0: (-1.0f64) }))).unwrap_or(Some((S2 { f0: (-0.0f64) }, S1 { f0: 28usize }, S2 { f0: (-0.0f64) })))) { None => (S2 { f0: 4294967296.0f64 }, S1 { f0: 12usize }, S2 { f0: 0.0f64 }), Some(x95) => x95 }).mix();
            }
        } else {
            let r98 = catch_unwind(AssertUnwindSafe(|| -> u8 {
                let mut v99: [usize; 8] = std::array::from_fn::<usize, 8, _>(|i101: usize| -> usize { i101 });
                { let (x, y) = (((a73 ^ a73) % 8), ("hello".parse::<usize>().unwrap_or(a73) % 8)); let start = v99.as_mut_ptr(); unsafe { std::ptr::swap(start.add(x), start.add(y)) }; }
                v99.mix();
                id::<&'static str>("Abc Def").split(' ').next().unwrap_or("x y".split(' ').last().unwrap_or("3.5")).mix();
                let v103: bool = pick::<Option<isize>>(true, &None::<isize>, &Some((-9223372036854775808isize))).clone().is_none();
                if v103 { std::panic::panic_any((15u32 << (8191u32 % 32))); }
                ((4670854631743863508i64 >> (31u32 % 64)) as u8)
            }));
            match r98 {
                Ok(value) => value.mix(),
                Err(caught) => payload(caught),
            }
            ((248837293u32 | 18u32) >> ((unsafe { std::mem::transmute::<u16, [u8; 2]>(8u16) }[0] as u32) % 32)).to_string().mix();
        }
        Ok::<bool, S3>(true).unwrap_or(false).then(|| -> S3 { pick::<S3>(false, &S3(S1 { f0: 524288usize }, 1usize), &S3(S1 { f0: 9223372036854775808usize }, 8usize)).clone() }).unwrap_or((if ((false | false)) { (if (false) { S3(S1 { f0: 1usize }, 3831185437021645755usize) } else { S3(S1 { f0: 38usize }, 0usize) }) } else { (loop { break S3(S1 { f0: 10usize }, 576460752303423488usize); }) }))
    }
    #[inline(always)]
    pub fn f109(a110: &S1) -> u32 {
        vec![S1 { f0: 20usize }, S1 { f0: 18446744073709551615usize }, S1 { f0: 549755813889usize }, S1 { f0: 536870911usize }, S1 { f0: 2usize }].first().cloned().unwrap().mix();
        let mut v112: Vec<bool> = Vec::<bool>::new();
        ((1u32 << (3u32 % 32)) >> (u32::from(16u8) % 32))
    }
    #[inline(never)]
    pub fn f113(a114: i128, a115: &i128, a116: bool, a117: &mut i128, a118: &str, a119: &[u16]) -> S3 {
        f72((unsafe { std::mem::transmute::<u32, u32>(34u32) } as usize))
    }
    #[inline(always)]
    pub fn f120(a121: &[u32]) -> u32 {
        (if (true) { "0123456789" } else { "hello" }).m6().mix();
        let v122: i128 = (if (true) { (-6811403646174735005038011235206178951i128) } else { (-15i128) });
        let mut v123: [u16; 2] = <[u16; 2]>::try_from(vec![17u16, 65535u16, 3u16]).unwrap_or([58981u16, 32u16]);
        v123.mix();
        if ((v122 == v122.reverse_bits())) {
            if (None::<f64>.is_some()) {
                {
                    for armed in [false, true] { caught(|| { take3(first_of(&Noisy(37), &Noisy(45)).0, Noisy(2), trip(armed, Noisy(6))); }); }
                    for armed in [false, true] { caught(|| { let _v = vec![Noisy(25), trip(armed, Noisy(29)), Noisy(10)]; }); }
                    for armed in [false, true] { caught(|| { let _v = if armed { (Noisy(17), trip(armed, Noisy(30))) } else { (Noisy(52), Noisy(39)) }; }); }
                }
                {
                    async fn work(seed: u32) -> u64 { let h128 = vec![Noisy(21), Noisy(seed % 5)]; let w129 = later(0, seed as u64 + 8).await; (h128.len() as u64).rotate_left(0) ^ (w129).rotate_left(1) }
                    poll_some(work(49), 4).mix();
                    poll_some(work(14), 0).mix();
                    poll_some(work(50), 0).mix();
                    poll_some(work(12), 1).mix();
                }
                let mut v130: [u8; 3] = std::array::from_fn::<u8, 3, _>(|i132: usize| -> u8 { 255u8 });
                { let (x, y) = (((a121.len() >> (4062716883u32 % 64)) % 3), 1); let start = v130.as_mut_ptr(); unsafe { std::ptr::swap(start.add(x), start.add(y)) }; }
                v130.mix();
            }
            if (String::from("ff").chars().rev().nth(1).unwrap_or('😀').is_whitespace()) {
                {
                    let (a, b) = (black_box((-129.0) as f64), black_box((0.30000000000000004) as f64));
                    feed(a.is_nan() as u64 + 2 * a.is_infinite() as u64 + 4 * a.is_finite() as u64 + 8 * a.is_sign_negative() as u64 + 16 * a.is_normal() as u64);
                    feed(a.total_cmp(&b) as i8 as u64); feed(a.partial_cmp(&b).map_or(9, |it| it as i8 as u8) as u64); feed((a < b) as u64 + 2 * (a <= b) as u64 + 4 * (a == b) as u64 + 8 * (a != b) as u64);
                    format!("{a} {a:?} {a:e} {b:.3} {b:10.2} {a:+}").mix();
                    let both = [a, b, a + b, a * 2.0]; both.iter().copied().fold(0.0, |acc, it| if it.is_nan() { acc } else { acc + it }).mix(); both.iter().filter(|it| it.is_finite()).count().mix();
                    a.sqrt().mix(); a.mul_add(b, 1.5).mix(); a.rem_euclid(3.0).mix(); a.div_euclid(3.0).mix(); a.powi(3).mix(); a.recip().mix();
                }
            } else {
                let mut o134: Vec<Box<dyn T4>> = vec![Box::new(Err::<f64, Vec<i16>>(Vec::<i16>::new()).unwrap_or((2147483647i32 as f64))), Box::new((4194305u32 as f64)), Box::new(f64::MIN)];
                for o135 in o134.iter() { o135.m7(id::<S2>(S2 { f0: (-9223372036854775808.0f64) })).mix(); }
            }
            if (Some(Some(S1 { f0: 16383usize })).map_or(Some(S1 { f0: 15usize }), |x138: Option<S1>| -> Option<S1> { x138 }).is_none()) {
                (2097152usize as f64).mix();
            }
        }
        1u32
    }
    pub fn run() {
        (0.1f32 as usize).to_be().mix();
        if ((f32::MIN_POSITIVE * f32::MAX).is_finite()) {
            if (Ok::<bool, bool>(true).or_else(|x140: bool| -> Result<bool, u16> { Ok::<bool, u16>(true) }).unwrap_or(f50(3u32, 10usize, false, 8u32, 39u8, 3isize))) {
                let mut n141 = 0u32;
                let v143: bool = 'l142: loop {
                    n141 += 1;
                    if n141 > 5 { break (!true); }
                    {
                        enum Loud { I(i32, Noisy), C(char, Noisy) }
                        #[inline(never)] fn items(k: u32, last: char) -> [Loud; 3] { black_box([Loud::I(k as i32 % 4, Noisy(k)), Loud::C('q', Noisy(k + 1)), if k % 3 == 0 { Loud::I(1, Noisy(k + 2)) } else { Loud::C(last, Noisy(k + 2)) }]) }
                        #[inline(never)] fn pick0(k: u32, last: char, flag: bool) -> u8 { match items(k, last) { [Loud::C('a', _) | Loud::I(1, _), _, Loud::C(_, _)] if black_box(flag) => 1, [Loud::C('A'..='Z', _), m @ ..] => 2, [.., _] if black_box(!flag) => 3, _ => 0 } }
                        for k in 0..4u32 { for last in ['a', 'Z', 'q'] { for flag in [false, true] { feed(pick0(k, last, flag) as u64); feed(0xf00d); } } }
                        #[inline(never)] fn pick1(k: u32, last: char, flag: bool) -> u8 { match items(k, last) { [Loud::C('b', _) | Loud::I(_, _), ..] if black_box(flag) => 1, [Loud::C('a', _) | Loud::I(2, _), m @ ..] => 2, [.., y] if black_box(!flag) => 3, [Loud::I(..), Loud::I(..), Loud::C('A'..='Z', _) | Loud::I(1, _)] if black_box(!flag) => 4, _ => 0 } }
                        for k in 0..4u32 { for last in ['a', 'Z', 'q'] { for flag in [false, true] { feed(pick1(k, last, flag) as u64); feed(0xf00d); } } }
                        #[inline(never)] fn pick2(k: u32, last: char, flag: bool) -> u8 { match items(k, last) { [Loud::C('a', _) | Loud::C(_, _), m @ ..] if black_box(flag) => 1, [.., Loud::I(0..=5, _)] if black_box(false) => 2, [Loud::I(2, _), m @ .., Loud::C(..)] if black_box(!flag) => 3, [.., y @ (Loud::I(..) | Loud::C('a'..='m', _))] if black_box(!flag) => 4, [_, m @ .., Loud::C('a', _) | Loud::C(_, _)] => 5, _ => 0 } }
                        for k in 0..4u32 { for last in ['a', 'Z', 'q'] { for flag in [false, true] { feed(pick2(k, last, flag) as u64); feed(0xf00d); } } }
                        #[inline(never)] fn pair(k: u32, last: char, flag: bool) -> u8 { match (items(k, last), k % 3, flag) { ([_, ..], 1, true) => 1, ([.., Loud::I(..)], 1, _) => 2, ([x, m @ ..], 1, false) => 3, (all, _, _) if black_box(flag) => 4, _ => 0 } }
                        for k in 0..4u32 { for last in ['a', 'Z'] { for flag in [false, true] { feed(pair(k, last, flag) as u64); feed(0xf00d); } } }
                    }
                    let mut v144: Vec<u32> = Some(Vec::<u32>::new()).map_or(vec![8u32, 2097152u32, 2147483647u32, 2498501327u32, 37u32], |x146: Vec<u32>| -> Vec<u32> { x146.clone() });
                };
                let mut v149: u32 = ((8u32.count_ones() as u32) << ((4277955847u32 as u32) % 32));
                {
                    let mut f150 = |a147: (S2, S1, S2), a148: String| -> i64 { v149 = ((213.6619f32 % 0.0f32) as u32); 140737488355329i64.checked_rem((-1546117921719124775i64)).unwrap_or(None::<i64>.unwrap_or((-12i64))) };
                    {
                        for armed in [false, true] { caught(|| { let mut kept = Vec::new(); let mut i = 0u32; while i < 3 { let n = Noisy(i); if i == 2 { trip(armed, ()); } if i % 2 == 0 { kept.push(n); i += 1; continue; } let _m = Noisy(10 + i); if i == 2 { break; } i += 1; } feed(kept.len() as u64); }); }
                    }
                    let mut o152: Box<dyn T4> = Box::new(String::from_utf8_lossy(&({ let mut items = Vec::<usize>::new().into_iter().rev().map(|x156: usize| -> u8 { 206u8 }).collect::<Vec<u8>>(); items.reverse(); items })).into_owned());
                    let mut v157: Box<S2> = pick::<Box<S2>>(v143, &Box::new(S2 { f0: (-2.5e-3f64) }), &Box::new(S2 { f0: 1e10f64 })).clone();
                    f150((S2 { f0: 3.25f64 }, S1 { f0: 10usize }, S2 { f0: f64::EPSILON }), ({ let mut text = String::from("héllo"); text.push_str("😀"); text })).mix();
                }
                v149.mix();
            } else {
                let mut n158 = 0u32;
                'l159: loop {
                    n158 += 1;
                    if n158 > 2 { break; }
                    {
                        let (a, b) = (black_box((5e-324) as f64), black_box((2147483648.0) as f64));
                        format!("{a} {a:?} {a:e} {b:.3} {b:10.2} {a:+}").mix();
                        let both = [a, b, a + b, a * 2.0]; both.iter().copied().fold(0.0, |acc, it| if it.is_nan() { acc } else { acc + it }).mix(); both.iter().filter(|it| it.is_finite()).count().mix();
                        if !a.is_nan() && !b.is_nan() && a != b { a.max(b).mix(); a.min(b).mix(); } a.clamp(-1.0, 1.0).mix();
                        feed(a.total_cmp(&b) as i8 as u64); feed(a.partial_cmp(&b).map_or(9, |it| it as i8 as u8) as u64); feed((a < b) as u64 + 2 * (a <= b) as u64 + 4 * (a == b) as u64 + 8 * (a != b) as u64);
                    }
                    let mut o161: Box<dyn T4> = Box::new("x y".split(',').nth(1).unwrap_or("0123456789"));
                    let v162: u8 = 227u8;
                    let mut v163: [bool; 2] = [f50(337160921u32, 1usize, true, 7u32, v162, 11isize); 2];
                    { let (x, y) = (((27usize >> (4175952319u32 % 64)) % 2), (7usize.saturating_add(262143usize) % 2)); let start = v163.as_mut_ptr(); unsafe { std::ptr::swap(start.add(x), start.add(y)) }; }
                    v163.mix();
                }
                {
                    trait Base { fn id(&self) -> u32; fn name(&self) -> &'static str { "base" } }
                    trait Mid: Base { fn twice(&self) -> u32 { self.id().wrapping_mul(2) } }
                    trait Top: Mid + Debug { fn all(&self) -> u64 { self.id() as u64 + self.twice() as u64 + self.name().len() as u64 } }
                    #[derive(Debug)] struct P(u32);
                    #[derive(Debug)] struct Q(u8, Noisy);
                    impl Base for P { fn id(&self) -> u32 { self.0 } }
                    impl Base for Q { fn id(&self) -> u32 { self.0 as u32 + self.1.0 } fn name(&self) -> &'static str { "q" } }
                    impl Mid for P {} impl Top for P {}
                    impl Mid for Q { fn twice(&self) -> u32 { 13 } } impl Top for Q {}
                    fn through(it: &dyn Mid) -> u32 { let base: &dyn Base = it; base.id() ^ it.twice() }
                    let mut tops: Vec<Box<dyn Top>> = vec![Box::new(P(51)), Box::new(Q(37, Noisy(13))), Box::new(P(37))];
                    { let mid: Box<dyn Mid> = tops.remove(1); let base: Box<dyn Base> = mid; base.name().mix(); }
                    { let sync: Arc<dyn Base + Send + Sync> = Arc::new(P(5)); feed(sync.id() as u64); }
                    { let last: Box<dyn Base> = tops.pop().unwrap(); feed(last.id() as u64); }
                    for top in &tops { let mid: &dyn Mid = &**top; let base: &dyn Base = mid; feed(base.id() as u64); base.name().mix(); feed(through(mid) as u64); feed(top.all()); }
                }
            }
            if (((match (None::<S1>) { None => S1 { f0: 2usize }, Some(x167) => x167 }) <= 989.1622f64.m5((-1.0f64), 16i128))) {
                let mut n168 = 0u32;
                'l169: while n168 < 8 && ((loop { break false; })) {
                    n168 += 1;
                    {
                        let (a, b): (u128, u128) = (black_box((1) as u128), black_box((18446744073709551557) as u128));
                        { let (sum, carry) = a.overflowing_add(b); let (product, over) = a.overflowing_mul(b); sum.mix(); product.mix(); feed(carry as u64 + 2 * over as u64); }
                        (a << 100).mix(); (a >> 100).mix(); a.rotate_left(100).mix(); a.checked_shl(100 + 64).mix(); a.wrapping_shr(100 + 100).mix(); (a >> (b as u32 % 128)).mix();
                        { let (hi, lo) = ((a >> 64) as u64, a as u64); let again = ((hi as u128) << 64) | lo as u128; feed((again == a as u128) as u64); ((lo as u128) * (hi as u128)).mix(); }
                    }
                    {
                        let (a, b): (u128, u128) = (black_box((340282366920938463463374607431768211297) as u128), black_box(((1u128 << 64)) as u128));
                        (a as u64).mix(); (a as i64).mix(); (a as u8).mix(); ((a >> 64) as u64).mix(); u64::try_from(a).ok().mix(); i32::try_from(b).ok().mix();
                        { let (sum, carry) = a.overflowing_add(b); let (product, over) = a.overflowing_mul(b); sum.mix(); product.mix(); feed(carry as u64 + 2 * over as u64); }
                        a.to_le_bytes().mix(); a.to_be_bytes().mix(); a.to_string().mix(); format!("{a:x} {b:#o} {a:>45} {b:e}").mix();
                        (a & b).mix(); (a | b).mix(); (a ^ b).mix(); feed((a < b) as u64 + 2 * (a == b) as u64); a.max(b).mix(); a.abs_diff(b).mix(); a.cmp(&b).mix();
                        a.isqrt().mix(); a.checked_ilog2().mix(); a.checked_ilog10().mix();
                        { let mut x = a; let mut steps = 0u32; while x != 0 && steps < 200 { x /= 10; steps += 1; } feed(steps as u64); }
                        { let items = [a, b, a ^ b, a.wrapping_add(b)]; let mut sorted = items; sorted.sort(); sorted.mix(); items.iter().copied().fold(0, |acc: u128, it| acc.wrapping_add(it)).mix(); }
                        u128::from_str_radix(&format!("{:x}", (a as u128) >> 1), 16).ok().mix(); (a as f64).mix(); (a as f32).mix(); ((a as f64) as u128).mix();
                    }
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
                        let (sq, rect) = (Sq(45), Rect(45, 4));
                        { let number: &dyn Conv<u64> = &sq; let text: &dyn Conv<String> = &sq; feed(number.conv()); text.conv().mix(); }
                        feed(total(&sq) + total(&rect as &dyn Shape) + sq.scaled(3));
                    }
                }
            } else {
                let mut o170: Box<dyn T4> = Box::new(Err::<Vec<S2>, u32>(32u32).unwrap_or_else(|x172: u32| -> Vec<S2> { vec![S2 { f0: 0.1f64 }, S2 { f0: (-283.5273f64) }] }).into_iter().step_by(2).map(|x174: S2| -> char { '\0'.to_uppercase().next().unwrap_or('\u{7f}') }).collect::<String>());
            }
        } else {
            let mut o175: Vec<Box<dyn T4>> = vec![Box::new(pick::<&'static str>((true || None::<&'static str>.is_some()), &id::<&'static str>("hello".split(',').nth(1).unwrap_or("héllo")), &"  pad  ").clone()), Box::new(pick::<&'static str>((Ok::<isize, u16>((-10isize)).unwrap_or_else(|x177: u16| -> isize { 0isize }) < 7isize), &('b178: { if (false) { break 'b178 "ff".split(',').last().unwrap_or("42"); } "x y".split(' ').next().unwrap_or("-7") }), &(if let Some(x179) = (({ let mut items = Vec::<&'static str>::new(); items.pop() })) { x179 } else { sel::<&'static str>(false, "-7", "Abc Def") })).clone())];
            for o176 in o175.iter() { o176.m6().mix(); }
        }
        match (&{ (3i32 << (536870912u32 % 32)) }) {
            (-9i32 | 268435455i32 | 512i32) if ("a,b,,c" != "😀") => {
                let mut n181 = 0u32;
                let v183: Box<S2> = 'l182: loop {
                    n181 += 1;
                    if n181 > 5 { break Box::new(S2 { f0: f64::EPSILON }); }
                    let r184 = catch_unwind(AssertUnwindSafe(|| -> Box<S2> {
                        let mut v185: i64 = ((-1i64).count_ones() as i64);
                        let mut v186: i64 = v185;
                        let v187: bool = (true || f50(268435456u32, 100usize, false, 32u32, 255u8, 137438953472isize));
                        assert!(!v187, "held {}", (1e10f64 as u32));
                        Box::new(S2 { f0: (-128.9f64) })
                    }));
                    match r184 {
                        Ok(value) => value.mix(),
                        Err(caught) => payload(caught),
                    }
                    {
                        for armed in [false, true] { let mut seen = Vec::new(); caught(|| { let mut note = |it: u32| { seen.push(Noisy(it)); trip(armed && it == 2, ()); }; note(1); note(2); note(3); }); feed(seen.len() as u64); }
                    }
                    let v189: S2 = id::<S2>(S2 { f0: 255.5f64, ..S2 { f0: 1e30f64 } });
                };
            }
            4i32 if f50(13u32, 15519802371523205905usize, true, 12u32, 255u8, 0isize) => {
                let mut v190: [isize; 1] = id::<[isize; 1]>(<[isize; 1]>::try_from(vec![(-6isize), 653543170144449015isize, 17592186044416isize, (-17isize), (-3isize)]).unwrap_or([(-2213454687690420301isize)]));
                { let at = 0; let value = 2305843009213693953isize.saturating_sub(4611686018427387903isize).checked_mul((-9223372036854775808isize).wrapping_mul((-135924488091697465isize))).unwrap_or(4398046511104isize); unsafe { raw_set(v190.as_mut_ptr(), at, value); raw_get(v190.as_ptr(), at).mix(); } }
                v190.mix();
                'l200: for x199 in (vec![(-9isize)].into_iter().enumerate()) {
                    {
                        { let mut kept = Vec::new(); for i in 0..5u32 { let n = Noisy(i); let by_ref = || n.0 * 2; by_ref().mix(); if i % 2 == 0 { kept.push(move || n.0); } } for f in kept.iter().rev() { f().mix(); } }
                        { let mut made: Vec<Box<dyn Fn(u32) -> u32>> = Vec::new(); for i in 0..5u32 { let k = i.wrapping_mul(5); made.push(Box::new(move |x| x.wrapping_add(k))); } for f in &made { f(1).mix(); } }
                    }
                    if (f50(None::<u32>.unwrap_or(7u32), x199.0, f50(12u32, 0usize, false, 37u32, 18u8, x199.1), 10u32, (match (Some(9u8)) { None => 2u8, Some(x202) => x202 }), (-9223372036854775807isize).saturating_mul((-4109067137828655080isize)))) {
                        let mut v203: S1 = S1 { f0: std::mem::size_of_val(&S1 { f0: 8usize }), ..S1 { f0: 8usize } };
                    } else {
                        100u16.saturating_sub(128u16).wrapping_shl(33554433u32.saturating_sub(2147483647u32)).mix();
                    }
                    if ((!f50(34u32, 4665805574777049266usize, false, 100u32, 27u8, v190[0]))) {
                        let v204: Vec<i16> = (if (f50(4294967294u32, x199.0, false, 32u32, 0u8, x199.1)) { vec![512i16, 32767i16, 8192i16, 1i16] } else { ({ let mut items = vec![(-9i16)]; items.reverse(); items }) });
                    } else {
                        (if (f50(19u32, std::mem::size_of_val(&100i8), (!true), u32::from(64u8), (13i64 as u8), v190[0])) { Box::new((if (true) { S2 { f0: f64::INFINITY } } else { S2 { f0: 1e10f64 } })) } else { (unsafe { let mut slot = MaybeUninit::<Box<S2>>::uninit(); slot.write(Box::new(S2 { f0: f64::INFINITY })); slot.assume_init() }) }).mix();
                        {
                            { let text = String::from("abc"); let mut lens = Vec::new(); for i in 0..4 { let view = || text.len() + i; lens.push(view()); let owned = text.clone(); let eat = move || owned.len(); lens.push(eat()); } lens.mix(); text.mix(); }
                            { let mut log = Vec::new(); let mut i = 0u32; while i < 4 { let mut push = |it: u32| log.push(it * 41); push(i); push(i + 1); i += 1; } log.mix(); }
                            { let data = [41u32, 2, 3, 4, 5]; let mut total = 0u32; for i in 0..4 { let add = |x: u32| x.wrapping_add(data[i]); total = total.wrapping_add(add(i as u32)); let mut bump = || total = total.wrapping_mul(3); bump(); } total.mix(); }
                        }
                    }
                }
            }
            -535305422i32 => {
                match ((7i8.trailing_zeros() as i8)) {
                    53i8 => {
                        if (((if let Some(x208) = (None::<bool>) { x208 } else { true }) || true)) {
                            {
                                { let mut fs: Vec<Box<dyn FnMut() -> u32>> = Vec::new(); for i in 0..4u32 { let mut own = i; fs.push(Box::new(move || { own += 25; own })); } for round in 0..2 { for f in fs.iter_mut() { feed(f() as u64 + round); } } }
                                { let mut log = Vec::new(); let mut i = 0u32; while i < 4 { let mut push = |it: u32| log.push(it * 25); push(i); push(i + 1); i += 1; } log.mix(); }
                                { let found = 'search: loop { for i in 0..4u32 { let hit = move |at: u32| at * i == 1; for j in 0..3 { if hit(j) { break 'search Some((i, j)); } } } break None; }; found.mix(); }
                            }
                            let mut v209: (S2, S1, S2) = (S2 { f0: S3(S1 { f0: 9usize }, 17403463409514005864usize).m6(), ..S2 { f0: 0.2f64, ..S2 { f0: 1e30f64 } } }, S1 { f0: 1usize }, S2 { f0: (1u16 as f64), ..None::<S2>.map_or(S2 { f0: f64::MIN }, |x210: S2| -> S2 { x210 }) });
                            let mut v211: [S2; 8] = Some([S2 { f0: (-226.7224f64) }, S2 { f0: (-128.9f64) }, S2 { f0: 0.0f64 }, S2 { f0: f64::NAN }, S2 { f0: f64::MIN }, S2 { f0: 1.5f64 }, S2 { f0: 1.0f64 }, S2 { f0: 65535.0f64 }]).unwrap_or([S2 { f0: 933.8739f64 }, S2 { f0: (-1.0f64) }, S2 { f0: 7.0f64 }, S2 { f0: 1e-40f64 }, S2 { f0: 1.0f64 }, S2 { f0: f64::EPSILON }, S2 { f0: f64::MAX }, S2 { f0: 16777217.0f64 }]);
                            (unsafe { std::slice::from_raw_parts(v211.as_ptr().add(7), 1) }).mix();
                            v211.mix();
                        } else {
                                async fn work(seed: u32) -> u64 { let h216 = (seed as f64 * 0.5, seed as u8, seed % 3 == 0); let w217 = if seed % 2 == 0 { later(1, 5u64).await } else { 2 }; let h218 = Noisy(seed % 13 + 1); let w219 = later(2, [seed as u64; 2]).await[0]; let h220 = [seed as u8; 1]; let mut w221 = 0u64; for i in 0..3u64 { let _g = Noisy(i as u32); w221 += later((i % 2) as u32, i).await; } let h222 = [seed as u8; 1]; let w223 = later(2, seed as u64 + 19).await; (h216.0 as u64 + h216.1 as u64 + h216.2 as u64).rotate_left(0) ^ (w217).rotate_left(1) ^ (h218.0 as u64).rotate_left(2) ^ (w219).rotate_left(3) ^ (h220[0] as u64).rotate_left(4) ^ (w221).rotate_left(5) ^ (h222[0] as u64).rotate_left(6) ^ (w223).rotate_left(7) }
                                poll_some(work(89), 3).mix();
                                poll_some(work(99), 6).mix();
                                poll_some(work(60), 5).mix();
                        }
                    }
                    _ => {}
                }
            }
            _ => (1099511627776usize.checked_ilog2().unwrap_or(0) as usize).mix(),
        }
        let mut v239: [i8; 4] = id::<[i8; 4]>(({ let mut items = [(-19i8); 4]; items.sort_by(|a, b| b.cmp(a)); items }));
        let r303 = catch_unwind(AssertUnwindSafe(|| -> i8 {
            match (&{ (if (false) { Box::new(S2 { f0: 65535.0f64 }) } else { Box::new(S2 { f0: 7.0f64 }) }) }) {
                p313 if (!true) => {
                        let (a, b) = (black_box((-2147483649.0) as f32), black_box((f64::MIN) as f32));
                        let both = [a, b, a + b, a * 2.0]; both.iter().copied().fold(0.0, |acc, it| if it.is_nan() { acc } else { acc + it }).mix(); both.iter().filter(|it| it.is_finite()).count().mix();
                        (a + b).mix(); (a - b).mix(); (a * b).mix(); (a / b).mix(); (a % b).mix(); (-a).mix();
                        if !a.is_nan() && !b.is_nan() && a != b { a.max(b).mix(); a.min(b).mix(); } a.clamp(-1.0, 1.0).mix();
                        a.sqrt().mix(); a.mul_add(b, 1.5).mix(); a.rem_euclid(3.0).mix(); a.div_euclid(3.0).mix(); a.powi(3).mix(); a.recip().mix();
                        format!("{a} {a:?} {a:e} {b:.3} {b:10.2} {a:+}").mix();
                        feed(a.total_cmp(&b) as i8 as u64); feed(a.partial_cmp(&b).map_or(9, |it| it as i8 as u8) as u64); feed((a < b) as u64 + 2 * (a <= b) as u64 + 4 * (a == b) as u64 + 8 * (a != b) as u64);
                }
                p315 => {
                }
            }
            (v239[2].checked_mul(v239[0]).unwrap_or(v239[0]) * (v239[2] * 11i8))
        }));
    }
}
