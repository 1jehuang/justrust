struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("generic function drop"); } }
fn consume<T>(value: T) { println!("consume generic"); }
fn main() { consume(D { id: 1 }); }
