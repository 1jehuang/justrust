struct D<'a> { value: &'a i32 }
impl<'a> Drop for D<'a> { fn drop(&mut self) { println!("reference field drop"); } }
fn main() { let scalar = 1; let owner = D { value: &scalar }; }
