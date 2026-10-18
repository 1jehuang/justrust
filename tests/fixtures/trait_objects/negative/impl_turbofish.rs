trait T {
    fn a(&self) -> i32;
}
struct S;
impl T for S {
    fn a(&self) -> i32 {
        1
    }
}
fn g(x: impl T) -> i32 {
    x.a()
}
fn main() {
    g::<S>(S);
}
