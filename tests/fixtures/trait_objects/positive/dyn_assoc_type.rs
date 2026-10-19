trait T {
    type O;
    fn b(&self) -> Self::O;
}
struct S;
impl T for S {
    type O = i32;
    fn b(&self) -> i32 {
        5
    }
}
fn f(d: &dyn T<O = i32>) -> i32 {
    d.b()
}
fn main() {
    if f(&S) == 5 {
        println!("ok");
    }
}
