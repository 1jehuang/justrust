struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
    }
}
fn rhs() -> D { println!("rhs"); D { id: 2 } }
fn main() {
    let mut value = D { id: 1 };
    value = rhs();
    println!("after replace");
    value = value;
    println!("after self move");
}
