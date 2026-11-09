mod geometry {
    pub mod shapes {
        #[derive(Debug)] pub struct Square { pub side: u32, id: u32 }
        impl Square { pub fn new(side: u32) -> Self { Square { side, id: super::next_id() } } pub fn id(&self) -> u32 { self.id } }
        pub(crate) fn perimeter(s: &Square) -> u32 { 4 * s.side }
    }
    fn next_id() -> u32 { 7 }
    pub use self::shapes::Square as Sq;
    pub const UNIT: u32 = 1;
    pub fn area(s: &Sq) -> u32 { s.side * s.side * UNIT }
}
mod util { pub fn twice<T: Clone>(x: T) -> (T, T) { (x.clone(), x) } pub static GREETING: &str = "hi"; }
use geometry::{area, shapes::{self, perimeter}, Sq};
use util::*;
fn main() {
    let s = Sq::new(5);
    println!("{:?} {} {} {} {}", s, s.id(), area(&s), perimeter(&s), shapes::Square::new(1).side);
    println!("{:?} {}", twice("ab"), GREETING);
    println!("{}", crate::geometry::UNIT + self::util::twice(1).0);
}
