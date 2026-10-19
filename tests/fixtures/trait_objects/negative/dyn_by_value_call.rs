trait T {
    fn a(self) -> i32;
}
struct S;
impl T for S {
    fn a(self) -> i32 {
        1
    }
}
fn main() {
    let s = S;
    let d: &dyn T = &s;
    d.a();
}
