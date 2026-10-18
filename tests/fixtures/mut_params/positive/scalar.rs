fn bump(mut x: i32, y: i32) -> i32 { x = x + y; x = x * 2; x }
fn main() { if bump(3, 4) == 14 { println!("scalar ok"); } }
