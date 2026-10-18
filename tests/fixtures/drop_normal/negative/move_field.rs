struct D { value: String }
impl Drop for D { fn drop(&mut self) { println!("drop"); } }
fn main() {
    let owner = D { value: String::from("owned") };
    let moved = owner.value;
}
