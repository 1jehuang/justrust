trait S {
    fn a(&self) -> f64;
    fn m(&mut self);
}
struct Q(f64);
impl S for Q {
    fn a(&self) -> f64 {
        self.0
    }
    fn m(&mut self) {
        self.0 += 1.0;
    }
}
struct H {
    p: *const dyn S,
}
fn main() {
    let mut q = Q(2.5);
    let r: &mut dyn S = &mut q;
    let p = r as *mut dyn S;
    unsafe {
        let m: &mut dyn S = &mut *p;
        m.m();
    }
    let s: &dyn S = &q;
    let h = H { p: s as *const dyn S };
    let back: &dyn S = unsafe { &*h.p };
    println!("{}", back.a());
}
