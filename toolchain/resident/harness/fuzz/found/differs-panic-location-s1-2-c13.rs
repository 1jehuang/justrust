// Found by fuzz.py: seed s1-2, case c13, edition 2021, backend cranelift.
// differs: panic location
// Reduced in 100 builds. Again: fuzz.py check differs-panic-location-s1-2-c13.rs
// rustc:
// panic: panicked at src/main.rs:353:497:
// attempt to subtract with overflow
// c13 0000000000000000 ok
// ra-jit:
// panic: panicked at src/main.rs:353:495:
// attempt to subtract with overflow
// c13 0000000000000000 ok
mod c13 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub struct S1<A> { pub f0: char, pub f1: i128, pub f2: char, pub f3: [A; 2], pub f4: bool, pub f5: &'static str }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub struct S3<A> { pub f0: S1<i128>, pub f1: (A, u8), pub f2: u128 }
    pub trait T7 {
        fn m8(&self, a0: S1<i128>) -> char;
    }
    impl T7 for S3<S1<i128>> {
        fn m8(&self, a11: S1<i128>) -> char {
            char::from_u32(4294967295u32.overflowing_add(4294967295u32).0).unwrap_or('中')
        }
    }
    pub fn run() {
                let r166 = catch_unwind(AssertUnwindSafe(|| -> (i128, i128, f32) {
                    let mut v190: [bool; 3] = [true, true, false];
                    { let value = (Box::new(S3 { f0: S1 { f0: '_', f1: 4503599627370496i128, f2: 'ß', f3: [(-12i128), (-26003093545826141405044804600002399540i128)], f4: false, f5: "a" }, f1: (S1 { f0: ' ', f1: 88068076106362348288189351453377089341i128, f2: '中', f3: [7i128; 2], f4: false, f5: "line\n" }, 8u8), f2: 2097151u128 }) as Box<dyn T7>).m8(S1 { f0: '\u{7f}', f1: 9i128, f2: 'é', f3: [170141183460469231731687303715884105727i128, 12i128], f4: true, f5: "😀" }).is_control(); let at = ((3usize - 36usize) % 3); unsafe { v190.as_mut_ptr().add(at).write(value) }; }
                    pick::<(i128, i128, f32)>(v190[0], &Some(vec![324518553658426726783156020576256i128, 40564819207303340847894502572032i128]).into_iter().filter_map(|x193: Vec<i128>| -> Option<(Box<char>, f64, (f64,), Option<i128>, &'static str, bool)> { None::<(Box<char>, f64, (f64,), Option<i128>, &'static str, bool)> }).map(|x194: (Box<char>, f64, (f64,), Option<i128>, &'static str, bool)| -> (i128, i128, f32) { (10i128, 4722366482869645213697i128, (-0.0f32)) }).nth(1).unwrap(), &id::<(i128, i128, f32)>(Some(((-5i128), 1267650600228229401496703205375i128, 1e-40f32)).unwrap())).clone()
                }));
    }
}
