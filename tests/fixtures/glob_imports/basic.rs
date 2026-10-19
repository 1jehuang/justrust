mod shapes {
    pub struct P { pub x: i32 }
    pub fn mk(x: i32) -> P { P { x } }
    pub const K: i32 = 5;
    fn hidden() -> i32 { 1 }
    pub mod inner { pub fn deep() -> i32 { 9 } }
}
mod re { pub use super::shapes::*; }
#[derive(Clone, Copy)]
enum Dir { North, South }
use Dir::*;
use shapes::*;
fn main() {
    let p = mk(3);
    let d = South;
    let n = match d { North => 0, South => 1 };
    let q = re::mk(4);
    let total = p.x + K;
    let deep = inner::deep();
    println!("{} {} {} {} {}", total, n, deep, q.x, tests::t());
}
mod tests {
    use super::*;
    pub fn t() -> i32 { add() }
    fn add() -> i32 { K + mk(1).x }
}
