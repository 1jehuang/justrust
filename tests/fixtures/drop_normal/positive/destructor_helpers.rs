struct D { id: i32 }
fn second(value: i32) {
    if value == 7 { println!("second helper observed field"); }
}
fn first(value: i32) { println!("first helper"); second(value); }
impl Drop for D {
    fn drop(&mut self) { println!("destructor"); first(self.id); }
}
fn main() { let value = D { id: 7 }; println!("body"); }
