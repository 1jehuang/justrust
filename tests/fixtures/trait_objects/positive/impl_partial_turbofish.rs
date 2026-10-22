trait T {
    fn a(&self) -> i32;
}
struct S;
impl T for S {
    fn a(&self) -> i32 {
        1
    }
}
fn g<Q: Copy>(_q: Q, x: impl T) -> i32 {
    x.a()
}
fn main() {
    if g::<u8>(1, S) == 1 {
        println!("ok");
    }
}
