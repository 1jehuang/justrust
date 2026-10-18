struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop"); } }
fn main() { let mut value = D { id: 1 }; Drop::drop(&mut value); }
