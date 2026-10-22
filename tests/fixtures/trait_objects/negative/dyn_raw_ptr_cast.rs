trait S {
    fn a(&self) -> f64;
}
trait U {}
struct Q(f64);
impl S for Q {
    fn a(&self) -> f64 {
        self.0
    }
}
fn main() {
    let q = Q(1.0);
    let r: &dyn S = &q;
    let _p = r as *const dyn U;
}
