trait T {
    fn a(&self) -> Self;
}
fn f(_d: &dyn T) {}
fn main() {}
