struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("tuple owner drop"); } }
fn main() { let pair = (D { id: 1 }, 2); }
