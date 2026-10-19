const A: usize = 18446744073709551615;
const B: usize = 3;
const C: usize = B * 4 + 1;
const D: usize = (C << 3) | 5;
const E: usize = A / 7 % 1000;
const F: usize = !0 - A;
const G: bool = B < C && D > 100;
const H: usize = A >> 63;
const I: usize = 1 << 63;
const J: usize = 17 ^ 5 & 12;
const UNUSED: usize = A - 1;
fn idx() -> usize { B }
fn main() {
    let (a, b, c, d, e, f, g, h, i, j) = (A, B, C, D, E, F, G, H, I, J);
    println!("{} {} {} {} {} {} {} {} {} {}", a, b, c, d, e, f, g, h, i, j);
    let arr = [10, 20, 30, 40];
    let k = arr[B];
    println!("{}", k);
    let s = idx() + C;
    println!("{}", s);
    if C > 12 { println!("big"); }
}
