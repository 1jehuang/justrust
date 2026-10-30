// Found by fuzz.py: seed A1-17, case c23, edition 2021, backend llvm.
// differs: died
// Reduced in 45 builds. Again: fuzz.py check differs-died-A1-17-c23.rs
// ra-jit's program ended with status -11 in this case
// panic: panicked at src/main.rs:488:13:
// held 268566528
// qemu: uncaught target signal 11 (Segmentation fault) - core dumped
mod c23 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    pub enum E1 {
        V0 = -1073741824,
    }
    pub fn run() {
        let mut v99: Vec<Vec<String>> = ({ let mut items = (28i8..31i8).chain(({ let mut items = vec![7i8, 0i8]; items.insert(0, 4i8); items })).enumerate().enumerate().map(|x112: (usize, (usize, i8))| -> Vec<String> { Vec::<String>::new() }).collect::<Vec<Vec<String>>>(); items.insert(0, [NonZeroU32::new(2049u32).unwrap(); 4].into_iter().flat_map(|x104: NonZeroU32| -> Vec<char> { vec!['中'; 0] }).map(|x106: char| -> String { String::from("hello").chars().rev().collect::<String>() }).collect::<Vec<String>>()); items });
        let v145: Vec<Vec<String>> = ({ let mut items = v99.clone(); items.push(vec![String::from("0123456789"); 2]); items });
        let r152 = catch_unwind(AssertUnwindSafe(|| -> Vec<char> {
            v99 = (match ((match (String::from("a,b,,c").chars().map(|x157: char| -> Vec<Vec<String>> { v99.clone() }).map(|x158: Vec<Vec<String>>| -> NonZeroI64 { NonZeroI64::new(4i64).unwrap() }).collect::<Vec<NonZeroI64>>().as_slice()) { [p159 @ .., p160] => None::<Vec<Vec<String>>>, [p162, ..] => (match (vec![(-4i8), (-14i8), (-127i8)].as_slice()) { [2i8..32i8, ..] => Some(vec![vec![String::from("3.5"), String::from("a,b,,c"), String::from("0123456789")]]), _ => None::<Vec<Vec<String>>> }), _ => Some(Some(vec![vec![String::from("  pad  "), String::from("-7"), String::from("0123456789")], vec![String::from("a")], vec![String::from("héllo"), String::from("  pad  ")]])).unwrap_or(Some(vec![vec![String::from("x y")], Vec::<String>::new(), vec![String::from("line\n"), String::from("x y")], vec![String::from("line\n")], Vec::<String>::new()])) })) { None => v145.clone(), Some(x154) => x154 });
            let v168: bool = ({ let mut items = vec![Vec::<Vec<String>>::new(), vec![vec![String::from("a,b,,c"), String::from("😀"), String::from("ff")], vec![String::from("0123456789"), String::from("Abc Def"), String::from("ff"), String::from("0123456789"), String::from("😀")], Vec::<String>::new(), vec![String::from("-7"), String::from("x y"), String::from("42")], vec![String::from("x y"), String::from("  pad  ")]]]; items.pop() }).is_some();
            assert!(!v168, "held {}", (131073u32 + 268435455u32));
            vec![(), (), ()].into_iter().flat_map(|x174: ()| -> Vec<E1> { vec![E1::V0] }).flat_map(|x175: E1| -> Vec<()> { vec![(), (), (), (), ()] }).map(|x176: ()| -> () { x176 }).map(|x177: ()| -> char { 'a' }).collect::<Vec<char>>()
        }));
    }
}
