trait T {
    fn new() -> i32;
    fn b(&self) -> i32;
}
fn f(_d: &dyn T) {}
fn main() {}
