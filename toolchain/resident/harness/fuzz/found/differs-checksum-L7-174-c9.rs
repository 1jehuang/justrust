// Found by fuzz.py: seed L7-174, case c9, edition 2024, backend cranelift.
// differs: checksum
// Reduced in 90 builds. Again: fuzz.py check differs-checksum-L7-174-c9.rs
// rustc:
// c9 41583b466c12c8cb ok
// ra-jit:
// c9 319390f73f44c7f7 ok
mod c9 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub struct S1 { pub f0: f64, pub f1: f64, pub f2: Vec<usize>, pub f3: u8 }
    impl Drop for S1 {
        fn drop(&mut self) {
            feed(0x4cf6a004);
        }
    }
    const K2: u128 = 35u128.wrapping_neg();
    pub fn run() {
                    let mut v316: [i64; 8] = (match (std::array::from_fn::<S1, 3, _>(|i320: usize| -> S1 { (match ([S1 { f0: f64::EPSILON, f1: 1e-40f64, f2: vec![12511242024977033466usize, 7usize, 35usize], f3: 4u8 }, S1 { f0: (-9223372036854775808.0f64), f1: 7.0f64, f2: vec![564303689257515916usize, 10usize, 16089418490195155294usize, 657929114774077188usize, 22usize], f3: 10u8 }, S1 { f0: 16777217.0f64, f1: 1.0f64, f2: vec![2usize, 31usize, 1usize, 64usize, 1531513255991181273usize], f3: 100u8 }]) { [S1 { f1: p323, f3: p324, .. }, p322 @ .., S1 { f1: _, f3: p325 @ 3u8..=237u8, .. }] if false => p322[0].clone(), _ => S1 { f0: 7.0f64, f1: f64::MAX, f2: vec![2usize, 18446744073709551615usize], f3: 141u8 } }) })) { [S1 { f0: _, f1: _, f2: ref p326, f3: _ }, p327, S1 { f1: p328, f2: ref p329, f3: p330, .. }] => [std::iter::repeat((-170141183460469231731687303715884105727i128)).take(1).step_by(3).step_by(1).fold(512i64, |acc335: i64, x336: i128| -> i64 { acc335 }); 8], _ => std::array::from_fn::<u128, 8, _>(|i339: usize| -> u128 { 19u128 }).map(|x338: u128| -> i64 { i64::try_from(11i32).unwrap_or(1i64) }), _ => (if (true) { [S1 { f0: 0.1f64, f1: 255.5f64, f2: vec![5usize], f3: 0u8 }, S1 { f0: 0.1f64, f1: 4294967296.0f64, f2: Vec::<usize>::new(), f3: 254u8 }, S1 { f0: 65535.0f64, f1: (-0.0f64), f2: Vec::<usize>::new(), f3: 5u8 }, S1 { f0: 1.0f64, f1: 3.25f64, f2: Vec::<usize>::new(), f3: 32u8 }, S1 { f0: f64::EPSILON, f1: 0.1f64, f2: vec![928790387422698165usize, 9usize, 8usize], f3: 28u8 }, S1 { f0: 255.5f64, f1: f64::NEG_INFINITY, f2: vec![17usize, 68719476736usize, 7usize, 18446744073709551614usize, 37usize], f3: 1u8 }, S1 { f0: 0.1f64, f1: 4294967296.0f64, f2: vec![28usize, 7968259276522723060usize, 33554432usize, 2164752751190424112usize, 12usize], f3: 100u8 }, S1 { f0: (-1.0f64), f1: 542.6575f64, f2: vec![268435455usize, 39usize, 70368744177663usize], f3: 2u8 }] } else { [S1 { f0: f64::EPSILON, f1: 1e-40f64, f2: Vec::<usize>::new(), f3: 2u8 }, S1 { f0: 751.0581f64, f1: (-128.9f64), f2: vec![10usize], f3: 21u8 }, S1 { f0: 590.1667f64, f1: f64::MIN, f2: vec![14usize], f3: 29u8 }, S1 { f0: (-240.3979f64), f1: 1e-40f64, f2: vec![34usize, 27usize], f3: 12u8 }, S1 { f0: 16777217.0f64, f1: 0.1f64, f2: vec![10usize], f3: 1u8 }, S1 { f0: 7.0f64, f1: 1e-40f64, f2: Vec::<usize>::new(), f3: 33u8 }, S1 { f0: (-689.8672f64), f1: 718.6316f64, f2: vec![11usize], f3: 39u8 }, S1 { f0: 0.5f64, f1: 0.1f64, f2: vec![9223372036854775807usize], f3: 35u8 }] }).map(|x342: S1| -> i64 { (match (K2) { p345 @ ..=63u128 if false => 2199023255553i64, p346 => (-19i64), _ => 13i64 }) }) });
    }
}
