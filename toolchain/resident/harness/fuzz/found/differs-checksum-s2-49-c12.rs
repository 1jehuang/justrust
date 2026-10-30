// Found by fuzz.py: seed s2-49, case c12, edition 2021, backend llvm.
// differs: checksum
// Reduced in 33 builds. Again: fuzz.py check differs-checksum-s2-49-c12.rs
// rustc:
// c12 d86b36679df41a08 ok
// ra-jit:
// c12 aaa4e1f5eae232f5 ok
mod c12 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub struct S1 { pub f0: i8, pub f1: i8 }
    pub fn run() {
        let mut v113: [(&'static str, usize, i64, i128, NonZeroU8, i128); 1] = sel::<Option<[(&'static str, usize, i64, i128, NonZeroU8, i128); 1]>>(apply_mut(|x118: S1| -> bool { true }, S1 { f0: 1i8, f1: (-33i8) }).0, apply_dyn(&|x120: i8| -> Option<[(&'static str, usize, i64, i128, NonZeroU8, i128); 1]> { Some([("hello", 12403209985466040463usize, 9223372036854775806i64, (-1i128), NonZeroU8::new(9u8).unwrap(), 106966403619765695435011859208774780864i128)]) }, 28i8), larger::<Option<[(&'static str, usize, i64, i128, NonZeroU8, i128); 1]>>(None::<[(&'static str, usize, i64, i128, NonZeroU8, i128); 1]>, None::<[(&'static str, usize, i64, i128, NonZeroU8, i128); 1]>)).unwrap_or(larger::<[S1; 1]>([S1 { f0: 0i8, f1: 3i8 }; 1], [S1 { f0: 127i8, f1: (-127i8) }]).map(|x123: S1| -> (&'static str, usize, i64, i128, NonZeroU8, i128) { ("a,b,,c", 18446744073709551615usize, 4i64, 18i128, NonZeroU8::new(28u8).unwrap(), 9444732965739290427392i128) }));
        match (vec![30u128; (v113[0].1 % 6)].as_slice()) {
            _ => ({ let t151: NonZeroU32 = NonZeroU32::new(131072u32).unwrap(); [[167304980390784136285327471411427453196u128, 12u128, 10u128, 65u128, 6u128], [281241855308997861128742615014321843102u128, 204604689582036701950585706022252854825u128, 340282366920938463463374607431768211455u128, 3u128, 100395894362774715194236775208024426213u128], [323196202493769878493413901612955828971u128; 5], [1024u128; 5], [100487204380004297884298608607456989193u128; 5], [1u128; 5], [26u128, 37u128, 10u128, 100u128, 168959575097239963952331689938361224867u128], [5316911983139663491615228241121378305u128; 5]] }).mix(),
        }
    }
}
