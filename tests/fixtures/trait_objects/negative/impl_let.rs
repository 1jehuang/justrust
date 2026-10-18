trait T {
    fn a(&self) -> i32;
}
fn main() {
    let x: impl T = 3;
}
