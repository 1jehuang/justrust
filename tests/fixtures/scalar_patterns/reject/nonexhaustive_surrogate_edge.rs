fn f(c: char) -> u32 { match c { '\0'..='\u{D7FF}' => 1, '\u{E000}'..='\u{10FFFE}' => 2 } }
fn main() {}
