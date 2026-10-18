#[derive(Clone, Copy)]
struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop"); } }
fn main() {}
