struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop matched owner"); } }
enum E { Some(D), None }
fn main() {
    let wrapped = E::Some(D { id: 1 });
    match wrapped {
        E::Some(owner) => { println!("matched owned value"); }
        E::None => { println!("unreachable empty"); }
    }
    println!("after match");
}
