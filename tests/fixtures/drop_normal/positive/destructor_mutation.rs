struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        println!("before mutation");
        self.id = self.id + 1;
        if self.id == 2 { println!("observed mutated field"); }
        if self.id == 1 { println!("incorrect unchanged field"); }
    }
}
fn main() { let owner = D { id: 1 }; println!("body"); }
