trait Shape {
    fn area(&self) -> i64;
    fn corners(&self) -> i64;
    fn weight(&self) -> i64 {
        self.area() * 10 + self.corners()
    }
}
struct Square {
    side: i64,
}
struct Rect {
    w: i64,
    h: i64,
}
struct Tri {
    base: i64,
    height: i64,
}
impl Shape for Square {
    fn area(&self) -> i64 {
        self.side * self.side
    }
    fn corners(&self) -> i64 {
        4
    }
}
impl Shape for Rect {
    fn area(&self) -> i64 {
        self.w * self.h
    }
    fn corners(&self) -> i64 {
        4
    }
    fn weight(&self) -> i64 {
        1
    }
}
impl Shape for Tri {
    fn area(&self) -> i64 {
        self.base * self.height / 2
    }
    fn corners(&self) -> i64 {
        3
    }
}
fn total(shapes: [&dyn Shape; 3]) -> i64 {
    let mut sum: i64 = 0;
    let mut i: usize = 0;
    while i < 3 {
        sum += shapes[i].weight();
        i += 1;
    }
    sum
}
fn bigger<'a>(a: &'a dyn Shape, b: &'a dyn Shape) -> &'a dyn Shape {
    if a.area() >= b.area() { a } else { b }
}
fn main() {
    let s = Square { side: 3 };
    let r = Rect { w: 2, h: 5 };
    let t = Tri { base: 4, height: 3 };
    if total([&s, &r, &t]) == 156 {
        println!("default and overridden methods dispatch");
    }
    if bigger(&r, &s).corners() == 4 && bigger(&t, &r).area() == 10 {
        println!("returned trait objects keep their vtable");
    }
    let d: &dyn Shape = &t;
    if d.area() == 6 {
        println!("local trait object");
    }
}
