//! arrow's `Array` has `fn into_data(self) -> ArrayData` without `where Self: Sized`, which
//! is in the vtable: the slot takes a pointer to the value and the method takes the value.

use std::fmt::Debug;

trait Shape: Debug {
    fn area(&self) -> u64;
    fn into_sides(self) -> Vec<u64>;
    fn into_name(self) -> String;
}

#[derive(Debug)]
struct Square(u64);

#[derive(Debug)]
struct Triangle {
    name: String,
    sides: [u64; 3],
}

impl Shape for Square {
    fn area(&self) -> u64 {
        self.0 * self.0
    }
    fn into_sides(self) -> Vec<u64> {
        vec![self.0; 4]
    }
    fn into_name(self) -> String {
        format!("square of {}", self.0)
    }
}

impl Shape for Triangle {
    fn area(&self) -> u64 {
        self.sides.iter().sum()
    }
    fn into_sides(self) -> Vec<u64> {
        self.sides.to_vec()
    }
    fn into_name(self) -> String {
        self.name
    }
}

fn describe(shape: &dyn Shape) -> String {
    format!("{shape:?} {}", shape.area())
}

pub fn run() -> (String, String, Vec<u64>, String) {
    let triangle = Triangle { name: "three".to_owned(), sides: [3, 4, 5] };
    let shapes: [Box<dyn Shape>; 2] = [Box::new(Square(2)), Box::new(triangle)];
    let text = shapes.iter().map(|it| describe(&**it)).collect::<Vec<_>>().join(", ");
    (text, Square(3).into_name(), Square(1).into_sides(), describe(&Square(5)))
}
