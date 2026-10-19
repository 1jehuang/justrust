trait T {
    fn a(&self, other: Self) -> i32;
}
fn f(_d: &dyn T) {}
fn main() {}
