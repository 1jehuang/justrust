struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop"); } }
fn branch(flag: bool) {
    let value = D { id: 1 };
    if flag { println!("early"); return; }
    println!("fallthrough");
}
fn main() { branch(true); println!("caller"); branch(false); println!("caller"); }
