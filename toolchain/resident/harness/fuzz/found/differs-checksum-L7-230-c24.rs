// Found by fuzz.py: seed L7-230, case c24, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 116 builds. Again: fuzz.py check differs-checksum-L7-230-c24.rs
// rustc:
// c24 c8035a17518900c2 ok
// ra-jit:
// c24 627053df23d8bb53 ok
mod c24 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub enum E1 {
        V0(i64),
        V1(char),
    }
    impl Mix for E1 {
        fn mix(&self) {
            match self {
                E1::V0(a0) => { feed(10); a0.mix(); }
                E1::V1(a0) => { feed(11); a0.mix(); }
            }
        }
    }
    impl Drop for E1 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    static K2: i64 = 15i64.saturating_sub(3i64);
    const K4: i64 = 2147483649i64;
    pub trait T82 {
        fn m83(&self, a0: usize) -> u128;
        fn m84(&mut self, a0: i128) -> isize;
        fn m85(&mut self, a0: [E1; 3]) -> u16;
    }
    impl T82 for Box<i128> {
        fn m83(&self, a86: usize) -> u128 {
            let mut v87: NonZeroU8 = ({ let t88: [E1; 3] = [E1::V0(K4), [E1::V0(1i64), E1::V1('😀'), E1::V0((-18i64))][1].clone(), sel::<E1>(false, E1::V0(134217727i64), E1::V1('a'))]; sel::<NonZeroU8>((true || true), sel::<NonZeroU8>(true, NonZeroU8::new(15u8).unwrap(), NonZeroU8::new(40u8).unwrap()), (match (t88.clone()) { [.., E1::V1(p91)] => NonZeroU8::new(63u8).unwrap(), [E1::V1('a'..='z'), E1::V1('z'), E1::V1('\u{7f}')] => NonZeroU8::new(30u8).unwrap(), [E1::V1(p93 @ 'A'..='Z'), p92 @ .., E1::V1('Z')] if false => NonZeroU8::new(69u8).unwrap(), _ => NonZeroU8::new(12u8).unwrap() })) });
            u128::from_le_bytes(33u128.to_ne_bytes())
        }
        fn m84(&mut self, a96: i128) -> isize {
            (15i32.clamp((-1344805994i32), 8193i32).clamp((-2087905628i32), 2147483646i32) as isize)
        }
        fn m85(&mut self, a103: [E1; 3]) -> u16 {
            apply_dyn(&|x104: char| -> u16 { 100u16 }, (254u8.wrapping_sub(7u8) as char))
        }
    }
    impl T82 for NonZeroU32 {
        fn m83(&self, a116: usize) -> u128 {
            100u128
        }
        fn m84(&mut self, a117: i128) -> isize {
            35184372088831isize.checked_neg().unwrap_or_default()
        }
        fn m85(&mut self, a118: [E1; 3]) -> u16 {
            (K2.max(9223372036854775807i64) as u16)
        }
    }
    pub fn run() {
        let mut v285: [u32; 8] = ({ let mut items = [564111783u32, 1463287283u32, 3u32, 7u32, 262145u32, 3106341629u32, 32767u32, 25u32]; items.sort_by(|a, b| b.cmp(a)); items });
        let mut v291: [bool; 8] = (loop { break apply_mut(|x297: u16| -> [bool; 8] { [vec![524288u128, 40541837267145752664109997536876306238u128, 1u128], Vec::<u128>::new(), vec![140737488355327u128, 16u128], vec![14u128, 1u128], vec![0u128, 100u128, 85070591730234615865843651857942052863u128], Vec::<u128>::new(), vec![145758306165933227224278878352530422478u128, 25u128], vec![12u128]].map(|x299: Vec<u128>| -> bool { true }) }, (unsafe { std::mem::transmute::<i32, [i16; 2]>(10i32) }[1] as u16)).1; });
        let mut v340: [usize; 2] = (match (Box::new(58289444974691687923068130291081598001i128)) { p344 => [17771073601443117414usize; 2], _ => [10444193314208589486usize; 2] });
        let mut o364: Vec<Box<dyn T82>> = vec![Box::new(id::<NonZeroU32>(sel::<NonZeroU32>(v291[4], NonZeroU32::new(67108864u32).unwrap(), NonZeroU32::new(17u32).unwrap()))), Box::new((match ((loop { break [E1::V1('😀'), E1::V0((-301622861592560901i64)), E1::V0(10i64)]; })) { _ => (loop { break Box::new(19i128); }), _ => Box::new(77371252455336267181195264i128) })), Box::new(NonZeroU32::new(v285[1]).unwrap_or(NonZeroU32::new(17u32).unwrap()))];
        for o365 in o364.iter() { o365.m83(v340[0]).mix(); }
    }
}
