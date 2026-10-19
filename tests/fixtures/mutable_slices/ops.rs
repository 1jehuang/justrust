fn replace<T: Copy>(s: &mut [T], value: T) -> T {
    let old = s[0];
    s[0] = value;
    old
}
fn bump(s: &mut [i32]) {
    let mut i: usize = 0;
    while i < s.len() {
        s[i] += 2;
        i += 1;
    }
    s[0] = 7;
}
struct Token { x: i32 }
fn inspect<T>(s: &mut [T]) -> usize { s.len() }
fn main() {
    let mut a = [1i32, 2, 3];
    let r: &mut [i32] = &mut a;
    bump(r);
    bump(r);
    let old = replace(r, 11);
    let n = r.len();
    let first = r[0];
    let second = r[1];
    println!("{} {} {} {}", old, n, first, second);
    let mut b = [true, false];
    let prior = replace(&mut b, false);
    println!("{} {}", prior, b[0]);
    let mut wide = [170141183460469231731687303715884105720i128];
    let s: &mut [i128] = &mut wide;
    s[0] += 3;
    let v = s[0];
    println!("{}", v);
    let mut pair = [(1i32, 2i32)];
    let before = replace(&mut pair, (4, 5));
    println!("{} {} {}", before.0, pair[0].0, pair[0].1);
    let mut empty: [i32; 0] = [];
    println!("{}", inspect(&mut empty));
    let mut tokens = [Token { x: 8 }];
    println!("{}", inspect(&mut tokens));
    println!("{}", tokens[0].x);
}
