fn i(tag: i32, n: usize) -> usize { println!("idx {}", tag); n }
fn total(v: &[i32]) -> i32 { let mut s = 0; let mut k = 0; while k < v.len() { s += v[k]; k += 1; } s }
fn main() {
    let a = [1i32, 2, 3, 4, 5];
    let s: &[i32] = &a;
    let t1 = total(&s[1..3]); println!("{}", t1);
    let t2 = total(&s[..2]); println!("{}", t2);
    let t3 = total(&s[3..]); println!("{}", t3);
    let t4 = total(&s[..]); println!("{}", t4);
    let t5 = total(&s[1..=3]); println!("{}", t5);
    let t6 = total(&s[..=0]); println!("{}", t6);
    let t7 = total(&a[i(1, 2)..i(2, 4)]); println!("{}", t7);
    let t8 = total(&s[2..2]); println!("{}", t8);
    let x = s[1..4][1]; println!("{}", x);
    let n = s[1..][0..2].len(); println!("{}", n);
    let e = &s[i(3, 4)..i(4, 2)];
    let m = e.len(); println!("{}", m);
}
