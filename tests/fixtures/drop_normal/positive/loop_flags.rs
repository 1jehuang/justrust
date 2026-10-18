struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
        if self.id == 3 { println!("drop 3"); }
        if self.id == 4 { println!("drop root"); }
        if self.id == 5 { println!("unreachable drop"); }
    }
}
fn consume(value: D) { println!("consume iteration"); }
fn main() {
    let root = D { id: 4 };
    let mut index = 0;
    while index < 3 {
        index = index + 1;
        let value = D { id: index };
        if index == 1 { consume(value); continue; }
        println!("retained iteration");
    }
    println!("natural exit");
    while index < 0 {
        let never = D { id: 5 };
        println!("unreachable body");
    }
    println!("after zero iterations");
}
