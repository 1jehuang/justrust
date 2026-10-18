fn mark(label: i32) -> i32 { println!("eval {}", label); label * 10 }
fn main() {
    println!("result {} {} {}", mark(1), mark(2), mark(3));
}
