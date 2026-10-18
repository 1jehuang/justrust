enum D { Value(i32) }
impl Drop for D { fn drop(&mut self) { println!("enum drop"); } }
fn main() { let value = D::Value(1); }
