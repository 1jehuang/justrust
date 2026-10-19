trait T: Clone {
    fn b(&self) -> i32;
}
fn f(_d: &dyn T) {}
fn main() {}
