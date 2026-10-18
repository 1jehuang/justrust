struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
    }
}
fn consume(first: D, second: i32) { println!("unreachable callee"); }
fn work() {
    let local = D { id: 2 };
    consume(D { id: 1 }, { println!("return from later argument"); return; });
}
fn main() { work(); println!("caller"); }
