struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("inner drop"); } }
struct Outer { inner: D }
impl Drop for Outer { fn drop(&mut self) { println!("outer drop"); } }
fn main() { let value = Outer { inner: D { id: 1 } }; }
