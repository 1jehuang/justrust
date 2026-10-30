// Found by fuzz.py: seed told-rpa-41, case c0, edition 2021, backend llvm.
// differs: checksum
// Reduced in 91 builds. Again: fuzz.py check differs-checksum-told-rpa-41-c0.rs
// rustc:
// c0 795515227a652b03 ok
// ra-jit:
// c0 12e121b328193b36 ok
mod c0 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(u8)]
    pub enum E1 {
        V0,
        V1,
        V2,
        V3,
        V4,
        V5,
        V6,
        V7,
    }
    impl Mix for E1 {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(u64)]
    pub enum E2 {
        V0,
        V1,
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(u64)]
    pub enum E3 {
        V0 = 54,
        V1 = 197,
    }
    impl Mix for E3 {
        fn mix(&self) {
            match self {
                E3::V0 => { feed(10); }
                E3::V1 => { feed(11); }
            }
        }
    }
    pub trait T4 {
        fn m5(&self, a0: E3) -> E3;
        fn m6(&self, a7: u8) -> E1 {
            E1::V1
        }
    }
    impl T4 for E1 {
        fn m5(&self, a8: E3) -> E3 {
            E3::V1
        }
    }
    impl T4 for u8 {
        fn m5(&self, a9: E3) -> E3 {
            E3::V0
        }
    }
    #[inline(never)]
    pub fn f16(a17: u128, a18: E2, a19: &dyn T4, a20: E3, a21: i32, a22: u128, a23: E2, a24: E2) -> i32 {
        let mut v25: u32 = 23u32;
        let mut n26 = 0u32;
        'l27: loop {
            n26 += 1;
            if n26 > 2 { break; }
            if ((if (1isize.is_positive()) { '0'.is_whitespace() } else { ({ let t29: E1 = E1::V7; true }) })) { break 'l27; }
            let v30: u32 = std::mem::replace(&mut v25, 1u32);
            (-20i32).wrapping_neg().mix();
        }
        let mut v34: u32 = (3949466014u32.trailing_ones() as u32);
        {
            let mut f35 = |a31: E2, a32: u8, a33: E3| -> i128 { v34 = Some(E3::V1).map(|x37: E3| -> u32 { v25 }).unwrap_or_else(|| -> u32 { 3u32.checked_sub(v25).unwrap_or(v34) }); ('\u{7f}' as i128) };
            let mut o38: Vec<Box<dyn T4>> = vec![Box::new(E1::V6)];
            for o39 in o38.iter() { o39.m6(11u8).mix(); }
            if let [.., E2::V0] = (vec![a24; 3].as_slice()) {
                let mut o42: Vec<Box<dyn T4>> = vec![Box::new(sel::<E1>('9'.is_whitespace(), E1::V4, (&216u8 as &dyn T4).m6(100u8)))];
                for o43 in o42.iter() { o43.m5(E3::V0).mix(); }
            }
            let mut v44: E1 = (unsafe { let mut slot = MaybeUninit::<E1>::uninit(); slot.write(E1::V0); slot.assume_init() });
            f35(E2::V0, (255u8.trailing_ones() as u8), E3::V1).mix();
        }
        v34.mix();
        'l46: for i45 in ((28i32..32i32).step_by(2)) {
            match ((-5655164588421041812isize).rotate_right(v34)) {
                p47 => {
                    let mut o48: Box<dyn T4> = Box::new(pick::<Option<E1>>(true, &None::<E1>, &Some(E1::V0)).clone().unwrap_or_else(|| -> E1 { a19.m6(8u8) }));
                }
                _ => {}
            }
            {
                let f51 = || -> Option<E3> { Some((unsafe { let mut slot = MaybeUninit::<E3>::uninit(); slot.write(a20); slot.assume_init() })) };
                let v53: Box<E3> = Box::new(E3::V0);
                if (({ let mut items = vec![false, false, false]; let end = items.len().min(1); items.drain(..end).collect::<Vec<bool>>() }).contains(&Some(104064877u32).is_some())) { continue 'l46; }
                f51().mix();
            }
        }
        std::iter::repeat((v25.leading_zeros() as u32)).take(2).fold((a21.leading_zeros() as i32), |acc55: i32, x56: u32| -> i32 { acc55.wrapping_neg() })
    }
    pub fn run() {
        let v65: usize = 8usize;
            let f70 = move |a68: i32, a69: E3| -> E1 { (if (Some(Ok::<E2, u64>(E2::V0)).is_some()) { E1::V3 } else { Ok::<E1, usize>(E1::V6).unwrap_or(E1::V1) }) };
            let mut v72: usize = v65.min(0usize);
            v72 ^= std::mem::size_of_val(&(if let Some(x73) = (Some(E3::V1)) { x73 } else { E3::V1 }));
            f70(f16(8u128, E2::V0, &255u8, E3::V0, 956752739i32, 12u128, E2::V0, E2::V1), E3::V1).mix();
        'l79: for i78 in (36u32..=39u32) {
            let mut v80: Result<u16, i128> = Ok::<u16, i128>(25187u16);
        }
        {
            let f84 = |a82: Box<E3>, a83: i128| -> u8 { 254u8 };
            'l86: for i85 in (15i16..=19i16) {
                if let [.., None] = (vec![Some(E3::V0), Some(E3::V1), None::<E3>, Some(E3::V0)].as_slice()) {
                }
            }
            let mut v90: Box<E3> = pick::<Box<E3>>(true, &({ let t91: usize = v65; Box::new(E3::V0) }), &sel::<Box<E3>>(true, Box::new(E3::V0), Box::new(E3::V1))).clone();
            f84((if (false) { v90.clone() } else { Box::new(E3::V0) }), (16i128.abs_diff((-1i128)) as i128)).mix();
        }
        let mut v92: [u128; 1] = (unsafe { *(&[29u128] as *const [u128; 1]) });
        (unsafe { std::slice::from_raw_parts(v92.as_ptr().add(0), 1) }).mix();
        ((match (Some(v65)) { None => String::from("x y").len(), Some(x96) => x96 }).count_ones() as usize).mix();
        match &mut v92 {
            _ => {}
        }
        Some([E3::V0; 5]).map_or([E3::V0, E3::V1, E3::V0, E3::V1, E3::V0], |x105: [E3; 5]| -> [E3; 5] { [E3::V1, E3::V0, E3::V0, E3::V1, E3::V1] }).mix();
        v92[0] &= v92[0];
        if let [p111 @ ..=268435455i32, p112 @ 2i32..=2i32] = (({ let mut items = vec![7i32, (-11i32), 2i32]; items.push(2147483646i32); items }).as_slice()) {
            let mut v113: E3 = (*Box::new(E3::V0));
        }
            let f116 = |a114: String, a115: E1| -> Box<E3> { (if ((false && true)) { (match (Some(Box::new(E3::V1))) { None => Box::new(E3::V0), Some(x117) => x117 }) } else { Some(Box::new(E3::V0)).map_or(Box::new(E3::V0), |x118: Box<E3>| -> Box<E3> { Box::new(E3::V1) }) }) };
            let v119: usize = (v65.checked_ilog2().unwrap_or(0) as usize);
            f116(262144u32.to_string(), E1::V1).mix();
    }
}
