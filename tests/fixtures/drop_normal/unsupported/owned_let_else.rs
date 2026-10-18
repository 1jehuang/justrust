struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop letelse owner"); } }
enum E { Some(D), None }
fn main() {
    let wrapped = E::Some(D { id: 1 });
    let E::Some(owner) = wrapped else { return; };
    println!("letelse owned value");
}
