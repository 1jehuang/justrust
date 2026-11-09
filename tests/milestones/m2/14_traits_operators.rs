use std::ops::{Index, IndexMut, Mul, Neg, AddAssign, Sub};
use std::cmp::Ordering;
#[derive(Debug, Clone, Copy, PartialEq)] struct Frac { n: i64, d: i64 }
fn gcd(a: i64, b: i64) -> i64 { if b == 0 { a.abs() } else { gcd(b, a % b) } }
impl Frac { fn new(n: i64, d: i64) -> Self { let g = gcd(n, d); let s = if d < 0 { -1 } else { 1 }; Frac { n: s * n / g, d: s * d / g } } }
impl Mul for Frac { type Output = Frac; fn mul(self, o: Frac) -> Frac { Frac::new(self.n * o.n, self.d * o.d) } }
impl Sub for Frac { type Output = Frac; fn sub(self, o: Frac) -> Frac { Frac::new(self.n * o.d - o.n * self.d, self.d * o.d) } }
impl Neg for Frac { type Output = Frac; fn neg(self) -> Frac { Frac::new(-self.n, self.d) } }
impl AddAssign for Frac { fn add_assign(&mut self, o: Frac) { *self = Frac::new(self.n * o.d + o.n * self.d, self.d * o.d); } }
impl PartialOrd for Frac { fn partial_cmp(&self, o: &Frac) -> Option<Ordering> { (self.n * o.d).partial_cmp(&(o.n * self.d)) } }
impl std::fmt::Display for Frac { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { if self.d == 1 { write!(f, "{}", self.n) } else { write!(f, "{}/{}", self.n, self.d) } } }
impl std::iter::Sum for Frac { fn sum<I: Iterator<Item = Frac>>(it: I) -> Frac { let mut acc = Frac::new(0, 1); for x in it { acc += x; } acc } }
struct Grid { w: usize, cells: Vec<char> }
impl Index<(usize, usize)> for Grid { type Output = char; fn index(&self, (r, c): (usize, usize)) -> &char { &self.cells[r * self.w + c] } }
impl IndexMut<(usize, usize)> for Grid { fn index_mut(&mut self, (r, c): (usize, usize)) -> &mut char { &mut self.cells[r * self.w + c] } }
fn main() {
    let h: Frac = (1..=6).map(|k| Frac::new(1, k)).sum();
    println!("H6 = {h}");
    let a = Frac::new(3, -9); println!("{a} {} {} {}", -a, a * Frac::new(6, 1), a - Frac::new(1, 6));
    let mut fs = vec![Frac::new(1, 2), Frac::new(1, 3), Frac::new(3, 4), Frac::new(-1, 5)];
    fs.sort_by(|x, y| x.partial_cmp(y).unwrap()); println!("{}", fs.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(" < "));
    let mut g = Grid { w: 3, cells: vec!['.'; 9] }; g[(1, 1)] = '#'; g[(2, 0)] = '@';
    for r in 0..3 { println!("{}", (0..3).map(|c| g[(r, c)]).collect::<String>()); }
}
