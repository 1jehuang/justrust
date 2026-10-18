fn f((mut a, b): (i32, i32)) -> i32 { a = a + b; a }
fn main() { if f((1, 2)) == 3 { println!("pattern ok"); } }
