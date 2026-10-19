struct S { f: i32, g: [i32; 3] }
fn ps(tag: i32, s: &mut S) -> &mut S { println!("place {}", tag); s }
fn v(tag: i32, n: i32) -> i32 { println!("value {}", tag); n }
fn i(tag: i32, n: usize) -> usize { println!("index {}", tag); n }
fn main() {
    let mut s = S { f: 1, g: [0, 0, 0] };
    ps(2, &mut s).g[i(2, 2)] += v(2, 3);
    println!("{} {}", s.f, s.g[2]);
    ps(3, &mut s).g[i(3, 7)] += v(3, 1);
}
