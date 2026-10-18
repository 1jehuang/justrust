struct D<T> { value: T }
impl<T> Drop for D<T> { fn drop(&mut self) { println!("generic drop"); } }
fn main() { let value = D { value: 1 }; }
