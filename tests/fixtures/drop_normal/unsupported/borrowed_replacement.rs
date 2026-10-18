struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop old"); }
        if self.id == 2 { println!("drop replacement"); }
    }
}
fn replace(value: &mut D) { *value = D { id: 2 }; }
fn main() {
    let mut value = D { id: 1 };
    replace(&mut value);
    println!("after borrowed replacement");
}
