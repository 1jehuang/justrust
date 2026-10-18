struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop first declared"); }
        if self.id == 2 { println!("drop second declared"); }
    }
}
fn main() {
    let first: D;
    let second: D;
    second = D { id: 2 };
    println!("initialized second");
    first = D { id: 1 };
    println!("initialized first");
}
