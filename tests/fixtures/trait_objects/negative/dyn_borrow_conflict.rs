trait T {
    fn n(&self) -> i32;
}
struct S {
    v: i32,
}
impl T for S {
    fn n(&self) -> i32 {
        self.v
    }
}
fn main() {
    let mut s = S { v: 1 };
    let d: &dyn T = &s;
    s.v = 2;
    if d.n() == 1 {
        println!("bad");
    }
}
