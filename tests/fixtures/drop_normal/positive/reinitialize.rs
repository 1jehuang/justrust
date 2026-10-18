struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
    }
}
fn consume(value: D) { println!("consume"); }
fn main() {
    let mut value = D { id: 1 };
    consume(value);
    println!("moved");
    value = D { id: 2 };
    println!("reinitialized");
}
