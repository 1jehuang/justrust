trait Shape {
    fn area(&self) -> f64;
    fn name(&self) -> String;
    fn describe(&self) -> String {
        format!("{} with area {:.2}", self.name(), self.area())
    }
}

struct Circle {
    r: f64,
}

struct Rect {
    w: f64,
    h: f64,
}

struct Triangle {
    a: f64,
    b: f64,
    c: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        3.141592653589793 * self.r * self.r
    }
    fn name(&self) -> String {
        String::from("circle")
    }
}

impl Shape for Rect {
    fn area(&self) -> f64 {
        self.w * self.h
    }
    fn name(&self) -> String {
        if self.w == self.h { "square".to_string() } else { "rectangle".to_string() }
    }
}

impl Shape for Triangle {
    fn area(&self) -> f64 {
        let s = (self.a + self.b + self.c) / 2.0;
        (s * (s - self.a) * (s - self.b) * (s - self.c)).sqrt()
    }
    fn name(&self) -> String {
        "triangle".into()
    }
}

fn total_area(shapes: &[Box<dyn Shape>]) -> f64 {
    shapes.iter().map(|s| s.area()).sum()
}

fn main() {
    let shapes: Vec<Box<dyn Shape>> = vec![
        Box::new(Circle { r: 1.0 }),
        Box::new(Rect { w: 3.0, h: 4.0 }),
        Box::new(Rect { w: 2.0, h: 2.0 }),
        Box::new(Triangle { a: 3.0, b: 4.0, c: 5.0 }),
    ];
    for s in &shapes {
        println!("{}", s.describe());
    }
    println!("total area: {:.3}", total_area(&shapes));
    let biggest = shapes
        .iter()
        .max_by(|a, b| a.area().partial_cmp(&b.area()).unwrap())
        .unwrap();
    println!("biggest: {}", biggest.name());
}
