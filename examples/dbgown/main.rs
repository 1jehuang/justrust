#[path = "../../tests/support/native_prelude.rs"]
mod native_prelude;
use justrust_engine::compiler::frontend;
fn main() {
    let src = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let p = native_prelude::parse(&src).unwrap();
    let t = std::time::Instant::now();
    println!("{:?} {:?}", frontend::check(&p), t.elapsed());
}
