trait T {
    fn inc(&mut self);
}
fn f(d: &dyn T) {
    d.inc();
}
fn main() {}
