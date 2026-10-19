struct S { f: i32, a: [i32; 2] }
fn p(x: &mut i32) -> &mut i32 { x }
fn ps(s: &mut S) -> &mut S { s }
fn flip(b: &mut bool) -> &mut bool { b }
fn main() {
    let mut x = 1;
    *p(&mut x) = x + 10;
    println!("{}", x);
    *p(&mut x) += x;
    println!("{}", x);
    *p(&mut x) = { x += 1; 9 };
    println!("{}", x);
    let mut y = 4;
    let r = &mut y;
    *p(r) *= 3;
    println!("{}", y);
    let mut s = S { f: 2, a: [5, 6] };
    ps(&mut s).a[0] -= s.f;
    ps(&mut s).f = s.a[1];
    println!("{} {} {}", s.f, s.a[0], s.a[1]);
    let mut b = false;
    *flip(&mut b) |= true;
    println!("{}", b);
}
