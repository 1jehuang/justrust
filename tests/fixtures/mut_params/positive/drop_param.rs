struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop first original"); }
        if self.id == 2 { println!("drop second parameter"); }
        if self.id == 3 { println!("drop local"); }
        if self.id == 4 { println!("drop first replacement"); }
    }
}
fn consume(value: D) { println!("consume first"); }
fn run(mut first: D, second: D, flag: bool) {
    let local = D { id: 3 };
    if flag {
        consume(first);
        first = D { id: 4 };
        println!("early after reinit");
        return;
    }
    first = D { id: 4 };
    println!("fallthrough after replacement");
}
fn main() {
    run(D { id: 1 }, D { id: 2 }, true);
    println!("caller after true");
    run(D { id: 1 }, D { id: 2 }, false);
    println!("caller after false");
}
