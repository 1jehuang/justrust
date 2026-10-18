trait T {
    fn a(&self) -> i32;
}
trait U {
    fn u(&self, x: impl T) -> i32;
}
struct S;
impl T for S {
    fn a(&self) -> i32 {
        3
    }
}
impl U for S {
    fn u(&self, x: impl T) -> i32 {
        x.a()
    }
}
fn main() {
    let s = S;
    if s.u(S) == 3 {
        println!("ok");
    }
}
