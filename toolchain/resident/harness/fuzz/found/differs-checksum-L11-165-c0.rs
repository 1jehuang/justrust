// Found by fuzz.py: seed L11-165, case c0, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 74 builds. Again: fuzz.py check differs-checksum-L11-165-c0.rs
// rustc:
// panic: panicked at src/main.rs:495:43:
// index out of bounds: the len is 3 but the index is 1046835886354876227
// c0 0a09209d08c0ca7a ok
// ra-jit:
// panic: panicked at src/main.rs:495:43:
// index out of bounds: the len is 3 but the index is 1046835886354876227
// c0 0c9fe4ea02dfcf04 ok
mod c0 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S3 { pub f0: isize, pub f1: char }
    impl Mix for S3 {
        fn mix(&self) {
            self.f0.mix();
        }
    }
    impl Drop for S3 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    static K4: isize = (6070378817093299182isize.count_ones() as isize);
    pub fn run() {
        let r173 = catch_unwind(AssertUnwindSafe(|| -> Result<Vec<String>, Option<String>> {
            'l176: for x175 in ((20u16..22u16).filter(|x174: &u16| -> bool { true })) {
                'l179: for x178 in ((14i32..19i32).zip(vec![S3 { f0: (-3835504955296611623isize), f1: '\u{80}' }, S3 { f0: 8isize, f1: ' ' }, S3 { f0: 2isize, f1: 'ß' }, S3 { f0: (-3627379272784362159isize), f1: '0' }, S3 { f0: (-9223372036854775807isize), f1: '\u{80}' }])) {
                    let mut v180: isize = [0isize.min(x178.1.f0), K4.to_be(), K4][17399908187354675389usize.overflowing_mul(18446744073709551615usize).0];
                }
            }
            id::<Result<Vec<String>, Option<String>>>(Err::<Vec<String>, Option<String>>(Some(String::from("3.5"))))
        }));
    }
}
