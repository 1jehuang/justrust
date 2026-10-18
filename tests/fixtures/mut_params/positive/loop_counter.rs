fn count(mut n: u32) -> u32 { let mut steps: u32 = 0; while n > 1 { if n % 2 == 0 { n = n / 2; } else { n = 3 * n + 1; } steps = steps + 1; } steps }
fn main() { if count(27) == 111 { println!("collatz 27 ok"); } }
