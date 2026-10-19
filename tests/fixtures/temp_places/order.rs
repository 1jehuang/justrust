struct S { f: i32, g: [i32; 2] }
fn p(tag: i32, x: &mut i32) -> &mut i32 { println!("place {}", tag); x }
fn ps(tag: i32, s: &mut S) -> &mut S { println!("place {}", tag); s }
fn v(tag: i32, n: i32) -> i32 { println!("value {}", tag); n }
fn main() {
    let mut x = 1;
    *p(1, &mut x) = v(1, 5);
    *p(2, &mut x) += v(2, 2);
    let mut s = S { f: 1, g: [0, 0] };
    ps(3, &mut s).f = v(3, 9);
    ps(4, &mut s).f += v(4, 1);
    ps(5, &mut s).g[1] = v(5, 4);
    ps(6, &mut s).g[1] *= v(6, 3);
    (*ps(7, &mut s)).f -= v(7, 2);
    *p(8, &mut x) <<= v(8, 1);
    println!("{} {} {}", x, s.f, s.g[1]);
}
