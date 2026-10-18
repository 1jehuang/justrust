struct P { x: i32, y: i32 }
fn shift(mut p: P, d: i32) -> i32 { p.x = p.x + d; p.y = p.y - d; p.x * 10 + p.y }
fn main() { if shift(P { x: 1, y: 9 }, 2) == 37 { println!("struct ok"); } }
