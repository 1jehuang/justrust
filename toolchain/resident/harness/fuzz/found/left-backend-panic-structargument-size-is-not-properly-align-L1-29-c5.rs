// Found by fuzz.py: seed L1-29, case c5, edition 2021, backend cranelift.
// left: backend panic: StructArgument size is not properly aligned
// Reduced in 74 builds. Again: fuzz.py check left-backend-panic-structargument-size-is-not-properly-align-L1-29-c5.rs
// fz::c5::run: backend: backend panic: StructArgument size is not properly aligned
mod c5 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub struct S3 { pub f0: bool, pub f1: bool }
    #[inline(never)]
    pub extern "C" fn f90(a91: bool, a92: bool, a93: S3, a94: bool, a95: bool, a96: bool, a97: bool, a98: S3, a99: S3) -> S3 {
        (if (({ let t100: bool = false; true })) { S3 { f0: true, f1: a92 } } else { (if (true) { a99 } else { a93 }) })
    }
    pub fn run() {
        let (p178, _, _): (bool, i128, u128) = (true, 33i128, 21u128);
        let mut v184: (NonZeroI64, i128, i128, u128, u128, bool) = (match (&{ ({ let t185: String = String::from("a"); (true, 15i128, 17u128) }) }) { (false, ..) => (if (p178) { (NonZeroI64::new(4611686018427387903i64).unwrap(), 10i128, 604462909807314587353089i128, 8u128, 21u128, false) } else { (NonZeroI64::new(4686346778153399593i64).unwrap(), 2i128, 103025359494256051203914267284006098056i128, 3u128, 14u128, false) }), _ => (if (p178) { (NonZeroI64::new((-1i64)).unwrap(), 20i128, 3i128, 134336854308869813897673647718465894337u128, 208706105477999804555289268281890028811u128, true) } else { (NonZeroI64::new(2i64).unwrap(), 139590046565379558855584351136082143064i128, (-19i128), 144115188075855872u128, 39614081257132168796771975169u128, true) }) });
        match &mut v184 {
            (p186, ..) => {
                    let g189: extern "C" fn(bool, bool, S3, bool, bool, bool, bool, S3, S3) -> S3 = f90;
            }
        }
    }
}
