struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
        if self.id == 3 { println!("drop 3"); }
    }
}
fn consume(value: D) { println!("consume"); }
fn branch(flag: bool) {
    let owner = D { id: 1 };
    if flag { consume(owner); }
    println!("after conditional move");
    let late: D;
    if flag { late = D { id: 2 }; }
    println!("after conditional init");
    let joined: D;
    if flag { joined = D { id: 3 }; } else { joined = D { id: 2 }; }
    println!("after joined init");
}
fn main() {
    println!("true");
    branch(true);
    println!("false");
    branch(false);
}
