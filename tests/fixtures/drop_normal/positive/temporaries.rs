#![allow(unused_variables)]
struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
        if self.id == 3 { println!("drop 3"); }
    }
}
fn make(id: i32) -> D { D { id: id } }
fn main() {
    make(1);
    println!("after statement");
    let bound = make(2);
    println!("after binding");
    let _ = make(3);
    println!("after wildcard");
}
