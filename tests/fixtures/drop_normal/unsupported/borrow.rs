struct D { id: i32 }
impl Drop for D { fn drop(&mut self) { println!("borrowed owner drop"); } }
fn inspect(value: &D) { println!("inspect"); }
fn main() { let owner = D { id: 1 }; inspect(&owner); println!("after borrow"); }
