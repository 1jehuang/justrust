// Found by fuzz.py: seed L7-50, case c6, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 74 builds. Again: fuzz.py check differs-checksum-L7-50-c6.rs
// rustc:
// c6 a96ce1a646767345 ok
// ra-jit:
// c6 d3cba1a9ac767349 ok
mod c6 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub struct S1 { pub f0: u32, pub f1: u32, pub f2: u32 }
    impl Mix for S1 {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S4 { pub f0: Vec<S1> }
    impl Mix for S4 {
        fn mix(&self) {
            self.f0.mix();
        }
    }
    impl Drop for S4 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S5(pub Vec<String>, pub S4, pub char, pub Vec<String>);
    pub fn run() {
        let mut v201: Vec<S5> = ({ let mut items = ({ let mut items = Some(vec![S5(Vec::<String>::new(), S4 { f0: vec![S1 { f0: 524288u32, f1: 1u32, f2: 18u32 }, S1 { f0: 1286587465u32, f1: 10u32, f2: 18u32 }] }, '\n', vec![String::from("héllo"), String::from("42"), String::from("ff")]), S5(vec![String::from("Abc Def"), String::from("42"), String::from("Abc Def")], S4 { f0: vec![S1 { f0: 2u32, f1: 6u32, f2: 0u32 }, S1 { f0: 1554345100u32, f1: 4u32, f2: 8u32 }, S1 { f0: 7u32, f1: 8191u32, f2: 40u32 }] }, '0', vec![String::from("42"), String::from("42"), String::from("héllo"), String::from("Abc Def"), String::from("42")]), S5(vec![String::from("  pad  "), String::from("😀"), String::from("héllo"), String::from("a"), String::from("-7")], S4 { f0: vec![S1 { f0: 0u32, f1: 8u32, f2: 1135731469u32 }, S1 { f0: 40u32, f1: 1817905525u32, f2: 4294967295u32 }] }, '_', vec![String::from("ff"), String::from(""), String::from("x y"), String::from("😀"), String::from("héllo")]), S5(Vec::<String>::new(), S4 { f0: vec![S1 { f0: 2u32, f1: 2441152905u32, f2: 0u32 }] }, '\n', Vec::<String>::new()), S5(Vec::<String>::new(), S4 { f0: vec![S1 { f0: 4294967295u32, f1: 4294967295u32, f2: 28u32 }, S1 { f0: 2097152u32, f1: 405048963u32, f2: 16u32 }, S1 { f0: 18u32, f1: 10u32, f2: 32u32 }, S1 { f0: 2047u32, f1: 1570329692u32, f2: 4194305u32 }, S1 { f0: 1u32, f1: 2u32, f2: 2u32 }] }, '\u{e000}', vec![String::from("héllo"), String::from("-7")])]).unwrap_or(vec![S5(vec![String::from("ff")], S4 { f0: vec![S1 { f0: 536870912u32, f1: 28u32, f2: 1867725714u32 }, S1 { f0: 3875324968u32, f1: 2097152u32, f2: 16u32 }] }, '\u{e000}', vec![String::from("😀")]), S5(Vec::<String>::new(), S4 { f0: vec![S1 { f0: 21u32, f1: 0u32, f2: 2147483647u32 }, S1 { f0: 0u32, f1: 23u32, f2: 15u32 }] }, '😀', vec![String::from("0123456789"), String::from("Abc Def"), String::from("0123456789")])]); let at = items.len() / 2; items.split_off(at) }); items.truncate(0); items });
        v201 = ({ let mut items = vec![S5(vec![String::from("héllo"), String::from("a"), String::from("ff"), String::from("-7"), String::from("")], S4 { f0: vec![S1 { f0: 2147483647u32, f1: 1u32, f2: 10u32 }, S1 { f0: 32u32, f1: 33554431u32, f2: 2046624676u32 }, S1 { f0: 23u32, f1: 3u32, f2: 26u32 }, S1 { f0: 20u32, f1: 3u32, f2: 1048577u32 }, S1 { f0: 2u32, f1: 4275347724u32, f2: 2377178375u32 }] }, ' ', vec![String::from("x y"), String::from("ff")])]; let at = items.len() / 2; items.split_off(at) });
        if let p390 = (&{ sel::<S5>(true, S5(vec![String::from("x y"), String::from("-7"), String::from("line\n")], S4 { f0: Vec::<S1>::new() }, '0', vec![String::from("ff"), String::from(""), String::from("  pad  "), String::from("3.5"), String::from("  pad  ")]), S5(Vec::<String>::new(), S4 { f0: vec![S1 { f0: 9u32, f1: 23u32, f2: 1u32 }, S1 { f0: 1024u32, f1: 314477894u32, f2: 3964291925u32 }, S1 { f0: 2097152u32, f1: 4294967295u32, f2: 5u32 }] }, '😀', vec![String::from("😀"), String::from("-7")])) }) {
        }
    }
}
