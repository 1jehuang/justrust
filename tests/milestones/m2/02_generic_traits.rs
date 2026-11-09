use std::fmt::{self, Display};
use std::ops::Add;

trait Shape { fn area(&self) -> f64; fn name(&self) -> String { "shape".to_string() } }
#[derive(Debug, Clone, Copy, PartialEq)] struct V2 { x: f64, y: f64 }
impl Add for V2 { type Output = V2; fn add(self, o: V2) -> V2 { V2 { x: self.x + o.x, y: self.y + o.y } } }
impl Display for V2 { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "({:.1}, {:.1})", self.x, self.y) } }
struct Circle(f64); struct Rect { w: f64, h: f64 }
impl Shape for Circle { fn area(&self) -> f64 { 3.0 * self.0 * self.0 } fn name(&self) -> String { format!("circle r={}", self.0) } }
impl Shape for Rect { fn area(&self) -> f64 { self.w * self.h } }
fn total<S: Shape + ?Sized>(items: &[Box<S>]) -> f64 { items.iter().map(|s| s.area()).sum() }
fn largest<T: PartialOrd + Copy>(xs: &[T]) -> T { let mut m = xs[0]; for &x in xs { if x > m { m = x; } } m }
fn sum_all<T: Add<Output = T> + Default + Copy>(xs: &[T]) -> T { xs.iter().fold(T::default(), |a, &b| a + b) }
trait Describe { fn describe(&self) -> String; }
impl<T: Display> Describe for T { fn describe(&self) -> String { format!("<{}>", self) } }
struct Pair<T> { a: T, b: T }
impl<T: PartialOrd + Display> Pair<T> { fn bigger(&self) -> &T { if self.a > self.b { &self.a } else { &self.b } } }
fn main() {
    let shapes: Vec<Box<dyn Shape>> = vec![Box::new(Circle(2.0)), Box::new(Rect { w: 3.0, h: 4.5 })];
    for s in &shapes { println!("{} -> {:.2}", s.name(), s.area()); }
    println!("total {:.2}", total(&shapes));
    println!("{} {} {}", largest(&[3, 9, 2]), largest(&[1.5, -2.0]), largest(&['q', 'z', 'a']));
    println!("{} {}", sum_all(&[1u32, 2, 3]), sum_all(&[0.25f64, 0.5]));
    let v = V2 { x: 1.0, y: 2.0 } + V2 { x: 0.5, y: -1.0 };
    println!("{v} {:?} {}", v, v == V2 { x: 1.5, y: 1.0 });
    println!("{} {}", 42.describe(), "hi".describe());
    println!("{}", Pair { a: "pear".to_string(), b: "apple".to_string() }.bigger());
}
