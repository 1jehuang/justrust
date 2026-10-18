struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop iflet owner"); } }
enum E { Some(D), None }
fn main() {
    let wrapped = E::Some(D { id: 1 });
    if let E::Some(owner) = wrapped { println!("iflet owned value"); }
    println!("after iflet");
}
