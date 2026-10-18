struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop"); } }
fn main() {
    let value = D { id: 1 };
    let _ = value;
    println!("after ignored place");
    let moved = value;
    println!("after move");
}
