trait T {
    fn a<X>(&self, x: X) -> i32;
}
fn f(_d: &dyn T) {}
fn main() {}
