#[derive(Clone, Copy)]
struct Big { a: [i64; 40], b: u8, c: (i32, i64) }
fn make(x: i64) -> Big { let mut b = Big { a: [0; 40], b: 7, c: (1, 2) }; b.a[39] = x; b }
fn take(b: Big, k: Big, z: i64) -> Big { let mut r = b; r.a[0] = k.a[39] + z; r }
fn many(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64, i: u8) -> i64 { a + b + c + d + e + f + g + h + i as i64 }
fn main() {
    let x = make(5);
    let y = take(x, make(9), 1);
    let p = y.a[0]; let q = y.a[39]; println!("{} {} {} {}", p, q, y.b, y.c.1);
    println!("{}", many(1, 2, 3, 4, 5, 6, 7, 8, 9));
    let v: [u8; 3] = [250, 3, 4];
    let i: usize = 4;
    let s: u8 = v[0] + v[1];
    println!("{}", s);
    let idx = [1, 2, 3];
    let w = idx[i - 2]; println!("{}", w);
}
