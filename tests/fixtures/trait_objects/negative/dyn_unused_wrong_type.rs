trait T {
    fn n(&self) -> i32;
}
fn unused(d: &dyn T) -> bool {
    d.n()
}
fn main() {}
