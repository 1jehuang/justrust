fn f(c: char) -> u32 { match c { '\0'..='\u{D7FF}' => 1, '\u{E000}'..='\u{10FFFF}' => 2 } }
fn g(c: char) -> u32 { match c { ..='m' => 1, 'n'.. => 2 } }
fn main() { if f('a') == 1 && f('\u{E000}') == 2 && g('m') == 1 && g('n') == 2 { println!("ok"); } }
