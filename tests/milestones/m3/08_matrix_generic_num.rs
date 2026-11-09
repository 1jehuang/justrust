use std::ops::{Add, Mul, Sub};
use std::fmt::Display;
trait Num: Copy + Add<Output = Self> + Mul<Output = Self> + Sub<Output = Self> + PartialEq + Display { fn zero() -> Self; fn one() -> Self; }
impl Num for i64 { fn zero() -> Self { 0 } fn one() -> Self { 1 } }
impl Num for f64 { fn zero() -> Self { 0.0 } fn one() -> Self { 1.0 } }
#[derive(Clone, PartialEq, Debug)] struct M<T> { n: usize, d: Vec<T> }
impl<T: Num> M<T> {
    fn ident(n: usize) -> Self { let mut d = vec![T::zero(); n * n]; for i in 0..n { d[i * n + i] = T::one(); } M { n, d } }
    fn from(n: usize, d: &[T]) -> Self { M { n, d: d.to_vec() } }
    fn mul(&self, o: &Self) -> Self { let n = self.n; let mut d = vec![T::zero(); n * n]; for i in 0..n { for j in 0..n { let mut s = T::zero(); for k in 0..n { s = s + self.d[i * n + k] * o.d[k * n + j]; } d[i * n + j] = s; } } M { n, d } }
    fn pow(&self, mut e: u32) -> Self { let mut r = Self::ident(self.n); let mut b = self.clone(); while e > 0 { if e & 1 == 1 { r = r.mul(&b); } b = b.mul(&b); e >>= 1; } r }
    fn show(&self) -> String { self.d.chunks(self.n).map(|r| r.iter().map(|x| format!("{x}")).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" / ") }
}
fn det(m: &M<f64>) -> f64 { let n = m.n; let mut a = m.d.clone(); let mut det = 1.0; for c in 0..n { let p = (c..n).max_by(|&x, &y| a[x * n + c].abs().partial_cmp(&a[y * n + c].abs()).unwrap()).unwrap(); if a[p * n + c] == 0.0 { return 0.0; } if p != c { for k in 0..n { a.swap(p * n + k, c * n + k); } det = -det; } det *= a[c * n + c]; for r in c + 1..n { let f = a[r * n + c] / a[c * n + c]; for k in c..n { a[r * n + k] -= f * a[c * n + k]; } } } det }
fn main() {
    let fib = M::from(2, &[1i64, 1, 1, 0]);
    for e in [1, 10, 40, 63] { println!("fib^{e} = {}", fib.pow(e).show()); }
    let m = M::from(3, &[2.0, -1.0, 0.0, -1.0, 2.0, -1.0, 0.0, -1.0, 2.0]);
    println!("{} det={:.3}", m.mul(&m).show(), det(&m));
    println!("{}", M::<f64>::ident(2) == M::from(2, &[1.0, 0.0, 0.0, 1.0]));
}
