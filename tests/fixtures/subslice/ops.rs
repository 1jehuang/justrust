fn sum(v: &[i64]) -> i64 { let mut s = 0; let mut k = 0; while k < v.len() { s += v[k]; k += 1; } s }
fn mid(v: &[u8]) -> u8 { let h = v.len() / 2; v[h] }
struct Pair { a: [i64; 6] }
fn main() {
    let p = Pair { a: [10, 20, 30, 40, 50, 60] };
    let s: &[i64] = &p.a;
    let mut acc = 0i64;
    let mut lo = 0usize;
    while lo < s.len() {
        let hi = lo + 2;
        acc = acc * 7 + sum(&s[lo..hi]);
        lo += 2;
    }
    println!("{}", acc);
    let w = sum(&s[..]) + sum(&s[..0]) + sum(&s[6..]) + sum(&p.a[2..=4]);
    println!("{}", w);
    let bytes: [u8; 8] = [3, 1, 4, 1, 5, 9, 2, 6];
    let b: &[u8] = &bytes;
    let m = mid(&b[2..]);
    let first = b[..3][2];
    let tail = b[4..][1..].len();
    println!("{} {} {}", m, first, tail);
    let flags = [true, false, true];
    let f = &flags[1..];
    let ok = f[1] && !f[0];
    println!("{}", ok);
}
