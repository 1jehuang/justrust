trait T {
    fn a(&self) -> i32;
}
fn g(x: impl T) -> i32 {
    x.b()
}
fn main() {}
