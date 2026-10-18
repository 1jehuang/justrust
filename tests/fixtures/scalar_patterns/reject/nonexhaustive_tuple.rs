fn f(t: (u8, bool)) -> u32 { match t { (0..=127, true) => 1, (128..=255, _) => 2 } }
fn main() {}
