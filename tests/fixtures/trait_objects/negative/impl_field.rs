trait T {
    fn a(&self) -> i32;
}
struct W {
    f: impl T,
}
fn main() {}
