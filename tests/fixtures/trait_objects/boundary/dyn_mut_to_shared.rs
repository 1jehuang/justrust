trait T {
    fn get(&self) -> i32;
}
struct S;
impl T for S {
    fn get(&self) -> i32 {
        4
    }
}
fn main() {
    let mut s = S;
    let d: &dyn T = &mut s;
    if d.get() == 4 {
        println!("ok");
    }
}
