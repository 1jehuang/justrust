struct D { v: i32 }
impl Drop for D { fn drop(&mut self) {} }
struct E { d: D }
impl Drop for E { fn drop(&mut self) {} }
fn main() { let e = E { d: D { v: 1 } }; let d = e.d; println!("{}", d.v); }
