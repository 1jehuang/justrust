struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("unreachable drop"); } }
fn main() {
    if false { let wrapped = (D { id: 1 }, 0); }
    println!("dead aggregate not executed");
}
