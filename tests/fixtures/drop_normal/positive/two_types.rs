struct A { id: i32 }
struct B { id: i32 }
impl Drop for A { fn drop(&mut self) { println!("drop A"); } }
impl Drop for B { fn drop(&mut self) { println!("drop B"); } }
fn main() {
    let first = A { id: 1 };
    let second = B { id: 2 };
    println!("done");
}
