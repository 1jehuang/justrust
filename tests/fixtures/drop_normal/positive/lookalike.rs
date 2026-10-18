mod custom {
    pub trait Drop { fn drop(&mut self); }
    pub struct D { pub id: i32 }
    impl Drop for D { fn drop(&mut self) { println!("explicit custom"); } }
}
fn main() {
    let mut value = custom::D { id: 1 };
    custom::Drop::drop(&mut value);
    println!("no automatic custom");
}
