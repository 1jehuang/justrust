trait T {
    fn b(&self) -> i32;
}
struct S;
fn unused() {
    let s = S;
    let _d: &dyn T = &s;
}
fn main() {}
