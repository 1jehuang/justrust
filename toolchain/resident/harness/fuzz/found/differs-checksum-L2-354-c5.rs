// Found by fuzz.py: seed L2-354, case c5, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 82 builds. Again: fuzz.py check differs-checksum-L2-354-c5.rs
// rustc:
// c5 db4eb2deda2d41ec ok
// ra-jit:
// c5 b77f7548831b402d ok
mod c5 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub enum E1 {
        V0(&'static str, &'static str),
        V1,
        V2,
    }
    impl Mix for E1 {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub enum E2 {
        V0 { f0: char },
        V1 { f0: (Option<bool>, E1, char), f1: E1, f2: (Option<bool>, E1, char) },
    }
    impl Mix for E2 {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub enum E3 {
        V0(String),
        V1(bool, bool),
        V2(E1, bool),
    }
    impl Mix for E3 {
        fn mix(&self) {
            match self {
                E3::V0(a0) => { feed(10); a0.mix(); }
                E3::V1(a0, a1) => { feed(11); a0.mix(); a1.mix(); }
                E3::V2(a0, a1) => { feed(12); a0.mix(); a1.mix(); }
            }
        }
    }
    impl Drop for E3 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub enum E4 {
        V0,
        V1,
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    #[repr(u16)]
    pub enum E5 {
        V0 { f0: bool, f1: u8 },
        V1 { f0: String },
        V2(E3, E1, &'static str),
    }
    static K6: char = '😀';
    static K7: E4 = E4::V1;
    pub trait T8 {
        fn m9(&mut self, a0: ((bool, bool), bool, Box<bool>, (bool, &'static str, bool))) -> ((bool, bool), bool, Box<bool>, (bool, &'static str, bool));
        fn m10(&self, a0: (Option<bool>, E1, char)) -> u8;
    }
    impl T8 for E2 {
        fn m9(&mut self, a57: ((bool, bool), bool, Box<bool>, (bool, &'static str, bool))) -> ((bool, bool), bool, Box<bool>, (bool, &'static str, bool)) {
            id::<((bool, bool), bool, Box<bool>, (bool, &'static str, bool))>((match (vec![2u64, 1u64].as_slice()) { [] => ((true, false), false, Box::new(false), (true, "line\n", true)), [2u64, ..] if a57.1 => ((false, false), true, Box::new(true), (false, "ff", false)), _ => ((false, false), false, Box::new(true), (false, "x y", true)) }))
        }
        fn m10(&self, a67: (Option<bool>, E1, char)) -> u8 {
            127u8.clamp(14u8, 254u8)
        }
    }
    pub fn run() {
        let v147: Result<String, E3> = 'l146: loop {
                if ((unsafe { *(&matches!(36u128, ((2u128 | 26u128..=340282366920938463463374607431768211455u128))) as *const bool) })) {
                    let E5::V0 { f0: p180, f1: p181 } = (&{ ({ let t179: ((bool, bool), bool, Box<bool>, (bool, &'static str, bool)) = ((true, false), true, Box::new(false), (false, "line\n", true)); E5::V2(E3::V0(String::from("-7")), E1::V0("Abc Def", "x y"), "😀") }) }) else { break 'l146 Some(String::from("hello")).ok_or(E3::V1(true, true)); };
                }
        };
    }
}
