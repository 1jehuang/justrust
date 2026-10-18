struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop patterned owner"); } }
fn main() {
    let owner = D { id: 7 };
    let D { id } = owner;
    if id == 7 { println!("copied pattern field"); }
}
