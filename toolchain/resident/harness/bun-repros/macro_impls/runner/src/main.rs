// dtoa's `write` calls `dtoa!`, which calls `diyfp!`, which writes `impl DiyFp` in the body of
// `write`: a dependency's impl that no `impl` in the function's own text shows.
fn main() {
    let mut buffer = dtoa::Buffer::new();
    for value in [1.25f64, 0.1, 6.02214076e23, -2.5e-8] {
        println!("{}", buffer.format(value));
    }
    for value in [1.25f32, 0.1, 3.4e38] {
        println!("{}", buffer.format(value));
    }
}
