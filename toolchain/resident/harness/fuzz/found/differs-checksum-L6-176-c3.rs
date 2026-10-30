// Found by fuzz.py: seed L6-176, case c3, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 63 builds. Again: fuzz.py check differs-checksum-L6-176-c3.rs
// rustc:
// c3 fc96257b1baeb227 ok
// ra-jit:
// c3 82ca529171d32e39 ok
mod c3 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S2 { pub f0: i128, pub f1: i128 }
    impl Mix for S2 {
        fn mix(&self) {
            self.f0.mix();
        }
    }
    impl Drop for S2 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    pub fn run() {
        let mut n154 = 0u32;
        let v156: String = 'l155: loop {
            n154 += 1;
            if n154 > 2 { break format!("{:x}", 1u32); }
        };
        let v196: ((u16, bool, u64), S2) = ((8688u16, true, 3u64), S2 { f0: (-16355079669179083692155783043807496297i128), f1: 147573952589676412929i128 });
        'l295: for x294 in ((match (&{ Err::<S2, S2>(S2 { f0: 42535295865117307932921825928971026433i128, f1: (-100837217986424658526669987620413678406i128) }) }) { Ok(S2 { f0: p287, f1: -19i128 }) => Vec::<String>::new(), _ => vec![String::from("hello"), String::from("42")], _ => Vec::<String>::new() }).into_iter().chain(({ let mut items = vec![String::from("-7"), String::from("a"), String::from("ff"), String::from("héllo"), String::from("x y")]; items.push(v156.clone()); items })).filter(|x290: &String| -> bool { true }).chain((|x293: bool| -> Vec<String> { Vec::<String>::new() })(v196.0.1)).collect::<Vec<_>>()) {
        }
    }
}
