struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop old"); }
        if self.id == 2 { println!("drop second"); }
        if self.id == 3 { println!("drop replacement"); }
    }
}
fn main() {
    let mut values = [D { id: 1 }, D { id: 2 }];
    values[0] = D { id: 3 };
    println!("after indexed replacement");
}
