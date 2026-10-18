struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
    }
}
fn main() {
    let result = loop {
        let inner = D { id: 1 };
        let payload = D { id: 2 };
        println!("break");
        break payload;
    };
    println!("receiver owns");
}
