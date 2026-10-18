trait Area {
    fn area(&self) -> i32;
}
trait Named {
    fn id(&self) -> i32;
}
struct Square {
    side: i32,
}
struct Rect {
    w: i32,
    h: i32,
}
impl Area for Square {
    fn area(&self) -> i32 {
        self.side * self.side
    }
}
impl Named for Square {
    fn id(&self) -> i32 {
        1
    }
}
impl Area for Rect {
    fn area(&self) -> i32 {
        self.w * self.h
    }
}
fn by_ref(shape: &impl Area) -> i32 {
    shape.area()
}
fn by_value(shape: impl Area + Named) -> i32 {
    shape.area() * 10 + shape.id()
}
fn two(a: impl Area, b: impl Area) -> i32 {
    a.area() + b.area()
}
fn mixed<T: Named>(named: &T, shape: &mut impl Area) -> i32 {
    named.id() + shape.area()
}
fn main() {
    let s = Square { side: 3 };
    if by_ref(&s) == 9 {
        println!("by ref");
    }
    if by_value(Square { side: 2 }) == 41 {
        println!("by value");
    }
    if two(Square { side: 1 }, Rect { w: 2, h: 5 }) == 11 {
        println!("two distinct");
    }
    let mut r = Rect { w: 3, h: 3 };
    if mixed(&s, &mut r) == 10 {
        println!("mixed");
    }
}
