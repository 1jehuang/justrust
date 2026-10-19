trait T {
    fn n(&self) -> i32;
}
struct S {
    v: i32,
}
impl Drop for S {
    fn drop(&mut self) {
        println!("drop");
    }
}
impl T for S {
    fn n(&self) -> i32 {
        self.v
    }
}
fn f(d: &dyn T) -> i32 {
    d.n()
}
fn main() {
    let s = S { v: 2 };
    if f(&s) == 2 {
        println!("ok");
    }
}
