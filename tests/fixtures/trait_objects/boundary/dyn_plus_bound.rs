trait T {
    fn b(&self) -> i32;
}
struct S;
impl T for S {
    fn b(&self) -> i32 {
        5
    }
}
fn f(d: &(dyn T + 'static)) -> i32 {
    d.b()
}
fn main() {
    if f(&S) == 5 {
        println!("ok");
    }
}
