fn code(x: i32) -> i32 { if x > 2 { std::process::exit(3) } x }
fn main() { println!("a"); let c = code(5); println!("unreached {}", c); }
