trait Shape { fn area(&self) -> f64; fn name(&self) -> String { "shape".to_string() } }
struct Circle(f64);
struct Rect { w: f64, h: f64 }
impl Shape for Circle { fn area(&self) -> f64 { 3.0 * self.0 * self.0 } fn name(&self) -> String { "circle".into() } }
impl Shape for Rect { fn area(&self) -> f64 { self.w * self.h } }
fn largest<T: Shape>(items: &[T]) -> f64 {
    let mut m = 0.0;
    for i in items { if i.area() > m { m = i.area(); } }
    m
}
fn main() {
    let shapes: Vec<Box<dyn Shape>> = vec![Box::new(Circle(1.0)), Box::new(Rect { w: 2.0, h: 5.0 })];
    for s in &shapes { println!("{} {}", s.name(), s.area()); }
    println!("{}", largest(&[Rect { w: 1.0, h: 1.0 }, Rect { w: 3.0, h: 2.0 }]));
}
