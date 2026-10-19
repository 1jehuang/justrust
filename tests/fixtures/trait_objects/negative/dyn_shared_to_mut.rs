trait T {
    fn n(&self) -> i32;
}
struct S;
impl T for S {
    fn n(&self) -> i32 {
        1
    }
}
fn main() {
    let s = S;
    let _d: &mut dyn T = &s;
}
