fn f(n: i32) -> u32 { match n { 0..=5 => 1, 3..=4 => 2, _ => 0 } }
fn g(c: char) -> u32 { match c { 'a' => 1, 'a' => 2, _ => 0 } }
fn main() { if f(3) == 1 && f(4) == 1 && f(6) == 0 && g('a') == 1 { println!("ok"); } }
