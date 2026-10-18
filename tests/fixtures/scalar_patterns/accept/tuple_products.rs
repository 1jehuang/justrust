fn f(t: (u8, bool)) -> u32 { match t { (0..=127, true) => 1, (128..=255, _) => 2, (_, false) => 3 } }
fn main() { if f((1, true)) == 1 && f((200, true)) == 2 && f((1, false)) == 3 { println!("ok"); } }
