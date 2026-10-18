struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop statement"); }
        if self.id == 2 { println!("drop binding"); }
        if self.id == 3 { println!("drop wildcard"); }
    }
}
fn main() {
    D { id: 1 };
    println!("after direct statement");
    let bound = D { id: 2 };
    println!("after direct binding");
    let _ = D { id: 3 };
    println!("after direct wildcard");
    let _ = bound;
    println!("after ignored existing place");
}
