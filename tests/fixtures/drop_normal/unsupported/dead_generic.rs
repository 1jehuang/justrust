struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("unreachable drop"); } }
fn consume<T>(value: T) { println!("unreachable generic body"); }
fn main() {
    if false { consume(D { id: 1 }); }
    println!("dead generic not executed");
}
