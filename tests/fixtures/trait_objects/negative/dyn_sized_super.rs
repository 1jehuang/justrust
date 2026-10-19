trait T: Sized {
    fn b(&self) -> i32;
}
fn f(_d: &dyn T) {}
fn main() {}
