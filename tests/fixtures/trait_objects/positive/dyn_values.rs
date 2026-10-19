struct Point {
    x: i32,
    y: i32,
}
trait Geo {
    fn shift(&self, p: Point, k: i64) -> (i64, i64);
    fn first(&self) -> &i32;
}
struct Offset {
    dx: i32,
    dy: i32,
}
impl Geo for Offset {
    fn shift(&self, p: Point, k: i64) -> (i64, i64) {
        ((p.x + self.dx) as i64 * k, (p.y + self.dy) as i64 * k)
    }
    fn first(&self) -> &i32 {
        &self.dx
    }
}
struct W<T> {
    value: T,
    zero: i32,
}
impl<T: Copy> Geo for W<T> {
    fn shift(&self, p: Point, k: i64) -> (i64, i64) {
        (p.x as i64 + k, p.y as i64 - k)
    }
    fn first(&self) -> &i32 {
        &self.zero
    }
}
fn run(g: &dyn Geo) -> i64 {
    let (a, b) = g.shift(Point { x: 1, y: 2 }, 3);
    a + b + *g.first() as i64
}
fn main() {
    let o = Offset { dx: 4, dy: 5 };
    let w = W { value: 9u8, zero: 0 };
    if run(&o) == 40 && run(&w) == 3 && w.value == 9 {
        println!("aggregate arguments and results");
    }
}
