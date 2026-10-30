// Found by fuzz.py: seed L1-61, case c19, edition 2021, backend cranelift.
// left: backend panic: StructArgument size is not properly aligned
// Reduced in 99 builds. Again: fuzz.py check left-backend-panic-structargument-size-is-not-properly-align-L1-61-c19.rs
// fz::c19::run: unsupported: callee `catch_unwind`: callee `catch_unwind`: callee `do_call`: callee `call_once`: callee `{closure}`: backend: backend panic: StructArgument size is not properly aligned
mod c19 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub struct S3 { pub f0: bool, pub f1: bool, pub f2: bool, pub f3: bool, pub f4: bool, pub f5: bool }
    impl Mix for S3 {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub struct S4(pub S3, pub bool);
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub struct S5<A>(pub [A; 2], pub i128, pub i128);
    static K7: S4 = S4(S3 { f0: false, f1: true, f2: true, f3: true, f4: true, f5: false }, false);
    pub trait T8 {
        fn m9(&self, a0: S3) -> bool;
        fn m10(&mut self, a0: S5<u128>) -> i128;
        fn m11(&self, a12: S4) -> i128 {
            (u16::from(15u8) as i128)
        }
    }
    impl T8 for S3 {
        fn m9(&self, a14: S3) -> bool {
            (*self).f3
        }
        fn m10(&mut self, a22: S5<u128>) -> i128 {
            147573952589676412927i128.rotate_right(1358009805u32)
        }
    }
    #[inline]
    pub extern "C" fn f27(a28: i32, a29: S4, a30: bool, a31: i32, a32: bool, a33: bool, a34: S3, a35: S4, a36: bool) -> S3 {
        let mut v37: [i128; 4] = ({ let mut items = [(-100652181473303893292402259114821120847i128); 4]; items.swap(0, 3); items });
        { let value = 1i128; let raw = Box::into_raw(Box::new(v37)); unsafe { (*raw)[0] = value; Box::from_raw(raw).mix(); } }
        v37.mix();
        ({ let t41: (S5<u128>,) = ({ let t42: i128 = (-12i128); (S5([0u128; 2], 5070602400912917605986812821504i128, (-153294722865891405050967287392250622670i128)),) }); a29.0 })
    }
    pub fn run() {
        let mut v57: S4 = pick::<S4>(('b59: { if (K7.0.f5) { break 'b59 Some(S5([8u128, 1u128], 45481963761316439752643635643900583033i128, 309485009821345068724781056i128)); } Some(S5([1u128, 324518553658426726783156020576257u128], 14i128, (-16i128))) }).is_some(), &sel::<S4>((false || K7.0.f4), S4(S3 { f0: false, f1: true, f2: true, f3: false, f4: true, f5: true }, false), pick::<S4>(K7.0.f1, &K7, &K7).clone()), &apply(|x60: i32| -> S4 { K7 }, sel::<i32>(K7.0.f1, 8i32, 1310054472i32))).clone();
        let r61 = catch_unwind(AssertUnwindSafe(|| -> i128 {
            ({ let mut it = S3 { f0: true, f1: true, f2: false, f3: true, f4: false, f5: false }; let out = it.m10(S5([340282366920938463463374607431768211455u128, 20u128], 144115188075855872i128, (-19i128))); it.mix(); out }).saturating_mul((&f27(744381296i32, v57, true, (-1947996498i32), false, true, S3 { f0: true, f1: true, f2: false, f3: false, f4: true, f5: false }, S4(S3 { f0: true, f1: true, f2: false, f3: false, f4: true, f5: false }, true), false) as &dyn T8).m11(v57))
        }));
    }
}
