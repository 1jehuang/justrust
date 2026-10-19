trait T {
    fn new() -> i32
    where
        Self: Sized;
    fn b(&self) -> i32;
}
struct S;
impl T for S {
    fn new() -> i32 {
        0
    }
    fn b(&self) -> i32 {
        2
    }
}
fn main() {
    let s = S;
    let d: &dyn T = &s;
    if d.b() + S::new() == 2 {
        println!("ok");
    }
}
