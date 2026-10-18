trait T {
    fn a(&self) -> i32;
}
struct S;
fn g(x: impl T) -> i32 {
    x.a()
}
fn unused() {
    g(S);
}
fn main() {}
